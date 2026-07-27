use chacha20poly1305::{
    aead::{Aead, KeyInit},
    ChaCha20Poly1305, Key, Nonce,
};
use futures::{SinkExt, StreamExt};
use rand::{thread_rng, RngCore};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use tauri::{AppHandle, Emitter, Manager};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, Mutex, RwLock};
use tokio_util::codec::Framed;

use crate::sync::codec::P2PCodec;
use crate::sync::db as sync_db;
use crate::sync::protocol::{ChangeOp, DomainPlan, P2PMessage, SyncDomain};
use crate::utils::{log_error, log_info, log_warn};

const PROTOCOL_VERSION: u32 = 12;
const MANIFEST_MIN_PROTOCOL: u32 = 10;
const READY_MIN_PROTOCOL: u32 = 11;
const MAX_PUSH_BATCH_BYTES: u64 = 4 * 1024 * 1024;
const MAX_PUSH_BATCH_COUNT: usize = 2000;
const ASSET_CHUNK_PROTOCOL: u32 = 11;
const ASSET_CHUNK_SIZE: usize = 256 * 1024;

struct PendingAssetFile {
    path: String,
    content_hash: String,
}

struct PendingAssetBatch {
    changes: Vec<crate::sync::protocol::ChangeRecord>,
    expected_files: HashMap<String, PendingAssetFile>,
    received_entity_ids: HashSet<String>,
    last_change_id: i64,
    bytes_received: u64,
    payload_bytes: u64,
    delete_count: u64,
}

struct PendingIncomingAsset {
    entity_id: String,
    path: String,
    content_hash: String,
    total_bytes: u64,
    bytes_received: u64,
    content: Vec<u8>,
}

fn derive_key(pin: &str, salt: &[u8]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new_derive_key("lettuce_sync_v1");
    hasher.update(salt);
    hasher.update(pin.as_bytes());
    let mut output = [0u8; 32];
    hasher.finalize_xof().fill(&mut output);
    output
}

fn version_mismatch_error(
    local_app_version: &str,
    peer_name: &str,
    peer_app_version: &str,
    peer_protocol_version: u32,
) -> Option<String> {
    if peer_app_version == local_app_version && peer_protocol_version == PROTOCOL_VERSION {
        return None;
    }

    let peer_label = if peer_name.is_empty() {
        "The other device"
    } else {
        peer_name
    };
    Some(format!(
        "Sync requires both devices to run exactly the same Lettuce AI version. {peer_label} uses app {peer_app_version} with protocol {peer_protocol_version}; this device uses app {local_app_version} with protocol {PROTOCOL_VERSION}. Update the mismatched device, fully restart both apps, and try again."
    ))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "status", content = "details")]
pub enum SyncStatus {
    Idle,
    DriverRunning {
        ip: String,
        port: u16,
        pin: String, // Added PIN to status
        clients: usize,
    },
    PassengerConnecting,
    PassengerConnected {
        driver_ip: String,
    },
    WaitingConfirmation {
        driver_ip: String,
    },
    Syncing {
        phase: String,
        progress: Option<f32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        current_domain: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        items_done: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        items_total: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bytes_done: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bytes_total: Option<u64>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        domain_progress: Vec<DomainProgress>,
    },
    Error {
        message: String,
    },
    PendingApproval {
        ip: String,
        device_name: String,
    },
    PendingSyncStart {
        ip: String,
        device_name: String,
    },
    SyncCompleted,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DomainProgress {
    pub domain: String,
    pub items_done: u64,
    pub items_total: u64,
}

#[derive(Debug, Default, Clone)]
struct SyncProgressTracker {
    plan: Vec<(SyncDomain, u64)>, // (domain, change_count)
    items_done_per_domain: HashMap<SyncDomain, u64>,
    total_items: u64,
    total_bytes: u64,
    bytes_done: u64,
}

impl SyncProgressTracker {
    fn from_plan(plan: &[DomainPlan]) -> Self {
        let mut total_items = 0u64;
        let mut total_bytes = 0u64;
        let mut entries = Vec::with_capacity(plan.len());
        for entry in plan {
            total_items += entry.change_count as u64;
            total_bytes += entry.estimated_bytes;
            entries.push((entry.domain, entry.change_count as u64));
        }
        Self {
            plan: entries,
            items_done_per_domain: HashMap::new(),
            total_items,
            total_bytes,
            bytes_done: 0,
        }
    }

    fn items_done(&self) -> u64 {
        self.items_done_per_domain.values().sum()
    }

    fn progress(&self) -> Option<f32> {
        if self.total_bytes > 0 {
            Some((self.bytes_done as f32 / self.total_bytes as f32).clamp(0.0, 1.0))
        } else if self.total_items > 0 {
            Some((self.items_done() as f32 / self.total_items as f32).clamp(0.0, 1.0))
        } else {
            None
        }
    }

    fn record(&mut self, domain: SyncDomain, items: u64, bytes: u64) {
        *self.items_done_per_domain.entry(domain).or_insert(0) += items;
        self.bytes_done = self.bytes_done.saturating_add(bytes);
    }

    fn record_items(&mut self, domain: SyncDomain, items: u64) {
        *self.items_done_per_domain.entry(domain).or_insert(0) += items;
    }

    fn record_bytes(&mut self, bytes: u64) {
        self.bytes_done = self.bytes_done.saturating_add(bytes);
    }

    fn breakdown(&self) -> Vec<DomainProgress> {
        self.plan
            .iter()
            .map(|(d, total)| DomainProgress {
                domain: domain_label(*d).to_string(),
                items_done: *self.items_done_per_domain.get(d).unwrap_or(&0),
                items_total: *total,
            })
            .collect()
    }

    fn syncing_status(&self, phase: String, current_domain: Option<SyncDomain>) -> SyncStatus {
        SyncStatus::Syncing {
            phase,
            progress: self.progress(),
            current_domain: current_domain.map(|d| domain_label(d).to_string()),
            items_done: Some(self.items_done()),
            items_total: Some(self.total_items),
            bytes_done: Some(self.bytes_done),
            bytes_total: Some(self.total_bytes),
            domain_progress: self.breakdown(),
        }
    }
}

fn syncing(phase: impl Into<String>) -> SyncStatus {
    SyncStatus::Syncing {
        phase: phase.into(),
        progress: None,
        current_domain: None,
        items_done: None,
        items_total: None,
        bytes_done: None,
        bytes_total: None,
        domain_progress: Vec::new(),
    }
}

fn domain_label(domain: SyncDomain) -> &'static str {
    match domain {
        SyncDomain::Core => "Core",
        SyncDomain::Tts => "Voices",
        SyncDomain::Lorebooks => "Lorebooks",
        SyncDomain::Characters => "Characters",
        SyncDomain::Groups => "Groups",
        SyncDomain::Sessions => "Sessions",
        SyncDomain::Messages => "Messages",
        SyncDomain::Assets => "Assets",
    }
}

pub struct SyncManagerState {
    pub status: RwLock<SyncStatus>,
    shutdown_tx: Mutex<Option<broadcast::Sender<()>>>,
    pub pending_approvals:
        RwLock<std::collections::HashMap<String, tokio::sync::oneshot::Sender<bool>>>,
    pub pending_starts: RwLock<std::collections::HashMap<String, tokio::sync::oneshot::Sender<()>>>,
    pub pin: RwLock<Option<String>>, // Added PIN storage
}

