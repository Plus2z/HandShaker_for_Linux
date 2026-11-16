use std::collections::{BTreeMap, HashSet};
use std::io::{ErrorKind, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::OnceLock;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use prost::Message;
use uuid::Uuid;

use crate::crypto::RsaIdentity;
use crate::device_profile::{
    format_brand_and_model, non_empty_owned, non_empty_trimmed, DeviceProfile, DeviceProfileStore,
};
use crate::discovery::{WifiDeviceCandidate, MDNS_SERVICE_TYPE};
use crate::transport::aoa::UsbDeviceCandidate;
use crate::framing::{encode_outbound_frame, DemuxEvent, FrameFlag, InboundDemuxer};
use crate::proto::{SspHandShakeRequest01, SspHandShakeResponse01, SspRequestType};
use crate::trust::IdentityStore;

const PROBE_PROTOCOL_VERSION: &str = "2.1.0";
const PROBE_APP_VERSION: &str = "2.5.0";
const PROBE_MIN_CLIENT_VERSION: &str = "1";

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct WifiProbeResult {
    pub device_uuid: Option<String>,
    pub device_name: Option<String>,
    pub apk_version_name: Option<String>,
    pub usb_serial: Option<String>,
    pub is_locked: bool,
}

fn probe_identity() -> Option<&'static (String, RsaIdentity)> {
    static PROBE_ID: OnceLock<Option<(String, RsaIdentity)>> = OnceLock::new();
    PROBE_ID
        .get_or_init(|| {
            RsaIdentity::generate()
                .ok()
                .map(|rsa| (Uuid::new_v4().to_string(), rsa))
        })
        .as_ref()
}

/// Strips `.local.` / trailing dots and drops generic or redundant mDNS hostnames.
pub fn clean_mdns_hostname(raw: &str, primary_ip: &str) -> Option<String> {
    let mut trimmed = raw.trim().trim_end_matches('.');
    if let Some(stripped) = trimmed
        .strip_suffix(".local")
        .or_else(|| trimmed.strip_suffix(".LOCAL"))
    {
        trimmed = stripped;
    } else if trimmed.to_ascii_lowercase().ends_with(".local") {
        let end = trimmed.len().saturating_sub(6);
        trimmed = &trimmed[..end];
    }
    let trimmed = trimmed.trim().trim_end_matches('.');
    if trimmed.is_empty()
        || trimmed.eq_ignore_ascii_case("localhost")
        || trimmed == primary_ip
        || trimmed.parse::<IpAddr>().is_ok()
        || trimmed.starts_with("handshaker_ssp_")
    {
        return None;
    }
    Some(trimmed.to_owned())
}

/// Produces a readable fallback label from a raw mDNS service full name when probe metadata is unavailable.
pub fn format_mdns_instance_fallback(service_name: &str) -> String {
    let suffix_with_dot = format!(".{}", MDNS_SERVICE_TYPE);
    let suffix_no_dot = suffix_with_dot.trim_end_matches('.');
    let mut base = service_name.trim().trim_end_matches('.');
    if let Some(prefix) = base
        .strip_suffix(suffix_with_dot.trim_end_matches('.'))
        .or_else(|| base.strip_suffix(suffix_no_dot))
    {
        base = prefix.trim_end_matches('.');
    } else if let Some((prefix, _)) = base.split_once("._handshaker_ssp.") {
        base = prefix.trim_end_matches('.');
    }

    if base == "handshaker_ssp_" {
        return "HandShaker Device".to_owned();
    }
    if let Some(rest) = base.strip_prefix("handshaker_ssp_") {
        let trimmed = rest.trim();
        if let Some(inner) = trimmed.strip_prefix('(').and_then(|s| s.strip_suffix(')')) {
            let num = inner.trim();
            if !num.is_empty() {
                return format!("HandShaker Device (#{num})");
            }
        }
        if !trimmed.is_empty() {
            return format!("HandShaker Device ({trimmed})");
        }
        return "HandShaker Device".to_owned();
    }
    if base.is_empty() {
        "HandShaker Device".to_owned()
    } else {
        base.to_owned()
    }
}

