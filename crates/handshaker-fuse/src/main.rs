mod fs;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use clap::Parser;
use handshaker_proto::discovery::discover_wifi_mdns;
use handshaker_proto::transport::aoa::{open_aoa_transport, reset_and_reopen_aoa_transport};
use handshaker_proto::transport::tcp::connect_tcp;
use handshaker_proto::{ConnectConfig, IdentityStore, ProtoError, Session};
use tokio::runtime::Runtime;
use tracing::{info, warn};

use crate::fs::{HandShakerFs, HandShakerFsConfig, SessionBackend};

#[derive(Debug, Parser)]
#[command(
    name = "handshaker-mount",
    about = "Mount an Android phone running Smartisan HandShaker as a FUSE filesystem on Linux"
)]
struct Args {
    /// Local mountpoint directory
    mountpoint: PathBuf,

    /// Connect over USB AOA instead of Wi-Fi
    #[arg(long)]
    usb: bool,

    /// Bus filter when selecting a USB AOA device
    #[arg(long)]
    bus: Option<u8>,

    /// Device address filter when selecting a USB AOA device
    #[arg(long)]
    address: Option<u8>,

    /// Allow sending Android Open Accessory mode-switch requests (51/52/53)
    #[arg(long)]
    allow_usb_switch: bool,

    /// Target phone Wi-Fi IP/host (auto-discovers via mDNS if omitted)
    #[arg(long)]
    host: Option<String>,

    /// Target phone TCP port
    #[arg(long, default_value_t = 44289)]
    port: u16,

    /// Mount as read-only filesystem
    #[arg(long)]
    read_only: bool,

    /// Move deleted files to HandShaker phone trash instead of permanent delete
    #[arg(long)]
    trash: bool,

    /// Pass FUSE allow_other option (requires user_allow_other in /etc/fuse.conf)
    #[arg(long)]
    allow_other: bool,

    /// Client name shown on the phone trust prompt
    #[arg(long, default_value = "HandShaker Linux FUSE")]
    client_name: String,

    /// Custom identity store file path
    #[arg(long)]
    identity: Option<PathBuf>,
}

async fn connect_session(args: &Args) -> Result<Session> {
    let store_path = args
        .identity
        .clone()
        .unwrap_or_else(IdentityStore::default_path);
    let mut store = IdentityStore::load_or_create(&store_path, false)
        .with_context(|| format!("Failed loading identity at {}", store_path.display()))?;
    let cfg = ConnectConfig {
        host_name: args.client_name.clone(),
        ..ConnectConfig::default()
    };

    let bus_filter = args.bus.zip(args.address);
    let allow_switch = args.allow_usb_switch;

    if args.usb {
        let mut link = tokio::task::spawn_blocking(move || {
            open_aoa_transport(bus_filter, allow_switch, Duration::from_secs(12))
        })
        .await
        .context("USB AOA open task panicked")?
        .context("Failed opening USB AOA transport (check udev rules or --allow-usb-switch)")?;

        match Session::connect_with_link(link, &mut store, cfg.clone()).await {
            Ok(session) => Ok(session),
            Err(
                ref err @ (ProtoError::Timeout("USB AOA handshake response")
                | ProtoError::Disconnected),
            ) if allow_switch => {
                warn!(
                    "USB AOA initial handshake failed ({err}); resetting accessory handle and retrying"
                );
                link = tokio::task::spawn_blocking(move || {
                    reset_and_reopen_aoa_transport(bus_filter, Duration::from_secs(12))
                })
                .await
                .context("USB AOA reset task panicked")??;
                Ok(Session::connect_with_link(link, &mut store, cfg).await?)
            }
            Err(err) => Err(err.into()),
        }
    } else {
        let link = if let Some(host) = &args.host {
            connect_tcp(host, args.port, Duration::from_secs(8)).await?
        } else {
            let found =
                tokio::task::spawn_blocking(|| discover_wifi_mdns(Duration::from_secs(4), true))
                    .await
                    .context("mDNS discovery task panicked")??;
            let first = found
                .into_iter()
                .next()
                .context("No HandShaker phone discovered over mDNS; pass --host or --usb")?;
            let host = first
                .addresses
                .first()
                .cloned()
                .unwrap_or(first.host);
            connect_tcp(&host, first.port, Duration::from_secs(8)).await?
        };
        Ok(Session::connect_with_link(link, &mut store, cfg).await?)
    }
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();
    if !args.mountpoint.exists() {
        std::fs::create_dir_all(&args.mountpoint).with_context(|| {
            format!("Failed creating mountpoint {}", args.mountpoint.display())
        })?;
    }
    if !args.mountpoint.is_dir() {
        bail!("Mountpoint {} is not a directory", args.mountpoint.display());
    }

    let rt = Runtime::new()?;
    let session = rt.block_on(connect_session(&args))?;
    let info = rt.block_on(session.get_device_info())?;
    let root_remote_path = info
        .root_path
        .unwrap_or_else(|| "/storage/emulated/0".to_owned());

    info!(
        "Connected to {} ({}); mounting {} at {}",
        session.handshake_info().device_name,
        session.handshake_info().transport_label,
        root_remote_path,
        args.mountpoint.display()
    );

    let backend = Arc::new(SessionBackend::new(session.clone(), rt.handle().clone()));
    let fs_impl = HandShakerFs::new(
        backend,
        HandShakerFsConfig {
            root_remote_path,
            read_only: args.read_only,
            use_trash: args.trash,
        },
    )?;

    let mount_cfg = fs::build_mount_config(args.read_only, args.allow_other);
    let bg_session = fuser::spawn_mount(fs_impl, &args.mountpoint, &mount_cfg)
        .with_context(|| format!("Failed mounting FUSE filesystem at {}", args.mountpoint.display()))?;

    info!("Mounted! Press Ctrl+C (or `fusermount3 -u {}`) to unmount.", args.mountpoint.display());
    rt.block_on(async {
        let _ = tokio::signal::ctrl_c().await;
    });

    info!("Unmounting and closing HandShaker session...");
    drop(bg_session);
    let _ = rt.block_on(session.quit());
    Ok(())
}