impl Default for SyncManagerState {
    fn default() -> Self {
        Self {
            status: RwLock::new(SyncStatus::Idle),
            shutdown_tx: Mutex::new(None),
            pending_approvals: RwLock::new(std::collections::HashMap::new()),
            pending_starts: RwLock::new(std::collections::HashMap::new()),
            pin: RwLock::new(None),
        }
    }
}

impl SyncManagerState {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn set_status(&self, app: &AppHandle, status: SyncStatus) {
        *self.status.write().await = status.clone();
        let _ = app.emit("sync-status-changed", status);
    }

    pub async fn clear_pending(&self) {
        self.pending_approvals.write().await.clear();
        self.pending_starts.write().await.clear();
    }
}

async fn set_driver_running_status(app: &AppHandle, state: &SyncManagerState, port: u16) {
    let my_ip = crate::utils::get_local_ip().unwrap_or_else(|_| "0.0.0.0".to_string());
    let pin = state.pin.read().await.clone().unwrap_or_default();
    state
        .set_status(
            app,
            SyncStatus::DriverRunning {
                ip: my_ip,
                port,
                pin,
                clients: 0,
            },
        )
        .await;
}

// Driver Logic (Host)
pub async fn start_driver(app: AppHandle, _port: u16) -> Result<String, String> {
    let state = app.state::<SyncManagerState>();
    let mut current_tx = state.shutdown_tx.lock().await;
    if current_tx.is_some() {
        return Err(crate::utils::err_msg(
            module_path!(),
            line!(),
            "Sync service is already running",
        ));
    }

    let listener = TcpListener::bind("0.0.0.0:0")
        .await
        .map_err(|e| crate::utils::err_to_string(module_path!(), line!(), e))?;
    let port = listener
        .local_addr()
        .map_err(|e| crate::utils::err_to_string(module_path!(), line!(), e))?
        .port();

    let my_ip = crate::utils::get_local_ip().unwrap_or_else(|_| "0.0.0.0".into());

    // Generate PIN
    let pin: String = (0..6)
        .map(|_| {
            let mut byte = [0u8; 1];
            thread_rng().fill_bytes(&mut byte);
            (byte[0] % 10).to_string()
        })
        .collect();

    let (tx, mut rx) = broadcast::channel(1);
    let task_tx = tx.clone();
    *current_tx = Some(tx);
    *state.pin.write().await = Some(pin.clone());
    state.clear_pending().await;

    let app_clone = app.clone();

    state
        .set_status(
            &app,
            SyncStatus::DriverRunning {
                ip: my_ip,
                port,
                pin: pin.clone(),
                clients: 0,
            },
        )
        .await;

    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = rx.recv() => {
                    break;
                }
                res = listener.accept() => {
                    match res {
                        Ok((stream, remote_addr)) => {
                            let app_inner = app_clone.clone();
                            tokio::spawn(async move {
                                if let Err(e) = handle_driver_connection(app_inner.clone(), stream, remote_addr, port).await {
                                    log_error(&app_inner, "sync_driver", format!("Driver connection error: {}", e));
                                }
                            });
                        }
                        Err(e) => {
                            log_error(&app_clone, "sync_driver", format!("Listener accept error: {}", e));
                        }
                    }
                }
            }
        }
        let state = app_clone.state::<SyncManagerState>();
        state.clear_pending().await;
        let should_cleanup = {
            let mut current_tx = state.shutdown_tx.lock().await;
            match current_tx.as_ref() {
                Some(current) if current.same_channel(&task_tx) => {
                    current_tx.take();
                    true
                }
                None => true,
                _ => false,
            }
        };
        if should_cleanup {
            *state.pin.write().await = None;
            state.set_status(&app_clone, SyncStatus::Idle).await;
        }
    });

    Ok(pin)
}