pub fn txt_lookup_ci<'a>(txt: &'a BTreeMap<String, String>, keys: &[&str]) -> Option<&'a str> {
    for want in keys {
        for (k, v) in txt {
            if k.eq_ignore_ascii_case(want) {
                if let Some(clean) = non_empty_trimmed(Some(v)) {
                    return Some(clean);
                }
            }
        }
    }
    None
}

/// Applies TXT property hints from mDNS before optional active probing or cache lookup.
pub fn apply_txt_hints(candidate: &mut WifiDeviceCandidate) {
    if candidate.device_name.is_none() {
        candidate.device_name = txt_lookup_ci(
            &candidate.txt_properties,
            &["device_name", "deviceName", "name", "fn"],
        )
        .map(str::to_owned);
    }
    if candidate.phone_model.is_none() {
        candidate.phone_model = txt_lookup_ci(
            &candidate.txt_properties,
            &["phone_model", "phoneModel", "model", "md"],
        )
        .map(str::to_owned);
    }
    if candidate.product_brand.is_none() {
        candidate.product_brand = txt_lookup_ci(
            &candidate.txt_properties,
            &["product_brand", "productBrand", "brand"],
        )
        .map(str::to_owned);
    }
    if candidate.product_manufacturer.is_none() {
        candidate.product_manufacturer = txt_lookup_ci(
            &candidate.txt_properties,
            &[
                "product_manufacturer",
                "productManufacturer",
                "manufacturer",
                "oem",
            ],
        )
        .map(str::to_owned);
    }
    if candidate.device_uuid.is_none() {
        candidate.device_uuid = txt_lookup_ci(
            &candidate.txt_properties,
            &["device_uuid", "deviceUuid", "uuid", "id"],
        )
        .map(str::to_owned);
    }
    if candidate.apk_version_name.is_none() {
        candidate.apk_version_name = txt_lookup_ci(
            &candidate.txt_properties,
            &["apk_version_name", "apkVersionName", "apk_version", "version"],
        )
        .map(str::to_owned);
    }
}

/// Computes the primary user-facing device title from enriched metadata.
pub fn compute_display_name(candidate: &WifiDeviceCandidate) -> String {
    if let Some(name) = non_empty_trimmed(candidate.device_name.as_deref()) {
        return name.to_owned();
    }
    if let Some(brand_model) = format_brand_and_model(
        candidate.product_brand.as_deref(),
        candidate.product_manufacturer.as_deref(),
        candidate.phone_model.as_deref(),
    ) {
        return brand_model;
    }
    if let Some(host) = non_empty_trimmed(candidate.hostname.as_deref()) {
        return host.to_owned();
    }
    format_mdns_instance_fallback(&candidate.service_name)
}

