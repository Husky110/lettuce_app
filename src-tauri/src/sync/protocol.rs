use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SyncDomain {
    Core,
    Tts,
    Lorebooks,
    Characters,
    Groups,
    Conversations,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct DomainManifest {
    pub domain: SyncDomain,
    pub fingerprint: String,
}

#[derive(Serialize, Deserialize, Debug, Default, Clone, PartialEq, Eq)]
pub struct SyncManifest {
    pub domains: Vec<DomainManifest>,
}

// 3. The Actual Messages over TCP
#[derive(Serialize, Deserialize, Debug)]
pub enum P2PMessage {
    // Handshake
    Handshake {
        #[serde(default = "default_protocol_version")]
        protocol_version: u32,
        device_name: String,
        salt: [u8; 16],
        challenge: [u8; 16], // Random bytes the other side must decrypt and return
    },
    AuthRequest {
        // The sender encrypts the received challenge with the derived key
        // and sends it back to prove they know the PIN.
        encrypted_challenge: Vec<u8>,
        // Sender also sends their own challenge for mutual auth
        my_challenge: [u8; 16],
    },
    AuthResponse {
        // Reply to the sender's challenge
        encrypted_challenge: Vec<u8>,
    },

    // Sync Coordination
    SyncManifest {
        manifest: SyncManifest,
    },

    // Data Transfer
    DomainSnapshot {
        domain: SyncDomain,
        payload: Vec<u8>,
    },

    // Control
    SyncComplete,
    StatusUpdate(String),
    FileTransfer {
        path: String,
        content: Vec<u8>,
    },
    Disconnect,
    Error(String),
}

fn default_protocol_version() -> u32 {
    1
}
