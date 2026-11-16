# HandShaker Protocol Notes (Clean-Room Reverse Engineering)

> Target compatibility: `rianlu/handshaker-android-maintained` (commit `6bf38be`, `handshaker_130128.apk`, targetSdk 34 / tested live against `vivo X200 Pro mini` `V2419A`/`PD2419`, Android 16 SDK 36).
> Verification status tags:
> - `[静态分析已确认]`: Verified directly from smali/decompiled Java/JNI C source/Mac symbols.
> - `[已真机验证]`: Observed live on `vivo X200 Pro mini` (`V2419A`, Android 16) via `tools/proto_probe.py` against `handshaker-m0-debug.apk` (`2026110801`/`1.2.0-r9`).
> - `[待真机验证]`: Inferred or pending live device confirmation.

---

## 1. Wi-Fi Discovery & Connection Topology

- **Role Assignment** `[已真机验证]`:
  - **Phone** is the **TCP server** (`com/smartisanos/smartfolder/aoa/service/m.smali`, `i.smali`).
  - **Desktop** is the **TCP client** discovering the phone via mDNS and initiating the TCP connection.
- **mDNS / NSD Service** `[已真机验证]` (`aoa/service/m.smali:156-208`):
  - `NsdServiceInfo.setServiceType("_handshaker_ssp._tcp.")` (desktop DNS-SD / `zeroconf` clients query **`_handshaker_ssp._tcp.local.`**)
  - `NsdServiceInfo.setServiceName("handshaker_ssp_")` -> discovered live as `'handshaker_ssp_._handshaker_ssp._tcp.local.'` *(Android `NsdManager` may append `(2)` etc. on collision)*
  - Port: OS-assigned via `new ServerSocket(0)` (`m$c.a(0)`), observed live on `192.168.0.103:44289` (`/proc/net/tcp6` `:::AD01`). No TXT record attributes are set by Android.
- **QR Code Loopback Fallback** `[待真机验证]` (`aoa/service/n.java`, `aoa/h/d.java`):
  - Handled only as optional future UX; primary M0–M3 Wi-Fi flow uses mDNS discovery + direct TCP connect.

---

## 2. Wire Framing (TCP & USB Stream Multiplexing)

### 2.1 Desktop -> Phone Packet Format `[已真机验证]` (`flag=0,1` 实测；`flag=2..5` `[静态分析已确认]`) (`aoa/service/i.java`, `aoa/decoder/a.smali`)

Every outbound message from Desktop to Phone uses a **9-byte header**:

```text
+-------------------+------------+-------------------+-------------------------+
| sid (4B, uint32BE)| flag (1B)  | len (4B, uint32BE)| payload (len bytes)     |
+-------------------+------------+-------------------+-------------------------+
```

- `sid`: Client-chosen session/request ID (`uint32` big-endian) echoed back by the phone on responses for that request.
- `flag` (`1 byte`):
  - `0`: Handshake packet (unsigned protobuf `SSPHandShakeRequest01` or `SSPHandShakeRequest02`)
  - `1`: Normal RPC request (**signed**: `[128B RSA-SHA256 signature] + [protobuf bytes]`)
  - `2`: Cancel request
  - `3`: Upload chunk data
  - `4`: Trust cancel (`TrustCancel`)
  - `5`: Quit request
- `payload`:
  - When `flag == 0`: raw protobuf message bytes (`SSPHandShakeRequest01` or `SSPHandShakeRequest02`).
  - When `flag == 1`: `RSA_PKCS1v15_SHA256(priv_1024, proto_bytes)` (128 bytes) immediately followed by `proto_bytes`.

### 2.2 Phone -> Desktop Packet & Chunk Format `[已真机验证]` (`aoa/g/a.java`)

Phone responses are segmented into **transport chunks of at most 32767 bytes** (`6B header + <= 32761B body`):

```text
Transport chunk:
+--------------------+-----------------------+----------------------------------+
| sid (4B, uint32BE) | chunk_len (2B, u16BE) | chunk_bytes (chunk_len <= 32761) |
+--------------------+-----------------------+----------------------------------+
```

All `chunk_bytes` for the same `sid` are concatenated in arrival order into a per-`sid` logical message stream:

