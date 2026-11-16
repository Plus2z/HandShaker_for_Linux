use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use handshaker_proto::device_profile::{format_brand_and_model, DeviceProfileStore};
use handshaker_proto::discovery::{discover_all, discover_wifi_mdns};
use handshaker_proto::transport::aoa::{
    list_usb_candidates, open_aoa_transport, reset_and_reopen_aoa_transport,
};
use handshaker_proto::transport::tcp::connect_tcp;
use handshaker_proto::{ConnectConfig, IdentityStore, ProtoError, Session, SessionEvent};

#[derive(Parser)]
#[command(name = "hs-cli", about = "HandShaker for Linux protocol CLI")]
struct Cli {
    /// Target device IP or hostname (if omitted, uses mDNS discovery unless --usb is set)
    #[arg(long, global = true)]
    host: Option<String>,

    /// Target device TCP port
    #[arg(long, global = true, default_value_t = 44289)]
    port: u16,

    /// Connect via USB Android Open Accessory (AOA)
    #[arg(long, global = true)]
    usb: bool,

    /// Filter USB device by BUS:ADDR (e.g. 001:006)
    #[arg(long, global = true)]
    usb_bus: Option<String>,

    /// Allow sending USB AOA control transfer 51/52/53 switch to Android phone
    #[arg(long, global = true)]
    allow_usb_switch: bool,

    /// Ignore existing trust_store.json and generate a fresh RSA-1024 identity
    #[arg(long, global = true)]
    fresh: bool,

    /// Custom identity store path (defaults to XDG data dir)
    #[arg(long, global = true)]
    identity: Option<PathBuf>,

    /// Trust confirmation timeout in seconds
    #[arg(long, global = true, default_value_t = 60)]
    trust_timeout: u64,

    /// Discovery timeout in seconds
    #[arg(long, global = true, default_value_t = 4)]
    discover_timeout: u64,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Discover Wi-Fi (mDNS) and USB devices
    Discover {
        #[arg(long)]
        json: bool,
    },
    /// Scan attached USB devices without switching modes
    UsbScan {
        #[arg(long)]
        json: bool,
    },
    /// Show connected phone info and storage capacity
    Info,
    /// List directory entries on phone
    Ls {
        path: Option<String>,
        #[arg(long, default_value_t = 1)]
        depth: u32,
    },
    /// Check if a remote path exists
    Exists { path: String },
    /// Create a remote directory
    Mkdir { path: String },
    /// Rename or move a remote file or directory
    Mv { src: String, dst: String },
    /// Delete or trash remote files or directories
    Rm {
        #[arg(long)]
        permanent: bool,
        #[arg(required = true)]
        paths: Vec<String>,
    },
    /// Download a file from phone to local filesystem
    Pull {
        remote_path: String,
        local_path: PathBuf,
    },
    /// Upload a local file to phone
    Push {
        local_path: PathBuf,
        remote_path: String,
    },
    /// List photo library metadata
    Photos {
        #[arg(long, default_value_t = 25)]
        limit: usize,
    },
    /// List video library metadata
    Videos {
        #[arg(long, default_value_t = 25)]
        limit: usize,
    },
    /// List audio library metadata
    Audio {
        #[arg(long, default_value_t = 25)]
        limit: usize,
    },
    /// Read, push, or clear phone clipboard entries
    Clipboard {
        #[command(subcommand)]
        action: ClipboardAction,
    },
}

#[derive(Subcommand)]
enum ClipboardAction {
    List,
    Push { text: String },
    Clear,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_target(false)
        .init();

    let cli = Cli::parse();

    match &cli.command {
        Commands::Discover { json } => {
            let snap = discover_all(Duration::from_secs(cli.discover_timeout));
            if *json {
                println!("{}", serde_json::to_string_pretty(&snap)?);
            } else {
                println!("Wi-Fi devices ({}):", snap.wifi.len());
                for w in &snap.wifi {
                    let vendor = format_brand_and_model(
                        w.product_brand.as_deref(),
                        w.product_manufacturer.as_deref(),
                        w.phone_model.as_deref(),
                    );
                    match vendor {
                        Some(v) => println!(
                            "  {} [{}] ({}:{}) [{}]",
                            w.display_name, v, w.host, w.port, w.service_name
                        ),
                        None => println!(
                            "  {} ({}:{}) [{}]",
                            w.display_name, w.host, w.port, w.service_name
                        ),
                    }
                }
                println!("USB candidates ({}):", snap.usb.len());
                for u in &snap.usb {
                    println!(
                        "  bus={} addr={} {:04x}:{:04x} aoa={} {} / {}",
                        u.bus,
                        u.address,
                        u.vendor_id,
                        u.product_id,
                        u.already_aoa,
                        u.manufacturer.as_deref().unwrap_or("-"),
                        u.product.as_deref().unwrap_or("-")
                    );
                }
            }
            return Ok(());
        }
        Commands::UsbScan { json } => {
            let list = list_usb_candidates()?;
            if *json {
                println!("{}", serde_json::to_string_pretty(&list)?);
            } else {
                for u in &list {
                    println!(
                        "bus={} addr={} {:04x}:{:04x} aoa={} serial={} {} {}",
                        u.bus,
                        u.address,
                        u.vendor_id,
                        u.product_id,
                        u.already_aoa,
                        u.serial.as_deref().unwrap_or("-"),
                        u.manufacturer.as_deref().unwrap_or("-"),
                        u.product.as_deref().unwrap_or("-")
                    );
                }
            }
            return Ok(());
        }
        _ => {}
    }

