use std::io::{Read, Write};

use aes::Aes256;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use cbc::cipher::{block_padding::Pkcs7, BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use flate2::{read::GzDecoder, write::GzEncoder, Compression};
use md5::{Digest as _, Md5};
use rsa::pkcs1::{DecodeRsaPublicKey, EncodeRsaPublicKey};
use rsa::pkcs1v15::{Signature, SigningKey, VerifyingKey};
use rsa::pkcs8::{DecodePrivateKey, EncodePrivateKey, LineEnding};
use rsa::sha2::Sha256;
use rsa::signature::{SignatureEncoding, Signer, Verifier};
use rsa::{Pkcs1v15Encrypt, RsaPrivateKey, RsaPublicKey};

use crate::error::{ProtoError, Result};

type Aes256CbcEnc = cbc::Encryptor<Aes256>;
type Aes256CbcDec = cbc::Decryptor<Aes256>;

/// AES-256-CBC key & IV extracted cleanly from smartfolder JNI (`smartfolder.c`).
pub const HANDSHAKE_AES_KEY: [u8; 32] = [
    0x28, 0xe3, 0xee, 0x32, 0xb0, 0xde, 0x27, 0xef, 0x6b, 0xc2, 0x97, 0x92, 0x05, 0x4e, 0xf9, 0x73,
    0x9c, 0xe8, 0xe8, 0x7b, 0xb4, 0x95, 0xf2, 0xea, 0x0d, 0x72, 0xd4, 0xf4, 0xf4, 0x0b, 0x3b, 0xde,
];

pub const HANDSHAKE_AES_IV: [u8; 16] = [
    0x2b, 0x9e, 0x34, 0xd4, 0xe1, 0xd9, 0x08, 0x89, 0x94, 0x93, 0x9e, 0xc4, 0xe3, 0xe9, 0x60, 0xc5,
];

/// Android `Decoder.java:56` hardcodes `Arrays.copyOfRange(bArr, 0, 128)` for `flag=1`
/// RSA-SHA256 signatures, so clients MUST use a 1024-bit RSA keypair.
pub const RSA_KEY_BITS: usize = 1024;
pub const RSA_SIGNATURE_LEN: usize = 128;

/// Client RSA-1024 identity used for Handshake 01/02 and `flag=1` request signing.
#[derive(Clone, Debug)]
pub struct RsaIdentity {
    private_key: RsaPrivateKey,
    public_key: RsaPublicKey,
}

impl RsaIdentity {
    /// Generates a fresh 1024-bit RSA keypair.
    pub fn generate() -> Result<Self> {
        let mut rng = rand::thread_rng();
        let private_key = RsaPrivateKey::new(&mut rng, RSA_KEY_BITS)
            .map_err(|e| ProtoError::Crypto(format!("RSA keygen failed: {e}")))?;
        let public_key = RsaPublicKey::from(&private_key);
        Ok(Self {
            private_key,
            public_key,
        })
    }

    /// Parses a persisted PKCS#8 PEM private key.
    pub fn from_pkcs8_pem(pem: &str) -> Result<Self> {
        let private_key = RsaPrivateKey::from_pkcs8_pem(pem)
            .map_err(|e| ProtoError::Crypto(format!("Invalid PKCS#8 private key PEM: {e}")))?;
        let public_key = RsaPublicKey::from(&private_key);
        Ok(Self {
            private_key,
            public_key,
        })
    }

    /// Serializes the private key as PKCS#8 PEM (`BEGIN PRIVATE KEY`).
    pub fn to_pkcs8_pem(&self) -> Result<String> {
        self.private_key
            .to_pkcs8_pem(LineEnding::LF)
            .map(|s| s.to_string())
            .map_err(|e| ProtoError::Crypto(format!("Failed encoding PKCS#8 PEM: {e}")))
    }

    /// Exports the public key in PKCS#1 `RSAPublicKey` DER format expected by Android.
    pub fn public_pkcs1_der(&self) -> Result<Vec<u8>> {
        self.public_key
            .to_pkcs1_der()
            .map(|doc| doc.into_vec())
            .map_err(|e| ProtoError::Crypto(format!("Failed encoding PKCS#1 public key: {e}")))
    }

    /// Builds `(md5_of_der, aes256_cbc_encrypted_b64_der)` for `SSPHandShakeRequest01`.
    pub fn enckey_and_md5(&self) -> Result<([u8; 16], Vec<u8>)> {
        let der = self.public_pkcs1_der()?;
        let digest = Md5::digest(&der);
        let mut md5_bytes = [0u8; 16];
        md5_bytes.copy_from_slice(&digest);

        let b64 = BASE64.encode(&der).into_bytes();
        let enckey = Aes256CbcEnc::new(&HANDSHAKE_AES_KEY.into(), &HANDSHAKE_AES_IV.into())
            .encrypt_padded_vec_mut::<Pkcs7>(&b64);

        Ok((md5_bytes, enckey))
    }

    /// Builds the 1-step USB handshake payload (`[md5 (16B) || enckey]`) parsed by
    /// `com.smartisanos.smartfolder.aoa.decoder.b.a(byte[])`.
    pub fn usb_handshake_payload(&self) -> Result<Vec<u8>> {
        let (md5_bytes, enckey) = self.enckey_and_md5()?;
        let mut out = Vec::with_capacity(16 + enckey.len());
        out.extend_from_slice(&md5_bytes);
        out.extend_from_slice(&enckey);
        Ok(out)
    }

    /// Signs serialized protobuf payload for `flag == 1` requests (`128B` RSASSA-PKCS1-v1_5 SHA-256).
    pub fn sign_rpc_payload(&self, proto_bytes: &[u8]) -> [u8; RSA_SIGNATURE_LEN] {
        let signing_key = SigningKey::<Sha256>::new(self.private_key.clone());
        let sig = signing_key.sign(proto_bytes);
        let sig_bytes = sig.to_bytes();
        let mut out = [0u8; RSA_SIGNATURE_LEN];
        out.copy_from_slice(&sig_bytes);
        out
    }

    /// Decrypts `SSPHandShakeResponse02.result` (passes through plaintext `"failed"`, `"locked"`, `"needauth"`).
    pub fn decrypt_handshake_result(&self, result_field: &str) -> Result<String> {
        let trimmed = result_field.trim();
        if matches!(trimmed, "failed" | "locked" | "needauth" | "") {
            return Ok(trimmed.to_owned());
        }
        let cleaned: String = trimmed.chars().filter(|c| !c.is_ascii_whitespace()).collect();
        let ciphertext = BASE64
            .decode(cleaned.as_bytes())
            .map_err(|e| ProtoError::Crypto(format!("Invalid handshake result base64: {e}")))?;
        let plaintext = self
            .private_key
            .decrypt(Pkcs1v15Encrypt, &ciphertext)
            .map_err(|e| ProtoError::Crypto(format!("RSA decrypt handshake result failed: {e}")))?;
        String::from_utf8(plaintext)
            .map_err(|e| ProtoError::Crypto(format!("Handshake result not UTF-8: {e}")))
    }
}

/// Server-side helper used by mock/test harness to reverse `enckey` and validate `md5` + signatures.
pub fn decode_enckey_and_verify_md5(md5_expected: &[u8], enckey: &[u8]) -> Result<RsaPublicKey> {
    let decrypted = Aes256CbcDec::new(&HANDSHAKE_AES_KEY.into(), &HANDSHAKE_AES_IV.into())
        .decrypt_padded_vec_mut::<Pkcs7>(enckey)
        .map_err(|e| ProtoError::Crypto(format!("AES decrypt enckey failed: {e}")))?;
    let trimmed = String::from_utf8(decrypted)
        .map_err(|e| ProtoError::Crypto(format!("Decrypted enckey not UTF-8: {e}")))?;
    let der = BASE64
        .decode(trimmed.trim().as_bytes())
        .map_err(|e| ProtoError::Crypto(format!("Base64 decode PKCS#1 DER failed: {e}")))?;
    let digest = Md5::digest(&der);
    if digest.as_slice() != md5_expected {
        return Err(ProtoError::Crypto(
            "MD5(der) does not match SSPHandShakeRequest01.md5".into(),
        ));
    }
    RsaPublicKey::from_pkcs1_der(&der)
        .map_err(|e| ProtoError::Crypto(format!("Parse PKCS#1 public key failed: {e}")))
}

/// Parses a 1-step USB handshake payload (`[md5 (16B) || enckey]`) and returns the client `RsaPublicKey`.
pub fn decode_usb_handshake_payload(payload: &[u8]) -> Result<RsaPublicKey> {
    if payload.len() <= 16 {
        return Err(ProtoError::Crypto(
            "USB handshake payload shorter than 16-byte MD5 header".into(),
        ));
    }
    decode_enckey_and_verify_md5(&payload[..16], &payload[16..])
}

/// Verifies a `flag=1` RSASSA-PKCS1-v1_5 SHA-256 signature against a client's RSA public key.
pub fn verify_rpc_signature(pub_key: &RsaPublicKey, signature: &[u8], proto_bytes: &[u8]) -> Result<()> {
    let sig = Signature::try_from(signature)
        .map_err(|e| ProtoError::Crypto(format!("Invalid signature length/format: {e}")))?;
    let verifying_key = VerifyingKey::<Sha256>::new(pub_key.clone());
    verifying_key
        .verify(proto_bytes, &sig)
        .map_err(|e| ProtoError::Crypto(format!("RSA-SHA256 verification failed: {e}")))
}

/// Encrypts a handshake status token (e.g. `"ok"`) with a client's RSA public key and Base64.
pub fn encrypt_handshake_result(pub_key: &RsaPublicKey, token: &str) -> Result<String> {
    let mut rng = rand::thread_rng();
    let ct = pub_key
        .encrypt(&mut rng, Pkcs1v15Encrypt, token.as_bytes())
        .map_err(|e| ProtoError::Crypto(format!("RSA encrypt failed: {e}")))?;
    Ok(BASE64.encode(ct))
}

/// Compresses UTF-8 clipboard text with GZIP (matching `aoa/h/d.java:288`).
pub fn gzip_compress_text(text: &str) -> Result<Vec<u8>> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(text.as_bytes())?;
    Ok(encoder.finish()?)
}

