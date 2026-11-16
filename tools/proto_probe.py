#!/usr/bin/env python3
"""M0 Wi-Fi protocol probe for HandShaker (clean-room verification oracle client).

Discovers an Android HandShaker server via mDNS (`_handshaker_ssp._tcp.`), performs
Handshake 01 / 02 (RSA-1024 + AES-256-CBC + optional trust dialog), queries
GetDeviceInfo, and lists root_path via GetDirFiles.
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import json
import os
import socket
import struct
import sys
import time
import uuid
from pathlib import Path
from typing import Any

from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import padding as asym_padding
from cryptography.hazmat.primitives.asymmetric import rsa
from cryptography.hazmat.primitives.ciphers import Cipher, algorithms, modes
from cryptography.hazmat.primitives.padding import PKCS7
from zeroconf import ServiceBrowser, ServiceInfo, Zeroconf

# Hardcoded AES-256-CBC key & IV from smartfolder JNI (`unknown/main/jni/smartfolder.c`)
AES_KEY = bytes.fromhex(
    "28e3ee32b0de27ef6bc29792054ef9739ce8e87bb495f2ea0d72d4f4f40b3bde"
)
AES_IV = bytes.fromhex("2b9e34d4e1d9088994939ec4e3e960c5")

SERVICE_TYPE = "_handshaker_ssp._tcp.local."

FLAG_HANDSHAKE = 0
FLAG_SIGNED_RPC = 1
FLAG_QUIT = 5

REQ_GET_DEVICE_INFO = 2
REQ_GET_DIR_FILES = 7
REQ_HANDSHAKE_01 = 31
RES_HANDSHAKE_01 = 32
REQ_HANDSHAKE_02 = 33
RES_HANDSHAKE_02 = 34

TRUST_NAMES = {
    1: "TRUST_WAITING",
    2: "TRUST_UNKNOW",
    3: "TRUST_NO",
    4: "TRUST_ONCE",
    5: "TRUST_ALWAYS",
    6: "TRUST_REMOVE",
}


# ---------------------------------------------------------------------------
# Minimal proto2 wire helpers (exact tags verified in smali a$*.smali)
# ---------------------------------------------------------------------------

def _encode_varint(value: int) -> bytes:
    if value < 0:
        value &= (1 << 64) - 1
    out = bytearray()
    while True:
        byte = value & 0x7F
        value >>= 7
        if value:
            out.append(byte | 0x80)
        else:
            out.append(byte)
            return bytes(out)


def _decode_varint(buf: bytes, pos: int) -> tuple[int, int]:
    result = 0
    shift = 0
    while True:
        if pos >= len(buf):
            raise ValueError("Truncated varint")
        b = buf[pos]
        pos += 1
        result |= (b & 0x7F) << shift
        if not (b & 0x80):
            return result, pos
        shift += 7
        if shift >= 70:
            raise ValueError("Varint too long")


def pb_varint(field_num: int, value: int) -> bytes:
    return _encode_varint((field_num << 3) | 0) + _encode_varint(value)


def pb_bytes(field_num: int, data: bytes) -> bytes:
    return _encode_varint((field_num << 3) | 2) + _encode_varint(len(data)) + data


def pb_str(field_num: int, text: str) -> bytes:
    return pb_bytes(field_num, text.encode("utf-8"))


def pb_decode(data: bytes) -> dict[int, list[tuple[int, Any]]]:
    """Decode a protobuf message into {field_num: [(wire_type, value), ...]}."""
    fields: dict[int, list[tuple[int, Any]]] = {}
    pos = 0
    while pos < len(data):
        tag, pos = _decode_varint(data, pos)
        field_num = tag >> 3
        wire_type = tag & 0x07
        if wire_type == 0:
            val, pos = _decode_varint(data, pos)
        elif wire_type == 1:
            if pos + 8 > len(data):
                raise ValueError("Truncated 64-bit field")
            val = data[pos : pos + 8]
            pos += 8
        elif wire_type == 2:
            length, pos = _decode_varint(data, pos)
            if pos + length > len(data):
                raise ValueError("Truncated length-delimited field")
            val = data[pos : pos + length]
            pos += length
        elif wire_type == 5:
            if pos + 4 > len(data):
                raise ValueError("Truncated 32-bit field")
            val = data[pos : pos + 4]
            pos += 4
        else:
            raise ValueError(f"Unsupported wire type {wire_type} on field {field_num}")
        fields.setdefault(field_num, []).append((wire_type, val))
    return fields


def pb_first_int(fields: dict[int, list[tuple[int, Any]]], num: int, default: int = 0) -> int:
    items = fields.get(num)
    if not items:
        return default
    return int(items[-1][1])


def pb_first_str(fields: dict[int, list[tuple[int, Any]]], num: int, default: str = "") -> str:
    items = fields.get(num)
    if not items:
        return default
    raw = items[-1][1]
    return raw.decode("utf-8", errors="replace") if isinstance(raw, (bytes, bytearray)) else str(raw)


def pb_first_bytes(fields: dict[int, list[tuple[int, Any]]], num: int, default: bytes = b"") -> bytes:
    items = fields.get(num)
    if not items:
        return default
    raw = items[-1][1]
    return bytes(raw)


# ---------------------------------------------------------------------------
# Crypto helpers (RSA-1024 PKCS#1 + AES-256-CBC + SHA256withRSA)
# ---------------------------------------------------------------------------

def encrypt_enckey_and_md5(pub_key: rsa.RSAPublicKey) -> tuple[bytes, bytes]:
    """Return (md5_of_der, aes256cbc_enckey) matching KeyUtils.a(ca) in smali."""
    der_bytes = pub_key.public_bytes(
        encoding=serialization.Encoding.DER,
        format=serialization.PublicFormat.PKCS1,
    )
    md5_digest = hashlib.md5(der_bytes).digest()
    b64_bytes = base64.b64encode(der_bytes)
    padder = PKCS7(128).padder()
    padded = padder.update(b64_bytes) + padder.finalize()
    encryptor = Cipher(algorithms.AES(AES_KEY), modes.CBC(AES_IV)).encryptor()
    enckey = encryptor.update(padded) + encryptor.finalize()
    return md5_digest, enckey


def decrypt_enckey_for_test(enckey: bytes) -> bytes:
    """Inverse check matching JNI smartfolder.parseIoBuffer + String.trim()."""
    decryptor = Cipher(algorithms.AES(AES_KEY), modes.CBC(AES_IV)).decryptor()
    padded = decryptor.update(enckey) + decryptor.finalize()
    unpadder = PKCS7(128).unpadder()
    b64_bytes = unpadder.update(padded) + unpadder.finalize()
    return base64.b64decode(b64_bytes.decode("ascii").strip())


def sign_proto(priv_key: rsa.RSAPrivateKey, proto_bytes: bytes) -> bytes:
    sig = priv_key.sign(
        proto_bytes,
        asym_padding.PKCS1v15(),
        hashes.SHA256(),
    )
    if len(sig) != 128:
        raise ValueError(f"Expected 128-byte RSA-1024 signature, got {len(sig)}")
    return sig


def decrypt_handshake_result(priv_key: rsa.RSAPrivateKey, result_str: str) -> str:
    if result_str in ("failed", "locked", "needauth", ""):
        return result_str
    raw = base64.b64decode(result_str)
    pt = priv_key.decrypt(raw, asym_padding.PKCS1v15())
    return pt.decode("utf-8", errors="replace")


# ---------------------------------------------------------------------------
# Identity / Trust persistence
# ---------------------------------------------------------------------------

def load_or_create_identity(
    state_path: Path, fresh: bool = False
) -> tuple[str, rsa.RSAPrivateKey, dict[str, str]]:
    """Load or create persistent host_uuid, RSA-1024 private key, and paired device_uuid -> b64 derived_key map."""
    if not fresh and state_path.exists():
        data = json.loads(state_path.read_text(encoding="utf-8"))
        priv_pem = data["private_key_pem"].encode("utf-8")
        priv_key = serialization.load_pem_private_key(priv_pem, password=None)
        if not isinstance(priv_key, rsa.RSAPrivateKey):
            raise TypeError("Expected RSA private key")
        return data["host_uuid"], priv_key, data.get("trusted_devices", {})

    host_uuid = str(uuid.uuid4())
    priv_key = rsa.generate_private_key(public_exponent=65537, key_size=1024)
    save_identity(state_path, host_uuid, priv_key, {})
    return host_uuid, priv_key, {}


def save_identity(
    state_path: Path,
    host_uuid: str,
    priv_key: rsa.RSAPrivateKey,
    trusted_devices: dict[str, str],
) -> None:
    state_path.parent.mkdir(parents=True, exist_ok=True)
    pem = priv_key.private_bytes(
        encoding=serialization.Encoding.PEM,
        format=serialization.PrivateFormat.PKCS8,
        encryption_algorithm=serialization.NoEncryption(),
    ).decode("utf-8")
    payload = {
        "host_uuid": host_uuid,
        "private_key_pem": pem,
        "trusted_devices": trusted_devices,
    }
    state_path.write_text(json.dumps(payload, indent=2), encoding="utf-8")
    os.chmod(state_path, 0o600)


# ---------------------------------------------------------------------------
# TCP Wire Transport (C->P 9B frame, P->C 6B chunk + 8B total length)
# ---------------------------------------------------------------------------

class HandShakerTcpClient:
    def __init__(self, sock: socket.socket) -> None:
        self.sock = sock
        self._rx_streams: dict[int, bytearray] = {}
        self._next_sid = 1

    def alloc_sid(self) -> int:
        sid = self._next_sid
        self._next_sid += 1
        return sid

    def send_frame(self, sid: int, flag: int, payload: bytes) -> None:
        header = struct.pack(">IBI", sid, flag & 0xFF, len(payload))
        self.sock.sendall(header + payload)

    def _recv_exact(self, n: int) -> bytes:
        buf = bytearray()
        while len(buf) < n:
            chunk = self.sock.recv(n - len(buf))
            if not chunk:
                raise EOFError(f"Socket closed while reading {n} bytes (got {len(buf)})")
            buf.extend(chunk)
        return bytes(buf)

    def recv_message(self, expected_sid: int | None = None, timeout: float = 15.0) -> tuple[int, bytes]:
        """Read 6-byte P->C chunks until a complete [u64 total_len][payload] message arrives."""
        self.sock.settimeout(timeout)
        while True:
            # Check if any already-buffered stream has a complete message
            for sid, stream in list(self._rx_streams.items()):
                if expected_sid is not None and sid != expected_sid:
                    continue
                if len(stream) >= 8:
                    total_len, = struct.unpack(">Q", stream[:8])
                    if len(stream) >= 8 + total_len:
                        payload = bytes(stream[8 : 8 + total_len])
                        del stream[: 8 + total_len]
                        return sid, payload

            chunk_hdr = self._recv_exact(6)
            sid, chunk_len = struct.unpack(">IH", chunk_hdr)
            chunk_body = self._recv_exact(chunk_len)
            self._rx_streams.setdefault(sid, bytearray()).extend(chunk_body)


# ---------------------------------------------------------------------------
# mDNS Discovery
# ---------------------------------------------------------------------------

def discover_mdns(timeout: float = 8.0) -> tuple[str, int, str]:
    found: list[tuple[str, int, str]] = []

    class Listener:
        def add_service(self, zc: Zeroconf, type_: str, name: str) -> None:
            info: ServiceInfo | None = zc.get_service_info(type_, name, timeout=2000)
            if info:
                addrs = info.parsed_addresses()
                # Prefer IPv4 first for local LAN reliability
                ipv4 = [a for a in addrs if "." in a]
                host = ipv4[0] if ipv4 else (addrs[0] if addrs else "")
                if host and info.port:
                    found.append((host, info.port, name))

        def update_service(self, zc: Zeroconf, type_: str, name: str) -> None:
            pass

        def remove_service(self, zc: Zeroconf, type_: str, name: str) -> None:
            pass

    zc = Zeroconf()
    try:
        ServiceBrowser(zc, SERVICE_TYPE, Listener())
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline and not found:
            time.sleep(0.1)
        if not found:
            raise TimeoutError(f"No {SERVICE_TYPE} service discovered within {timeout:.1f}s")
        return found[0]
    finally:
        zc.close()


# ---------------------------------------------------------------------------
# Message Builders / Parsers
# ---------------------------------------------------------------------------

def build_handshake_01(
    host_uuid: str, host_name: str, pub_key: rsa.RSAPublicKey
) -> bytes:
    md5_digest, enckey = encrypt_enckey_and_md5(pub_key)
    ts = int(time.time())
    return b"".join([
        pb_varint(1, REQ_HANDSHAKE_01),
        pb_str(2, host_uuid),
        pb_str(3, host_name),
        pb_varint(4, ts),
        pb_str(5, "2.1.0"),
        pb_str(6, "2.5.0"),
        pb_str(7, "1"),
        pb_bytes(8, md5_digest),
        pb_bytes(9, enckey),
        pb_str(10, "Linux"),
        pb_varint(11, 10),
    ])


def build_handshake_02(
    host_uuid: str, derived_key: bytes = b"", trust_type: int = 1
) -> bytes:
    parts = [
        pb_varint(1, REQ_HANDSHAKE_02),
        pb_str(2, host_uuid),
    ]
    if derived_key:
        parts.append(pb_bytes(3, derived_key))
    parts.append(pb_varint(4, trust_type))
    return b"".join(parts)


def build_get_device_info_req() -> bytes:
    return b"".join([
        pb_varint(1, REQ_GET_DEVICE_INFO),
        pb_varint(2, int(time.time())),
        pb_str(3, "2.1.0"),
        pb_varint(4, 1),  # need_device_info_callback
        pb_varint(5, 0),
        pb_varint(6, 0),
        pb_varint(7, 0),
        pb_str(8, "2.5.0"),
        pb_str(9, "1"),
        pb_varint(10, 1),   # host_type=1 -> client_min_host 2.1.0 / 333
        pb_varint(11, 333),
    ])


def build_get_dir_files_req(path: str, maxdepth: int = 1) -> bytes:
    ssp_file = pb_str(1, path) + pb_varint(6, 1)
    return b"".join([
        pb_varint(1, REQ_GET_DIR_FILES),
        pb_bytes(2, ssp_file),
        pb_varint(3, maxdepth),
    ])


def parse_ssp_file(raw: bytes) -> dict[str, Any]:
    f = pb_decode(raw)
    return {
        "path": pb_first_str(f, 1),
        "file_size": pb_first_int(f, 2),
        "created_ts": pb_first_int(f, 3),
        "modified_ts": pb_first_int(f, 4),
        "is_directory": bool(pb_first_int(f, 6)),
        "file_type": pb_first_int(f, 8),
        "media_id": pb_first_int(f, 14),
    }


def run_self_test() -> None:
    print("[self-test] Generating 1024-bit RSA keypair and checking AES/MD5/Sign/Framing...")
    priv = rsa.generate_private_key(public_exponent=65537, key_size=1024)
    pub = priv.public_key()
    md5_digest, enckey = encrypt_enckey_and_md5(pub)
    assert len(md5_digest) == 16
    assert len(enckey) % 16 == 0
    recovered_der = decrypt_enckey_for_test(enckey)
    assert hashlib.md5(recovered_der).digest() == md5_digest

    hs1 = build_handshake_01("test-uuid", "linux-host", pub)
    d1 = pb_decode(hs1)
    assert pb_first_int(d1, 1) == REQ_HANDSHAKE_01
    assert pb_first_str(d1, 2) == "test-uuid"
    assert pb_first_str(d1, 3) == "linux-host"
    assert pb_first_bytes(d1, 8) == md5_digest
    assert pb_first_bytes(d1, 9) == enckey

    req_info = build_get_device_info_req()
    sig = sign_proto(priv, req_info)
    assert len(sig) == 128

    ct_ok = pub.encrypt(b"ok", asym_padding.PKCS1v15())
    b64_ok = base64.b64encode(ct_ok).decode("ascii")
    assert decrypt_handshake_result(priv, b64_ok) == "ok"

    # Simulated Phone -> Desktop chunk reassembly over socketpair
    a, b = socket.socketpair()
    try:
        cli = HandShakerTcpClient(a)
        sample_resp = pb_varint(1, RES_HANDSHAKE_01) + pb_str(8, "vivo X200 Pro mini")
        framed = struct.pack(">Q", len(sample_resp)) + sample_resp
        # Send split across 2 transport chunks
        c1, c2 = framed[:10], framed[10:]
        b.sendall(struct.pack(">IH", 7, len(c1)) + c1)
        b.sendall(struct.pack(">IH", 7, len(c2)) + c2)
        sid, got = cli.recv_message(expected_sid=7, timeout=2.0)
        assert sid == 7 and got == sample_resp
        df = pb_decode(got)
        assert pb_first_int(df, 1) == RES_HANDSHAKE_01
        assert pb_first_str(df, 8) == "vivo X200 Pro mini"
    finally:
        a.close()
        b.close()
    print("[self-test] PASS")


def main() -> int:
    parser = argparse.ArgumentParser(description="HandShaker M0 Wi-Fi protocol verification probe")
    parser.add_argument("--self-test", action="store_true", help="Run offline codec/crypto/framing self-test")
    parser.add_argument("--host", help="Skip mDNS and connect directly to phone IP")
    parser.add_argument("--port", type=int, help="TCP port on phone (required with --host)")
    parser.add_argument("--fresh", action="store_true", help="Generate a fresh host_uuid & RSA keypair")
    parser.add_argument("--trust-timeout", type=float, default=45.0, help="Seconds to wait for user Trust tap")
    parser.add_argument("--list-limit", type=int, default=30, help="Max directory entries to print")
    args = parser.parse_args()

    if args.self_test:
        run_self_test()
        return 0

    state_path = Path.home() / ".local/share/handshaker-linux/m0_probe_identity.json"
    host_uuid, priv_key, trusted = load_or_create_identity(state_path, fresh=args.fresh)
    pub_key = priv_key.public_key()
    host_name = socket.gethostname() or "Linux-HandShaker"

    if args.host:
        if not args.port:
            parser.error("--port is required when --host is set")
        host, port, svc_name = args.host, args.port, "manual"
    else:
        print(f"[mDNS] Searching for {SERVICE_TYPE} ...")
        host, port, svc_name = discover_mdns(timeout=8.0)
    print(f"[mDNS] Discovered service={svc_name!r} at {host}:{port}")

    with socket.create_connection((host, port), timeout=10.0) as sock:
        client = HandShakerTcpClient(sock)

        # 1. Handshake 01
        sid1 = client.alloc_sid()
        client.send_frame(sid1, FLAG_HANDSHAKE, build_handshake_01(host_uuid, host_name, pub_key))
        _, raw_hs1 = client.recv_message(expected_sid=sid1, timeout=10.0)
        hs1 = pb_decode(raw_hs1)
        device_uuid = pb_first_str(hs1, 7)
        device_name = pb_first_str(hs1, 8)
        print(
            "[HS01] "
            f"type={pb_first_int(hs1, 1)} device_name={device_name!r} device_uuid={device_uuid!r} "
            f"proto={pb_first_str(hs1, 5)!r} apk={pb_first_str(hs1, 2)!r}/{pb_first_str(hs1, 3)!r} "
            f"smartisan={bool(pb_first_int(hs1, 10))} min_host_code={pb_first_int(hs1, 11)}"
        )

        # 2. Handshake 02 (using cached derived_key if paired before)
        derived_key = base64.b64decode(trusted[device_uuid]) if device_uuid in trusted else b""
        sid2 = client.alloc_sid()
        client.send_frame(sid2, FLAG_HANDSHAKE, build_handshake_02(host_uuid, derived_key=derived_key))

        trust_ok = False
        deadline = time.monotonic() + args.trust_timeout
        while time.monotonic() < deadline:
            remaining = max(1.0, deadline - time.monotonic())
            _, raw_hs2 = client.recv_message(expected_sid=sid2, timeout=remaining)
            hs2 = pb_decode(raw_hs2)
            trust_type = pb_first_int(hs2, 2)
            trust_label = TRUST_NAMES.get(trust_type, f"UNKNOWN({trust_type})")
            res_str = pb_first_str(hs2, 6)
            dk = pb_first_bytes(hs2, 5)
            print(
                f"[HS02] type={pb_first_int(hs2, 1)} trust={trust_label} "
                f"derived_key_len={len(dk)} result_len={len(res_str)}"
            )
            if trust_type == 1:  # TRUST_WAITING
                print("[HS02] Phone is waiting for user approval — please tap '信任' on your phone...")
                continue
            decrypted = decrypt_handshake_result(priv_key, res_str)
            print(f"[HS02] Decrypted challenge result = {decrypted!r}")
            if decrypted != "ok":
                print("[HS02] ERROR: Handshake rejected or failed", file=sys.stderr)
                return 2
            if trust_type == 5 and dk:
                trusted[device_uuid] = base64.b64encode(dk).decode("ascii")
                save_identity(state_path, host_uuid, priv_key, trusted)
                print(f"[HS02] Saved TRUST_ALWAYS derived_key ({len(dk)}B) for {device_uuid}")
            trust_ok = True
            break

        if not trust_ok:
            print("[HS02] Timed out waiting for trust confirmation", file=sys.stderr)
            return 3

        # 3. GetDeviceInfo (flag=1, signed)
        sid3 = client.alloc_sid()
        req_info = build_get_device_info_req()
        client.send_frame(sid3, FLAG_SIGNED_RPC, sign_proto(priv_key, req_info) + req_info)
        _, raw_info = client.recv_message(expected_sid=sid3, timeout=10.0)
        info = pb_decode(raw_info)
        root_path = pb_first_str(info, 17)
        print(
            "[GetDeviceInfo] "
            f"phone={pb_first_str(info, 15)!r} ({pb_first_str(info, 18)} {pb_first_str(info, 9)}) "
            f"battery={pb_first_int(info, 14)}% "
            f"disk={pb_first_int(info, 16)/(1024**3):.2f}/{pb_first_int(info, 11)/(1024**3):.2f} GiB "
            f"ram={pb_first_int(info, 12)/(1024**3):.2f} GiB locked={bool(pb_first_int(info, 21))} "
            f"root_path={root_path!r} ext_perm={pb_first_int(info, 25)}"
        )

        # 4. GetDirFiles(root_path, maxdepth=1)
        if not root_path:
            root_path = "/storage/emulated/0"
        sid4 = client.alloc_sid()
        req_dir = build_get_dir_files_req(root_path, maxdepth=1)
        client.send_frame(sid4, FLAG_SIGNED_RPC, sign_proto(priv_key, req_dir) + req_dir)
        _, raw_dir = client.recv_message(expected_sid=sid4, timeout=15.0)
        dres = pb_decode(raw_dir)
        entries = [parse_ssp_file(val) for _, val in dres.get(5, [])]
        print(
            f"[GetDirFiles] path={root_path!r} maxdepth={pb_first_int(dres, 3)} "
            f"timecost={pb_first_int(dres, 4)}ms entries={len(entries)}"
        )
        for item in entries[: args.list_limit]:
            kind = "DIR " if item["is_directory"] else "FILE"
            print(f"  {kind} {item['file_size']:>10d}  id={item['media_id']:<10d} {item['path']}")
        if len(entries) > args.list_limit:
            print(f"  ... ({len(entries) - args.list_limit} more entries)")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