    let session = connect_session(&cli).await?;
    let mut ev_rx = session.subscribe();
    tokio::spawn(async move {
        while let Ok(ev) = ev_rx.recv().await {
            if let SessionEvent::TrustWaiting { device_name, .. } = ev {
                eprintln!("[trust] Waiting for confirmation on {device_name}...");
            }
        }
    });

    let res = run_session_command(&cli.command, &session).await;
    let _ = session.quit().await;
    res
}

async fn connect_session(cli: &Cli) -> Result<Session> {
    let store_path = cli
        .identity
        .clone()
        .unwrap_or_else(IdentityStore::default_path);
    let mut store = IdentityStore::load_or_create(&store_path, cli.fresh)
        .with_context(|| format!("Failed loading identity at {}", store_path.display()))?;

    let bus_filter = if cli.usb {
        cli.usb_bus.as_deref().map(parse_bus_addr).transpose()?
    } else {
        None
    };

    let link = if cli.usb {
        let allow_switch = cli.allow_usb_switch;
        tokio::task::spawn_blocking(move || {
            open_aoa_transport(bus_filter, allow_switch, Duration::from_secs(12))
        })
        .await
        .context("USB AOA open task panicked")??
    } else if let Some(host) = &cli.host {
        connect_tcp(host, cli.port, Duration::from_secs(8)).await?
    } else {
        let timeout = Duration::from_secs(cli.discover_timeout);
        let devices = tokio::task::spawn_blocking(move || discover_wifi_mdns(timeout, true))
            .await
            .context("mDNS discovery task panicked")??;
        let first = devices
            .into_iter()
            .next()
            .context("No HandShaker Wi-Fi device found via mDNS; pass --host or --usb")?;
        eprintln!(
            "[discover] Connecting to {} ({}:{})",
            first.service_name, first.host, first.port
        );
        connect_tcp(&first.host, first.port, Duration::from_secs(8)).await?
    };

    let cfg = ConnectConfig {
        trust_timeout: Duration::from_secs(cli.trust_timeout),
        ..ConnectConfig::default()
    };
    let session = match Session::connect_with_link(link, &mut store, cfg.clone()).await {
        Ok(session) => session,
        Err(
            ProtoError::Timeout("USB AOA handshake response") | ProtoError::Disconnected,
        ) if cli.usb && cli.allow_usb_switch =>
        {
            let retry_link = tokio::task::spawn_blocking(move || {
                reset_and_reopen_aoa_transport(bus_filter, Duration::from_secs(12))
            })
            .await
            .context("USB AOA reset task panicked")??;
            Session::connect_with_link(retry_link, &mut store, cfg).await?
        }
        Err(e) => return Err(e.into()),
    };
    let hs = session.handshake_info();
    eprintln!(
        "[connected] {} ({}) via {} apk={}",
        hs.device_name, hs.device_uuid, hs.transport_label, hs.apk_version_name
    );
    Ok(session)
}