async fn handle_driver_connection(
    app: AppHandle,
    stream: TcpStream,
    _addr: SocketAddr,
    port: u16,
) -> Result<(), String> {
    let remote_ip = stream
        .peer_addr()
        .map(|a| a.ip().to_string())
        .unwrap_or_else(|_| "unknown".to_string());
    let mut framed = Framed::new(stream, P2PCodec::new());

    // Security Handshake
    let state = app.state::<SyncManagerState>();

    let pin = state
        .pin
        .read()
        .await
        .clone()
        .ok_or("Driver not running or no PIN")?;

    let mut salt = [0u8; 16];
    let mut challenge = [0u8; 16];
    thread_rng().fill_bytes(&mut salt);
    thread_rng().fill_bytes(&mut challenge);
    let device_id = {
        let conn = crate::storage_manager::db::open_db(&app)?;
        sync_db::get_or_create_local_device_id(&conn)?
    };

    // Send Handshake
    framed
        .send(P2PMessage::Handshake {
            protocol_version: PROTOCOL_VERSION,
            app_version: app.package_info().version.to_string(),
            device_name: whoami::devicename(),
            device_id,
            salt,
            challenge,
        })
        .await
        .map_err(|e| crate::utils::err_to_string(module_path!(), line!(), e))?;

    // Wait for AuthRequest
    let (encrypted_challenge, their_challenge) = match framed.next().await {
        Some(Ok(P2PMessage::AuthRequest {
            encrypted_challenge,
            my_challenge,
        })) => (encrypted_challenge, my_challenge),
        Some(Ok(msg)) => {
            return Err(crate::utils::err_msg(
                module_path!(),
                line!(),
                format!("Expected AuthRequest, got {:?}", msg),
            ))
        }
        Some(Err(e)) => return Err(e.to_string()),
        None => {
            return Err(crate::utils::err_msg(
                module_path!(),
                line!(),
                "Connection closed during handshake",
            ))
        }
    };

    // Verify
    let key = derive_key(&pin, &salt);
    let cipher = ChaCha20Poly1305::new(&Key::from(key));

    // Try to decrypt their response to our challenge
    // The encrypted_challenge should be [Nonce 12][Ciphertext] if we follow P2PCodec pattern,
    // BUT P2PCodec is NOT encryption-enabled yet.
    // The other side manually encrypted the blob.
    // We assume they prepended the nonce.
    if encrypted_challenge.len() < 12 {
        return Err(crate::utils::err_msg(
            module_path!(),
            line!(),
            "Auth challenge too short",
        ));
    }
    let mut n_bytes = [0u8; 12];
    n_bytes.copy_from_slice(&encrypted_challenge[..12]);
    let nonce = Nonce::from(n_bytes);
    let ciphertext = &encrypted_challenge[12..];

    let decrypted = cipher
        .decrypt(&nonce, ciphertext)
        .map_err(|_| "Auth failed (bad PIN)".to_string())?;

    if decrypted != challenge {
        return Err(crate::utils::err_msg(
            module_path!(),
            line!(),
            "Auth failed (challenge mismatch)",
        ));
    }

    // Auth Success!
    // Encrypt their challenge to prove we are the driver
    let mut response_nonce_bytes = [0u8; 12];
    thread_rng().fill_bytes(&mut response_nonce_bytes);
    let response_nonce = Nonce::from(response_nonce_bytes);
    let response_ciphertext = cipher
        .encrypt(&response_nonce, their_challenge.as_ref())
        .map_err(|e| crate::utils::err_to_string(module_path!(), line!(), e))?;

    let mut response_payload = Vec::new();
    response_payload.extend_from_slice(&response_nonce_bytes);
    response_payload.extend_from_slice(&response_ciphertext);

    framed
        .send(P2PMessage::AuthResponse {
            encrypted_challenge: response_payload,
        })
        .await
        .map_err(|e| crate::utils::err_to_string(module_path!(), line!(), e))?;

    // Enable Encryption for session
    framed.codec_mut().set_key(&key);

    // Wait for Handshake (WAIT, we already did Handshake)
    // The previous code waited for Handshake here to get device_name.
    // But we already got Handshake? No, wait.
    // The protocol was:
    // 1. Driver sends Handshake (Server -> Client)
    // 2. Client sends Handshake (Client -> Server)
    //
    // MY NEW PROTOCOL:
    // 1. Driver sends Handshake { salt, challenge }
    // 2. Client sends AuthRequest { enc_challenge, my_challenge }
    // 3. Driver sends AuthResponse
    //
    // Where does the Client send ITS device name?
    // The original code had Client send Handshake.
    // We should piggyback Device Name on AuthRequest? Or have Client send Handshake FIRST?
    // If Client sends Handshake first, it can include device name.
    // But Driver needs to send Salt/Challenge FIRST for Client to derive key.
    //
    // Adjusted Flow:
    // 1. Client connects.
    // 2. Driver sends Handshake { version, name, salt, challenge }.
    // 3. Client receives. Derives key.
    // 4. Client sends AuthRequest { enc_challenge, my_challenge, device_name? }.
    //    Ah, I didn't add `device_name` to `AuthRequest`.
    //    I should have.
    //    OR, Client sends `Handshake` packet *after* Auth? But that would be encrypted.
    //    That works! Once encrypted, Client sends `Handshake` with device name.
    //    Then Driver knows device name.
    //
    // So:
    // Driver: ... Auth Success, Enable Encryption, Send AuthResponse.
    // Client: ... Receive AuthResponse, Verify, Enable Encryption.
    // THEN Client sends `Handshake` (Encrypted).
    // Driver expects `Handshake`.

    // So after `set_key`, Driver should wait for `Handshake`.
    let (device_name, peer_device_id, peer_app_version, peer_protocol_version) =
        match framed.next().await {
            Some(Ok(P2PMessage::Handshake {
                device_name,
                device_id,
                app_version,
                protocol_version,
                ..
            })) => (device_name, device_id, app_version, protocol_version),
            Some(Ok(msg)) => {
                return Err(crate::utils::err_msg(
                    module_path!(),
                    line!(),
                    format!("Expected Encrypted Handshake, got {:?}", msg),
                ))
            }
            Some(Err(e)) => return Err(e.to_string()),
            None => {
                return Err(crate::utils::err_msg(
                    module_path!(),
                    line!(),
                    "Connection closed after auth",
                ))
            }
        };

    if let Some(message) = version_mismatch_error(
        &app.package_info().version.to_string(),
        &device_name,
        &peer_app_version,
        peer_protocol_version,
    ) {
        framed.send(P2PMessage::Error(message.clone())).await.ok();
        return Err(crate::utils::err_msg(module_path!(), line!(), message));
    }

    // Approval Check
    let (tx, rx) = tokio::sync::oneshot::channel();

    let state = app.state::<SyncManagerState>();
    {
        state
            .pending_approvals
            .write()
            .await
            .insert(remote_ip.clone(), tx);
    }

    state
        .set_status(
            &app,
            SyncStatus::PendingApproval {
                ip: remote_ip.clone(),
                device_name: device_name.clone(),
            },
        )
        .await;

    let approval_result = rx
        .await
        .map_err(|_| "Connection approval was cancelled".to_string());
    state.pending_approvals.write().await.remove(&remote_ip);

    match approval_result {
        Ok(true) => {
            log_info(
                &app,
                "sync_driver",
                format!("Connection from {} approved", remote_ip),
            );
        }
        Ok(false) => {
            set_driver_running_status(&app, state.inner(), port).await;
            return Err(crate::utils::err_msg(
                module_path!(),
                line!(),
                "Connection rejected by host",
            ));
        }
        Err(reason) => {
            set_driver_running_status(&app, state.inner(), port).await;
            return Err(crate::utils::err_msg(module_path!(), line!(), reason));
        }
    }

    let (start_tx, start_rx) = tokio::sync::oneshot::channel();
    {
        state
            .pending_starts
            .write()
            .await
            .insert(remote_ip.clone(), start_tx);
    }
    state
        .set_status(
            &app,
            SyncStatus::PendingSyncStart {
                ip: remote_ip.clone(),
                device_name: device_name.clone(),
            },
        )
        .await;

    let start_result = start_rx
        .await
        .map_err(|_| "Sync start was cancelled".to_string());
    state.pending_starts.write().await.remove(&remote_ip);

    if let Err(reason) = start_result {
        set_driver_running_status(&app, state.inner(), port).await;
        return Err(crate::utils::err_msg(module_path!(), line!(), reason));
    }

    if peer_protocol_version >= READY_MIN_PROTOCOL {
        framed
            .send(P2PMessage::Ready)
            .await
            .map_err(|e| crate::utils::err_to_string(module_path!(), line!(), e))?;
    }

    // Main Loop
    while let Some(msg) = framed.next().await {
        match msg {
            Ok(P2PMessage::AdvertiseCursors { cursors }) => {
                handle_advertise_cursors(
                    &app,
                    &mut framed,
                    &peer_device_id,
                    cursors,
                    peer_protocol_version,
                )
                .await?;
            }
            Ok(P2PMessage::Disconnect) => break,
            Ok(other) => log_warn(
                &app,
                "sync_driver",
                format!("Driver received unexpected message: {:?}", other),
            ),
            Err(e) => return Err(e.to_string()),
        }
    }

    Ok(())
}

