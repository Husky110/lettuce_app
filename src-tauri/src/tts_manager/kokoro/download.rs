use futures_util::StreamExt;
use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex as TokioMutex;

use super::engine::{kokoro_platform_allows_variant, KokoroModelVariant};
use super::model::KokoroError;
use super::voices::list_installed_voice_ids;

const HF_MODEL_REPO: &str = "onnx-community/Kokoro-82M-v1.0-ONNX";
const HF_RESOLVE_BASE: &str = "https://huggingface.co/onnx-community/Kokoro-82M-v1.0-ONNX/resolve/main";
const HF_MODEL_API: &str = "https://huggingface.co/api/models/onnx-community/Kokoro-82M-v1.0-ONNX";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KokoroDownloadProgress {
    pub downloaded: u64,
    pub total: u64,
    pub status: String,
    pub current_file_index: usize,
    pub total_files: usize,
    pub current_file_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asset_root: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub install_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub voice_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KokoroAvailableVoice {
    pub id: String,
    pub installed: bool,
}

#[derive(Debug, Clone)]
struct DownloadFileSpec {
    remote_path: &'static str,
    local_relative_path: &'static str,
    progress_name: &'static str,
}

#[derive(Debug, Clone)]
struct ActiveDownloadContext {
    asset_root: PathBuf,
    owned_paths: Vec<PathBuf>,
    preexisting_paths: HashSet<PathBuf>,
}

#[derive(Debug, Clone)]
struct DownloadState {
    is_downloading: bool,
    cancel_requested: bool,
    progress: KokoroDownloadProgress,
    active_context: Option<ActiveDownloadContext>,
}

#[derive(Debug, Deserialize)]
struct HuggingFaceModelResponse {
    siblings: Option<Vec<HuggingFaceSibling>>,
}

#[derive(Debug, Deserialize)]
struct HuggingFaceSibling {
    rfilename: String,
}

lazy_static! {
    static ref DOWNLOAD_STATE: Arc<TokioMutex<DownloadState>> = Arc::new(TokioMutex::new(
        DownloadState {
            is_downloading: false,
            cancel_requested: false,
            progress: idle_progress(),
            active_context: None,
        }
    ));
}

fn idle_progress() -> KokoroDownloadProgress {
    KokoroDownloadProgress {
        downloaded: 0,
        total: 0,
        status: "idle".to_string(),
        current_file_index: 0,
        total_files: 0,
        current_file_name: String::new(),
        asset_root: None,
        install_kind: None,
        variant: None,
        voice_id: None,
    }
}

pub fn default_asset_root(app: &AppHandle) -> Result<PathBuf, KokoroError> {
    let lettuce_dir = crate::infra::utils::ensure_lettuce_dir(app).map_err(|message| {
        KokoroError::Io(std::io::Error::new(std::io::ErrorKind::Other, message))
    })?;
    let root = lettuce_dir.join("kokoro");
    fs::create_dir_all(&root)?;
    Ok(root)
}

pub async fn get_download_progress() -> Result<KokoroDownloadProgress, KokoroError> {
    let state = DOWNLOAD_STATE.lock().await;
    Ok(state.progress.clone())
}

pub async fn cancel_download(app: &AppHandle) -> Result<(), KokoroError> {
    {
        let mut state = DOWNLOAD_STATE.lock().await;
        if !state.is_downloading {
            state.progress = KokoroDownloadProgress {
                status: "cancelled".to_string(),
                ..idle_progress()
            };
            let _ = app.emit("kokoro_download_progress", &state.progress);
            return Ok(());
        }
        state.cancel_requested = true;
    }

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    cleanup_cancelled_download().await?;

    let mut state = DOWNLOAD_STATE.lock().await;
    state.is_downloading = false;
    state.cancel_requested = false;
    state.active_context = None;
    state.progress = KokoroDownloadProgress {
        status: "cancelled".to_string(),
        ..idle_progress()
    };
    let _ = app.emit("kokoro_download_progress", &state.progress);
    Ok(())
}

pub async fn install_model(
    app: AppHandle,
    asset_root: PathBuf,
    variant: KokoroModelVariant,
) -> Result<(), KokoroError> {
    if !kokoro_platform_allows_variant(variant) {
        return Err(KokoroError::UnsupportedVariant(format!(
            "{} is not allowed on this platform",
            variant.id()
        )));
    }

    let plan = model_download_plan(variant);
    run_download_plan(
        &app,
        asset_root,
        plan,
        "model",
        Some(variant.id().to_string()),
        None,
    )
    .await
}

pub async fn install_voice(
    app: AppHandle,
    asset_root: PathBuf,
    voice_id: String,
) -> Result<(), KokoroError> {
    if !is_valid_voice_id(&voice_id) {
        return Err(KokoroError::VoiceParse(format!(
            "Unsupported voice id: {}",
            voice_id
        )));
    }

    let leaked_remote = Box::leak(format!("voices/{voice_id}.bin").into_boxed_str());
    let leaked_local = Box::leak(format!("voices/{voice_id}.bin").into_boxed_str());
    let leaked_name = Box::leak(format!("{voice_id}.bin").into_boxed_str());
    let plan = vec![DownloadFileSpec {
        remote_path: leaked_remote,
        local_relative_path: leaked_local,
        progress_name: leaked_name,
    }];

    run_download_plan(&app, asset_root, plan, "voice", None, Some(voice_id)).await
}

pub async fn list_available_voices(asset_root: &Path) -> Result<Vec<KokoroAvailableVoice>, KokoroError> {
    let installed = list_installed_voice_ids(asset_root)?
        .into_iter()
        .collect::<HashSet<_>>();
    let client = reqwest::Client::new();
    let response = client
        .get(HF_MODEL_API)
        .send()
        .await
        .map_err(|err| KokoroError::Config(format!("Failed to query {HF_MODEL_REPO}: {err}")))?;
    if !response.status().is_success() {
        return Err(KokoroError::Config(format!(
            "Failed to query {HF_MODEL_REPO}: HTTP {}",
            response.status()
        )));
    }

    let payload = response
        .json::<HuggingFaceModelResponse>()
        .await
        .map_err(|err| KokoroError::Config(format!("Invalid HF model response: {err}")))?;

    let mut voices = payload
        .siblings
        .unwrap_or_default()
        .into_iter()
        .filter_map(|sibling| sibling.rfilename.strip_prefix("voices/").map(str::to_string))
        .filter_map(|filename| filename.strip_suffix(".bin").map(str::to_string))
        .filter(|voice_id| is_valid_voice_id(voice_id))
        .map(|id| KokoroAvailableVoice {
            installed: installed.contains(&id),
            id,
        })
        .collect::<Vec<_>>();

    voices.sort_by(|left, right| left.id.cmp(&right.id));
    voices.dedup_by(|left, right| left.id == right.id);
    Ok(voices)
}

fn model_download_plan(variant: KokoroModelVariant) -> Vec<DownloadFileSpec> {
    let model_filename = match variant {
        KokoroModelVariant::Fp32 => "model.onnx",
        KokoroModelVariant::Fp16 => "model_fp16.onnx",
        KokoroModelVariant::Int8 => "model_quantized.onnx",
    };

    vec![
        DownloadFileSpec {
            remote_path: "config.json",
            local_relative_path: "config.json",
            progress_name: "config.json",
        },
        DownloadFileSpec {
            remote_path: "tokenizer.json",
            local_relative_path: "tokenizer.json",
            progress_name: "tokenizer.json",
        },
        DownloadFileSpec {
            remote_path: "tokenizer_config.json",
            local_relative_path: "tokenizer_config.json",
            progress_name: "tokenizer_config.json",
        },
        DownloadFileSpec {
            remote_path: Box::leak(format!("onnx/{model_filename}").into_boxed_str()),
            local_relative_path: Box::leak(format!("onnx/{model_filename}").into_boxed_str()),
            progress_name: model_filename,
        },
    ]
}

fn is_valid_voice_id(value: &str) -> bool {
    !value.trim().is_empty()
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
}

async fn run_download_plan(
    app: &AppHandle,
    asset_root: PathBuf,
    plan: Vec<DownloadFileSpec>,
    install_kind: &str,
    variant: Option<String>,
    voice_id: Option<String>,
) -> Result<(), KokoroError> {
    {
        let mut state = DOWNLOAD_STATE.lock().await;
        if state.is_downloading {
            return Err(KokoroError::Config(
                "Kokoro download already in progress".to_string(),
            ));
        }

        fs::create_dir_all(&asset_root)?;
        let owned_paths = plan
            .iter()
            .map(|spec| asset_root.join(spec.local_relative_path))
            .collect::<Vec<_>>();
        let preexisting_paths = owned_paths
            .iter()
            .filter(|path| path.exists())
            .cloned()
            .collect::<HashSet<_>>();

        state.is_downloading = true;
        state.cancel_requested = false;
        state.active_context = Some(ActiveDownloadContext {
            asset_root: asset_root.clone(),
            owned_paths,
            preexisting_paths,
        });
        state.progress = KokoroDownloadProgress {
            downloaded: 0,
            total: 0,
            status: "downloading".to_string(),
            current_file_index: 1,
            total_files: plan.len(),
            current_file_name: plan
                .first()
                .map(|item| item.progress_name.to_string())
                .unwrap_or_default(),
            asset_root: Some(asset_root.to_string_lossy().to_string()),
            install_kind: Some(install_kind.to_string()),
            variant,
            voice_id,
        };
        let _ = app.emit("kokoro_download_progress", &state.progress);
    }

    for (index, file_spec) in plan.iter().enumerate() {
        {
            let mut state = DOWNLOAD_STATE.lock().await;
            state.progress.current_file_index = index + 1;
            state.progress.current_file_name = file_spec.progress_name.to_string();
            state.progress.status = format!("Downloading {}", file_spec.progress_name);
            let _ = app.emit("kokoro_download_progress", &state.progress);
        }

        let url = format!("{HF_RESOLVE_BASE}/{}", file_spec.remote_path);
        let destination = asset_root.join(file_spec.local_relative_path);
        if let Err(err) = download_file(app, &url, &destination).await {
            let was_cancelled = {
                let state = DOWNLOAD_STATE.lock().await;
                state.cancel_requested
            };
            cleanup_failed_download().await?;
            let mut state = DOWNLOAD_STATE.lock().await;
            state.is_downloading = false;
            state.cancel_requested = false;
            state.active_context = None;
            state.progress.status = if was_cancelled {
                "cancelled".to_string()
            } else {
                "failed".to_string()
            };
            let _ = app.emit("kokoro_download_progress", &state.progress);
            return Err(err);
        }
    }

    let mut state = DOWNLOAD_STATE.lock().await;
    state.is_downloading = false;
    state.cancel_requested = false;
    state.active_context = None;
    state.progress.status = "completed".to_string();
    let _ = app.emit("kokoro_download_progress", &state.progress);
    Ok(())
}

async fn download_file(app: &AppHandle, url: &str, destination: &Path) -> Result<(), KokoroError> {
    let client = reqwest::Client::new();
    let response = client.get(url).send().await.map_err(|err| {
        KokoroError::Config(format!("Failed to start Kokoro download {url}: {err}"))
    })?;

    if !response.status().is_success() {
        return Err(KokoroError::Config(format!(
            "Download failed for {url}: HTTP {}",
            response.status()
        )));
    }

    let total_size = response.content_length().unwrap_or(0);
    {
        let mut state = DOWNLOAD_STATE.lock().await;
        state.progress.total += total_size;
        let _ = app.emit("kokoro_download_progress", &state.progress);
    }

    if let Some(parent) = destination.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let temp_path = destination.with_extension("tmp");
    let mut file = tokio::fs::File::create(&temp_path).await?;
    let mut stream = response.bytes_stream();
    let mut last_emit = std::time::Instant::now();

    while let Some(chunk_result) = stream.next().await {
        {
            let state = DOWNLOAD_STATE.lock().await;
            if state.cancel_requested {
                drop(file);
                let _ = tokio::fs::remove_file(&temp_path).await;
                return Err(KokoroError::Config("Download cancelled".to_string()));
            }
        }

        let chunk = chunk_result
            .map_err(|err| KokoroError::Config(format!("Error reading download chunk: {err}")))?;
        file.write_all(&chunk).await?;

        {
            let mut state = DOWNLOAD_STATE.lock().await;
            state.progress.downloaded += chunk.len() as u64;
            if last_emit.elapsed().as_millis() >= 100 {
                let _ = app.emit("kokoro_download_progress", &state.progress);
                last_emit = std::time::Instant::now();
            }
        }
    }

    file.flush().await?;
    drop(file);

    tokio::fs::copy(&temp_path, destination).await?;
    let _ = tokio::fs::remove_file(&temp_path).await;
    Ok(())
}

async fn cleanup_failed_download() -> Result<(), KokoroError> {
    let active = {
        let state = DOWNLOAD_STATE.lock().await;
        state.active_context.clone()
    };

    if let Some(active) = active {
        cleanup_paths(&active.asset_root, &active.owned_paths, &active.preexisting_paths).await?;
    }
    Ok(())
}

async fn cleanup_cancelled_download() -> Result<(), KokoroError> {
    let active = {
        let state = DOWNLOAD_STATE.lock().await;
        state.active_context.clone()
    };

    if let Some(active) = active {
        cleanup_paths(&active.asset_root, &active.owned_paths, &active.preexisting_paths).await?;
    }
    Ok(())
}

async fn cleanup_paths(
    _asset_root: &Path,
    owned_paths: &[PathBuf],
    preexisting_paths: &HashSet<PathBuf>,
) -> Result<(), KokoroError> {
    for path in owned_paths {
        let temp_path = path.with_extension("tmp");
        if temp_path.exists() {
            let _ = tokio::fs::remove_file(&temp_path).await;
        }
        if !preexisting_paths.contains(path) && path.exists() {
            let _ = tokio::fs::remove_file(path).await;
        }
    }
    Ok(())
}