async fn run_session_command(cmd: &Commands, session: &Session) -> Result<()> {
    match cmd {
        Commands::Info => {
            let info = session.get_device_info().await?;
            let _ = DeviceProfileStore::load_default()
                .save_device_info(&session.handshake_info().device_uuid, &info);
            let total = info.disk_size.unwrap_or(0);
            let used = info.used_disk_size.unwrap_or(0);
            let free = total.saturating_sub(used);
            println!("phone_name: {}", info.phone_name.as_deref().unwrap_or("-"));
            println!(
                "manufacturer: {}",
                info.product_manufacturer.as_deref().unwrap_or("-")
            );
            println!(
                "brand_model: {} {}",
                info.product_brand.as_deref().unwrap_or("-"),
                info.phone_model.as_deref().unwrap_or("-")
            );
            println!(
                "apk_version: {}",
                info.apk_version_name.as_deref().unwrap_or("-")
            );
            println!(
                "root_path: {}",
                info.root_path.as_deref().unwrap_or("/storage/emulated/0")
            );
            println!("storage: used={} free={} total={} bytes", used, free, total);
        }
        Commands::Ls { path, depth } => {
            let target = match path {
                Some(p) => p.clone(),
                None => session
                    .get_device_info()
                    .await?
                    .root_path
                    .unwrap_or_else(|| "/storage/emulated/0".to_owned()),
            };
            let resp = session.list_dir(&target, *depth).await?;
            for f in &resp.file {
                let kind = if f.is_directory.unwrap_or(false) { "dir " } else { "file" };
                let size = f.file_size.unwrap_or(0);
                let p = f.path.as_deref().unwrap_or("");
                println!("{kind} {size:>12}  {p}");
            }
            println!("total_entries={}", resp.file.len());
        }
        Commands::Exists { path } => {
            println!("exists={}", session.file_exists(path).await?);
        }
        Commands::Mkdir { path } => {
            session.create_folder(path).await?;
            println!("created {path}");
        }
        Commands::Mv { src, dst } => {
            session.rename_file(src, dst).await?;
            println!("moved {src} -> {dst}");
        }
        Commands::Rm { permanent, paths } => {
            session.delete_files(paths, !*permanent).await?;
            println!(
                "{} {} item(s)",
                if *permanent { "deleted" } else { "trashed" },
                paths.len()
            );
        }
        Commands::Pull {
            remote_path,
            local_path,
        } => {
            let mut last_pct = 200u64;
            let hdr = session
                .download_file(remote_path, local_path, |done, total| {
                    let pct = (done * 100).checked_div(total).unwrap_or(100);
                    if pct != last_pct && (pct % 20 == 0 || done == total) {
                        last_pct = pct;
                        eprintln!("[pull] {done}/{total} ({pct}%)");
                    }
                })
                .await?;
            println!(
                "downloaded {} -> {} ({} bytes)",
                remote_path,
                local_path.display(),
                hdr.range.and_then(|r| r.length).unwrap_or(0)
            );
        }
        Commands::Push {
            local_path,
            remote_path,
        } => {
            let mut last_pct = 200u64;
            session
                .upload_file(local_path, remote_path, |done, total| {
                    let pct = (done * 100).checked_div(total).unwrap_or(100);
                    if pct != last_pct && (pct % 20 == 0 || done == total) {
                        last_pct = pct;
                        eprintln!("[push] {done}/{total} ({pct}%)");
                    }
                })
                .await?;
            println!("uploaded {} -> {}", local_path.display(), remote_path);
        }
        Commands::Photos { limit } => {
            let resp = session.get_photo_library().await?;
            println!(
                "albums={} images={}",
                resp.album.len(),
                resp.image.len()
            );
            for img in resp.image.iter().take(*limit) {
                let path = img.path.as_deref().unwrap_or("-");
                println!(
                    "  id={:?} {}x{} {}",
                    img.media_id,
                    img.width.unwrap_or(0),
                    img.height.unwrap_or(0),
                    path
                );
            }
        }
        Commands::Videos { limit } => {
            let resp = session.get_video_library().await?;
            println!("videos={}", resp.video.len());
            for v in resp.video.iter().take(*limit) {
                let path = v.path.as_deref().unwrap_or("-");
                println!(
                    "  id={:?} dur={:.1}s {}",
                    v.media_id,
                    v.duration.unwrap_or(0.0),
                    path
                );
            }
        }
        Commands::Audio { limit } => {
            let resp = session.get_audio_library().await?;
            println!(
                "albums={} tracks={}",
                resp.album.len(),
                resp.audio.len()
            );
            for a in resp.audio.iter().take(*limit) {
                let path = a.path.as_deref().unwrap_or("-");
                println!(
                    "  title={:?} dur={:.1}s {}",
                    a.title,
                    a.duration.unwrap_or(0.0),
                    path
                );
            }
        }
        Commands::Clipboard { action } => match action {
            ClipboardAction::List => {
                let entries = session.get_clipboard().await?;
                println!("clipboard_entries={}", entries.len());
                for e in &entries {
                    println!("  [{}] {}", e.timestamp_ms, e.text);
                }
            }
            ClipboardAction::Push { text } => {
                let ok = session.post_clipboard(text).await?;
                println!("pushed succeed={ok}");
            }
            ClipboardAction::Clear => {
                let ok = session.clear_clipboard().await?;
                println!("cleared succeed={ok}");
            }
        },
        Commands::Discover { .. } | Commands::UsbScan { .. } => unreachable!(),
    }
    Ok(())
}

fn parse_bus_addr(raw: &str) -> Result<(u8, u8)> {
    let (b, a) = raw
        .split_once(':')
        .context("--usb-bus must be formatted as BUS:ADDR (e.g. 001:006)")?;
    Ok((b.trim().parse()?, a.trim().parse()?))
}