async fn handle_advertise_cursors(
    app: &AppHandle,
    framed: &mut Framed<TcpStream, P2PCodec>,
    _peer_device_id: &str,
    passenger_cursors: crate::sync::protocol::CursorSet,
    peer_protocol_version: u32,
) -> Result<(), String> {
    let mut conn = crate::storage_manager::db::open_db(app)?;
    sync_db::rebuild_change_log(app, &mut conn)?;

    log_info(app, "sync_driver", "Preparing sync manifest");

    // Pre-fetch all changes per domain to build the manifest and to estimate bytes.
    let mut domain_changes: Vec<(SyncDomain, Vec<crate::sync::protocol::ChangeRecord>, u64)> =
        Vec::new();
    for cursor in passenger_cursors.cursors {
        let changes = sync_db::fetch_changes_since(&conn, cursor.domain, cursor.last_change_id)?;
        if changes.is_empty() {
            continue;
        }
        let mut bytes: u64 = changes.iter().map(|c| c.payload.len() as u64).sum();
        if cursor.domain == SyncDomain::Assets {
            bytes = bytes.saturating_add(estimate_asset_bytes(app, &changes));
        }
        domain_changes.push((cursor.domain, changes, bytes));
    }

    let plan: Vec<DomainPlan> = domain_changes
        .iter()
        .map(|(domain, changes, bytes)| DomainPlan {
            domain: *domain,
            change_count: changes.len() as u32,
            estimated_bytes: *bytes,
        })
        .collect();

    let total_changes: u32 = plan.iter().map(|p| p.change_count).sum();
    let total_bytes: u64 = plan.iter().map(|p| p.estimated_bytes).sum();

    let mut tracker = SyncProgressTracker::from_plan(&plan);
    let state = app.state::<SyncManagerState>();

    if peer_protocol_version >= MANIFEST_MIN_PROTOCOL {
        framed
            .send(P2PMessage::SyncManifest {
                plan,
                total_changes,
                total_bytes,
            })
            .await
            .map_err(|e| crate::utils::err_to_string(module_path!(), line!(), e))?;
    }

    state
        .set_status(
            app,
            tracker.syncing_status("Preparing transfer".into(), None),
        )
        .await;

    log_info(
        app,
        "sync_driver",
        format!(
            "Sending {} changes ({} bytes) across {} domains",
            total_changes,
            total_bytes,
            domain_changes.len()
        ),
    );

    for (domain, mut changes, _bytes) in domain_changes {
        let phase = sync_status_text(domain).to_string();
        state
            .set_status(app, tracker.syncing_status(phase.clone(), Some(domain)))
            .await;

        framed
            .send(P2PMessage::StatusUpdate(phase.clone()))
            .await
            .map_err(|e| crate::utils::err_to_string(module_path!(), line!(), e))?;

        if domain == SyncDomain::Assets {
            let prepared_asset_contents = prepare_asset_changes_for_transfer(app, &mut changes)?;

            framed
                .send(P2PMessage::PushChanges {
                    domain,
                    changes: changes.clone(),
                })
                .await
                .map_err(|e| crate::utils::err_to_string(module_path!(), line!(), e))?;

            let payload_bytes: u64 = changes.iter().map(|c| c.payload.len() as u64).sum();
            let delete_count = changes
                .iter()
                .filter(|change| change.op == ChangeOp::Delete)
                .count() as u64;

            tracker.record(domain, delete_count, payload_bytes);
            state
                .set_status(app, tracker.syncing_status(phase.clone(), Some(domain)))
                .await;

            send_asset_change_contents(
                app,
                framed,
                &changes,
                &mut tracker,
                state.inner(),
                phase.as_str(),
                peer_protocol_version,
                &prepared_asset_contents,
            )
            .await?;
            let last_change_id = changes.last().map(|change| change.change_id).unwrap_or(0);
            framed
                .send(P2PMessage::AssetBatchComplete { last_change_id })
                .await
                .map_err(|e| crate::utils::err_to_string(module_path!(), line!(), e))?;
        } else {
            let mut start = 0;
            while start < changes.len() {
                let mut end = start;
                let mut batch_bytes: u64 = 0;
                while end < changes.len() && end - start < MAX_PUSH_BATCH_COUNT {
                    let payload_len = changes[end].payload.len() as u64;
                    if end > start && batch_bytes + payload_len > MAX_PUSH_BATCH_BYTES {
                        break;
                    }
                    batch_bytes += payload_len;
                    end += 1;
                }

                framed
                    .send(P2PMessage::PushChanges {
                        domain,
                        changes: changes[start..end].to_vec(),
                    })
                    .await
                    .map_err(|e| crate::utils::err_to_string(module_path!(), line!(), e))?;

                tracker.record(domain, (end - start) as u64, batch_bytes);
                state
                    .set_status(app, tracker.syncing_status(phase.clone(), Some(domain)))
                    .await;

                start = end;
            }
        }
    }

    framed
        .send(P2PMessage::SyncComplete)
        .await
        .map_err(|e| crate::utils::err_to_string(module_path!(), line!(), e))?;

    state
        .set_status(
            app,
            tracker.syncing_status("Waiting for receiver confirmation".into(), None),
        )
        .await;

    match framed.next().await {
        Some(Ok(P2PMessage::SyncApplied)) => {
            state.set_status(app, SyncStatus::SyncCompleted).await;
        }
        Some(Ok(P2PMessage::Disconnect)) => {
            return Err(crate::utils::err_msg(
                module_path!(),
                line!(),
                "Passenger disconnected before confirming sync completion",
            ));
        }
        Some(Ok(other)) => {
            return Err(crate::utils::err_msg(
                module_path!(),
                line!(),
                format!("Expected SyncApplied, got {:?}", other),
            ));
        }
        Some(Err(e)) => {
            return Err(crate::utils::err_to_string(module_path!(), line!(), e));
        }
        None => {
            return Err(crate::utils::err_msg(
                module_path!(),
                line!(),
                "Connection closed before passenger confirmed sync completion",
            ));
        }
    }
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

    Ok(())
}

pub async fn connect_as_passenger(
    app: AppHandle,
    ip: String,
    port: u16,
    pin: String,
) -> Result<(), String> {
    let state = app.state::<SyncManagerState>();
    let mut current_tx = state.shutdown_tx.lock().await;
    if current_tx.is_some() {
        return Err(crate::utils::err_msg(
            module_path!(),
            line!(),
            "Sync service is already running",
        ));
    }

    let addr = format!("{}:{}", ip, port);
    let stream = TcpStream::connect(&addr)
        .await
        .map_err(|e| crate::utils::err_to_string(module_path!(), line!(), e))?;

    let (tx, mut rx) = broadcast::channel(1);
    *current_tx = Some(tx);

    state
        .set_status(&app, SyncStatus::PassengerConnecting)
        .await;

    let app_clone = app.clone();
    tokio::spawn(async move {
        // Re-acquire state here to avoid lifetime issues
        let state = app_clone.state::<SyncManagerState>();

        let result = run_passenger_session(app_clone.clone(), stream, &mut rx, pin).await;

        *state.shutdown_tx.lock().await = None;

        match result {
            Err(e) => {
                state
                    .set_status(
                        &app_clone,
                        SyncStatus::Error {
                            message: e.to_string(),
                        },
                    )
                    .await;
            }
            Ok(()) => {
                state
                    .set_status(&app_clone, SyncStatus::SyncCompleted)
                    .await;
            }
        }
    });

    Ok(())
}

fn finalize_pending_domain(
    app: &AppHandle,
    conn: &mut crate::storage_manager::db::DbConnection,
    driver_device_id: &str,
    pending_domain: &mut Option<(SyncDomain, i64)>,
) -> Result<(), String> {
    if let Some((domain, last_change_id)) = pending_domain.take() {
        sync_db::materialize_domain(conn, domain)?;
        if last_change_id > 0 {
            sync_db::record_peer_cursor(conn, driver_device_id, domain, last_change_id)?;
        }
        log_info(app, "sync_passenger", format!("Materialized {:?}", domain));
    }
    Ok(())
}

