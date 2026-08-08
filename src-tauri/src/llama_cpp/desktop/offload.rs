use super::engine::shared_backend;
use llama_cpp_2::gguf::GgufContext;
use llama_cpp_2::model::{params::LlamaModelParams, LlamaModel};
use llama_cpp_sys_2::llama_flash_attn_type;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::{Mutex, OnceLock};

#[derive(Clone, Copy, Debug)]
pub(super) struct LlamaModelMetadata {
    pub(super) model_size_bytes: u64,
    pub(super) layer_count: u32,
    pub(super) nextn_layer_count: u32,
    pub(super) max_context_length: u32,
    pub(super) n_embd: u64,
    pub(super) n_head: u64,
    pub(super) n_head_kv: u64,
}

impl LlamaModelMetadata {
    pub(super) fn model_layer_count(&self) -> u32 {
        self.layer_count
            .max(1)
            .saturating_add(self.nextn_layer_count)
    }

    pub(super) fn offload_layer_count(&self) -> u32 {
        self.model_layer_count().saturating_add(1)
    }

    pub(super) fn normalize_requested_gpu_layers(&self, requested: u32) -> u32 {
        if requested >= self.layer_count.max(1) {
            self.offload_layer_count()
        } else {
            requested
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct SmartGpuOffloadPlan {
    pub(super) total_layers: u32,
    pub(super) recommended_context: Option<u32>,
    pub(super) planned_context: u32,
    pub(super) estimated_gpu_layers: u32,
    pub(super) candidate_gpu_layers: Vec<u32>,
    pub(super) kqv_vram_reserved: bool,
    pub(super) planning_offload_kqv: Option<bool>,
    pub(super) estimated_kv_bytes: u64,
    pub(super) kv_bytes_per_layer: u64,
    pub(super) estimated_sidecar_vram_reserve_bytes: u64,
    pub(super) estimated_runtime_reserve_bytes: u64,
    pub(super) effective_vram_budget_bytes: u64,
    pub(super) bytes_per_layer: u64,
}

/// Byte cost of every llama.cpp offload unit, indexed the way llama.cpp
/// indexes them.
///
/// `llama-model.cpp` walks `il` over `0..=n_layer_all`, where `0..n_layer_all`
/// are the repeating blocks and `il == n_layer_all` is the output layer, and
/// sends a unit to the GPU when `il >= max(n_layer_all + 1 - n_gpu_layers, 0)`.
/// So `n_gpu_layers` always takes the *last* units of that list, output layer
/// first. The input embedding is pinned to the CPU no matter what
/// (`dev_input`), so it is not a unit here.
///
/// Sizes come straight from the GGUF tensor index. They have to be per-unit:
/// the output layer routinely dwarfs a block (2.4 GiB vs 258 MiB on a
/// large-vocab model with a high-precision head), and it is the very first
/// unit offloaded, so charging every unit an average of the file understates
/// the first GPU layers badly.
#[derive(Clone, Debug)]
pub(super) struct ModelOffloadCosts {
    unit_bytes: Vec<u64>,
}

impl ModelOffloadCosts {
    /// Number of offload units, equal to llama.cpp's `n_layer_all + 1`.
    pub(super) fn unit_count(&self) -> u32 {
        u32::try_from(self.unit_bytes.len()).unwrap_or(u32::MAX)
    }

    /// Weight bytes that land on the GPU for `gpu_layers`, i.e. the last
    /// `gpu_layers` units.
    pub(super) fn gpu_bytes(&self, gpu_layers: u32) -> u64 {
        let take = (gpu_layers as usize).min(self.unit_bytes.len());
        self.unit_bytes[self.unit_bytes.len() - take..]
            .iter()
            .fold(0u64, |acc, bytes| acc.saturating_add(*bytes))
    }

    /// Largest `gpu_layers` whose weights plus per-block KV fit in `budget`.
    ///
    /// `kv_bytes_per_layer` is charged only for block units, and only for the
    /// first `kv_layer_count` of them, because the KV cache is sized from the
    /// attention layers rather than from the offload unit list.
    fn max_units_within(&self, budget: u64, kv_bytes_per_layer: u64, kv_layer_count: u32) -> u32 {
        let mut running = 0u64;
        let mut kv_charged = 0u32;
        let mut fitted = 0u32;
        let output_index = self.unit_bytes.len().saturating_sub(1);
        for (offset, index) in (0..self.unit_bytes.len()).rev().enumerate() {
            running = running.saturating_add(self.unit_bytes[index]);
            if index != output_index && kv_charged < kv_layer_count {
                running = running.saturating_add(kv_bytes_per_layer);
                kv_charged += 1;
            }
            if running > budget {
                break;
            }
            fitted = u32::try_from(offset + 1).unwrap_or(u32::MAX);
        }
        fitted
    }
}

/// Reads the per-unit costs out of the GGUF tensor index.
///
/// Returns `None` when the file cannot be opened or carries no blocks, in
/// which case callers fall back to the flat file average.
fn load_offload_costs_uncached(model_path: &str) -> Option<ModelOffloadCosts> {
    let gguf = GgufContext::from_file(Path::new(model_path))?;
    let mut blocks: BTreeMap<u32, u64> = BTreeMap::new();
    let mut output_bytes = 0u64;
    let mut input_bytes = 0u64;
    let mut has_output_weight = false;

    for index in 0..gguf.n_tensors() {
        let Some(name) = gguf.tensor_name(index) else {
            continue;
        };
        let size = gguf.tensor_size(index);
        if let Some(block) = block_index(name) {
            let entry = blocks.entry(block).or_default();
            *entry = entry.saturating_add(size);
        } else if name.starts_with("token_embd") {
            input_bytes = input_bytes.saturating_add(size);
        } else {
            if name == "output.weight" {
                has_output_weight = true;
            }
            output_bytes = output_bytes.saturating_add(size);
        }
    }

    let n_layer_all = usize::try_from(*blocks.keys().max()? + 1).ok()?;
    // Models with a tied head carry no `output.weight`; llama.cpp then builds
    // the output tensor as a duplicate of the embedding, which is a second
    // allocation of that size on the output device.
    if !has_output_weight {
        output_bytes = output_bytes.saturating_add(input_bytes);
    }

    let mut unit_bytes = vec![0u64; n_layer_all + 1];
    for (block, bytes) in blocks {
        if let Some(slot) = unit_bytes.get_mut(block as usize) {
            *slot = bytes;
        }
    }
    unit_bytes[n_layer_all] = output_bytes;

    Some(ModelOffloadCosts { unit_bytes })
}

/// `blk.<n>.` prefix parser, matching llama.cpp's repeating-layer naming.
fn block_index(tensor_name: &str) -> Option<u32> {
    let rest = tensor_name.strip_prefix("blk.")?;
    let (digits, _) = rest.split_once('.')?;
    digits.parse().ok()
}

pub(super) fn load_offload_costs(model_path: &str) -> Option<ModelOffloadCosts> {
    if let Some(costs) = offload_costs_cache().lock().ok()?.get(model_path).cloned() {
        return costs;
    }
    let costs = load_offload_costs_uncached(model_path);
    offload_costs_cache()
        .lock()
        .ok()?
        .insert(model_path.to_string(), costs.clone());
    costs
}

static MODEL_OFFLOAD_COSTS_CACHE: OnceLock<Mutex<HashMap<String, Option<ModelOffloadCosts>>>> =
    OnceLock::new();

fn offload_costs_cache() -> &'static Mutex<HashMap<String, Option<ModelOffloadCosts>>> {
    MODEL_OFFLOAD_COSTS_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

static MODEL_METADATA_CACHE: OnceLock<Mutex<HashMap<String, LlamaModelMetadata>>> = OnceLock::new();

fn metadata_cache() -> &'static Mutex<HashMap<String, LlamaModelMetadata>> {
    MODEL_METADATA_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn kv_bytes_per_value(llama_kv_type: Option<&str>) -> f64 {
    match llama_kv_type
        .map(|value| value.trim().to_ascii_lowercase())
        .as_deref()
    {
        Some("f32") => 4.0,
        Some("f16") => 2.0,
        Some("q8_1") | Some("q8_0") => 1.0,
        Some("q6_k") => 0.75,
        Some("q5_k") | Some("q5_1") | Some("q5_0") => 0.625,
        Some("q4_k") | Some("q4_1") | Some("q4_0") => 0.5,
        Some("q3_k") | Some("iq3_s") | Some("iq3_xxs") => 0.375,
        Some("q2_k") | Some("iq2_xs") | Some("iq2_xxs") | Some("iq1_s") => 0.25,
        Some("iq4_nl") => 0.5,
        _ => 2.0,
    }
}

fn estimate_kv_bytes_per_token(
    metadata: &LlamaModelMetadata,
    llama_kv_type: Option<&str>,
) -> Option<u64> {
    let n_layer = u64::from(metadata.layer_count.max(1));
    let n_embd = metadata.n_embd.max(1);
    let n_head = metadata.n_head.max(1);
    let n_head_kv = metadata.n_head_kv.max(1);
    let gqa_correction = n_head_kv as f64 / n_head as f64;
    let effective_n_embd = (n_embd as f64 * gqa_correction) as u64;
    let bytes_per_value = kv_bytes_per_value(llama_kv_type);
    let bytes = (n_layer as f64) * (effective_n_embd as f64) * 2.0 * bytes_per_value;
    Some(bytes.max(0.0) as u64)
}

fn default_memory_reserve_bytes(available_memory_bytes: u64) -> u64 {
    (available_memory_bytes / 5).max(512 * 1024 * 1024)
}

fn ram_budget_for_context(metadata: &LlamaModelMetadata, available_memory_bytes: u64) -> u64 {
    let reserve = default_memory_reserve_bytes(available_memory_bytes);
    available_memory_bytes.saturating_sub(metadata.model_size_bytes.saturating_add(reserve))
}

fn compute_recommended_context(
    metadata: &LlamaModelMetadata,
    available_memory_bytes: Option<u64>,
    available_vram_bytes: Option<u64>,
    llama_offload_kqv: Option<bool>,
    llama_kv_type: Option<&str>,
) -> Option<u32> {
    let available_for_ctx = if llama_offload_kqv == Some(true) {
        let vram = available_vram_bytes?;
        let reserve = default_memory_reserve_bytes(vram);
        vram.saturating_sub(reserve)
    } else {
        let ram = available_memory_bytes?;
        ram_budget_for_context(metadata, ram)
    };
    let kv_bytes_per_token = estimate_kv_bytes_per_token(metadata, llama_kv_type)?;
    if kv_bytes_per_token == 0 {
        return None;
    }
    let mut recommended = available_for_ctx / kv_bytes_per_token;
    if recommended > u64::from(metadata.max_context_length) {
        recommended = u64::from(metadata.max_context_length);
    }
    Some(recommended as u32)
}

fn load_model_metadata_uncached(model_path: &str) -> Result<LlamaModelMetadata, String> {
    let backend = shared_backend()?;
    let model = LlamaModel::load_from_file(
        backend.as_ref(),
        model_path,
        &LlamaModelParams::default().with_n_gpu_layers(0),
    )
    .map_err(|e| {
        crate::utils::err_msg(
            module_path!(),
            line!(),
            format!("Failed to load llama model metadata for smart offload: {e}"),
        )
    })?;

    Ok(LlamaModelMetadata {
        model_size_bytes: model.size(),
        layer_count: model.n_layer().max(1),
        nextn_layer_count: model.n_layer_nextn(),
        max_context_length: model.n_ctx_train().max(1),
        n_embd: u64::try_from(model.n_embd()).unwrap_or(0).max(1),
        n_head: u64::from(model.n_head()).max(1),
        n_head_kv: u64::from(model.n_head_kv()).max(1),
    })
}

pub(super) fn load_model_metadata(model_path: &str) -> Result<LlamaModelMetadata, String> {
    if let Some(metadata) = metadata_cache()
        .lock()
        .map_err(|_| "llama.cpp metadata cache lock poisoned".to_string())?
        .get(model_path)
        .copied()
    {
        return Ok(metadata);
    }

    let metadata = load_model_metadata_uncached(model_path)?;
    metadata_cache()
        .lock()
        .map_err(|_| "llama.cpp metadata cache lock poisoned".to_string())?
        .insert(model_path.to_string(), metadata);
    Ok(metadata)
}

fn push_unique(out: &mut Vec<u32>, value: u32) {
    if !out.contains(&value) {
        out.push(value);
    }
}

const ATTENTION_SCORE_BYTES: u64 = 4;
const COMPUTE_BUFFER_SAFETY_FACTOR: u64 = 2;
const COMPUTE_RESERVE_FLOOR_BYTES: u64 = 256 * 1024 * 1024;
const MTP_COMPUTE_REFERENCE_CONTEXT: u64 = 16_384;
const MTP_COMPUTE_REFERENCE_BYTES: u64 = 384 * 1024 * 1024;
const MTP_COMPUTE_MIN_BYTES: u64 = 128 * 1024 * 1024;

pub(super) fn estimate_mtp_gpu_reserve_bytes(
    model_path: &str,
    planned_context: u32,
) -> Result<u64, String> {
    let metadata = load_model_metadata(model_path)?;
    Ok(metadata
        .model_size_bytes
        .saturating_add(estimate_mtp_compute_reserve_bytes(planned_context)))
}

fn estimate_mtp_compute_reserve_bytes(planned_context: u32) -> u64 {
    MTP_COMPUTE_REFERENCE_BYTES
        .saturating_mul(u64::from(planned_context.max(1)))
        .checked_div(MTP_COMPUTE_REFERENCE_CONTEXT)
        .unwrap_or(MTP_COMPUTE_REFERENCE_BYTES)
        .max(MTP_COMPUTE_MIN_BYTES)
}

pub(super) fn select_mtp_gpu_device(
    selected_device_ids: &[usize],
    device_free_vram: &[u64],
) -> Option<usize> {
    selected_device_ids
        .iter()
        .copied()
        .zip(device_free_vram.iter().copied())
        .max_by_key(|(_, free)| *free)
        .map(|(device_id, _)| device_id)
}

pub(super) fn reserve_device_vram(
    selected_device_ids: &[usize],
    device_free_vram: &[u64],
    device_id: Option<usize>,
    reserve_bytes: u64,
) -> Vec<u64> {
    let mut adjusted = device_free_vram.to_vec();
    if let Some(position) = device_id.and_then(|device_id| {
        selected_device_ids
            .iter()
            .position(|selected| *selected == device_id)
    }) {
        if let Some(free) = adjusted.get_mut(position) {
            *free = free.saturating_sub(reserve_bytes);
        }
    }
    adjusted
}

fn estimated_runtime_reserve_bytes(
    metadata: &LlamaModelMetadata,
    available_vram_bytes: u64,
    planned_context: u32,
    n_batch: u32,
    flash_attention_policy: llama_flash_attn_type,
) -> u64 {
    let floor = (available_vram_bytes / 20).max(COMPUTE_RESERVE_FLOOR_BYTES);
    // AUTO (-1) means llama.cpp will use flash attention when the backend supports it
    // (always true on CUDA). Only reserve the full attention matrix for the DISABLED case.
    let attention_reserve =
        if flash_attention_policy != llama_cpp_sys_2::LLAMA_FLASH_ATTN_TYPE_DISABLED {
            0
        } else {
            u64::from(planned_context.max(1))
                .saturating_mul(u64::from(n_batch.max(1)))
                .saturating_mul(metadata.n_head_kv.max(1))
                .saturating_mul(ATTENTION_SCORE_BYTES)
                .saturating_mul(COMPUTE_BUFFER_SAFETY_FACTOR)
        };
    floor.saturating_add(attention_reserve)
}

fn candidate_gpu_layers(total_layers: u32, estimated_gpu_layers: u32) -> Vec<u32> {
    if total_layers == 0 {
        return vec![0];
    }

    let estimate = estimated_gpu_layers.min(total_layers);
    if estimate == 0 {
        return vec![0];
    }

    let mut candidates = Vec::new();
    push_unique(&mut candidates, estimate);
    push_unique(&mut candidates, estimate.saturating_mul(3) / 4);
    push_unique(&mut candidates, estimate / 2);
    push_unique(&mut candidates, estimate / 4);
    push_unique(&mut candidates, 0);
    candidates.sort_unstable_by(|a, b| b.cmp(a));
    candidates
}

pub(super) fn context_bucket_upper(context: u32) -> u32 {
    match context {
        0..=4096 => 4096,
        4097..=8192 => 8192,
        8193..=12288 => 12288,
        12289..=16384 => 16384,
        16385..=24576 => 24576,
        24577..=32768 => 32768,
        32769..=49152 => 49152,
        49153..=65536 => 65536,
        _ => ((context.saturating_add(8191)) / 8192) * 8192,
    }
}

pub(super) fn merge_cached_candidate_layers(
    total_layers: u32,
    cached_gpu_layers: u32,
    heuristic_candidates: &[u32],
) -> Vec<u32> {
    let mut merged = Vec::new();
    let cached = cached_gpu_layers.min(total_layers);
    if cached > 0 {
        push_unique(&mut merged, cached);
        push_unique(&mut merged, cached.saturating_mul(3) / 4);
        push_unique(&mut merged, cached / 2);
        push_unique(&mut merged, cached / 4);
    }
    for candidate in heuristic_candidates {
        push_unique(&mut merged, (*candidate).min(total_layers));
    }
    push_unique(&mut merged, 0);
    merged
}

pub(super) fn model_weight_split_bytes(
    metadata: &LlamaModelMetadata,
    costs: Option<&ModelOffloadCosts>,
    gpu_layers: u32,
) -> (u64, u64) {
    if let Some(costs) = costs {
        let gpu_weight_bytes = costs.gpu_bytes(gpu_layers);
        // The embedding never leaves the CPU, so bill it against host memory
        // alongside whatever units stayed behind.
        let cpu_weight_bytes = metadata
            .model_size_bytes
            .saturating_sub(gpu_weight_bytes.min(metadata.model_size_bytes));
        return (cpu_weight_bytes, gpu_weight_bytes);
    }
    let total_layers = metadata.offload_layer_count();
    let clamped_gpu_layers = gpu_layers.min(total_layers);
    let gpu_weight_bytes = metadata
        .model_size_bytes
        .saturating_mul(u64::from(clamped_gpu_layers))
        .checked_div(u64::from(total_layers))
        .unwrap_or(0);
    let cpu_weight_bytes = metadata.model_size_bytes.saturating_sub(gpu_weight_bytes);
    (cpu_weight_bytes, gpu_weight_bytes)
}

pub(super) fn compute_recommended_context_for_gpu_layers(
    metadata: &LlamaModelMetadata,
    costs: Option<&ModelOffloadCosts>,
    available_memory_bytes: Option<u64>,
    available_vram_bytes: Option<u64>,
    gpu_layers: u32,
    llama_offload_kqv: Option<bool>,
    llama_kv_type: Option<&str>,
    sidecar_vram_reserve_bytes: u64,
) -> Option<u32> {
    let (cpu_weight_bytes, gpu_weight_bytes) =
        model_weight_split_bytes(metadata, costs, gpu_layers);
    let available_for_ctx = if llama_offload_kqv == Some(true) {
        let vram = available_vram_bytes?;
        let reserve = default_memory_reserve_bytes(vram);
        vram.saturating_sub(gpu_weight_bytes.saturating_add(reserve))
            .saturating_sub(sidecar_vram_reserve_bytes)
    } else {
        let ram = available_memory_bytes?;
        let reserve = default_memory_reserve_bytes(ram);
        ram.saturating_sub(cpu_weight_bytes.saturating_add(reserve))
    };
    let kv_bytes_per_token = estimate_kv_bytes_per_token(metadata, llama_kv_type)?;
    if kv_bytes_per_token == 0 {
        return None;
    }
    let mut recommended = available_for_ctx / kv_bytes_per_token;
    if recommended > u64::from(metadata.max_context_length) {
        recommended = u64::from(metadata.max_context_length);
    }
    Some(recommended as u32)
}

pub(super) fn plan_smart_gpu_offload(
    model_path: &str,
    available_memory_bytes: Option<u64>,
    available_vram_bytes: Option<u64>,
    requested_context: Option<u32>,
    n_batch: u32,
    resolved_offload_kqv: Option<bool>,
    llama_kv_type: Option<&str>,
    flash_attention_policy: llama_flash_attn_type,
    sidecar_vram_reserve_bytes: u64,
    bundled_mtp_draft: bool,
) -> Result<SmartGpuOffloadPlan, String> {
    let metadata = load_model_metadata(model_path)?;
    let costs = load_offload_costs(model_path);
    let total_layers = costs
        .as_ref()
        .map(ModelOffloadCosts::unit_count)
        .unwrap_or_else(|| metadata.offload_layer_count());
    let recommended_context = compute_recommended_context(
        &metadata,
        available_memory_bytes,
        available_vram_bytes,
        resolved_offload_kqv,
        llama_kv_type,
    );
    let planned_context = requested_context
        .or(recommended_context)
        .unwrap_or(metadata.max_context_length)
        .clamp(1, metadata.max_context_length);

    let available_vram = available_vram_bytes.unwrap_or(0);
    let effective_vram_budget_bytes = available_vram.saturating_mul(9) / 10;
    let estimated_runtime_reserve_bytes = estimated_runtime_reserve_bytes(
        &metadata,
        available_vram,
        planned_context,
        n_batch,
        flash_attention_policy,
    );
    let bytes_per_layer = metadata
        .model_size_bytes
        .checked_add(u64::from(metadata.model_layer_count()) - 1)
        .and_then(|bytes| bytes.checked_div(u64::from(metadata.model_layer_count())))
        .unwrap_or(0);
    let kv_bytes_per_token = estimate_kv_bytes_per_token(&metadata, llama_kv_type).unwrap_or(0);

    // Plan for the mode the runtime will actually use. `None` hands the choice
    // to llama.cpp, whose `llama_context_default_params` sets
    // `offload_kqv = true`, so it costs VRAM just like an explicit `true`.
    // Shopping for whichever mode fits the most layers used to budget zero KV
    // VRAM and then let the runtime put the KV cache on the GPU anyway.
    let planning_offload_kqv = resolved_offload_kqv;
    let kqv_vram_reserved = planning_offload_kqv != Some(false);
    // Only the GPU-resident layers' KV goes to VRAM, not the whole model's, so
    // this is charged per layer as units are accumulated.
    // A bundled-MTP draft reuses the model's weights but stands up a second
    // context, whose KV cache is placed per layer exactly like the main one.
    // That doubles the per-layer KV price rather than adding a fixed block, so
    // it stays correct however many layers end up on the GPU.
    let kv_contexts = if bundled_mtp_draft { 2 } else { 1 };
    let kv_bytes_per_layer = if kqv_vram_reserved {
        kv_bytes_per_token
            .saturating_mul(u64::from(planned_context))
            .checked_div(u64::from(metadata.layer_count.max(1)))
            .unwrap_or(0)
            .saturating_mul(kv_contexts)
    } else {
        0
    };
    let available_base = effective_vram_budget_bytes
        .saturating_sub(estimated_runtime_reserve_bytes)
        .saturating_sub(sidecar_vram_reserve_bytes);
    let estimated_gpu_layers = match costs.as_ref() {
        Some(costs) => costs
            .max_units_within(
                available_base,
                kv_bytes_per_layer,
                metadata.layer_count.max(1),
            )
            .min(total_layers),
        None => {
            let effective_bytes_per_layer = bytes_per_layer.saturating_add(kv_bytes_per_layer);
            if available_base == 0 || effective_bytes_per_layer == 0 {
                0
            } else {
                u32::try_from(
                    (available_base / effective_bytes_per_layer).min(u64::from(total_layers)),
                )
                .unwrap_or(total_layers)
                .min(total_layers)
            }
        }
    };
    // Report the KV bytes that will actually land on GPU (scales with GPU layers).
    let estimated_kv_bytes = kv_bytes_per_layer.saturating_mul(u64::from(
        estimated_gpu_layers.min(metadata.layer_count.max(1)),
    ));

    Ok(SmartGpuOffloadPlan {
        total_layers,
        recommended_context,
        planned_context,
        estimated_gpu_layers,
        candidate_gpu_layers: candidate_gpu_layers(total_layers, estimated_gpu_layers),
        kqv_vram_reserved,
        planning_offload_kqv,
        estimated_kv_bytes,
        kv_bytes_per_layer,
        estimated_sidecar_vram_reserve_bytes: sidecar_vram_reserve_bytes,
        estimated_runtime_reserve_bytes,
        effective_vram_budget_bytes,
        bytes_per_layer,
    })
}

#[derive(Debug, Clone, Default)]
pub(super) struct MultiGpuDistribution {
    pub(super) n_gpu_layers: u32,
    pub(super) tensor_split: Vec<f32>,
    pub(super) main_gpu: Option<i32>,
    pub(super) per_device_layers: Vec<u32>,
}

fn normalize_weights(weights: &[f32]) -> Vec<f32> {
    let n = weights.len();
    if n == 0 {
        return Vec::new();
    }
    let sum: f32 = weights.iter().copied().filter(|w| *w > 0.0).sum();
    if sum <= 0.0 {
        return vec![1.0 / n as f32; n];
    }
    weights.iter().map(|w| w.max(0.0) / sum).collect()
}

/// Split `total` whole layers across devices following `weights`, summing exactly
/// to `total` (largest-remainder method). Used for the UI placement estimate.
fn distribute_by_weights(total: u32, weights: &[f32]) -> Vec<u32> {
    let n = weights.len();
    if n == 0 {
        return Vec::new();
    }
    if total == 0 {
        return vec![0u32; n];
    }
    let sum: f32 = weights.iter().copied().filter(|w| *w > 0.0).sum();
    let raw: Vec<f32> = if sum <= 0.0 {
        vec![total as f32 / n as f32; n]
    } else {
        weights
            .iter()
            .map(|w| (w.max(0.0) / sum) * total as f32)
            .collect()
    };
    let mut out: Vec<u32> = raw.iter().map(|r| r.floor() as u32).collect();
    let assigned: u32 = out.iter().copied().sum();
    let mut remainder = total.saturating_sub(assigned);
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|a, b| {
        let fa = raw[*a] - raw[*a].floor();
        let fb = raw[*b] - raw[*b].floor();
        fb.partial_cmp(&fa).unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut i = 0;
    while remainder > 0 {
        let idx = order[i % n];
        out[idx] += 1;
        remainder -= 1;
        i += 1;
    }
    out
}

/// Translate a distribution strategy into concrete llama.cpp load parameters.
/// `device_free_vram` and `manual` are aligned to the selected-device order.
pub(super) fn plan_multi_gpu_distribution(
    mode: &str,
    device_free_vram: &[u64],
    total_layers: u32,
    bytes_per_layer: u64,
    kv_bytes_per_layer: u64,
    smart_total_estimate: u32,
    manual: Option<&[u32]>,
    priority_limit_bytes: Option<u64>,
) -> MultiGpuDistribution {
    let n = device_free_vram.len();
    if n == 0 {
        return MultiGpuDistribution::default();
    }
    let auto_total = smart_total_estimate.min(total_layers);

    match mode {
        "manual" => {
            let counts: Vec<u32> = (0..n)
                .map(|i| manual.and_then(|m| m.get(i).copied()).unwrap_or(0))
                .collect();
            let total: u32 = counts.iter().copied().sum::<u32>().min(total_layers);
            let weights: Vec<f32> = counts.iter().map(|c| *c as f32).collect();
            MultiGpuDistribution {
                n_gpu_layers: total,
                tensor_split: if total > 0 {
                    normalize_weights(&weights)
                } else {
                    Vec::new()
                },
                main_gpu: None,
                per_device_layers: counts,
            }
        }
        "priority" => {
            let effective_per_layer = bytes_per_layer.saturating_add(kv_bytes_per_layer);
            let mut remaining = auto_total;
            let mut per_device = vec![0u32; n];
            for (i, free) in device_free_vram.iter().enumerate() {
                if remaining == 0 {
                    break;
                }
                let budget = if i == 0 {
                    priority_limit_bytes
                        .map(|lim| lim.min(*free))
                        .unwrap_or(*free)
                } else {
                    *free
                };
                let cap = if effective_per_layer == 0 {
                    remaining
                } else {
                    u32::try_from(budget / effective_per_layer).unwrap_or(remaining)
                };
                let assigned = cap.min(remaining);
                per_device[i] = assigned;
                remaining -= assigned;
            }
            if remaining > 0 {
                if let Some(last) = per_device.last_mut() {
                    *last += remaining;
                }
            }
            let total: u32 = per_device.iter().copied().sum::<u32>().min(total_layers);
            let weights: Vec<f32> = per_device.iter().map(|c| *c as f32).collect();
            MultiGpuDistribution {
                n_gpu_layers: total,
                tensor_split: if total > 0 {
                    normalize_weights(&weights)
                } else {
                    Vec::new()
                },
                main_gpu: Some(0),
                per_device_layers: per_device,
            }
        }
        "proportional" => {
            let effective_per_layer = bytes_per_layer.saturating_add(kv_bytes_per_layer);
            let capped_total = if effective_per_layer == 0 {
                auto_total
            } else {
                let feasible: u64 = device_free_vram
                    .iter()
                    .map(|free| free / effective_per_layer)
                    .sum();
                auto_total.min(u32::try_from(feasible).unwrap_or(auto_total))
            };
            let weights: Vec<f32> = device_free_vram.iter().map(|f| *f as f32).collect();
            let split = normalize_weights(&weights);
            MultiGpuDistribution {
                n_gpu_layers: capped_total,
                per_device_layers: distribute_by_weights(capped_total, &split),
                tensor_split: if capped_total > 0 { split } else { Vec::new() },
                main_gpu: None,
            }
        }
        // "balanced" and any unknown strategy fall through to an even split.
        _ => {
            let effective_per_layer = bytes_per_layer.saturating_add(kv_bytes_per_layer);
            let capacities: Vec<u32> = device_free_vram
                .iter()
                .map(|free| {
                    if effective_per_layer == 0 {
                        auto_total
                    } else {
                        u32::try_from(free / effective_per_layer).unwrap_or(auto_total)
                    }
                })
                .collect();
            let mut per_device = vec![0u32; n];
            let mut remaining = auto_total;
            while remaining > 0 {
                let mut progressed = false;
                for (assigned, capacity) in per_device.iter_mut().zip(capacities.iter()) {
                    if remaining == 0 {
                        break;
                    }
                    if *assigned < *capacity {
                        *assigned += 1;
                        remaining -= 1;
                        progressed = true;
                    }
                }
                if !progressed {
                    break;
                }
            }
            let assigned_total = per_device.iter().copied().sum();
            let split: Vec<f32> = per_device.iter().map(|layers| *layers as f32).collect();
            MultiGpuDistribution {
                n_gpu_layers: assigned_total,
                per_device_layers: per_device,
                tensor_split: if assigned_total > 0 {
                    normalize_weights(&split)
                } else {
                    Vec::new()
                },
                main_gpu: None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        candidate_gpu_layers, estimate_mtp_compute_reserve_bytes, estimated_runtime_reserve_bytes,
        model_weight_split_bytes, plan_multi_gpu_distribution, reserve_device_vram,
        select_mtp_gpu_device, LlamaModelMetadata,
    };

    fn large_context_metadata() -> LlamaModelMetadata {
        LlamaModelMetadata {
            model_size_bytes: 16 * 1024 * 1024 * 1024,
            layer_count: 60,
            nextn_layer_count: 0,
            max_context_length: 262_144,
            n_embd: 4096,
            n_head: 32,
            n_head_kv: 8,
        }
    }

    #[test]
    fn mtp_compute_reserve_scales_with_context_and_has_a_floor() {
        assert_eq!(
            estimate_mtp_compute_reserve_bytes(16_384),
            384 * 1024 * 1024
        );
        assert_eq!(estimate_mtp_compute_reserve_bytes(8_192), 192 * 1024 * 1024);
        assert_eq!(estimate_mtp_compute_reserve_bytes(1), 128 * 1024 * 1024);
    }

    #[test]
    fn mtp_uses_the_selected_device_with_the_most_free_vram() {
        let selected = [4, 7, 9];
        let free = [8, 24, 16];

        assert_eq!(select_mtp_gpu_device(&selected, &free), Some(7));
        assert_eq!(
            reserve_device_vram(&selected, &free, Some(7), 6),
            vec![8, 18, 16]
        );
    }

    #[test]
    fn runtime_reserve_holds_attention_scratch_when_flash_attention_disabled() {
        let available = 16_u64 * 1024 * 1024 * 1024;

        let reserve = estimated_runtime_reserve_bytes(
            &large_context_metadata(),
            available,
            32_768,
            2048,
            llama_cpp_sys_2::LLAMA_FLASH_ATTN_TYPE_DISABLED,
        );

        assert_eq!(reserve, available / 20 + 4_294_967_296);
    }

    #[test]
    fn runtime_reserve_assumes_flash_attention_for_auto_policy_on_every_backend() {
        let available = 16_u64 * 1024 * 1024 * 1024;

        let auto_reserve = estimated_runtime_reserve_bytes(
            &large_context_metadata(),
            available,
            32_768,
            2048,
            llama_cpp_sys_2::LLAMA_FLASH_ATTN_TYPE_AUTO,
        );
        let enabled_reserve = estimated_runtime_reserve_bytes(
            &large_context_metadata(),
            available,
            32_768,
            2048,
            llama_cpp_sys_2::LLAMA_FLASH_ATTN_TYPE_ENABLED,
        );

        assert_eq!(auto_reserve, enabled_reserve);
        assert_eq!(auto_reserve, available / 20);
    }

    #[test]
    fn metadata_counts_output_tensor_as_an_offload_layer() {
        let metadata = large_context_metadata();

        assert_eq!(metadata.offload_layer_count(), 61);
        assert_eq!(metadata.normalize_requested_gpu_layers(59), 59);
        assert_eq!(metadata.normalize_requested_gpu_layers(60), 61);
        assert_eq!(metadata.normalize_requested_gpu_layers(99), 61);
    }

    #[test]
    fn metadata_counts_bundled_nextn_and_output_layers() {
        let metadata = LlamaModelMetadata {
            nextn_layer_count: 1,
            ..large_context_metadata()
        };

        assert_eq!(metadata.model_layer_count(), 61);
        assert_eq!(metadata.offload_layer_count(), 62);
        assert_eq!(metadata.normalize_requested_gpu_layers(60), 62);
        assert_eq!(metadata.normalize_requested_gpu_layers(62), 62);
    }

    #[test]
    fn candidate_ladder_does_not_exceed_the_vram_estimate() {
        let candidates = candidate_gpu_layers(61, 60);

        assert_eq!(candidates.first(), Some(&60));
        assert!(!candidates.contains(&61));
        assert_eq!(candidates.last(), Some(&0));
    }

    #[test]
    fn full_offload_places_all_model_weights_on_gpu() {
        let metadata = large_context_metadata();

        let (cpu_bytes, gpu_bytes) =
            model_weight_split_bytes(&metadata, None, metadata.offload_layer_count());

        assert_eq!(cpu_bytes, 0);
        assert_eq!(gpu_bytes, metadata.model_size_bytes);
    }

    #[test]
    fn proportional_distribution_caps_total_to_per_device_free_capacity() {
        let dist = plan_multi_gpu_distribution("proportional", &[8, 24], 60, 1, 0, 60, None, None);

        assert_eq!(dist.n_gpu_layers, 32);
        assert_eq!(dist.per_device_layers, vec![8, 24]);
    }

    #[test]
    fn balanced_distribution_keeps_even_split_for_identical_cards() {
        let dist = plan_multi_gpu_distribution("balanced", &[16, 16], 60, 1, 0, 32, None, None);

        assert_eq!(dist.n_gpu_layers, 32);
        assert_eq!(dist.per_device_layers, vec![16, 16]);
        assert_eq!(dist.tensor_split, vec![0.5, 0.5]);
    }

    #[test]
    fn balanced_distribution_respects_a_sidecar_reduced_device_budget() {
        let dist = plan_multi_gpu_distribution("balanced", &[4, 16], 20, 1, 0, 16, None, None);

        assert_eq!(dist.n_gpu_layers, 16);
        assert_eq!(dist.per_device_layers, vec![4, 12]);
        assert_eq!(dist.tensor_split, vec![0.25, 0.75]);
    }
}

#[cfg(test)]
mod offload_cost_tests {
    use super::*;

    fn costs(units: &[u64]) -> ModelOffloadCosts {
        ModelOffloadCosts {
            unit_bytes: units.to_vec(),
        }
    }

    #[test]
    fn gpu_bytes_takes_the_last_units_output_layer_first() {
        // blocks 0..2 then the output layer, matching llama.cpp's il ordering.
        let costs = costs(&[10, 20, 30, 1000]);
        assert_eq!(costs.gpu_bytes(0), 0);
        assert_eq!(
            costs.gpu_bytes(1),
            1000,
            "first unit offloaded is the output"
        );
        assert_eq!(costs.gpu_bytes(2), 1030);
        assert_eq!(costs.gpu_bytes(4), 1060);
        assert_eq!(costs.gpu_bytes(99), 1060, "saturates at the unit count");
    }

    #[test]
    fn a_heavy_output_layer_is_not_averaged_away() {
        // The failure this guards: a flat file average prices the output layer
        // like a block and overshoots. 1060 total over 4 units averages 265,
        // which would claim 3 units fit in 900. Only one actually does.
        let costs = costs(&[10, 20, 30, 1000]);
        assert_eq!(costs.max_units_within(900, 0, 3), 0);
        assert_eq!(costs.max_units_within(1000, 0, 3), 1);
        assert_eq!(costs.max_units_within(1029, 0, 3), 1);
        assert_eq!(costs.max_units_within(1030, 0, 3), 2);
    }

    #[test]
    fn kv_is_charged_per_block_but_not_for_the_output_unit() {
        let costs = costs(&[10, 20, 30, 1000]);
        // Output alone: no KV.
        assert_eq!(costs.max_units_within(1000, 5, 3), 1);
        // Output + one block: one KV charge.
        assert_eq!(costs.max_units_within(1034, 5, 3), 1);
        assert_eq!(costs.max_units_within(1035, 5, 3), 2);
    }

    #[test]
    fn kv_charges_stop_at_the_attention_layer_count() {
        // nextn blocks sit in the unit list but are not attention layers.
        let costs = costs(&[10, 20, 30, 1000]);
        assert_eq!(
            costs.max_units_within(1030, 5, 0),
            2,
            "no KV charged at all"
        );
    }

    #[test]
    fn block_index_parses_only_repeating_layers() {
        assert_eq!(block_index("blk.0.attn_q.weight"), Some(0));
        assert_eq!(block_index("blk.64.nextn.eh_proj.weight"), Some(64));
        assert_eq!(block_index("output.weight"), None);
        assert_eq!(block_index("token_embd.weight"), None);
        assert_eq!(block_index("blk.notanumber.weight"), None);
    }
}
