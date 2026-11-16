use std::collections::HashMap;
use std::net::IpAddr;
use std::time::{Duration, Instant};

use mdns_sd::{ServiceDaemon, ServiceEvent};
use serde::{Deserialize, Serialize};

use crate::error::{ProtoError, Result};
use crate::transport::aoa::{list_usb_candidates, UsbDeviceCandidate};

pub const MDNS_SERVICE_TYPE: &str = "_handshaker_ssp._tcp.local.";

/// Discovered Wi-Fi HandShaker endpoint advertised by Android `NsdManager`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, Eq, PartialEq)]
pub struct WifiDeviceCandidate {
    pub service_name: String,
    pub host: String,
    pub port: u16,
    pub addresses: Vec<String>,
    #[serde(default)]
    pub hostname: Option<String>,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub device_uuid: Option<String>,
    #[serde(default)]
    pub device_name: Option<String>,
    #[serde(default)]
    pub phone_model: Option<String>,
    #[serde(default)]
    pub product_brand: Option<String>,
    #[serde(default)]
    pub product_manufacturer: Option<String>,
    #[serde(default)]
    pub apk_version_name: Option<String>,
    #[serde(default)]
    pub usb_serial: Option<String>,
    #[serde(default)]
    pub is_trusted: bool,
    #[serde(default)]
    pub is_locked: bool,
    #[serde(default)]
    pub txt_properties: std::collections::BTreeMap<String, String>,
}

/// Combined discovery snapshot across Wi-Fi mDNS and USB devices.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DiscoverySnapshot {
    pub wifi: Vec<WifiDeviceCandidate>,
    pub usb: Vec<UsbDeviceCandidate>,
}

/// Browses `_handshaker_ssp._tcp.local.` via mDNS for up to `timeout`, returning all resolved phones.
/// Stops early once at least one device is resolved and `settle_after_first` elapses (or immediately
/// if `stop_on_first` is true).
pub fn discover_wifi_mdns(timeout: Duration, stop_on_first: bool) -> Result<Vec<WifiDeviceCandidate>> {
    let daemon = ServiceDaemon::new()
        .map_err(|e| ProtoError::Discovery(format!("Failed starting mDNS daemon: {e}")))?;
    let rx = daemon
        .browse(MDNS_SERVICE_TYPE)
        .map_err(|e| ProtoError::Discovery(format!("Failed browsing {MDNS_SERVICE_TYPE}: {e}")))?;

    let deadline = Instant::now() + timeout;
    let mut found: HashMap<String, WifiDeviceCandidate> = HashMap::new();
    let mut first_seen: Option<Instant> = None;
    let settle_window = Duration::from_millis(600);

    loop {
        let now = Instant::now();
        if now >= deadline {
            break;
        }
        if let Some(t0) = first_seen {
            if stop_on_first || now.duration_since(t0) >= settle_window {
                break;
            }
        }

        let wait_step = deadline
            .saturating_duration_since(now)
            .min(Duration::from_millis(120));

        match rx.recv_timeout(wait_step) {
            Ok(ServiceEvent::ServiceResolved(info)) => {
                let mut addrs: Vec<IpAddr> = info.get_addresses().iter().copied().collect();
                addrs.sort_by_key(|a| if a.is_ipv4() { 0 } else { 1 });
                if let Some(primary) = addrs.first() {
                    let fullname = info.get_fullname().to_owned();
                    let primary_str = primary.to_string();
                    let hostname = crate::wifi_probe::clean_mdns_hostname(
                        info.get_hostname(),
                        &primary_str,
                    );
                    let mut txt_properties = std::collections::BTreeMap::new();
                    for prop in info.get_properties().iter() {
                        txt_properties
                            .insert(prop.key().to_owned(), prop.val_str().to_owned());
                    }
                    let mut candidate = WifiDeviceCandidate {
                        service_name: fullname.clone(),
                        host: primary_str,
                        port: info.get_port(),
                        addresses: addrs.into_iter().map(|a| a.to_string()).collect(),
                        hostname,
                        txt_properties,
                        ..Default::default()
                    };
                    candidate.display_name =
                        crate::wifi_probe::compute_display_name(&candidate);
                    found.insert(fullname, candidate);
                    if first_seen.is_none() {
                        first_seen = Some(Instant::now());
                    }
                }
            }
            Ok(ServiceEvent::ServiceRemoved(_, fullname)) => {
                found.remove(&fullname);
            }
            Ok(_) => {}
            Err(_) if rx.is_disconnected() => break,
            Err(_) => {}
        }
    }

    let _ = daemon.stop_browse(MDNS_SERVICE_TYPE);
    drop(rx);
    if let Ok(status_rx) = daemon.shutdown() {
        let _ = status_rx.recv_timeout(Duration::from_millis(300));
    }

    let mut list: Vec<_> = found.into_values().collect();
    let (trusted_uuids, profiles) = crate::wifi_probe::load_trust_and_profiles();
    let probe_timeout = if stop_on_first {
        None
    } else {
        Some(Duration::from_millis(600))
    };
    crate::wifi_probe::enrich_wifi_candidates(&mut list, probe_timeout, &profiles, &trusted_uuids);
    Ok(list)
}

/// Discovers both Wi-Fi mDNS phones and attached USB AOA / switchable Android phones.
pub fn discover_all(wifi_timeout: Duration) -> DiscoverySnapshot {
    let mut wifi = discover_wifi_mdns(wifi_timeout, false).unwrap_or_default();
    let usb = list_usb_candidates().unwrap_or_default();
    crate::wifi_probe::enrich_wifi_from_usb(&mut wifi, &usb);
    DiscoverySnapshot { wifi, usb }
}