async fn run_passenger_session(
    app: AppHandle,
    stream: TcpStream,
    stop_signal: &mut broadcast::Receiver<()>,
    pin: String,
) -> Result<(), String> {
    let mut framed = Framed::new(stream, P2PCodec::new());
    let state = app.state::<SyncManagerState>();

    // 1. Wait for Handshake from Driver (contains Salt + Challenge)
    let (
        salt,
        challenge,
        driver_device_id,
        driver_device_name,
        driver_app_version,
        driver_protocol_version,
    ) = match framed.next().await {
        Some(Ok(P2PMessage::Handshake {
            salt,
            challenge,
            device_id,
            device_name,
            app_version,
            protocol_version,
            ..
        })) => (
            salt,
            challenge,
            device_id,
            device_name,
            app_version,
            protocol_version,
        ),
        Some(Ok(msg)) => {
            return Err(crate::utils::err_msg(
                module_path!(),
                line!(),
                format!("Expected Handshake, got {:?}", msg),
            ))
        }
        Some(Err(e)) => return Err(e.to_string()),
        None => {
            return Err(crate::utils::err_msg(
                module_path!(),
                line!(),
                "Connection closed during handshake",
            ))
        }
    };

    if let Some(message) = version_mismatch_error(
        &app.package_info().version.to_string(),
        &driver_device_name,
        &driver_app_version,
        driver_protocol_version,
    ) {
        return Err(crate::utils::err_msg(module_path!(), line!(), message));
    }

    // 2. Derive Key & Encrypt Challenge & Send AuthRequest
    let key = derive_key(&pin, &salt);
    let cipher = ChaCha20Poly1305::new(&Key::from(key));

    let mut my_challenge = [0u8; 16];
    thread_rng().fill_bytes(&mut my_challenge);

    let mut nonce_bytes = [0u8; 12];
    thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from(nonce_bytes);

    // Encrypt the Driver's challenge to prove we know the PIN
    let encrypted_challenge_ciphertext = cipher
        .encrypt(&nonce, challenge.as_ref())
        .map_err(|e| crate::utils::err_to_string(module_path!(), line!(), e))?;

    let mut encrypted_challenge = Vec::new();
    encrypted_challenge.extend_from_slice(&nonce_bytes);
    encrypted_challenge.extend_from_slice(&encrypted_challenge_ciphertext);

    framed
        .send(P2PMessage::AuthRequest {
            encrypted_challenge,
            my_challenge,
        })
        .await
        .map_err(|e| crate::utils::err_to_string(module_path!(), line!(), e))?;

    // 3. Wait for AuthResponse
    let encrypted_response = match framed.next().await {
        Some(Ok(P2PMessage::AuthResponse {
            encrypted_challenge,
        })) => encrypted_challenge,
        Some(Ok(msg)) => {
            return Err(crate::utils::err_msg(
                module_path!(),
                line!(),
                format!("Expected AuthResponse, got {:?}", msg),
            ))
        }
        Some(Err(e)) => return Err(e.to_string()),
        None => {
            return Err(crate::utils::err_msg(
                module_path!(),
                line!(),
                "Connection closed during auth",
            ))
        }
    };

    // 4. Verify Driver's response to OUR challenge
    if encrypted_response.len() < 12 {
        return Err(crate::utils::err_msg(
            module_path!(),
            line!(),
            "Auth response too short",
        ));
    }
    let mut n_bytes = [0u8; 12];
    n_bytes.copy_from_slice(&encrypted_response[..12]);
    let resp_nonce = Nonce::from(n_bytes);
    let resp_ciphertext = &encrypted_response[12..];

    let decrypted_my_challenge = cipher
        .decrypt(&resp_nonce, resp_ciphertext)
        .map_err(|_| "Auth failed (Driver Sent Bad Response)".to_string())?;

    if decrypted_my_challenge != my_challenge {
        return Err(crate::utils::err_msg(
            module_path!(),
            line!(),
            "Auth failed (response mismatch)",
        ));
    }

    // Auth Success! Enable Encryption.
    framed.codec_mut().set_key(&key);

    // 5. Send our Handshake (Encrypted) with Device Name
    let mut conn = crate::storage_manager::db::open_db(&app)?;
    let local_device_id = sync_db::get_or_create_local_device_id(&conn)?;
    framed
        .send(P2PMessage::Handshake {
            protocol_version: PROTOCOL_VERSION,
            app_version: app.package_info().version.to_string(),
            device_name: whoami::devicename(),
            device_id: local_device_id,
            salt: [0u8; 16],      // Not used post-auth
            challenge: [0u8; 16], // Not used post-auth
        })
        .await
        .map_err(|e| crate::utils::err_to_string(module_path!(), line!(), e))?;

    state
        .set_status(
            &app,
            SyncStatus::WaitingConfirmation {
                driver_ip: "unknown".into(),
            },
        )
        .await;

    if sync_db::local_device_has_onboarded(&conn) {
        sync_db::rebuild_change_log(&app, &mut conn)?;
    } else {
        log_info(
            &app,
            "sync_passenger",
            "Fresh install detected; skipping local change log seeding so synced data wins conflicts",
        );
    }
    if driver_protocol_version >= READY_MIN_PROTOCOL {
        match framed.next().await {
            Some(Ok(P2PMessage::Ready)) => {}
            Some(Ok(P2PMessage::Error(message))) => {
                return Err(crate::utils::err_msg(module_path!(), line!(), message))
            }
            Some(Ok(P2PMessage::Disconnect)) => {
                return Err(crate::utils::err_msg(
                    module_path!(),
                    line!(),
                    "Driver disconnected before sending Ready",
                ))
            }
            Some(Ok(other)) => {
                return Err(crate::utils::err_msg(
                    module_path!(),
                    line!(),
                    format!("Expected Ready, got {:?}", other),
                ))
            }
            Some(Err(e)) => return Err(e.to_string()),
            None => {
                return Err(crate::utils::err_msg(
                    module_path!(),
                    line!(),
                    "Connection closed before Ready",
                ))
            }
        }
    }

    let cursors = sync_db::load_peer_cursors(&conn, &driver_device_id)?;
    framed
        .send(P2PMessage::AdvertiseCursors { cursors })
        .await
        .map_err(|e| crate::utils::err_to_string(module_path!(), line!(), e))?;

    let mut pending_asset_batch: Option<PendingAssetBatch> = None;
    let mut pending_asset: Option<PendingIncomingAsset> = None;
    let mut tracker: Option<SyncProgressTracker> = None;
    let mut latest_phase: String = "Connecting".into();
    let mut pending_domain: Option<(SyncDomain, i64)> = None;

    // Client Loop
    loop {
        tokio::select! {
            _ = stop_signal.recv() => {
                framed.send(P2PMessage::Disconnect).await.ok();
                break;
            }
            msg = framed.next() => {
                match msg {
                    Some(Ok(P2PMessage::SyncManifest { plan, total_changes, total_bytes })) => {
                        log_info(&app, "sync_passenger", format!(
                            "Received manifest: {} changes, {} bytes across {} domains",
                            total_changes, total_bytes, plan.len()
                        ));
                        let new_tracker = SyncProgressTracker::from_plan(&plan);
                        latest_phase = "Preparing transfer".into();
                        state.set_status(&app, new_tracker.syncing_status(latest_phase.clone(), None)).await;
                        tracker = Some(new_tracker);
                    }
                    Some(Ok(P2PMessage::PushChanges { domain, changes })) => {
                        log_info(&app, "sync_passenger", format!("Received {} changes for {:?}", changes.len(), domain));
                        if pending_domain.as_ref().is_some_and(|(prev, _)| *prev != domain) {
                            if let Err(e) = finalize_pending_domain(&app, &mut conn, &driver_device_id, &mut pending_domain) {
                                log_error(&app, "sync_passenger", format!("Failed to materialize pending domain: {}", e));
                                framed.send(P2PMessage::Disconnect).await.ok();
                                return Err(e);
                            }
                        }
                        latest_phase = sync_status_text(domain).to_string();
                        if let Some(t) = tracker.as_ref() {
                            state.set_status(&app, t.syncing_status(latest_phase.clone(), Some(domain))).await;
                        } else {
                            state.set_status(&app, syncing(latest_phase.clone())).await;
                        }
                        let last_change_id = changes.last().map(|change| change.change_id).unwrap_or(0);
                        if domain == SyncDomain::Assets {
                            let payload_bytes: u64 = changes.iter().map(|c| c.payload.len() as u64).sum();
                            let delete_count: u64 = changes
                                .iter()
                                .filter(|change| change.op == ChangeOp::Delete)
                                .count() as u64;
                            if pending_asset_batch.is_some() {
                                return Err(crate::utils::err_msg(
                                    module_path!(),
                                    line!(),
                                    "Received a new asset batch before the previous batch completed",
                                ));
                            }

                            let mut expected_files = HashMap::new();
                            for change in &changes {
                                if change.op != ChangeOp::Upsert {
                                    continue;
                                }

                                let asset: sync_db::AssetRecord = bincode::deserialize(&change.payload)
                                    .map_err(|e| crate::utils::err_to_string(module_path!(), line!(), e))?;
                                expected_files.insert(
                                    change.entity_id.clone(),
                                    PendingAssetFile {
                                        path: asset.path,
                                        content_hash: asset.content_hash,
                                    },
                                );
                            }

                            pending_asset_batch = Some(PendingAssetBatch {
                                changes,
                                expected_files,
                                received_entity_ids: HashSet::new(),
                                last_change_id,
                                bytes_received: 0,
                                payload_bytes,
                                delete_count,
                            });
                            if let Some(t) = tracker.as_mut() {
                                let pending_batch = pending_asset_batch.as_ref().expect("asset batch just set");
                                t.record(SyncDomain::Assets, pending_batch.delete_count, pending_batch.payload_bytes);
                                state.set_status(&app, t.syncing_status(latest_phase.clone(), Some(SyncDomain::Assets))).await;
                            }
                            continue;
                        }
                        if let Err(e) = sync_db::append_change_batch(&mut conn, domain, &changes) {
                            log_error(&app, "sync_passenger", format!("Failed to apply domain {:?}: {}", domain, e));
                            framed.send(P2PMessage::Disconnect).await.ok();
                            return Err(crate::utils::err_msg(
                                module_path!(),
                                line!(),
                                format!("Sync aborted: failed to apply {:?} changes: {}", domain, e),
                            ));
                        }
                        let pending_cursor = pending_domain
                            .filter(|(prev, _)| *prev == domain)
                            .map(|(_, cursor)| cursor)
                            .unwrap_or(0);
                        pending_domain = Some((domain, pending_cursor.max(last_change_id)));
                        let bytes: u64 = changes.iter().map(|c| c.payload.len() as u64).sum();
                        if let Some(t) = tracker.as_mut() {
                            t.record(domain, changes.len() as u64, bytes);
                            state.set_status(&app, t.syncing_status(latest_phase.clone(), Some(domain))).await;
                        }
                    }
                    Some(Ok(P2PMessage::StatusUpdate(msg))) => {
                        log_info(&app, "sync_passenger", format!("StatusUpdate: {}", msg));
                        latest_phase = msg.clone();
                        if let Some(t) = tracker.as_ref() {
                            state.set_status(&app, t.syncing_status(msg, None)).await;
                        } else {
                            state.set_status(&app, syncing(msg)).await;
                        }
                    }
                    Some(Ok(P2PMessage::AssetContent { entity_id, path, content_hash, content })) => {
                        log_info(&app, "sync_passenger", format!("Received asset content: {}", path));
                        let pending_batch = pending_asset_batch.as_mut().ok_or_else(|| {
                            crate::utils::err_msg(
                                module_path!(),
                                line!(),
                                format!("Received unexpected asset content for {}", path),
                            )
                        })?;
                        let pending_file = pending_batch.expected_files.get(&entity_id).ok_or_else(|| {
                            crate::utils::err_msg(
                                module_path!(),
                                line!(),
                                format!("Received asset content for unknown entity {}", entity_id),
                            )
                        })?;
                        if pending_file.path != path {
                            return Err(crate::utils::err_msg(
                                module_path!(),
                                line!(),
                                format!(
                                    "Asset path mismatch for {}: expected {}, got {}",
                                    entity_id, pending_file.path, path
                                ),
                            ));
                        }
                        if pending_file.content_hash != content_hash {
                            return Err(crate::utils::err_msg(
                                module_path!(),
                                line!(),
                                format!(
                                    "Asset hash metadata mismatch for {}: expected {}, got {}",
                                    entity_id, pending_file.content_hash, content_hash
                                ),
                            ));
                        }
                        let actual_hash = blake3::hash(&content).to_hex().to_string();
                        if actual_hash != content_hash {
                            return Err(crate::utils::err_msg(
                                module_path!(),
                                line!(),
                                format!(
                                    "Received corrupted asset content for {}: expected {}, got {}",
                                    entity_id, content_hash, actual_hash
                                ),
                            ));
                        }
                        let content_len = content.len() as u64;
                        write_asset_path(&app, &path, &content).await?;
                        pending_batch.received_entity_ids.insert(entity_id);
                        pending_batch.bytes_received = pending_batch.bytes_received.saturating_add(content_len);
                        if let Some(t) = tracker.as_mut() {
                            t.record_bytes(content_len);
                            t.record_items(SyncDomain::Assets, 1);
                            state.set_status(&app, t.syncing_status(latest_phase.clone(), Some(SyncDomain::Assets))).await;
                        }
                    }
                    Some(Ok(P2PMessage::AssetContentStart { entity_id, path, content_hash, total_bytes })) => {
                        let pending_batch = pending_asset_batch.as_ref().ok_or_else(|| {
                            crate::utils::err_msg(
                                module_path!(),
                                line!(),
                                format!("Received unexpected asset start for {}", path),
                            )
                        })?;
                        let pending_file = pending_batch.expected_files.get(&entity_id).ok_or_else(|| {
                            crate::utils::err_msg(
                                module_path!(),
                                line!(),
                                format!("Received asset start for unknown entity {}", entity_id),
                            )
                        })?;
                        if pending_asset.is_some() {
                            return Err(crate::utils::err_msg(
                                module_path!(),
                                line!(),
                                "Received AssetContentStart while another asset is still being transferred",
                            ));
                        }
                        if pending_file.path != path || pending_file.content_hash != content_hash {
                            return Err(crate::utils::err_msg(
                                module_path!(),
                                line!(),
                                format!("Asset metadata mismatch for {}", entity_id),
                            ));
                        }
                        pending_asset = Some(PendingIncomingAsset {
                            entity_id,
                            path,
                            content_hash,
                            total_bytes,
                            bytes_received: 0,
                            content: Vec::with_capacity(total_bytes.min(8 * 1024 * 1024) as usize),
                        });
                    }
                    Some(Ok(P2PMessage::AssetContentChunk { entity_id, chunk })) => {
                        let current_asset = pending_asset.as_mut().ok_or_else(|| {
                            crate::utils::err_msg(
                                module_path!(),
                                line!(),
                                format!("Received asset chunk without active asset for {}", entity_id),
                            )
                        })?;
                        if current_asset.entity_id != entity_id {
                            return Err(crate::utils::err_msg(
                                module_path!(),
                                line!(),
                                format!(
                                    "Received asset chunk for {} while receiving {}",
                                    entity_id, current_asset.entity_id
                                ),
                            ));
                        }
                        current_asset.bytes_received = current_asset.bytes_received.saturating_add(chunk.len() as u64);
                        current_asset.content.extend_from_slice(&chunk);
                        pending_batch_bytes_received(&mut pending_asset_batch, chunk.len() as u64)?;
                        if let Some(t) = tracker.as_mut() {
                            t.record_bytes(chunk.len() as u64);
                            state.set_status(&app, t.syncing_status(latest_phase.clone(), Some(SyncDomain::Assets))).await;
                        }
                    }
                    Some(Ok(P2PMessage::AssetContentComplete { entity_id })) => {
                        let current_asset = pending_asset.take().ok_or_else(|| {
                            crate::utils::err_msg(
                                module_path!(),
                                line!(),
                                format!("Received asset completion without active asset for {}", entity_id),
                            )
                        })?;
                        if current_asset.entity_id != entity_id {
                            return Err(crate::utils::err_msg(
                                module_path!(),
                                line!(),
                                format!(
                                    "Received asset completion for {} while receiving {}",
                                    entity_id, current_asset.entity_id
                                ),
                            ));
                        }
                        if current_asset.bytes_received != current_asset.total_bytes {
                            return Err(crate::utils::err_msg(
                                module_path!(),
                                line!(),
                                format!(
                                    "Asset {} incomplete: received {} of {} bytes",
                                    current_asset.path, current_asset.bytes_received, current_asset.total_bytes
                                ),
                            ));
                        }
                        let actual_hash = blake3::hash(&current_asset.content).to_hex().to_string();
                        if actual_hash != current_asset.content_hash {
                            return Err(crate::utils::err_msg(
                                module_path!(),
                                line!(),
                                format!(
                                    "Received corrupted chunked asset content for {}: expected {}, got {}",
                                    current_asset.entity_id, current_asset.content_hash, actual_hash
                                ),
                            ));
                        }
                        write_asset_path(&app, &current_asset.path, &current_asset.content).await?;
                        let pending_batch = pending_asset_batch.as_mut().ok_or_else(|| {
                            crate::utils::err_msg(
                                module_path!(),
                                line!(),
                                format!("Completed asset {} without active batch", current_asset.entity_id),
                            )
                        })?;
                        pending_batch.received_entity_ids.insert(current_asset.entity_id);
                        if let Some(t) = tracker.as_mut() {
                            t.record_items(SyncDomain::Assets, 1);
                            state.set_status(&app, t.syncing_status(latest_phase.clone(), Some(SyncDomain::Assets))).await;
                        }
                    }
                    Some(Ok(P2PMessage::AssetBatchComplete { last_change_id })) => {
                        if pending_asset.is_some() {
                            return Err(crate::utils::err_msg(
                                module_path!(),
                                line!(),
                                "Received AssetBatchComplete while an asset file was still being transferred",
                            ));
                        }
                        let pending_batch = pending_asset_batch.take().ok_or_else(|| {
                            crate::utils::err_msg(
                                module_path!(),
                                line!(),
                                "Received AssetBatchComplete without an active asset batch",
                            )
                        })?;
                        if pending_batch.last_change_id != last_change_id {
                            return Err(crate::utils::err_msg(
                                module_path!(),
                                line!(),
                                format!(
                                    "Asset batch completion mismatch: expected {}, got {}",
                                    pending_batch.last_change_id, last_change_id
                                ),
                            ));
                        }
                        let missing_assets = pending_batch
                            .expected_files
                            .keys()
                            .filter(|entity_id| !pending_batch.received_entity_ids.contains(*entity_id))
                            .cloned()
                            .collect::<Vec<_>>();
                        if !missing_assets.is_empty() {
                            return Err(crate::utils::err_msg(
                                module_path!(),
                                line!(),
                                format!("Asset batch incomplete, missing entities: {}", missing_assets.join(", ")),
                            ));
                        }
                        for change in &pending_batch.changes {
                            if change.op == ChangeOp::Delete {
                                remove_asset_path(&app, &change.entity_id)?;
                            }
                        }
                        sync_db::apply_change_batch(&mut conn, SyncDomain::Assets, &pending_batch.changes)?;
                        if last_change_id > 0 {
                            sync_db::record_peer_cursor(&conn, &driver_device_id, SyncDomain::Assets, last_change_id)?;
                        }
                    }
                    Some(Ok(P2PMessage::SyncComplete)) => {
                        if pending_asset_batch.is_some() {
                            return Err(crate::utils::err_msg(
                                module_path!(),
                                line!(),
                                "Sync completed while an asset batch was still pending",
                            ));
                        }
                        if let Err(e) = finalize_pending_domain(&app, &mut conn, &driver_device_id, &mut pending_domain) {
                            log_error(&app, "sync_passenger", format!("Failed to materialize pending domain: {}", e));
                            framed.send(P2PMessage::Disconnect).await.ok();
                            return Err(e);
                        }
                        log_info(&app, "sync_passenger", "Received SyncComplete");
                        let _ = app.emit("database-reloaded", ());
                        framed
                            .send(P2PMessage::SyncApplied)
                            .await
                            .map_err(|e| crate::utils::err_to_string(module_path!(), line!(), e))?;
                        state.set_status(&app, SyncStatus::SyncCompleted).await;
                        break;
                    }
                    Some(Ok(P2PMessage::Disconnect)) => {
                        log_info(&app, "sync_passenger", "Received Disconnect");
                        if tracker.is_some() {
                            return Err(crate::utils::err_msg(
                                module_path!(),
                                line!(),
                                "Driver disconnected before the sync finished",
                            ));
                        }
                        break;
                    }
                    Some(Ok(other)) => {
                        log_info(&app, "sync_passenger", format!("Received unexpected message: {:?}", other));
                    }
                    Some(Err(e)) => {
                        log_error(&app, "sync_passenger", format!("Frame error: {}", e));
                        return Err(e.to_string());
                    }
                    None => {
                        log_info(&app, "sync_passenger", "Stream ended/Connection closed");
                        if tracker.is_some() {
                            return Err(crate::utils::err_msg(
                                module_path!(),
                                line!(),
                                "Connection lost before the sync finished",
                            ));
                        }
                        break;
                    }
                }
            }
        }
    }

    Ok(())
}