/// Sends ONLY `SSPHandShakeRequest01` (never `Handshake02`) to read device metadata silently.
pub fn probe_wifi_endpoint(
    addresses: &[String],
    host_fallback: &str,
    port: u16,
    timeout: Duration,
) -> Option<WifiProbeResult> {
    let (probe_uuid, rsa) = probe_identity()?;
    let (md5_bytes, enckey_bytes) = rsa.enckey_and_md5().ok()?;
    let deadline = Instant::now() + timeout;

    let mut addrs = Vec::new();
    for raw in addresses.iter().map(String::as_str).chain(std::iter::once(host_fallback)) {
        if let Ok(ip) = raw.parse::<IpAddr>() {
            let sa = SocketAddr::new(ip, port);
            if !addrs.contains(&sa) {
                addrs.push(sa);
            }
        }
    }
    if addrs.is_empty() {
        if let Ok(resolved) = (host_fallback, port).to_socket_addrs() {
            addrs.extend(resolved);
        }
    }
    if addrs.is_empty() {
        return None;
    }

    let mut stream = None;
    for addr in addrs {
        let rem = deadline.checked_duration_since(Instant::now())?;
        if let Ok(s) = TcpStream::connect_timeout(&addr, rem) {
            stream = Some(s);
            break;
        }
    }
    let mut stream = stream?;
    let _ = stream.set_nodelay(true);

    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let req = SspHandShakeRequest01 {
        r#type: Some(SspRequestType::HandshakeRequest01 as i32),
        host_uuid: Some(probe_uuid.clone()),
        host_name: Some("Linux-Discovery".to_owned()),
        host_timestamp: Some(now_ms),
        host_smart_sync_protocol_version: Some(PROBE_PROTOCOL_VERSION.to_owned()),
        host_app_version: Some(PROBE_APP_VERSION.to_owned()),
        host_min_client_version: Some(PROBE_MIN_CLIENT_VERSION.to_owned()),
        md5: Some(md5_bytes.to_vec()),
        enckey: Some(enckey_bytes),
        host_model: Some("Linux".to_owned()),
        heartbeat_timeout_second: Some(10),
    };
    let mut payload = Vec::new();
    req.encode(&mut payload).ok()?;
    let frame = encode_outbound_frame(1, FrameFlag::Handshake, &payload);

    let write_rem = deadline.checked_duration_since(Instant::now())?;
    let _ = stream.set_write_timeout(Some(write_rem));
    stream.write_all(&frame).ok()?;
    stream.flush().ok()?;

    let mut demux = InboundDemuxer::new();
    let mut events = Vec::new();
    let mut buf = [0u8; 4096];

    loop {
        let rem = deadline.checked_duration_since(Instant::now())?;
        let _ = stream.set_read_timeout(Some(rem));
        match stream.read(&mut buf) {
            Ok(0) => return None,
            Ok(n) => {
                demux.push_wire_bytes(&buf[..n], &mut events).ok()?;
                for ev in events.drain(..) {
                    if let DemuxEvent::ProtoMessage { payload, .. } = ev {
                        if payload.as_ref() == b"locked" {
                            return Some(WifiProbeResult {
                                is_locked: true,
                                ..Default::default()
                            });
                        }
                        if let Ok(hs1) = SspHandShakeResponse01::decode(payload) {
                            return Some(WifiProbeResult {
                                device_uuid: non_empty_owned(hs1.device_uuid),
                                device_name: non_empty_owned(hs1.device_name),
                                apk_version_name: non_empty_owned(hs1.apk_version_name),
                                usb_serial: non_empty_owned(hs1.usb_serial),
                                is_locked: false,
                            });
                        }
                        return None;
                    }
                }
            }
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(_) => return None,
        }
    }
}

/// Runs concurrent bounded `Handshake 01` probes and merges cached/trusted profiles.
pub fn enrich_wifi_candidates(
    candidates: &mut [WifiDeviceCandidate],
    probe_timeout: Option<Duration>,
    profiles: &DeviceProfileStore,
    trusted_uuids: &HashSet<String>,
) {
    for c in candidates.iter_mut() {
        apply_txt_hints(c);
    }

    if let Some(timeout) = probe_timeout {
        if !candidates.is_empty() {
            let results: Vec<Option<WifiProbeResult>> = std::thread::scope(|scope| {
                let handles: Vec<_> = candidates
                    .iter()
                    .map(|c| {
                        let addrs = c.addresses.clone();
                        let host = c.host.clone();
                        let port = c.port;
                        scope.spawn(move || probe_wifi_endpoint(&addrs, &host, port, timeout))
                    })
                    .collect();
                handles
                    .into_iter()
                    .map(|h| h.join().unwrap_or(None))
                    .collect()
            });

            for (c, res) in candidates.iter_mut().zip(results) {
                if let Some(r) = res {
                    if let Some(uuid) = r.device_uuid {
                        c.device_uuid = Some(uuid);
                    }
                    if let Some(name) = r.device_name {
                        c.device_name = Some(name);
                    }
                    if let Some(ver) = r.apk_version_name {
                        c.apk_version_name = Some(ver);
                    }
                    if let Some(serial) = r.usb_serial {
                        c.usb_serial = Some(serial);
                    }
                    c.is_locked = r.is_locked;
                }
            }
        }
    }

    for c in candidates.iter_mut() {
        let profile = c
            .device_uuid
            .as_deref()
            .and_then(|uuid| profiles.get(uuid))
            .or_else(|| {
                c.usb_serial
                    .as_deref()
                    .and_then(|serial| profiles.find_by_usb_serial(serial))
            });
        if let Some(p) = profile {
            apply_profile_to_candidate(c, p);
        }
        if let Some(uuid) = c.device_uuid.as_deref() {
            c.is_trusted = trusted_uuids.contains(uuid);
        }
        c.display_name = compute_display_name(c);
    }

    sort_wifi_candidates(candidates);
}

