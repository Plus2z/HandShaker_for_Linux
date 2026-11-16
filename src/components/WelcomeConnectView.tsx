import { useState, type FormEvent } from 'react';
import { RefreshCw, ShieldCheck, Smartphone, Trash2, Usb, Wifi } from 'lucide-react';
import { formatTimestamp } from '../lib/format';
import { useI18n } from '../lib/useI18n';
import { useAppStore } from '../store/useAppStore';
import type { WifiCandidate } from '../types/api';

function formatVendorModel(
  brand?: string | null,
  manufacturer?: string | null,
  model?: string | null,
): string | null {
  const vendor = brand?.trim() || manufacturer?.trim() || '';
  const m = model?.trim() || '';
  if (vendor && m) {
    return m.toLowerCase().startsWith(vendor.toLowerCase()) ? m : `${vendor} · ${m}`;
  }
  return vendor || m || null;
}

function formatWifiMeta(w: WifiCandidate): string {
  const parts: string[] = [];
  const vendorModel = formatVendorModel(w.productBrand, w.productManufacturer, w.phoneModel);
  if (vendorModel) {
    parts.push(vendorModel);
  } else if (w.hostname) {
    parts.push(w.hostname);
  }
  parts.push(`${w.host}:${w.port}`);
  if (w.apkVersionName) {
    parts.push(`HandShaker v${w.apkVersionName}`);
  }
  return parts.join(' · ');
}