pub async fn stop_sync(app: AppHandle) -> Result<(), String> {
    let state = app.state::<SyncManagerState>();
    let mut tx_guard = state.shutdown_tx.lock().await;

    if let Some(tx) = tx_guard.take() {
        let _ = tx.send(());
    }

    drop(tx_guard);
    state.clear_pending().await;
    *state.pin.write().await = None;
    state.set_status(&app, SyncStatus::Idle).await;
    Ok(())
}

pub async fn approve_connection(app: AppHandle, ip: String, allow: bool) -> Result<(), String> {
    let state = app.state::<SyncManagerState>();
    let mut map = state.pending_approvals.write().await;

    if let Some(tx) = map.remove(&ip) {
        let _ = tx.send(allow);
    } else {
        return Err(crate::utils::err_msg(
            module_path!(),
            line!(),
            "No pending connection found for this IP",
        ));
    }

    Ok(())
}

pub async fn start_sync_session(app: AppHandle, ip: String) -> Result<(), String> {
    let state = app.state::<SyncManagerState>();
    let mut map = state.pending_starts.write().await;

    if let Some(tx) = map.remove(&ip) {
        let _ = tx.send(());
    } else {
        return Err(crate::utils::err_msg(
            module_path!(),
            line!(),
            "No pending sync session found for this IP",
        ));
    }

    Ok(())
}

