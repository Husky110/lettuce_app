use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value as JsonValue;
use tauri::AppHandle;

use super::db::open_db;
use crate::storage_manager::memory_embeddings::SessionKind;

#[derive(Clone, Debug)]
pub struct EffectiveMemoryOwner {
    pub owner_id: String,
    pub kind: SessionKind,
    pub shared: bool,
}

#[derive(Clone, Debug)]
pub struct SharedMemoryState {
    pub memories_json: String,
    pub memory_summary: Option<String>,
    pub memory_summary_token_count: i64,
    pub memory_tool_events_json: String,
    pub memory_status: Option<String>,
    pub memory_error: Option<String>,
    pub memory_progress_step: Option<i64>,
    pub soul_growth_json: String,
    pub relationship_states_json: String,
}

impl Default for SharedMemoryState {
    fn default() -> Self {
        Self {
            memories_json: "[]".to_string(),
            memory_summary: None,
            memory_summary_token_count: 0,
            memory_tool_events_json: "[]".to_string(),
            memory_status: None,
            memory_error: None,
            memory_progress_step: None,
            soul_growth_json: "[]".to_string(),
            relationship_states_json: "{}".to_string(),
        }
    }
}

pub fn character_uses_companion_mode(
    conn: &Connection,
    character_id: &str,
    mode: &str,
) -> Result<bool, String> {
    let row: Option<(Option<String>, Option<String>)> = conn
        .query_row(
            "SELECT companion, mode FROM characters WHERE id = ?1",
            params![character_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|e| crate::utils::err_to_string(module_path!(), line!(), e))?;

    let (_, character_mode) = match row {
        Some(value) => value,
        None => return Ok(false),
    };

    let is_companion = mode.eq_ignore_ascii_case("companion")
        || character_mode
            .as_deref()
            .map(|m| m.eq_ignore_ascii_case("companion"))
            .unwrap_or(false);
    Ok(is_companion)
}

fn companion_shared_memory_enabled_for_character(
    conn: &Connection,
    character_id: &str,
    mode: &str,
) -> Result<bool, String> {
    if !character_uses_companion_mode(conn, character_id, mode)? {
        return Ok(false);
    }

    let companion_json = conn
        .query_row(
            "SELECT companion FROM characters WHERE id = ?1",
            params![character_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()
        .map_err(|e| crate::utils::err_to_string(module_path!(), line!(), e))?
        .flatten();

    Ok(companion_json
        .as_deref()
        .map(
            crate::chat_manager::companion::shared_memory_across_sessions_enabled_from_companion_json,
        )
        .unwrap_or(false))
}

pub fn resolve_effective_memory_owner(
    conn: &Connection,
    session_id: &str,
    character_id: &str,
    mode: &str,
) -> Result<EffectiveMemoryOwner, String> {
    let shared = companion_shared_memory_enabled_for_character(conn, character_id, mode)?;
    Ok(if shared {
        EffectiveMemoryOwner {
            owner_id: character_id.to_string(),
            kind: SessionKind::CompanionShared,
            shared: true,
        }
    } else {
        EffectiveMemoryOwner {
            owner_id: session_id.to_string(),
            kind: SessionKind::Session,
            shared: false,
        }
    })
}

pub fn resolve_effective_memory_owner_for_session(
    conn: &Connection,
    session_id: &str,
) -> Result<EffectiveMemoryOwner, String> {
    let (character_id, mode): (String, String) = conn
        .query_row(
            "SELECT character_id, mode FROM sessions WHERE id = ?1",
            params![session_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|e| crate::utils::err_to_string(module_path!(), line!(), e))?;
    resolve_effective_memory_owner(conn, session_id, &character_id, &mode)
}

pub fn resolve_effective_memory_owner_for_session_app(
    app: &AppHandle,
    session_id: &str,
) -> Result<EffectiveMemoryOwner, String> {
    let conn = open_db(app)?;
    resolve_effective_memory_owner_for_session(&conn, session_id)
}

pub fn load_state(conn: &Connection, character_id: &str) -> Result<SharedMemoryState, String> {
    conn.query_row(
        "SELECT memories, memory_summary, memory_summary_token_count, memory_tool_events, memory_status, memory_error, memory_progress_step, soul_growth, relationship_states
         FROM companion_shared_memory_state WHERE character_id = ?1",
        params![character_id],
        |row| {
            Ok(SharedMemoryState {
                memories_json: row.get::<_, String>(0)?,
                memory_summary: row.get::<_, Option<String>>(1)?,
                memory_summary_token_count: row.get::<_, i64>(2)?,
                memory_tool_events_json: row.get::<_, String>(3)?,
                memory_status: row.get::<_, Option<String>>(4)?,
                memory_error: row.get::<_, Option<String>>(5)?,
                memory_progress_step: row.get::<_, Option<i64>>(6)?,
                soul_growth_json: row.get::<_, String>(7)?,
                relationship_states_json: row.get::<_, String>(8)?,
            })
        },
    )
    .optional()
    .map_err(|e| crate::utils::err_to_string(module_path!(), line!(), e))?
    .map(Ok)
    .unwrap_or_else(|| Ok(SharedMemoryState::default()))
}

pub fn upsert_state(
    conn: &Connection,
    character_id: &str,
    state: &SharedMemoryState,
) -> Result<(), String> {
    let now = super::db::now_ms() as i64;
    conn.execute(
        r#"
        INSERT INTO companion_shared_memory_state (
            character_id, memories, memory_summary, memory_summary_token_count,
            memory_tool_events, memory_status, memory_error, memory_progress_step,
            soul_growth, relationship_states, created_at, updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11)
        ON CONFLICT(character_id) DO UPDATE SET
            memories = excluded.memories,
            memory_summary = excluded.memory_summary,
            memory_summary_token_count = excluded.memory_summary_token_count,
            memory_tool_events = excluded.memory_tool_events,
            memory_status = excluded.memory_status,
            memory_error = excluded.memory_error,
            memory_progress_step = excluded.memory_progress_step,
            soul_growth = excluded.soul_growth,
            relationship_states = excluded.relationship_states,
            updated_at = excluded.updated_at
        "#,
        params![
            character_id,
            &state.memories_json,
            state.memory_summary.as_deref(),
            state.memory_summary_token_count,
            &state.memory_tool_events_json,
            state.memory_status.as_deref(),
            state.memory_error.as_deref(),
            state.memory_progress_step,
            &state.soul_growth_json,
            &state.relationship_states_json,
            now,
        ],
    )
    .map_err(|e| crate::utils::err_to_string(module_path!(), line!(), e))?;
    Ok(())
}

pub fn export_all(app: &AppHandle) -> Result<Vec<JsonValue>, String> {
    let conn = open_db(app)?;
    let mut stmt = conn
        .prepare(
            r#"
            SELECT character_id, memories, memory_summary, memory_summary_token_count,
                   memory_tool_events, memory_status, memory_error, memory_progress_step,
                   soul_growth, relationship_states, created_at, updated_at
            FROM companion_shared_memory_state
            ORDER BY character_id ASC
            "#,
        )
        .map_err(|e| crate::utils::err_to_string(module_path!(), line!(), e))?;

    let rows = stmt
        .query_map([], |row| {
            Ok(serde_json::json!({
                "character_id": row.get::<_, String>(0)?,
                "memories": row.get::<_, String>(1)?,
                "memory_summary": row.get::<_, Option<String>>(2)?,
                "memory_summary_token_count": row.get::<_, i64>(3)?,
                "memory_tool_events": row.get::<_, String>(4)?,
                "memory_status": row.get::<_, Option<String>>(5)?,
                "memory_error": row.get::<_, Option<String>>(6)?,
                "memory_progress_step": row.get::<_, Option<i64>>(7)?,
                "soul_growth": row.get::<_, String>(8)?,
                "relationship_states": row.get::<_, String>(9)?,
                "created_at": row.get::<_, i64>(10)?,
                "updated_at": row.get::<_, i64>(11)?,
            }))
        })
        .map_err(|e| crate::utils::err_to_string(module_path!(), line!(), e))?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| crate::utils::err_to_string(module_path!(), line!(), e))
}

fn relationship_key(persona_id: Option<&str>) -> &str {
    persona_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("__default__")
}

pub fn merge_continuity_into_state(
    conn: &Connection,
    character_id: &str,
    persona_id: Option<&str>,
    state: Option<JsonValue>,
) -> Result<Option<JsonValue>, String> {
    let Some(mut state) = state else {
        return Ok(None);
    };
    let Some(state_object) = state.as_object_mut() else {
        return Ok(Some(state));
    };

    let continuity = load_state(conn, character_id)?;
    let soul_growth = serde_json::from_str::<JsonValue>(&continuity.soul_growth_json)
        .unwrap_or_else(|_| JsonValue::Array(Vec::new()));
    if soul_growth.as_array().is_some_and(|items| !items.is_empty()) {
        state_object.insert("soulGrowth".to_string(), soul_growth);
    }

    let relationship_states =
        serde_json::from_str::<JsonValue>(&continuity.relationship_states_json)
            .unwrap_or_else(|_| JsonValue::Object(Default::default()));
    if let Some(relationship) = relationship_states
        .get(relationship_key(persona_id))
        .filter(|value| value.is_object())
    {
        state_object.insert("relationshipState".to_string(), relationship.clone());
    }

    Ok(Some(state))
}

pub fn persist_continuity_from_state(
    conn: &Connection,
    character_id: &str,
    persona_id: Option<&str>,
    state: &JsonValue,
) -> Result<(), String> {
    let Some(state_object) = state.as_object() else {
        return Ok(());
    };
    let mut continuity = load_state(conn, character_id)?;

    if let Some(soul_growth) = state_object.get("soulGrowth").filter(|value| value.is_array()) {
        continuity.soul_growth_json =
            serde_json::to_string(soul_growth).unwrap_or_else(|_| "[]".to_string());
    }

    if let Some(relationship) = state_object
        .get("relationshipState")
        .filter(|value| value.is_object())
    {
        let mut relationships =
            serde_json::from_str::<JsonValue>(&continuity.relationship_states_json)
                .ok()
                .and_then(|value| value.as_object().cloned())
                .unwrap_or_default();
        relationships.insert(
            relationship_key(persona_id).to_string(),
            relationship.clone(),
        );
        continuity.relationship_states_json =
            serde_json::to_string(&relationships).unwrap_or_else(|_| "{}".to_string());
    }

    upsert_state(conn, character_id, &continuity)
}

#[cfg(test)]
mod tests {
    use super::{merge_continuity_into_state, persist_continuity_from_state};
    use rusqlite::Connection;
    use serde_json::json;

    fn connection() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE companion_shared_memory_state (
               character_id TEXT PRIMARY KEY,
               memories TEXT NOT NULL DEFAULT '[]',
               memory_summary TEXT,
               memory_summary_token_count INTEGER NOT NULL DEFAULT 0,
               memory_tool_events TEXT NOT NULL DEFAULT '[]',
               memory_status TEXT,
               memory_error TEXT,
               memory_progress_step INTEGER,
               soul_growth TEXT NOT NULL DEFAULT '[]',
               relationship_states TEXT NOT NULL DEFAULT '{}',
               created_at INTEGER NOT NULL,
               updated_at INTEGER NOT NULL
             );",
        )
        .unwrap();
        conn
    }

    #[test]
    fn continuity_carries_soul_and_persona_relationship_between_sessions() {
        let conn = connection();
        let first = json!({
            "soulGrowth": [{"id": "growth-1", "category": "goals", "value": "Protect the garden"}],
            "relationshipState": {"trust": 0.82, "interactionCount": 14},
            "emotionalState": {"felt": {"calm": 0.1}}
        });
        persist_continuity_from_state(&conn, "character-1", Some("persona-1"), &first)
            .unwrap();

        let fresh_session = json!({
            "soulGrowth": [],
            "relationshipState": {"trust": 0.2, "interactionCount": 0},
            "emotionalState": {"felt": {"calm": 0.9}}
        });
        let hydrated = merge_continuity_into_state(
            &conn,
            "character-1",
            Some("persona-1"),
            Some(fresh_session),
        )
        .unwrap()
        .unwrap();

        assert_eq!(hydrated["soulGrowth"][0]["id"], "growth-1");
        assert_eq!(hydrated["relationshipState"]["trust"], 0.82);
        assert_eq!(hydrated["relationshipState"]["interactionCount"], 14);
        assert_eq!(hydrated["emotionalState"]["felt"]["calm"], 0.9);
    }

    #[test]
    fn relationships_remain_isolated_per_persona() {
        let conn = connection();
        persist_continuity_from_state(
            &conn,
            "character-1",
            Some("persona-1"),
            &json!({"relationshipState": {"trust": 0.9}}),
        )
        .unwrap();
        persist_continuity_from_state(
            &conn,
            "character-1",
            Some("persona-2"),
            &json!({"relationshipState": {"trust": -0.4}}),
        )
        .unwrap();

        let first = merge_continuity_into_state(
            &conn,
            "character-1",
            Some("persona-1"),
            Some(json!({"relationshipState": {"trust": 0.0}})),
        )
        .unwrap()
        .unwrap();
        let second = merge_continuity_into_state(
            &conn,
            "character-1",
            Some("persona-2"),
            Some(json!({"relationshipState": {"trust": 0.0}})),
        )
        .unwrap()
        .unwrap();

        assert_eq!(first["relationshipState"]["trust"], 0.9);
        assert_eq!(second["relationshipState"]["trust"], -0.4);
    }
}
