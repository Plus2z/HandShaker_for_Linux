pub mod crypto;
pub mod device_profile;
pub mod discovery;
pub mod error;
pub mod framing;
pub mod session;
pub mod transport;
pub mod trust;
pub mod wifi_probe;

pub mod proto {
    include!(concat!(env!("OUT_DIR"), "/smartsync.rs"));
}

pub use device_profile::{DeviceProfile, DeviceProfileStore};
pub use error::{ProtoError, Result};
pub use session::{ClipboardEntry, ConnectConfig, HandshakeInfo, Session, SessionEvent};
pub use transport::{TransportKind, TransportLink};
pub use trust::{IdentityStore, TrustedDevice};