fn sync_status_text(domain: SyncDomain) -> &'static str {
    match domain {
        SyncDomain::Core => "Syncing Core Data...",
        SyncDomain::Tts => "Syncing Voice Settings...",
        SyncDomain::Lorebooks => "Syncing Lorebooks...",
        SyncDomain::Characters => "Syncing Characters...",
        SyncDomain::Groups => "Syncing Groups...",
        SyncDomain::Sessions => "Syncing Sessions...",
        SyncDomain::Messages => "Syncing Messages...",
        SyncDomain::Assets => "Syncing Assets...",
    }
}

fn resolve_asset_path(app: &AppHandle, relative_path: &str) -> Result<std::path::PathBuf, String> {
    if relative_path.contains("..")
        || relative_path.starts_with('/')
        || relative_path.contains('\\')
    {
        return Err(crate::utils::err_msg(
            module_path!(),
            line!(),
            format!("Invalid asset path: {}", relative_path),
        ));
    }

    if !relative_path.starts_with("avatars/")
        && !relative_path.starts_with("sessions/")
        && !relative_path.starts_with("images/")
        && !relative_path.starts_with("generated_images/")
    {
        return Err(crate::utils::err_msg(
            module_path!(),
            line!(),
            format!("Unauthorized asset path: {}", relative_path),
        ));
    }

    if relative_path.starts_with("generated_images/") {
        Ok(app
            .path()
            .app_data_dir()
            .map_err(|e| crate::utils::err_to_string(module_path!(), line!(), e))?
            .join(relative_path))
    } else {
        Ok(crate::storage_manager::legacy::storage_root(app)?.join(relative_path))
    }
}