```text
Reassembled message per sid:
+--------------------------+-------------------------------+
| total_len (8B, uint64BE) | payload (total_len bytes)     |
+--------------------------+-------------------------------+
```

- `[已真机验证]`: `total_len` matches the exact protobuf payload byte length, and multiple logical responses arrive sequentially on the **same `sid`** during Handshake 02 (`TRUST_WAITING` followed by `TRUST_ALWAYS`).

---

## 3. Cryptography & Trust Pairing

### 3.1 Ephemeral / Persistent Client RSA-1024 Keypair `[已真机验证]` (`aoa/decoder/b.smali`, `org/a/a/a/a.java`, `aoa/g/j.smali`)

- Desktop holds a **1024-bit RSA keypair** (`e = 65537`).
- Public key serialization for `Handshake01`:
  1. Encode public key as **PKCS#1 `RSAPublicKey` DER**: `SEQUENCE { INTEGER modulus, INTEGER publicExponent }` (matches `org.a.a.a.a`).
  2. Base64-encode the DER bytes -> ASCII UTF-8 bytes (`b64_der`).
  3. Pad `b64_der` to a 16-byte multiple (PKCS#7 padding or NULs `<= 0x20`; Android runs `new String(decrypted).trim()` before Base64 decoding).
  4. Encrypt with **AES-256-CBC** (`unknown/main/jni/smartfolder.c`, `codex.h`):
     - `key` (32 bytes): `28e3ee32b0de27ef6bc29792054ef9739ce8e87bb495f2ea0d72d4f4f40b3bde`
     - `iv`  (16 bytes): `2b9e34d4e1d9088994939ec4e3e960c5`
- **`md5` field quirk** `[已真机验证]` (`aoa/decoder/b.smali`):
  - Despite the misleading comment in `SmartSyncProtocol.proto`, Android compares `SSPHandShakeRequest01.md5` against **`MD5(der_bytes)`** (16 raw bytes computed from the raw PKCS#1 DER public key **after** AES decryption + `.trim()` + Base64 decode), **NOT** `MD5(enckey)`.

### 3.2 Request Signing (`flag == 1`) & Response Verification `[已真机验证]` (`aoa/decoder/a.smali`, `aoa/g/j.smali:65-146`)

- **Signing RPC requests (`flag == 1`)**:
  - `signature = RSASSA-PKCS1-v1_5(SHA-256, priv_1024, proto_bytes)` -> fixed 128 bytes.
  - Wire payload: `signature || proto_bytes`.
- **Encrypting Handshake Result (`SSPHandShakeResponse02.result`)**:
  - Failure states: literal ASCII `"failed"` or `"locked"` or `"needauth"`.
  - Success state: Android runs `Cipher("RSA/ECB/PKCS1Padding").doFinal("ok".getBytes())`, then `Base64.encodeToString(..., 0)` (note: Base64 flags=`0` inserts `\n` every 76 chars). Desktop strips whitespace, Base64-decodes, and decrypts with `priv_1024` (PKCS#1 v1.5) to verify `"ok"`.

### 3.3 Trust State & `derived_key` (`PBKDF2WithHmacSHA1`) `[已真机验证: TRUST_WAITING + TRUST_ALWAYS 发放与复用]` (`aoa/g/j.smali:2307-2689, 2984-3416`, `aoa/h/z.smali`)

- Android stores trust per `host_uuid` in `SharedPreferences(host_uuid)`:
  - `hostUuid`, `salt` (256 random bytes, hex/encoded), `derivedKey` (256 bytes = `PBKDF2WithHmacSHA1(host_uuid.toCharArray(), salt, iterations=1500, keyLengthBits=2048)`), `trustType`.
- First-time Wi-Fi connection (`SSPHandShakeRequest02.derived_key` empty or unknown):
  1. Phone immediately sends `SSPHandShakeResponse02` with `trust_type = TRUST_WAITING (1)` and triggers `TrustRequestEvent` -> UI dialog (`DialogUtils.a`).
  2. Once user selects Trust Once (`TRUST_ONCE = 4`), Trust Always (`TRUST_ALWAYS = 5`), or Reject (`TRUST_NO = 3`), phone sends a **second** `SSPHandShakeResponse02` on the same `sid`:
     - If `TRUST_ALWAYS (5)`: includes `derived_key` (256 bytes) + RSA-encrypted `"ok"` in `result`; desktop should persist `derived_key` for `device_uuid`.
     - If `TRUST_ONCE (4)`: includes RSA-encrypted `"ok"` in `result` (no `derived_key`).
     - If `TRUST_NO (3)`: `result = "failed"`.
- Subsequent connections: sending stored `derived_key` skips the prompt (`TRUST_ALWAYS (5)` + encrypted `"ok"` immediately).

---

## 4. Handshake & Core Protobuf Messages (`proto2`, `package smartsync`)

> Sources: `SmartSyncProtocol.proto`, `a$*.smali` (`a(Lcom/a/a/h;)V`), and `SmartFinderCore` Mach-O property metadata.

### 4.1 Relevant `SSPRequestType` Enum Values (`a$dp.smali`) `[静态分析已确认]`

| Enum Name | Value | Notes |
| :--- | :---: | :--- |
| `HEART_BEAT_REQUEST` | `1` | Heartbeat ping/pong |
| `GET_DEVICE_INFO_REQUEST` | `2` | Query device metadata, storage paths & stats |
| `GET_DIR_FILES_REQUEST` | `7` | List directory entries |
| `HANDSHAKE_REQUEST_01` | `31` | Desktop -> Phone key exchange |
| `HANDSHAKE_RESPONSE_01` | `32` | Phone -> Desktop identity & capabilities |
| `HANDSHAKE_REQUEST_02` | `33` | Desktop -> Phone trust / derived key verification |
| `HANDSHAKE_RESPONSE_02` | `34` | Phone -> Desktop trust decision + RSA `"ok"` challenge |
| `QUIT_REQUEST` | `35` | Clean session close |
| `CANCEL_REQUEST` | `36` | Cancel active `sid` |

### 4.2 `SSPHandShakeTrustType` Enum Values (`a$ci.smali`) `[静态分析已确认]`

| Enum Name | Value | Meaning |
| :--- | :---: | :--- |
| `TRUST_WAITING` | `1` | Waiting for user confirmation on phone |
| `TRUST_UNKNOW` | `2` | Uninitialized / unknown |
| `TRUST_NO` | `3` | User rejected |
| `TRUST_ONCE` | `4` | Approved for current session only |
| `TRUST_ALWAYS` | `5` | Permanently trusted (`derived_key` issued/verified) |
| `TRUST_REMOVE` | `6` | Client requests phone clear stored trust for `host_uuid` |

### 4.3 `SSPHandShakeRequest01` (`a$ca`) & `SSPHandShakeResponse01` (`a$ce`) `[已真机验证]`

**`SSPHandShakeRequest01` (`a$ca.smali`)**:
- `1`: `type` (enum `SSPRequestType = 31`, wire varint)
- `2`: `host_uuid` (`string`, persistent desktop UUID)
- `3`: `host_name` (`string`, shown in Android trust dialog)
- `4`: `host_timestamp` (`uint64`)
- `5`: `host_smart_sync_protocol_version` (`string`, e.g. `"2.1.0"`)
- `6`: `host_app_version` (`string`, e.g. `"2.5.0"`)
- `7`: `host_min_client_version` (`string`, e.g. `"1"`)
- `8`: `md5` (`bytes`, 16-byte raw `MD5(der)`)
- `9`: `enckey` (`bytes`, AES-256-CBC ciphertext of `base64(der)`)
- `10`: `host_model` (`string`, optional)
- `11`: `heartbeat_timeout_second` (`uint64`, e.g. `10`)

**`SSPHandShakeResponse01` (`a$ce.smali`, populated in `j.smali:2156-2255`)**:
- `1`: `type` (enum `32`)
- `2`: `apk_version` (`string`)
- `3`: `apk_version_name` (`string`)
- `4`: `client_timestamp` (`uint64`)
- `5`: `client_smart_sync_protocol_version` (`string`, `"1"`)
- `6`: `client_min_host_version` (`string`, `"2.1.0"`, from `d.a(1)`)
- `7`: `device_uuid` (`string`, `FolderApp.d` = `Settings.Secure.ANDROID_ID` fallback `ro.serialno`)
- `8`: `device_name` (`string`)
- `9`: `usb_serial` (`string`)
- `10`: `is_smartisan_device` (`bool`, hardcoded `true` in `j.smali:2241` for clipboard gating)
- `11`: `client_min_host_version_code` (`uint64`, `333` = `0x14d`, from `d.b(1)`)

### 4.4 `SSPHandShakeRequest02` (`a$cc`) & `SSPHandShakeResponse02` (`a$cg`) `[已真机验证]`

**`SSPHandShakeRequest02` (`a$cc.smali`)**:
- `1`: `type` (enum `33`)
- `2`: `host_uuid` (`string`)
- `3`: `derived_key` (`bytes`)
- `4`: `trust_type` (enum `SSPHandShakeTrustType`)

**`SSPHandShakeResponse02` (`a$cg.smali`)**:
- `1`: `type` (enum `34`)
- `2`: `trust_type` (enum `SSPHandShakeTrustType`)
- `3`: `device_uuid` (`string`)
- `4`: `device_name` (`string`)
- `5`: `derived_key` (`bytes`)
- `6`: `result` (`string`: RSA-encrypted `"ok"` Base64, or `"failed"`)

### 4.5 `SSPGetDeviceInfoRequest` (`a$bg`) & `SSPGetDeviceInfoResponse` (`a$bi`) `[已真机验证]`

**`SSPGetDeviceInfoRequest` (`a$bg.smali`)**:
- `1`: `type` (enum `2`)
- `2`: `host_timestamp` (`uint64`)
- `3`: `host_smart_sync_protocol_version` (`string`)
- `4`: `need_device_info_callback` (`bool`)
- `5`: `need_photo_library_callback` (`bool`)
- `6`: `need_audio_library_callback` (`bool`)
- `7`: `need_video_library_callback` (`bool`)
- `8`: `host_app_version` (`string`)
- `9`: `host_min_client_version` (`string`)
- `10`: `host_type` (`uint32`: `1` = Mac (`"2.1.0"`/`333`), `2` = Windows (`"2.5.0"`/`12`) in `aoa/h/d.smali:264-282, 1541-1558`)
- `11`: `host_app_version_code` (`uint32`)

**`SSPGetDeviceInfoResponse` (`a$bi.smali`, 36 fields)**:
- `1: type` (enum `2`), `2: host_timestamp` (`uint64`), `3: host_smart_sync_protocol_version` (`string`), `4: apk_version` (`string`), `5: client_timestamp` (`uint64`), `6: client_smart_sync_protocol_version` (`string`), `7: host_app_version` (`string`), `8: host_min_client_version` (`string`), `9: phone_model` (`string`), `10: phone_color` (`string`), `11: disk_size` (`uint64`), `12: ram_size` (`uint64`), `13: battery_capacity` (`double`, 64-bit IEEE float), `14: battery_percentage` (`uint32`), `15: phone_name` (`string`), `16: used_disk_size` (`uint64`), `17: root_path` (`string`, e.g. `/storage/emulated/0`), `18: product_brand` (`string`), `19: product_manufacturer` (`string`), `20: smartisan_version` (`string`), `21: phone_locked` (`bool`), `22: client_min_host_version` (`string`), `23: apk_version_name` (`string`), `24: external_storage_path` (`string`), `25: external_storage_permission` (`enum` / varint), `26: ext_disk_size` (`uint64`), `27: ext_used_disk_size` (`uint64`), `28: phone_id` (`string`, `FolderApp.d`), `29: audio_size` (`int64`), `30: pic_video_size` (`int64`), `31: download_size` (`int64`), `32: other_size` (`int64`), `33: app_size` (`int64`), `34: cache_size` (`int64`), `35: debug_build_time` (`string`), `36: client_min_host_version_code` (`int64`).

### 4.6 `SSPGetDirFilesRequest` (`a$bk`), `SSPGetDirFilesResponse` (`a$bm`), `SSPFile` (`a$aj`) `[已真机验证]`

**`SSPGetDirFilesRequest` (`a$bk.smali`)**:
- `1`: `type` (enum `7`)
- `2`: `dir` (`SSPFile`, only `path` needs to be populated, typically starting from `SSPGetDeviceInfoResponse.root_path`, observed live as `'/storage/emulated/0'`)
- `3`: `maxdepth` (`uint32`: **must be `>= 1`**; `aoa/d/c.java` returns empty if `maxdepth == 0`)

**`SSPGetDirFilesResponse` (`a$bm.smali`)**:
- `1`: `type` (enum `7`)
- `2`: `dir` (`SSPFile`)
- `3`: `maxdepth` (`uint32`)
- `4`: `timecost` (`uint32`, observed `12–20 ms` for 84 root entries)
- `5`: `repeated SSPFile file`

**`SSPFile` (`a$aj.smali`, `SmartSyncProtocol.proto:14-28`)**:
- `1`: `path` (`string`)
- `2`: `file_size` (`uint64`)
- `3`: `created_timestamp` (`uint64`)
- `4`: `modified_timestamp` (`uint64`)
- *(tag 5 unused)*
- `6`: `is_directory` (`bool`)
- `7`: `checksum` (`string`)
- `8`: `file_type` (enum `SSPFileType`, e.g. `NORMAL=0`, `DATA=1` in `a$az.smali`)
- `9`: `prefix_md5` (`string`)
- `10`: `ext_data` (`string`)
- `11`: `is_trash` (`bool`, move to trash on delete request)
- `12`: `succeed` (`bool`, delete success flag)
- `13`: `error_code` (`enum SSPFileIOError`, `a$aw.smali`)
- `14`: `id` (`uint64`, MediaStore file ID, e.g. observed `1000003015` on `/storage/emulated/0/音乐`, `1000003020` on `.clear_sdcard.ini`, or `0` when unindexed like `.vivoFileRecycleBin` / `BaiduNetdisk`)

---

## 5. Android 16 / Instrumented Debug Oracle Notes `[已真机验证]`

- `handshaker-android-maintained` declares `MANAGE_EXTERNAL_STORAGE` (`AndroidManifest.xml:10`) and includes `StorageRoots.java` for Android 30+ storage volumes.
- Live behavior on **`vivo X200 Pro mini` (`V2419A` / `PD2419`, Android 16 SDK 36, OriginOS)**:
  - Launching `MainActivity` (`adb shell am start -W`) immediately opens `com.android.settings/.Settings$AppManageExternalStorageActivity` until **所有文件访问权限 (`MANAGE_EXTERNAL_STORAGE`)** is granted, followed by location / notification / battery permissions.
  - Once granted, `ConnectionManagerService` binds an ephemeral TCP socket (`44289` -> `/proc/net/tcp6` `:::AD01`), advertises `handshaker_ssp_._handshaker_ssp._tcp.local.`, reports `product_brand='Smartisan'` / `is_smartisan_device=True` (via `aoa/h/f$c.smali` and `aoa/g/j.smali:2241`), and returns `root_path='/storage/emulated/0'` (`84` entries listed in `12–20 ms`, including Chinese folder names & hidden dotfiles).
  - First-time Wi-Fi pairing returned `TRUST_WAITING` -> `TRUST_ALWAYS` (256-byte `derived_key` + 175-char Base64 RSA `"ok"`). Reconnecting with the persisted `derived_key` bypassed `TRUST_WAITING` immediately (`TRUST_ALWAYS` + `"ok"`).
- **OriginOS Logging Caveat**:
  - Stock `LogUtils` (`aoa/h/t.smali:28`) was flipped (`t.a = true`) and `WireTrace.rx`/`tx` (`HS_WIRE`, `Log.i`) is compiled into `classes.dex` inside `handshaker-m0-debug.apk`. However, OriginOS ships with `[log.tag]: [I]`, `[persist.sys.log.ctrl]: [no]`, and `[persist.sys.bbklog.adblimit]: [2048]`, which suppresses third-party app (`u0_a383`) `main`-buffer `adb logcat` output (while `events` buffer lifecycle logs, `/proc/net/tcp6`, and `tools/proto_probe.py` wire verification succeed cleanly). If future milestones (M1–M3) require `HS_WIRE` device logs, enable Vivo device logging (`bbklog` / developer log switch) or add a local ring-buffer file log in `WireTrace` similar to `UsbDiagnostics`.