/// Applies USB manufacturer descriptor when a probed Wi-Fi device matches an attached USB serial.
pub fn enrich_wifi_from_usb(candidates: &mut [WifiDeviceCandidate], usb: &[UsbDeviceCandidate]) {
    for c in candidates.iter_mut() {
        let Some(serial) = non_empty_trimmed(c.usb_serial.as_deref()) else {
            continue;
        };
        if let Some(matched) = usb
            .iter()
            .find(|u| non_empty_trimmed(u.serial.as_deref()) == Some(serial))
        {
            if c.product_manufacturer.is_none() {
                c.product_manufacturer = non_empty_owned(matched.manufacturer.clone());
            }
            c.display_name = compute_display_name(c);
        }
    }
    sort_wifi_candidates(candidates);
}

pub fn load_trust_and_profiles() -> (HashSet<String>, DeviceProfileStore) {
    let trusted = IdentityStore::load_or_create(IdentityStore::default_path(), false)
        .map(|s| s.trusted_uuids())
        .unwrap_or_default();
    let profiles = DeviceProfileStore::load_default();
    (trusted, profiles)
}

fn apply_profile_to_candidate(c: &mut WifiDeviceCandidate, p: &DeviceProfile) {
    if c.device_name.is_none() {
        c.device_name = non_empty_owned(Some(p.device_name.clone()));
    }
    if c.phone_model.is_none() {
        c.phone_model = non_empty_owned(p.phone_model.clone());
    }
    if c.product_brand.is_none() {
        c.product_brand = non_empty_owned(p.product_brand.clone());
    }
    if c.product_manufacturer.is_none() {
        c.product_manufacturer = non_empty_owned(p.product_manufacturer.clone());
    }
    if c.apk_version_name.is_none() {
        c.apk_version_name = non_empty_owned(p.apk_version_name.clone());
    }
}