async fn write_asset_path(
    app: &AppHandle,
    relative_path: &str,
    content: &[u8],
) -> Result<(), String> {
    let full_path = resolve_asset_path(app, relative_path)?;

    if let Some(parent) = full_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| crate::utils::err_to_string(module_path!(), line!(), e))?;
    }
    tokio::fs::write(&full_path, content)
        .await
        .map_err(|e| crate::utils::err_to_string(module_path!(), line!(), e))
}

fn remove_asset_path(app: &AppHandle, relative_path: &str) -> Result<(), String> {
    let full_path = resolve_asset_path(app, relative_path)?;
    if full_path.exists() {
        std::fs::remove_file(&full_path)
            .map_err(|e| crate::utils::err_to_string(module_path!(), line!(), e))?;
    }
    Ok(())
}

fn estimate_asset_bytes(app: &AppHandle, changes: &[crate::sync::protocol::ChangeRecord]) -> u64 {
    let mut total: u64 = 0;
    for change in changes {
        if change.op != ChangeOp::Upsert {
            continue;
        }
        let Ok(asset) = bincode::deserialize::<sync_db::AssetRecord>(&change.payload) else {
            continue;
        };
        let Ok(path) = resolve_asset_path(app, &asset.path) else {
            continue;
        };
        if let Ok(meta) = std::fs::metadata(&path) {
            total = total.saturating_add(meta.len());
        }
    }
    total
}

async fn send_asset_change_contents(
    app: &AppHandle,
    framed: &mut Framed<TcpStream, P2PCodec>,
    changes: &[crate::sync::protocol::ChangeRecord],
    tracker: &mut SyncProgressTracker,
    state: &SyncManagerState,
    phase: &str,
    peer_protocol_version: u32,
    prepared_contents: &HashMap<String, Vec<u8>>,
) -> Result<(), String> {
    for change in changes {
        if change.op != ChangeOp::Upsert {
            continue;
        }

        let asset: sync_db::AssetRecord = bincode::deserialize(&change.payload)
            .map_err(|e| crate::utils::err_to_string(module_path!(), line!(), e))?;
        let content = prepared_contents
            .get(&change.entity_id)
            .cloned()
            .ok_or_else(|| {
                crate::utils::err_msg(
                    module_path!(),
                    line!(),
                    format!("Prepared asset content missing for {}", change.entity_id),
                )
            })?;
        let actual_hash = asset.content_hash.clone();
        if peer_protocol_version >= ASSET_CHUNK_PROTOCOL {
            let total_bytes = content.len() as u64;
            framed
                .send(P2PMessage::AssetContentStart {
                    entity_id: change.entity_id.clone(),
                    path: asset.path,
                    content_hash: actual_hash,
                    total_bytes,
                })
                .await
                .map_err(|e| crate::utils::err_to_string(module_path!(), line!(), e))?;

            for chunk in content.chunks(ASSET_CHUNK_SIZE) {
                framed
                    .send(P2PMessage::AssetContentChunk {
                        entity_id: change.entity_id.clone(),
                        chunk: chunk.to_vec(),
                    })
                    .await
                    .map_err(|e| crate::utils::err_to_string(module_path!(), line!(), e))?;
                tracker.record_bytes(chunk.len() as u64);
                state
                    .set_status(
                        app,
                        tracker.syncing_status(phase.to_string(), Some(SyncDomain::Assets)),
                    )
                    .await;
            }

            framed
                .send(P2PMessage::AssetContentComplete {
                    entity_id: change.entity_id.clone(),
                })
                .await
                .map_err(|e| crate::utils::err_to_string(module_path!(), line!(), e))?;
            tracker.record_items(SyncDomain::Assets, 1);
            state
                .set_status(
                    app,
                    tracker.syncing_status(phase.to_string(), Some(SyncDomain::Assets)),
                )
                .await;
        } else {
            framed
                .send(P2PMessage::AssetContent {
                    entity_id: change.entity_id.clone(),
                    path: asset.path,
                    content_hash: actual_hash,
                    content,
                })
                .await
                .map_err(|e| crate::utils::err_to_string(module_path!(), line!(), e))?;
            let file_bytes = asset.size_bytes;
            tracker.record_bytes(file_bytes);
            tracker.record_items(SyncDomain::Assets, 1);
            state
                .set_status(
                    app,
                    tracker.syncing_status(phase.to_string(), Some(SyncDomain::Assets)),
                )
                .await;
        }
    }

    Ok(())
}

fn pending_batch_bytes_received(
    pending_asset_batch: &mut Option<PendingAssetBatch>,
    bytes: u64,
) -> Result<(), String> {
    let pending_batch = pending_asset_batch.as_mut().ok_or_else(|| {
        crate::utils::err_msg(
            module_path!(),
            line!(),
            "Received asset bytes without an active batch",
        )
    })?;
    pending_batch.bytes_received = pending_batch.bytes_received.saturating_add(bytes);
    Ok(())
}

fn prepare_asset_changes_for_transfer(
    app: &AppHandle,
    changes: &mut [crate::sync::protocol::ChangeRecord],
) -> Result<HashMap<String, Vec<u8>>, String> {
    let mut prepared = HashMap::new();

    for change in changes.iter_mut() {
        if change.op != ChangeOp::Upsert {
            continue;
        }

        let mut asset: sync_db::AssetRecord = bincode::deserialize(&change.payload)
            .map_err(|e| crate::utils::err_to_string(module_path!(), line!(), e))?;
        let absolute_path = resolve_asset_path(app, &asset.path)?;
        if !absolute_path.is_file() {
            continue;
        }

        let content = std::fs::read(&absolute_path)
            .map_err(|e| crate::utils::err_to_string(module_path!(), line!(), e))?;
        let actual_hash = blake3::hash(&content).to_hex().to_string();
        let actual_size = content.len() as u64;

        if asset.content_hash != actual_hash || asset.size_bytes != actual_size {
            asset.content_hash = actual_hash;
            asset.size_bytes = actual_size;
            change.payload = bincode::serialize(&asset)
                .map_err(|e| crate::utils::err_to_string(module_path!(), line!(), e))?;
            change.payload_hash = blake3::hash(&change.payload).to_hex().to_string();
        }

        prepared.insert(change.entity_id.clone(), content);
    }

    Ok(prepared)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matching_versions_are_accepted() {
        assert!(version_mismatch_error(
            "1.2.0",
            "Laptop",
            "1.2.0",
            PROTOCOL_VERSION,
        )
        .is_none());
    }

    #[test]
    fn mismatched_peer_is_rejected_with_actionable_device_name() {
        let error = version_mismatch_error("1.2.0", "Laptop", "1.1.0", 11).unwrap();
        assert!(error.contains("Laptop uses app 1.1.0"));
        assert!(error.contains("exactly the same"));
        assert!(error.contains("fully restart both"));
    }

    #[test]
    fn newer_peer_is_rejected_as_a_version_mismatch() {
        let error =
            version_mismatch_error("1.2.0", "Desktop", "1.3.0", PROTOCOL_VERSION + 1).unwrap();
        assert!(error.contains("Desktop uses app 1.3.0"));
        assert!(error.contains("Update the mismatched device"));
    }
}
