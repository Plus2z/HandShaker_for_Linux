use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use bytes::Bytes;
use rusb::{Context, Device, DeviceHandle, Direction, TransferType, UsbContext};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::error::{ProtoError, Result};
use crate::transport::{TransportKind, TransportLink};

pub const GOOGLE_AOA_VID: u16 = 0x18D1;
pub const AOA_PID_MIN: u16 = 0x2D00;
pub const AOA_PID_MAX: u16 = 0x2D05;

pub const AOA_REQ_GET_PROTOCOL: u8 = 51;
pub const AOA_REQ_SEND_STRING: u8 = 52;
pub const AOA_REQ_START: u8 = 53;

/// Android Open Accessory strings matching `res/xml/accessory_filter.xml` and `AoaSwitch.cs`.
pub const AOA_IDENTIFICATION_STRINGS: [&str; 6] = [
    "Smartisan",
    "HandShaker",
    "HandShaker",
    "1.0",
    "https://github.com/rianlu/handshaker-android-maintained/releases/latest",
    "",
];

/// Common Android OEM USB Vendor IDs that may expose AOA control transfer `51`.
const KNOWN_ANDROID_VIDS: &[u16] = &[
    0x18D1, // Google
    0x2D95, // vivo
    0x2717, // Xiaomi
    0x04E8, // Samsung
    0x22D9, // OPPO / OnePlus / Realme
    0x2A70, // OnePlus
    0x12D1, // Huawei
    0x29A9, // Smartisan
    0x19D2, // ZTE
    0x17EF, // Lenovo
    0x0BB4, // HTC
    0x0FCE, // Sony
    0x22B8, // Motorola
    0x0B05, // ASUS
    0x2B4C, // Nothing
];

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct UsbDeviceCandidate {
    pub bus: u8,
    pub address: u8,
    pub vendor_id: u16,
    pub product_id: u16,
    pub manufacturer: Option<String>,
    pub product: Option<String>,
    pub serial: Option<String>,
    pub already_aoa: bool,
    pub interface_count: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AoaEndpoints {
    pub interface_number: u8,
    pub bulk_in: u8,
    pub bulk_out: u8,
}

/// Returns `true` if the VID/PID already identifies an Android Open Accessory interface.
pub fn is_aoa_pid(vendor_id: u16, product_id: u16) -> bool {
    vendor_id == GOOGLE_AOA_VID && (AOA_PID_MIN..=AOA_PID_MAX).contains(&product_id)
}

/// Encodes an AOA `SEND_STRING` value as NUL-terminated UTF-8 bytes.
pub fn encode_aoa_string_payload(value: &str) -> Vec<u8> {
    let mut bytes = value.as_bytes().to_vec();
    bytes.push(0);
    bytes
}

/// Scans USB buses for devices in AOA mode (`18d1:2d00..2d05`) or known Android phones.
pub fn list_usb_candidates() -> Result<Vec<UsbDeviceCandidate>> {
    let ctx = Context::new()?;
    let devices = ctx.devices()?;
    let mut out = Vec::new();

    for device in devices.iter() {
        let Ok(desc) = device.device_descriptor() else {
            continue;
        };
        let vid = desc.vendor_id();
        let pid = desc.product_id();
        let already_aoa = is_aoa_pid(vid, pid);
        if !already_aoa && (!KNOWN_ANDROID_VIDS.contains(&vid) || find_aoa_endpoints(&device).is_err()) {
            continue;
        }

        let (manufacturer, product, serial) = read_string_descriptors(&device, &desc);
        let interface_count = device
            .active_config_descriptor()
            .or_else(|_| device.config_descriptor(0))
            .map(|c| c.num_interfaces())
            .unwrap_or(0);

        out.push(UsbDeviceCandidate {
            bus: device.bus_number(),
            address: device.address(),
            vendor_id: vid,
            product_id: pid,
            manufacturer,
            product,
            serial,
            already_aoa,
            interface_count,
        });
    }

    Ok(out)
}

fn read_string_descriptors(
    device: &Device<Context>,
    desc: &rusb::DeviceDescriptor,
) -> (Option<String>, Option<String>, Option<String>) {
    let Ok(handle) = device.open() else {
        return (None, None, None);
    };
    let timeout = Duration::from_millis(200);
    let Ok(langs) = handle.read_languages(timeout) else {
        return (None, None, None);
    };
    let Some(&lang) = langs.first() else {
        return (None, None, None);
    };
    let mfg = desc
        .manufacturer_string_index()
        .and_then(|_| handle.read_manufacturer_string(lang, desc, timeout).ok());
    let prod = desc
        .product_string_index()
        .and_then(|_| handle.read_product_string(lang, desc, timeout).ok());
    let ser = desc
        .serial_number_string_index()
        .and_then(|_| handle.read_serial_number_string(lang, desc, timeout).ok());
    (mfg, prod, ser)
}

/// Finds the AOA Bulk IN/OUT endpoint pair (preferring Interface 0, per Android AOA spec).
pub fn find_aoa_endpoints(device: &Device<Context>) -> Result<AoaEndpoints> {
    let config = device
        .active_config_descriptor()
        .or_else(|_| device.config_descriptor(0))?;

    let mut fallback = None;
    for iface in config.interfaces() {
        for desc in iface.descriptors() {
            let mut bulk_in = None;
            let mut bulk_out = None;
            for ep in desc.endpoint_descriptors() {
                if ep.transfer_type() != TransferType::Bulk {
                    continue;
                }
                match ep.direction() {
                    Direction::In if bulk_in.is_none() => bulk_in = Some(ep.address()),
                    Direction::Out if bulk_out.is_none() => bulk_out = Some(ep.address()),
                    _ => {}
                }
            }
            if let (Some(ep_in), Some(ep_out)) = (bulk_in, bulk_out) {
                let candidate = AoaEndpoints {
                    interface_number: desc.interface_number(),
                    bulk_in: ep_in,
                    bulk_out: ep_out,
                };
                if candidate.interface_number == 0 {
                    return Ok(candidate);
                }
                if fallback.is_none() {
                    fallback = Some(candidate);
                }
            }
        }
    }

    fallback.ok_or_else(|| ProtoError::Discovery("No Bulk IN/OUT endpoints found on AOA device".into()))
}

/// Sends AOA control transfers 51 -> 52(0..5) -> 53 to switch an Android USB device into accessory mode.
/// Calling this re-enumerates the device on USB (dropping active ADB until replug/reset).
pub fn switch_device_to_aoa(device: &Device<Context>) -> Result<u16> {
    let desc = device.device_descriptor()?;

    let handle = device.open()?;
    let timeout = Duration::from_secs(2);
    let mut proto_buf = [0u8; 2];
    handle.read_control(0xC0, AOA_REQ_GET_PROTOCOL, 0, 0, &mut proto_buf, timeout)?;
    let aoa_version = u16::from_le_bytes(proto_buf);
    if !(1..=2).contains(&aoa_version) {
        return Err(ProtoError::Discovery(format!(
            "Unsupported AOA protocol version {aoa_version} reported by device {:04x}:{:04x}",
            desc.vendor_id(),
            desc.product_id()
        )));
    }

    for (idx, val) in AOA_IDENTIFICATION_STRINGS.iter().enumerate() {
        let payload = encode_aoa_string_payload(val);
        handle.write_control(0x40, AOA_REQ_SEND_STRING, 0, idx as u16, &payload, timeout)?;
    }

    handle.write_control(0x40, AOA_REQ_START, 0, 0, &[], timeout)?;
    Ok(aoa_version)
}

/// Opens a device already in AOA mode (or switches an Android device first when `allow_switch=true`)
/// and returns a [`TransportLink`] bridged to Bulk IN/OUT worker threads.
pub fn open_aoa_transport(
    bus_filter: Option<(u8, u8)>,
    allow_switch: bool,
    wait_timeout: Duration,
) -> Result<TransportLink> {
    let ctx = Context::new()?;

    if let Some(dev) = find_matching_aoa_device(&ctx, bus_filter)? {
        return spawn_aoa_link(dev);
    }

    if !allow_switch {
        return Err(ProtoError::Discovery(
            "No USB device in Android Open Accessory mode (18d1:2d00..2d05) found".into(),
        ));
    }

    if !try_switch_any_oem(&ctx, bus_filter)? {
        return Err(ProtoError::Discovery(
            "No switchable Android USB device accepted AOA protocol request 51".into(),
        ));
    }

    poll_aoa_link(&ctx, wait_timeout)?.ok_or(ProtoError::Timeout("USB AOA re-enumeration"))
}

/// Resets an already-enumerated AOA device (e.g., when a stale `/dev/usb_accessory`
/// descriptor causes the initial USB handshake to time out) and polls/re-switches for a
/// fresh AOA enumeration.
pub fn reset_and_reopen_aoa_transport(
    bus_filter: Option<(u8, u8)>,
    wait_timeout: Duration,
) -> Result<TransportLink> {
    // Allow any just-dropped Bulk IN/OUT worker threads time to exit and release
    // the USB interface before opening a fresh handle or resetting the device.
    thread::sleep(Duration::from_millis(200));

    let ctx = Context::new()?;
    let existing = match find_matching_aoa_device(&ctx, bus_filter)? {
        Some(dev) => Some(dev),
        None if bus_filter.is_some() => find_matching_aoa_device(&ctx, None)?,
        None => None,
    };

    let mut old_addr = None;
    let mut saw_absent = true;
    if let Some(dev) = existing {
        old_addr = Some(dev.address());
        saw_absent = false;
        if let Ok(handle) = dev.open() {
            let _ = handle.reset();
        }
    }

    let settle_deadline = Instant::now() + wait_timeout;
    while Instant::now() < settle_deadline {
        if let Some(reattached) = find_matching_aoa_device(&ctx, None)? {
            if saw_absent || old_addr.is_some_and(|a| reattached.address() != a) {
                return spawn_aoa_link(reattached);
            }
        } else {
            saw_absent = true;
            if try_switch_matching_oem(&ctx, bus_filter)? {
                break;
            }
        }
        thread::sleep(Duration::from_millis(250));
    }

    poll_aoa_link(&ctx, wait_timeout)?
        .ok_or(ProtoError::Timeout("USB AOA reset/re-enumeration"))
}

fn try_switch_matching_oem(ctx: &Context, bus_filter: Option<(u8, u8)>) -> Result<bool> {
    if try_switch_any_oem(ctx, bus_filter)? {
        return Ok(true);
    }
    if bus_filter.is_some() {
        return try_switch_any_oem(ctx, None);
    }
    Ok(false)
}

fn poll_aoa_link(ctx: &Context, wait_timeout: Duration) -> Result<Option<TransportLink>> {
    let deadline = Instant::now() + wait_timeout;
    while Instant::now() < deadline {
        thread::sleep(Duration::from_millis(250));
        if let Some(dev) = find_matching_aoa_device(ctx, None)? {
            return Ok(Some(spawn_aoa_link(dev)?));
        }
    }
    Ok(None)
}

fn try_switch_any_oem(ctx: &Context, bus_filter: Option<(u8, u8)>) -> Result<bool> {
    for dev in ctx.devices()?.iter() {
        if dev.address() == 0 {
            continue;
        }
        let Ok(desc) = dev.device_descriptor() else {
            continue;
        };
        if is_aoa_pid(desc.vendor_id(), desc.product_id()) {
            continue;
        }
        if let Some((b, a)) = bus_filter {
            if dev.bus_number() != b || dev.address() != a {
                continue;
            }
        } else if !KNOWN_ANDROID_VIDS.contains(&desc.vendor_id()) || find_aoa_endpoints(&dev).is_err() {
            continue;
        }
        if switch_device_to_aoa(&dev).is_ok() {
            return Ok(true);
        }
    }
    Ok(false)
}

fn find_matching_aoa_device(
    ctx: &Context,
    bus_filter: Option<(u8, u8)>,
) -> Result<Option<Device<Context>>> {
    for dev in ctx.devices()?.iter() {
        if dev.address() == 0 {
            continue;
        }
        let Ok(desc) = dev.device_descriptor() else {
            continue;
        };
        if let Some((b, a)) = bus_filter {
            if dev.bus_number() != b || dev.address() != a {
                continue;
            }
        }
        if is_aoa_pid(desc.vendor_id(), desc.product_id()) {
            return Ok(Some(dev));
        }
    }
    Ok(None)
}

fn spawn_aoa_link(dev: Device<Context>) -> Result<TransportLink> {
    let desc = dev.device_descriptor()?;
    let ep = find_aoa_endpoints(&dev)?;
    let handle: DeviceHandle<Context> = dev.open()?;
    let _ = handle.set_auto_detach_kernel_driver(true);
    handle.claim_interface(ep.interface_number)?;

    let label = format!(
        "usb:{:03}:{:03} ({:04x}:{:04x} if{})",
        dev.bus_number(),
        dev.address(),
        desc.vendor_id(),
        desc.product_id(),
        ep.interface_number
    );

    let handle = Arc::new(handle);
    let stop = Arc::new(AtomicBool::new(false));
    let (rx_tx, rx) = mpsc::channel::<Result<Bytes>>(128);
    let (tx, mut tx_rx) = mpsc::channel::<Bytes>(128);

    // Bulk IN reader thread
    {
        let handle = Arc::clone(&handle);
        let stop = Arc::clone(&stop);
        let iface = ep.interface_number;
        thread::Builder::new()
            .name("hs-aoa-in".into())
            .spawn(move || {
                let mut buf = vec![0u8; 32_768];
                while !stop.load(Ordering::Relaxed) {
                    match handle.read_bulk(ep.bulk_in, &mut buf, Duration::from_millis(150)) {
                        Ok(0) => {}
                        Ok(n) => {
                            if rx_tx
                                .blocking_send(Ok(Bytes::copy_from_slice(&buf[..n])))
                                .is_err()
                            {
                                break;
                            }
                        }
                        Err(rusb::Error::Timeout) => {}
                        Err(e) => {
                            let _ = rx_tx.blocking_send(Err(e.into()));
                            break;
                        }
                    }
                }
                stop.store(true, Ordering::Relaxed);
                let _ = handle.release_interface(iface);
            })?;
    }

    // Bulk OUT writer thread
    {
        let handle = Arc::clone(&handle);
        let stop = Arc::clone(&stop);
        thread::Builder::new()
            .name("hs-aoa-out".into())
            .spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    let Some(frame) = tx_rx.blocking_recv() else {
                        break;
                    };
                    let mut offset = 0;
                    while offset < frame.len() && !stop.load(Ordering::Relaxed) {
                        match handle.write_bulk(
                            ep.bulk_out,
                            &frame[offset..],
                            Duration::from_secs(5),
                        ) {
                            Ok(n) => offset += n,
                            Err(rusb::Error::Timeout) => continue,
                            Err(_) => {
                                stop.store(true, Ordering::Relaxed);
                                break;
                            }
                        }
                    }
                }
                stop.store(true, Ordering::Relaxed);
            })?;
    }

    Ok(TransportLink {
        kind: TransportKind::UsbAoa,
        label,
        rx,
        tx,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aoa_pid_range_classification() {
        for pid in 0x2D00..=0x2D05 {
            assert!(is_aoa_pid(GOOGLE_AOA_VID, pid));
        }
        assert!(!is_aoa_pid(GOOGLE_AOA_VID, 0x2D06));
        assert!(!is_aoa_pid(0x2D95, 0x2D00));
    }

    #[test]
    fn aoa_string_payload_nul_terminated() {
        assert_eq!(encode_aoa_string_payload("HandShaker"), b"HandShaker\0");
        assert_eq!(encode_aoa_string_payload(""), b"\0");
    }
}