/// Decompresses GZIP clipboard payload bytes into UTF-8 text.
pub fn gzip_decompress_text(compressed: &[u8]) -> Result<String> {
    let mut decoder = GzDecoder::new(compressed);
    let mut out = String::new();
    decoder.read_to_string(&mut out)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rsa_1024_enckey_md5_signature_and_handshake_result_roundtrip() {
        let id = RsaIdentity::generate().unwrap();
        let pem = id.to_pkcs8_pem().unwrap();
        let reloaded = RsaIdentity::from_pkcs8_pem(&pem).unwrap();

        let (md5_bytes, enckey) = reloaded.enckey_and_md5().unwrap();
        let pub_recovered = decode_enckey_and_verify_md5(&md5_bytes, &enckey).unwrap();

        let usb_payload = reloaded.usb_handshake_payload().unwrap();
        let usb_pub_recovered = decode_usb_handshake_payload(&usb_payload).unwrap();
        assert_eq!(usb_pub_recovered, pub_recovered);

        let payload = b"smart-sync-proto-sample";
        let sig = reloaded.sign_rpc_payload(payload);
        assert_eq!(sig.len(), RSA_SIGNATURE_LEN);
        verify_rpc_signature(&pub_recovered, &sig, payload).unwrap();
        assert!(verify_rpc_signature(&pub_recovered, &sig, b"tampered").is_err());

        let encrypted_ok = encrypt_handshake_result(&pub_recovered, "ok").unwrap();
        assert_eq!(reloaded.decrypt_handshake_result(&encrypted_ok).unwrap(), "ok");
        assert_eq!(reloaded.decrypt_handshake_result("locked").unwrap(), "locked");
        assert_eq!(reloaded.decrypt_handshake_result("failed").unwrap(), "failed");
    }

    #[test]
    fn gzip_clipboard_utf8_roundtrip() {
        let sample = "Hello HandShaker! 剪贴板双向同步 🚀\nLine 2";
        let gz = gzip_compress_text(sample).unwrap();
        let text = gzip_decompress_text(&gz).unwrap();
        assert_eq!(text, sample);
    }
}
