# USB AOA udev Permissions (`70-handshaker.rules`)

HandShaker USB mode switches an Android phone into Android Open Accessory (AOA, `18d1:2d00`..`2d05`) and communicates directly over libusb Bulk IN/OUT endpoints on Interface 0.

On Linux, opening USB devices directly as a regular desktop user requires `TAG+="uaccess"` udev permissions (preferred over `MODE="0666"`, as `systemd-logind` grants ACL access exclusively to the active local desktop session).

## Install Manually (when ready to test USB AOA)

```bash
sudo install -m 0644 packaging/udev/70-handshaker.rules /etc/udev/rules.d/70-handshaker.rules
sudo udevadm control --reload-rules
sudo udevadm trigger
```

Unplug and replug your phone once after reloading udev rules if it was already attached.

## Important Notes on ADB & OriginOS / Android 16

- Sending AOA switch control requests (`51` / `52` / `53`) re-enumerates the phone's USB gadget into `18d1:2d00` (`aoa`), which temporarily drops active `adb` until switched back or replugged.
- Verify live AOA with:
  ```bash
  cargo run -p handshaker-proto --bin hs-cli -- usb-scan
  cargo run -p handshaker-proto --bin hs-cli -- --usb --allow-usb-switch info
  ```
- Keep Android's HandShaker app **Open USB / Accessory permission dialog** in mind on first plug-in: tap **OK / Always allow** on the phone screen when prompted.
