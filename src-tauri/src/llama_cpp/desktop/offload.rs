use super::engine::shared_backend;
use llama_cpp_2::model::{params::LlamaModelParams, LlamaModel};
use llama_cpp_sys_2::llama_flash_attn_type;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

#[derive(Clone, Copy, Debug)]
pub(super) struct LlamaModelMetadata {
    pub(super) model_size_bytes: u64,
    pub(super) layer_count: u32,
    pub(super) max_context_length: u32,
    pub(super) n_embd: u64,
    pub(super) n_head: u64,
    pub(super) n_head_kv: u64,
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
    // If the estimate is within 10% of total, optimistically try all layers first.
    // The smart fallback ladder will step down gracefully on OOM.
    if estimate >= total_layers.saturating_mul(9) / 10 {
        push_unique(&mut candidates, total_layers);
    }
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
    gpu_layers: u32,
) -> (u64, u64) {
    let total_layers = metadata.layer_count.max(1);
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
    available_memory_bytes: Option<u64>,
    available_vram_bytes: Option<u64>,
    gpu_layers: u32,
    llama_offload_kqv: Option<bool>,
    llama_kv_type: Option<&str>,
    sidecar_vram_reserve_bytes: u64,
) -> Option<u32> {
    let (cpu_weight_bytes, gpu_weight_bytes) = model_weight_split_bytes(metadata, gpu_layers);
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
) -> Result<SmartGpuOffloadPlan, String> {
    let metadata = load_model_metadata(model_path)?;
    let total_layers = metadata.layer_count.max(1);
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
        .checked_add(u64::from(total_layers) - 1)
        .and_then(|bytes| bytes.checked_div(u64::from(total_layers)))
        .unwrap_or(0);
    let kv_bytes_per_token = estimate_kv_bytes_per_token(&metadata, llama_kv_type).unwrap_or(0);

    let planning_modes: &[Option<bool>] = match resolved_offload_kqv {
        Some(true) => &[Some(true)],
        Some(false) => &[Some(false)],
        None => &[Some(false), Some(true), None],
    };

    let mut selected_plan: Option<(Option<bool>, bool, u64, u64, u32)> = None;
    for planning_offload_kqv in planning_modes {
        let kqv_vram_reserved = *planning_offload_kqv == Some(true);
        // When KV is GPU-resident, only the GPU-resident layers' KV goes to VRAM —
        // not the full model's KV. Include KV cost in the per-layer price so the
        // estimate self-corrects: more layers → more KV, fewer layers → less KV.
        let kv_bytes_per_layer = if kqv_vram_reserved {
            kv_bytes_per_token
                .saturating_mul(u64::from(planned_context))
                .checked_div(u64::from(total_layers.max(1)))
                .unwrap_or(0)
        } else {
            0
        };
        let effective_bytes_per_layer = bytes_per_layer.saturating_add(kv_bytes_per_layer);
        let available_base = effective_vram_budget_bytes
            .saturating_sub(estimated_runtime_reserve_bytes)
            .saturating_sub(sidecar_vram_reserve_bytes);
        let estimated_gpu_layers = if available_base == 0 || effective_bytes_per_layer == 0 {
            0
        } else {
            u32::try_from((available_base / effective_bytes_per_layer).min(u64::from(total_layers)))
                .unwrap_or(total_layers)
                .min(total_layers)
        };
        // Report the KV bytes that will actually land on GPU (scales with GPU layers).
        let estimated_kv_bytes = kv_bytes_per_layer.saturating_mul(u64::from(estimated_gpu_layers));

        if selected_plan.is_none() {
            selected_plan = Some((
                *planning_offload_kqv,
                kqv_vram_reserved,
                estimated_kv_bytes,
                kv_bytes_per_layer,
                estimated_gpu_layers,
            ));
        }

        if estimated_gpu_layers > 0 || *planning_offload_kqv == Some(false) {
            selected_plan = Some((
                *planning_offload_kqv,
                kqv_vram_reserved,
                estimated_kv_bytes,
                kv_bytes_per_layer,
                estimated_gpu_layers,
            ));
            if estimated_gpu_layers > 0 {
                break;
            }
        }
    }

    let (
        planning_offload_kqv,
        kqv_vram_reserved,
        estimated_kv_bytes,
        kv_bytes_per_layer,
        estimated_gpu_layers,
    ) = selected_plan.unwrap_or((Some(false), false, 0, 0, 0));

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
            let split = vec![1.0f32; n];
            MultiGpuDistribution {
                n_gpu_layers: auto_total,
                per_device_layers: distribute_by_weights(auto_total, &split),
                tensor_split: if auto_total > 0 { split } else { Vec::new() },
                main_gpu: None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        candidate_gpu_layers, estimated_runtime_reserve_bytes, plan_multi_gpu_distribution,
        LlamaModelMetadata,
    };

    fn large_context_metadata() -> LlamaModelMetadata {
        LlamaModelMetadata {
            model_size_bytes: 16 * 1024 * 1024 * 1024,
            layer_count: 60,
            max_context_length: 262_144,
            n_embd: 4096,
            n_head: 32,
            n_head_kv: 8,
        }
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
    fn candidate_ladder_tries_full_offload_when_estimate_is_near_total() {
        let candidates = candidate_gpu_layers(60, 55);

        assert_eq!(candidates.first(), Some(&60));
        assert!(candidates.contains(&55));
        assert_eq!(candidates.last(), Some(&0));
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
        assert_eq!(dist.tensor_split, vec![1.0, 1.0]);
    }
}