pub fn sort_wifi_candidates(candidates: &mut [WifiDeviceCandidate]) {
    candidates.sort_by(|a, b| {
        let a_identified = a.device_name.is_some() || a.phone_model.is_some();
        let b_identified = b.device_name.is_some() || b.phone_model.is_some();
        b.is_trusted
            .cmp(&a.is_trusted)
            .then_with(|| b_identified.cmp(&a_identified))
            .then_with(|| a.is_locked.cmp(&b.is_locked))
            .then_with(|| {
                a.display_name
                    .to_ascii_lowercase()
                    .cmp(&b.display_name.to_ascii_lowercase())
            })
            .then_with(|| a.host.cmp(&b.host))
            .then_with(|| a.port.cmp(&b.port))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use tempfile::tempdir;

    fn read_inbound_frame_flag_and_bytes(stream: &mut TcpStream) -> (u8, Vec<u8>) {
        let mut hdr = [0u8; 9];
        stream.read_exact(&mut hdr).expect("read outbound frame hdr");
        let flag = hdr[4];
        let len = u32::from_be_bytes([hdr[5], hdr[6], hdr[7], hdr[8]]) as usize;
        let mut body = vec![0u8; len];
        stream.read_exact(&mut body).expect("read outbound frame body");
        (flag, body)
    }

    fn write_phone_chunk_proto(stream: &mut TcpStream, sid: u32, payload: &[u8]) {
        let mut inner = Vec::with_capacity(8 + payload.len());
        inner.extend_from_slice(&(payload.len() as u64).to_be_bytes());
        inner.extend_from_slice(payload);

        let mut pkt = Vec::with_capacity(6 + inner.len());
        pkt.extend_from_slice(&sid.to_be_bytes());
        pkt.extend_from_slice(&(inner.len() as u16).to_be_bytes());
        pkt.extend_from_slice(&inner);
        stream.write_all(&pkt).expect("write phone chunk");
        stream.flush().expect("flush phone chunk");
    }

    #[test]
    fn cleans_mdns_hostnames_and_instance_names() {
        assert_eq!(
            format_mdns_instance_fallback("handshaker_ssp_._handshaker_ssp._tcp.local."),
            "HandShaker Device"
        );
        assert_eq!(
            format_mdns_instance_fallback("handshaker_ssp_ (3)._handshaker_ssp._tcp.local."),
            "HandShaker Device (#3)"
        );
        assert_eq!(
            format_mdns_instance_fallback("Pixel-8-Pro._handshaker_ssp._tcp.local."),
            "Pixel-8-Pro"
        );
        assert_eq!(clean_mdns_hostname("localhost.local.", "192.168.0.10"), None);
        assert_eq!(
            clean_mdns_hostname("192.168.0.10.local.", "192.168.0.10"),
            None
        );
        assert_eq!(
            clean_mdns_hostname("oneplus-7-pro.local.", "192.168.0.10"),
            Some("oneplus-7-pro".to_owned())
        );
    }

    #[test]
    fn probes_multiple_mock_phones_without_handshake_02() {
        let listener_oneplus = TcpListener::bind("127.0.0.1:0").unwrap();
        let port_oneplus = listener_oneplus.local_addr().unwrap().port();
        let h1 = std::thread::spawn(move || {
            let (mut s, _) = listener_oneplus.accept().unwrap();
            let (flag, body) = read_inbound_frame_flag_and_bytes(&mut s);
            assert_eq!(flag, FrameFlag::Handshake as u8);
            let req = SspHandShakeRequest01::decode(body.as_slice()).unwrap();
            assert_eq!(req.r#type, Some(SspRequestType::HandshakeRequest01 as i32));
            let resp = SspHandShakeResponse01 {
                r#type: Some(SspRequestType::HandshakeResponse01 as i32),
                device_uuid: Some("uuid-op".to_owned()),
                device_name: Some("OnePlus 7Pro".to_owned()),
                apk_version_name: Some("2.5.0".to_owned()),
                usb_serial: Some("7ba53e81".to_owned()),
                ..Default::default()
            };
            let mut out = Vec::new();
            resp.encode(&mut out).unwrap();
            write_phone_chunk_proto(&mut s, 1, &out);
            // Verify client closes cleanly without sending HandshakeRequest02.
            let mut extra = [0u8; 16];
            assert_eq!(s.read(&mut extra).unwrap(), 0);
        });

        let listener_vivo = TcpListener::bind("127.0.0.1:0").unwrap();
        let port_vivo = listener_vivo.local_addr().unwrap().port();
        let h2 = std::thread::spawn(move || {
            let (mut s, _) = listener_vivo.accept().unwrap();
            let (_flag, _body) = read_inbound_frame_flag_and_bytes(&mut s);
            let resp = SspHandShakeResponse01 {
                r#type: Some(SspRequestType::HandshakeResponse01 as i32),
                device_uuid: Some("uuid-vivo".to_owned()),
                device_name: Some("vivo X200 Pro mini".to_owned()),
                apk_version_name: Some("1.3.1.28".to_owned()),
                ..Default::default()
            };
            let mut out = Vec::new();
            resp.encode(&mut out).unwrap();
            write_phone_chunk_proto(&mut s, 1, &out);
        });

        let listener_locked = TcpListener::bind("127.0.0.1:0").unwrap();
        let port_locked = listener_locked.local_addr().unwrap().port();
        let h3 = std::thread::spawn(move || {
            let (mut s, _) = listener_locked.accept().unwrap();
            let _ = read_inbound_frame_flag_and_bytes(&mut s);
            write_phone_chunk_proto(&mut s, 1, b"locked");
        });

        let dir = tempdir().unwrap();
        let mut profiles = DeviceProfileStore::load(dir.path().join("device_profiles.json"));
        profiles
            .upsert(DeviceProfile {
                device_uuid: "uuid-op".to_owned(),
                device_name: "OnePlus 7Pro".to_owned(),
                phone_model: Some("GM1917".to_owned()),
                product_brand: Some("OnePlus".to_owned()),
                product_manufacturer: Some("OnePlus".to_owned()),
                apk_version_name: Some("2.5.0".to_owned()),
                usb_serial: Some("7ba53e81".to_owned()),
                updated_at_unix: 100,
            })
            .unwrap();

        let mut trusted = HashSet::new();
        trusted.insert("uuid-op".to_owned());

        let mut candidates = vec![
            WifiDeviceCandidate {
                service_name: "handshaker_ssp_ (3)._handshaker_ssp._tcp.local.".to_owned(),
                host: "127.0.0.1".to_owned(),
                port: port_vivo,
                addresses: vec!["127.0.0.1".to_owned()],
                ..Default::default()
            },
            WifiDeviceCandidate {
                service_name: "handshaker_ssp_._handshaker_ssp._tcp.local.".to_owned(),
                host: "127.0.0.1".to_owned(),
                port: port_oneplus,
                addresses: vec!["127.0.0.1".to_owned()],
                ..Default::default()
            },
            WifiDeviceCandidate {
                service_name: "handshaker_ssp_ (2)._handshaker_ssp._tcp.local.".to_owned(),
                host: "127.0.0.1".to_owned(),
                port: port_locked,
                addresses: vec!["127.0.0.1".to_owned()],
                ..Default::default()
            },
        ];

        enrich_wifi_candidates(
            &mut candidates,
            Some(Duration::from_millis(800)),
            &profiles,
            &trusted,
        );

        h1.join().unwrap();
        h2.join().unwrap();
        h3.join().unwrap();

        // Trusted OnePlus sorts first and carries full cached brand/model metadata.
        assert_eq!(candidates[0].display_name, "OnePlus 7Pro");
        assert_eq!(candidates[0].phone_model.as_deref(), Some("GM1917"));
        assert_eq!(candidates[0].product_brand.as_deref(), Some("OnePlus"));
        assert!(candidates[0].is_trusted);

        // Unpaired vivo still receives live device_name from silent Handshake 01!
        assert_eq!(candidates[1].display_name, "vivo X200 Pro mini");
        assert_eq!(candidates[1].apk_version_name.as_deref(), Some("1.3.1.28"));
        assert!(!candidates[1].is_trusted);

        // Locked phone falls back cleanly without crashing or dropping.
        assert_eq!(candidates[2].display_name, "HandShaker Device (#2)");
        assert!(candidates[2].is_locked);
    }

    #[test]
    fn txt_and_usb_serial_hints_enrich_brand_and_model() {
        let mut txt = BTreeMap::new();
        txt.insert("model".to_owned(), "V2419A".to_owned());
        txt.insert("brand".to_owned(), "vivo".to_owned());
        let mut c = WifiDeviceCandidate {
            service_name: "handshaker_ssp_._handshaker_ssp._tcp.local.".to_owned(),
            host: "192.168.0.103".to_owned(),
            port: 46647,
            addresses: vec!["192.168.0.103".to_owned()],
            usb_serial: Some("SER123".to_owned()),
            txt_properties: txt,
            ..Default::default()
        };
        let empty_profiles = DeviceProfileStore::load("/nonexistent/device_profiles.json");
        enrich_wifi_candidates(std::slice::from_mut(&mut c), None, &empty_profiles, &HashSet::new());
        assert_eq!(c.display_name, "vivo V2419A");

        let mut c2 = WifiDeviceCandidate {
            service_name: "handshaker_ssp_._handshaker_ssp._tcp.local.".to_owned(),
            host: "192.168.0.101".to_owned(),
            port: 37831,
            addresses: vec!["192.168.0.101".to_owned()],
            usb_serial: Some("7ba53e81".to_owned()),
            device_name: Some("OnePlus 7Pro".to_owned()),
            ..Default::default()
        };
        let usb = vec![UsbDeviceCandidate {
            bus: 1,
            address: 82,
            vendor_id: 0x22d9,
            product_id: 0x2769,
            manufacturer: Some("OnePlus".to_owned()),
            product: Some("SM8150".to_owned()),
            serial: Some("7ba53e81".to_owned()),
            already_aoa: false,
            interface_count: 1,
        }];
        enrich_wifi_candidates(std::slice::from_mut(&mut c2), None, &empty_profiles, &HashSet::new());
        enrich_wifi_from_usb(std::slice::from_mut(&mut c2), &usb);
        assert_eq!(c2.product_manufacturer.as_deref(), Some("OnePlus"));
        assert_eq!(c2.display_name, "OnePlus 7Pro");
    }
}
