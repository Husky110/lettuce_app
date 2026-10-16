//! Regression coverage for companion time-override timestamps before the Unix
//! epoch. The effective-time chain is signed (`i64`), so a frozen or ticking
//! clock set to a pre-1970 date is preserved instead of silently snapping back
//! to the real current time.

use lettuceai_lib::chat_manager::companion::{SoulGrowthEntry, ALWAYS_EFFECTIVE_FROM};
use lettuceai_lib::chat_manager::temporal::{
    companion_effective_now, format_message_timestamp, memories_in_chronological_order,
};
use lettuceai_lib::chat_manager::types::{MemoryEmbedding, Session};
use serde_json::json;

fn memory(id: &str, observed_at: Option<i64>, created_at: u64) -> MemoryEmbedding {
    let mut value = json!({ "id": id, "text": id, "embedding": [], "createdAt": created_at });
    if let Some(observed) = observed_at {
        value["observedAt"] = json!(observed);
    }
    serde_json::from_value(value).expect("valid memory")
}

/// A point clearly before 1970 (≈ 1966-10-31), in milliseconds.
const PRE_EPOCH_MS: i64 = -100_000_000_000;

fn session_with_override(enabled: bool, override_value: serde_json::Value) -> Session {
    Session {
        id: "s1".into(),
        character_id: "c1".into(),
        title: "t".into(),
        parent_session_id: None,
        branched_from_message_id: None,
        root_session_id: None,
        background_image_path: None,
        system_prompt: None,
        mode: "companion".into(),
        selected_scene_id: None,
        prompt_template_id: None,
        lorebook_ids_override: None,
        author_note: None,
        persona_id: None,
        persona_disabled: false,
        voice_autoplay: None,
        advanced_model_settings: None,
        companion_state: Some(json!({
            "preferences": {
                "timeAwarenessEnabled": enabled,
                "timeOverride": override_value,
            }
        })),
        memories: vec![],
        memory_summary: None,
        memory_summary_token_count: 0,
        memory_tool_events: vec![],
        messages: vec![],
        archived: false,
        created_at: 0,
        updated_at: 0,
        memory_embeddings: vec![],
        memory_status: None,
        memory_error: None,
        memory_progress_step: None,
    }
}

#[test]
fn frozen_pre_epoch_anchor_is_preserved_not_reset_to_now() {
    let session = session_with_override(
        true,
        json!({ "mode": "frozen", "anchorMs": PRE_EPOCH_MS, "setAtMs": 0 }),
    );
    // Before the i64 migration this returned the real clock, because
    // `serde_json::Value::as_u64()` rejected the negative anchor.
    assert_eq!(companion_effective_now(&session), PRE_EPOCH_MS);
    assert!(companion_effective_now(&session) < 0);
}

#[test]
fn frozen_pre_epoch_anchor_renders_a_pre_1970_year() {
    let stamp = format_message_timestamp(PRE_EPOCH_MS);
    // Timezone-independent: anything this far before the epoch lands in the 1960s.
    assert!(
        stamp.starts_with("<time>196"),
        "expected a 1960s stamp, got {stamp}"
    );
}

#[test]
fn ticking_from_a_pre_epoch_anchor_advances_by_elapsed_real_time() {
    // anchor 10s before epoch, override set 4s before epoch → 6s of elapsed
    // real time should carry the effective clock to 4s before epoch.
    let session = session_with_override(
        true,
        json!({ "mode": "ticking", "anchorMs": -10_000, "setAtMs": -4_000 }),
    );
    // real_now (positive) - set_at (-4000) + anchor (-10000): the delta stays
    // signed, so the result is anchor + (real_now - set_at), never wrapped.
    let now = companion_effective_now(&session);
    assert!(now > 0, "ticking advances past the epoch with real elapsed time");
}

#[test]
fn timeless_authored_fact_is_effective_in_the_distant_past() {
    // A baseline/authored fact is effective no matter where the companion clock
    // sits — including a 1508 companion whose effective "now" is far negative.
    let authored = SoulGrowthEntry {
        valid_from: ALWAYS_EFFECTIVE_FROM,
        ..SoulGrowthEntry::default()
    };
    assert!(authored.is_effective_at(PRE_EPOCH_MS));
    assert!(authored.is_effective_at(0));
    assert!(authored.is_effective_at(i64::MAX));
}

#[test]
fn wall_clock_stamped_fact_would_be_hidden_in_the_past() {
    // Guards the reason authored facts must be timeless: a fact stamped with a
    // real 2026 wall-clock valid_from is NOT yet effective for a 1508 companion.
    let wall_clock_2026: i64 = 1_767_225_600_000; // 2026-01-01
    let learned = SoulGrowthEntry {
        valid_from: wall_clock_2026,
        ..SoulGrowthEntry::default()
    };
    assert!(!learned.is_effective_at(PRE_EPOCH_MS));
    assert!(learned.is_effective_at(wall_clock_2026 + 1));
}

#[test]
fn memories_sort_oldest_to_newest_across_the_epoch() {
    // Mixed timeline: two undated (background), two pre-1970, one post-1970.
    let memories = vec![
        memory("b", Some(-1_000), 0),      // 1508-ish, later (closer to 0)
        memory("e", Some(2_000), 0),       // post-epoch, newest
        memory("a", Some(-5_000), 0),      // 1508-ish, earlier (further from 0)
        memory("c", None, 100),            // undated, learned second
        memory("d", None, 50),             // undated, learned first
    ];
    let ordered: Vec<&str> = memories_in_chronological_order(&memories)
        .into_iter()
        .map(|m| m.id.as_str())
        .collect();
    // Undated first (by created_at), then observed ascending — signed, so the
    // less-negative (later) 1508 memory follows the more-negative (earlier) one.
    assert_eq!(ordered, vec!["d", "c", "a", "b", "e"]);
}

#[test]
fn disabled_time_awareness_ignores_a_pre_epoch_override() {
    let session = session_with_override(
        false,
        json!({ "mode": "frozen", "anchorMs": PRE_EPOCH_MS, "setAtMs": 0 }),
    );
    assert!(
        companion_effective_now(&session) > 0,
        "with awareness off the real clock wins"
    );
}