export function WelcomeConnectView() {
  const discovery = useAppStore((s) => s.discovery);
  const scanning = useAppStore((s) => s.scanning);
  const connection = useAppStore((s) => s.connection);
  const trustedDevices = useAppStore((s) => s.trustedDevices);
  const refreshScan = useAppStore((s) => s.refreshScan);
  const connectWifi = useAppStore((s) => s.connectWifi);
  const connectUsb = useAppStore((s) => s.connectUsb);
  const unpairTrustedDevice = useAppStore((s) => s.unpairTrustedDevice);
  const { lang, t } = useI18n();

  const [manualHost, setManualHost] = useState('');
  const [manualPort, setManualPort] = useState('44289');
  const [allowUsbSwitch, setAllowUsbSwitch] = useState(false);

  const busy = connection.status === 'connecting' || connection.status === 'trust_waiting';
  const parsedPort = Number.parseInt(manualPort, 10);
  const validPort = Number.isInteger(parsedPort) && parsedPort >= 1 && parsedPort <= 65535;
  const canDirectConnect = Boolean(manualHost.trim()) && validPort && !busy;

  const handleManualConnect = (e: FormEvent) => {
    e.preventDefault();
    if (!canDirectConnect) return;
    void connectWifi(manualHost.trim(), parsedPort);
  };

  return (
    <div className="view-scroll">
      <div style={{ maxWidth: 920, margin: '0 auto', display: 'flex', flexDirection: 'column', gap: 18 }}>
        <div className="card" style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 16 }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
            <Smartphone size={28} aria-hidden="true" />
            <div>
              <h1 style={{ margin: 0, fontSize: 18 }}>{t.welcome.title}</h1>
              <p style={{ margin: '2px 0 0', color: 'var(--text-secondary)', fontSize: 13 }}>
                {t.welcome.subtitle}
              </p>
            </div>
          </div>
          <button type="button" className="btn primary" disabled={scanning || busy} onClick={() => void refreshScan()}>
            <RefreshCw size={14} aria-hidden="true" />
            <span>{scanning ? t.welcome.scanning : t.welcome.scanDevices}</span>
          </button>
        </div>

        {connection.status === 'trust_waiting' && (
          <div
            className="card"
            role="status"
            style={{ borderColor: 'var(--warning)', display: 'flex', alignItems: 'center', gap: 10 }}
          >
            <ShieldCheck size={18} aria-hidden="true" />
            <div>
              <strong>{t.welcome.trustTitle}</strong>
              <div style={{ fontSize: 12.5, color: 'var(--text-secondary)' }}>
                {t.welcome.trustDesc}
              </div>
            </div>
          </div>
        )}

        <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(290px, 1fr))', gap: 14 }}>
          <section className="card" aria-labelledby="wifi-heading" style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
              <Wifi size={16} aria-hidden="true" />
              <h2 id="wifi-heading" style={{ margin: 0, fontSize: 14.5 }}>
                {t.welcome.wifiHeading(discovery.wifi.length)}
              </h2>
            </div>

            {discovery.wifi.length === 0 ? (
              <p style={{ margin: 0, color: 'var(--text-muted)', fontSize: 12.5 }}>
                {t.welcome.noWifiDevices}
              </p>
            ) : (
              discovery.wifi.map((w) => (
                <div
                  key={`${w.serviceName}-${w.host}-${w.port}`}
                  title={w.serviceName}
                  style={{
                    display: 'flex',
                    alignItems: 'center',
                    justifyContent: 'space-between',
                    gap: 10,
                    padding: 10,
                    borderRadius: 8,
                    background: 'var(--bg-elevated)',
                    border: '1px solid var(--border)',
                  }}
                >
                  <div style={{ minWidth: 0, flex: 1 }}>
                    <div style={{ display: 'flex', alignItems: 'center', gap: 6, minWidth: 0 }}>
                      <span
                        style={{
                          fontWeight: 600,
                          overflow: 'hidden',
                          textOverflow: 'ellipsis',
                          whiteSpace: 'nowrap',
                        }}
                      >
                        {w.displayName || w.deviceName || w.serviceName}
                      </span>
                      {w.isTrusted && (
                        <span className="pill ok" style={{ flexShrink: 0, padding: '1px 6px', fontSize: 10.5 }}>
                          {t.welcome.pairedBadge}
                        </span>
                      )}
                      {w.isLocked && (
                        <span className="pill warn" style={{ flexShrink: 0, padding: '1px 6px', fontSize: 10.5 }}>
                          {t.welcome.lockedBadge}
                        </span>
                      )}
                    </div>
                    <div
                      style={{
                        fontSize: 12,
                        color: 'var(--text-muted)',
                        overflow: 'hidden',
                        textOverflow: 'ellipsis',
                        whiteSpace: 'nowrap',
                      }}
                    >
                      {formatWifiMeta(w)}
                    </div>
                  </div>
                  <button
                    type="button"
                    className="btn primary"
                    disabled={busy}
                    onClick={() => void connectWifi(w.host, w.port)}
                  >
                    {t.welcome.connect}
                  </button>
                </div>
              ))
            )}

            <form onSubmit={handleManualConnect} style={{ display: 'flex', gap: 8, marginTop: 4 }}>
              <input
                aria-label={t.welcome.phoneHostAria}
                className="text-input"
                style={{ flex: 1, minWidth: 0 }}
                value={manualHost}
                onChange={(e) => setManualHost(e.target.value)}
                placeholder={t.welcome.phoneHostPlaceholder}
              />
              <input
                aria-label={t.welcome.tcpPortAria}
                className="text-input"
                style={{ width: 78 }}
                value={manualPort}
                onChange={(e) => setManualPort(e.target.value)}
                placeholder="44289"
              />
              <button type="submit" className="btn" disabled={!canDirectConnect}>
                {t.welcome.directConnect}
              </button>
            </form>
          </section>

          <section className="card" aria-labelledby="usb-heading" style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
              <Usb size={16} aria-hidden="true" />
              <h2 id="usb-heading" style={{ margin: 0, fontSize: 14.5 }}>
                {t.welcome.usbHeading(discovery.usb.length)}
              </h2>
            </div>

            <label style={{ display: 'flex', alignItems: 'flex-start', gap: 8, fontSize: 12.5, color: 'var(--text-secondary)', cursor: 'pointer' }}>
              <input
                type="checkbox"
                checked={allowUsbSwitch}
                onChange={(e) => setAllowUsbSwitch(e.target.checked)}
              />
              <span>{t.welcome.usbSwitchConsent}</span>
            </label>

            {discovery.usb.length === 0 ? (
              <p style={{ margin: 0, color: 'var(--text-muted)', fontSize: 12.5 }}>
                {t.welcome.noUsbDevices}
              </p>
            ) : (
              discovery.usb.map((u) => (
                <div
                  key={`${u.bus}-${u.address}`}
                  style={{
                    display: 'flex',
                    alignItems: 'center',
                    justifyContent: 'space-between',
                    gap: 10,
                    padding: 10,
                    borderRadius: 8,
                    background: 'var(--bg-elevated)',
                    border: '1px solid var(--border)',
                  }}
                >
                  <div style={{ minWidth: 0 }}>
                    <div style={{ fontWeight: 600 }}>
                      {u.product ||
                        u.manufacturer ||
                        t.welcome.usbFallbackName(u.vendorId.toString(16), u.productId.toString(16))}
                    </div>
                    <div style={{ fontSize: 12, color: 'var(--text-muted)' }}>
                      {t.welcome.usbMeta(u.bus, u.address, u.alreadyAoa)}
                    </div>
                  </div>
                  <button
                    type="button"
                    className="btn primary"
                    disabled={busy || (!u.alreadyAoa && !allowUsbSwitch)}
                    onClick={() => void connectUsb(u.bus, u.address, allowUsbSwitch)}
                  >
                    {t.welcome.connectUsb}
                  </button>
                </div>
              ))
            )}
          </section>
        </div>

        <section className="card" aria-labelledby="trusted-heading" style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
          <h2 id="trusted-heading" style={{ margin: 0, fontSize: 14.5 }}>
            {t.welcome.trustedHeading(trustedDevices.length)}
          </h2>
          {trustedDevices.length === 0 ? (
            <p style={{ margin: 0, color: 'var(--text-muted)', fontSize: 12.5 }}>
              {t.welcome.noTrustedDevices}
            </p>
          ) : (
            trustedDevices.map((d) => {
              const vendorModel = formatVendorModel(
                d.productBrand,
                d.productManufacturer,
                d.phoneModel,
              );
              return (
                <div
                  key={d.deviceUuid}
                  style={{
                    display: 'flex',
                    alignItems: 'center',
                    justifyContent: 'space-between',
                    padding: '8px 10px',
                    borderRadius: 8,
                    background: 'var(--bg-elevated)',
                    border: '1px solid var(--border)',
                  }}
                >
                  <div>
                    <div style={{ fontWeight: 600 }}>{d.deviceName}</div>
                    <div style={{ fontSize: 11.5, color: 'var(--text-muted)' }}>
                      {vendorModel ? `${vendorModel} · ` : ''}UUID: {d.deviceUuid} ·{' '}
                      {t.welcome.pairedAt(formatTimestamp(d.pairedAtUnix, lang))}
                    </div>
                  </div>
                  <button
                    type="button"
                    className="btn danger"
                    onClick={() => void unpairTrustedDevice(d.deviceUuid)}
                  >
                    <Trash2 size={13} aria-hidden="true" />
                    <span>{t.welcome.unpair}</span>
                  </button>
                </div>
              );
            })
          )}
        </section>
      </div>
    </div>
  );
}
