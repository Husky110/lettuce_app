use regex::Regex;
use rusqlite::{params, ToSql};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

use crate::storage_manager::db::open_db;

const DEFAULT_SCOPE: &str = "global";
const MAX_PROMPT_CHARS: usize = 240;
const MAX_PROMPT_TERMS: usize = 24;
const MAX_REPLACEMENT_WORDS: usize = 5;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AsrVocabularyTerm {
    pub id: Option<i64>,
    pub term: String,
    pub normalized_term: Option<String>,
    pub language: Option<String>,
    pub category: Option<String>,
    pub scope: Option<String>,
    pub priority: Option<i64>,
    pub use_count: Option<i64>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AsrCorrection {
    pub id: Option<i64>,
    pub wrong: String,
    pub normalized_wrong: Option<String>,
    pub correct: String,
    pub normalized_correct: Option<String>,
    pub language: Option<String>,
    pub scope: Option<String>,
    pub confidence: Option<f64>,
    pub use_count: Option<i64>,
    pub user_approved: Option<bool>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AsrVoiceExample {
    pub id: Option<i64>,
    pub audio_path: String,
    pub expected_text: String,
    pub normalized_expected_text: Option<String>,
    pub whisper_output: Option<String>,
    pub normalized_whisper_output: Option<String>,
    pub language: Option<String>,
    pub scope: Option<String>,
    pub term_id: Option<i64>,
    pub correction_id: Option<i64>,
    pub created_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AsrCorrectionApplication {
    pub correction_id: i64,
    pub wrong: String,
    pub correct: String,
    pub match_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AsrCorrectionResult {
    pub raw_text: String,
    pub corrected_text: String,
    pub applied: Vec<AsrCorrectionApplication>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AsrLearnedSuggestion {
    pub wrong: String,
    pub normalized_wrong: String,
    pub correct: String,
    pub normalized_correct: String,
    pub language: Option<String>,
    pub scope: String,
    pub confidence: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AsrExportBundle {
    pub version: u32,
    pub vocabulary: Vec<AsrVocabularyTerm>,
    pub corrections: Vec<AsrCorrection>,
    pub voice_examples: Vec<AsrVoiceExample>,
}

#[derive(Debug, Clone)]
struct StoredCorrection {
    id: i64,
    wrong: String,
    correct: String,
}

fn normalize_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn normalize_lookup_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last_was_space = true;
    for ch in text.chars() {
        if ch.is_alphanumeric() {
            out.extend(ch.to_lowercase());
            last_was_space = false;
        } else if !last_was_space {
            out.push(' ');
            last_was_space = true;
        }
    }
    out.trim().to_string()
}

fn normalize_scope(scope: Option<&str>) -> String {
    let value = scope.unwrap_or(DEFAULT_SCOPE).trim();
    if value.is_empty() {
        DEFAULT_SCOPE.to_string()
    } else {
        value.to_ascii_lowercase()
    }
}

fn normalize_language(language: Option<&str>) -> Option<String> {
    language
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
}

fn correction_regex_for_phrase(phrase: &str) -> Result<Regex, String> {
    let escaped_parts: Vec<String> = phrase.split_whitespace().map(regex::escape).collect();
    let pattern = format!(r"(?i)\b{}\b", escaped_parts.join(r"\s+"));
    Regex::new(&pattern).map_err(|e| crate::utils::err_to_string(module_path!(), line!(), e))
}

fn tokenize_words(text: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        if ch.is_alphanumeric() || ch == '\'' {
            current.push(ch);
        } else if !current.is_empty() {
            words.push(current.clone());
            current.clear();
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

fn stopwords() -> HashSet<&'static str> {
    [
        "a", "an", "and", "are", "be", "but", "can", "did", "do", "for", "from", "go", "have",
        "he", "her", "him", "i", "in", "is", "it", "its", "me", "my", "not", "of", "on", "or",
        "our", "she", "that", "the", "their", "them", "there", "they", "this", "to", "us", "was",
        "we", "were", "with", "you", "your",
    ]
    .into_iter()
    .collect()
}

fn is_low_value_replacement(before: &[String], after: &[String]) -> bool {
    let stopwords = stopwords();
    let before_all_common = before
        .iter()
        .all(|token| stopwords.contains(normalize_lookup_text(token).as_str()));
    let after_all_common = after
        .iter()
        .all(|token| stopwords.contains(normalize_lookup_text(token).as_str()));
    before_all_common && after_all_common
}

fn lcs_matches(before: &[String], after: &[String]) -> Vec<(usize, usize)> {
    let n = before.len();
    let m = after.len();
    let mut dp = vec![vec![0usize; m + 1]; n + 1];

    for i in (0..n).rev() {
        for j in (0..m).rev() {
            if before[i] == after[j] {
                dp[i][j] = dp[i + 1][j + 1] + 1;
            } else {
                dp[i][j] = dp[i + 1][j].max(dp[i][j + 1]);
            }
        }
    }

    let mut i = 0usize;
    let mut j = 0usize;
    let mut matches = Vec::new();
    while i < n && j < m {
        if before[i] == after[j] {
            matches.push((i, j));
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }

    matches
}

fn map_scopes_for_query(scopes: &[String]) -> Vec<String> {
    if scopes.is_empty() {
        vec![DEFAULT_SCOPE.to_string()]
    } else {
        scopes
            .iter()
            .map(|scope| normalize_scope(Some(scope)))
            .collect()
    }
}

fn query_scope_clause(scopes_len: usize) -> String {
    let placeholders = vec!["?"; scopes_len].join(", ");
    format!("scope IN ({})", placeholders)
}

fn scope_and_language_params<'a>(
    scopes: &'a [String],
    language: &'a Option<String>,
) -> Vec<&'a dyn ToSql> {
    let mut values: Vec<&dyn ToSql> = Vec::with_capacity(scopes.len() + 2);
    for scope in scopes {
        values.push(scope as &dyn ToSql);
    }
    values.push(language as &dyn ToSql);
    values.push(language as &dyn ToSql);
    values
}

#[tauri::command]
pub fn asr_vocabulary_list(
    app: tauri::AppHandle,
    language: Option<String>,
    scopes: Option<Vec<String>>,
) -> Result<Vec<AsrVocabularyTerm>, String> {
    let conn = open_db(&app)?;
    let scopes = map_scopes_for_query(&scopes.unwrap_or_default());
    let language = normalize_language(language.as_deref());
    let sql = format!(
        "SELECT id, term, normalized_term, language, category, scope, priority, use_count, created_at, updated_at
         FROM asr_vocabulary_terms
         WHERE ({})
           AND (? IS NULL OR language IS NULL OR language = ?)
         ORDER BY priority DESC, use_count DESC, updated_at DESC, id DESC",
        query_scope_clause(scopes.len())
    );
    let params = scope_and_language_params(&scopes, &language);
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| crate::utils::err_to_string(module_path!(), line!(), e))?;
    let terms = stmt
        .query_map(rusqlite::params_from_iter(params), |row| {
            Ok(AsrVocabularyTerm {
                id: Some(row.get(0)?),
                term: row.get(1)?,
                normalized_term: Some(row.get(2)?),
                language: row.get(3)?,
                category: row.get(4)?,
                scope: Some(row.get(5)?),
                priority: Some(row.get(6)?),
                use_count: Some(row.get(7)?),
                created_at: Some(row.get(8)?),
                updated_at: Some(row.get(9)?),
            })
        })
        .map_err(|e| crate::utils::err_to_string(module_path!(), line!(), e))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| crate::utils::err_to_string(module_path!(), line!(), e))?;
    Ok(terms)
}

#[tauri::command]
pub fn asr_vocabulary_upsert(
    app: tauri::AppHandle,
    term: AsrVocabularyTerm,
) -> Result<AsrVocabularyTerm, String> {
    let conn = open_db(&app)?;
    let normalized_term = normalize_lookup_text(&term.term);
    if normalized_term.is_empty() {
        return Err(crate::utils::err_msg(
            module_path!(),
            line!(),
            "Vocabulary term cannot be empty",
        ));
    }

    let language = normalize_language(term.language.as_deref());
    let scope = normalize_scope(term.scope.as_deref());
    let priority = term.priority.unwrap_or(50);
    let use_count = term.use_count.unwrap_or(0);

    match term.id {
        Some(id) => {
            conn.execute(
                "UPDATE asr_vocabulary_terms
                 SET term = ?1,
                     normalized_term = ?2,
                     language = ?3,
                     category = ?4,
                     scope = ?5,
                     priority = ?6,
                     use_count = ?7,
                     updated_at = CURRENT_TIMESTAMP
                 WHERE id = ?8",
                params![
                    term.term,
                    normalized_term,
                    language,
                    term.category,
                    scope,
                    priority,
                    use_count,
                    id
                ],
            )
            .map_err(|e| crate::utils::err_to_string(module_path!(), line!(), e))?;
            Ok(AsrVocabularyTerm {
                id: Some(id),
                term: term.term,
                normalized_term: Some(normalized_term),
                language,
                category: term.category,
                scope: Some(scope),
                priority: Some(priority),
                use_count: Some(use_count),
                created_at: None,
                updated_at: None,
            })
        }
        None => {
            conn.execute(
                "INSERT INTO asr_vocabulary_terms
                 (term, normalized_term, language, category, scope, priority, use_count)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    term.term,
                    normalized_term,
                    language,
                    term.category,
                    scope,
                    priority,
                    use_count
                ],
            )
            .map_err(|e| crate::utils::err_to_string(module_path!(), line!(), e))?;
            let id = conn.last_insert_rowid();
            Ok(AsrVocabularyTerm {
                id: Some(id),
                term: term.term,
                normalized_term: Some(normalized_term),
                language,
                category: term.category,
                scope: Some(scope),
                priority: Some(priority),
                use_count: Some(use_count),
                created_at: None,
                updated_at: None,
            })
        }
    }
}

#[tauri::command]
pub fn asr_vocabulary_delete(app: tauri::AppHandle, id: i64) -> Result<(), String> {
    let conn = open_db(&app)?;
    conn.execute(
        "DELETE FROM asr_vocabulary_terms WHERE id = ?1",
        params![id],
    )
    .map_err(|e| crate::utils::err_to_string(module_path!(), line!(), e))?;
    Ok(())
}

#[tauri::command]
pub fn asr_corrections_list(
    app: tauri::AppHandle,
    language: Option<String>,
    scopes: Option<Vec<String>>,
    user_approved_only: Option<bool>,
) -> Result<Vec<AsrCorrection>, String> {
    let conn = open_db(&app)?;
    let scopes = map_scopes_for_query(&scopes.unwrap_or_default());
    let language = normalize_language(language.as_deref());
    let approved = user_approved_only.map(|value| if value { 1 } else { 0 });
    let sql = format!(
        "SELECT id, wrong, normalized_wrong, correct, normalized_correct, language, scope, confidence, use_count, user_approved, created_at, updated_at
         FROM asr_corrections
         WHERE ({})
           AND (? IS NULL OR language IS NULL OR language = ?)
           AND (? IS NULL OR user_approved = ?)
         ORDER BY user_approved DESC, confidence DESC, use_count DESC, updated_at DESC, id DESC",
        query_scope_clause(scopes.len())
    );
    let mut params = scope_and_language_params(&scopes, &language);
    params.push(&approved as &dyn ToSql);
    params.push(&approved as &dyn ToSql);
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| crate::utils::err_to_string(module_path!(), line!(), e))?;
    let items = stmt
        .query_map(rusqlite::params_from_iter(params), |row| {
            Ok(AsrCorrection {
                id: Some(row.get(0)?),
                wrong: row.get(1)?,
                normalized_wrong: Some(row.get(2)?),
                correct: row.get(3)?,
                normalized_correct: Some(row.get(4)?),
                language: row.get(5)?,
                scope: Some(row.get(6)?),
                confidence: Some(row.get(7)?),
                use_count: Some(row.get(8)?),
                user_approved: Some(row.get::<_, i64>(9)? != 0),
                created_at: Some(row.get(10)?),
                updated_at: Some(row.get(11)?),
            })
        })
        .map_err(|e| crate::utils::err_to_string(module_path!(), line!(), e))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| crate::utils::err_to_string(module_path!(), line!(), e))?;
    Ok(items)
}

#[tauri::command]
pub fn asr_correction_upsert(
    app: tauri::AppHandle,
    correction: AsrCorrection,
) -> Result<AsrCorrection, String> {
    let conn = open_db(&app)?;
    let normalized_wrong = normalize_lookup_text(&correction.wrong);
    let normalized_correct = normalize_lookup_text(&correction.correct);
    if normalized_wrong.is_empty() || normalized_correct.is_empty() {
        return Err(crate::utils::err_msg(
            module_path!(),
            line!(),
            "Correction wrong/correct values cannot be empty",
        ));
    }

    let language = normalize_language(correction.language.as_deref());
    let scope = normalize_scope(correction.scope.as_deref());
    let confidence = correction.confidence.unwrap_or(0.75);
    let use_count = correction.use_count.unwrap_or(1);
    let user_approved = if correction.user_approved.unwrap_or(false) {
        1
    } else {
        0
    };

    match correction.id {
        Some(id) => {
            conn.execute(
                "UPDATE asr_corrections
                 SET wrong = ?1,
                     normalized_wrong = ?2,
                     correct = ?3,
                     normalized_correct = ?4,
                     language = ?5,
                     scope = ?6,
                     confidence = ?7,
                     use_count = ?8,
                     user_approved = ?9,
                     updated_at = CURRENT_TIMESTAMP
                 WHERE id = ?10",
                params![
                    correction.wrong,
                    normalized_wrong,
                    correction.correct,
                    normalized_correct,
                    language,
                    scope,
                    confidence,
                    use_count,
                    user_approved,
                    id
                ],
            )
            .map_err(|e| crate::utils::err_to_string(module_path!(), line!(), e))?;
            Ok(AsrCorrection {
                id: Some(id),
                wrong: correction.wrong,
                normalized_wrong: Some(normalized_wrong),
                correct: correction.correct,
                normalized_correct: Some(normalized_correct),
                language,
                scope: Some(scope),
                confidence: Some(confidence),
                use_count: Some(use_count),
                user_approved: Some(user_approved != 0),
                created_at: None,
                updated_at: None,
            })
        }
        None => {
            conn.execute(
                "INSERT INTO asr_corrections
                 (wrong, normalized_wrong, correct, normalized_correct, language, scope, confidence, use_count, user_approved)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    correction.wrong,
                    normalized_wrong,
                    correction.correct,
                    normalized_correct,
                    language,
                    scope,
                    confidence,
                    use_count,
                    user_approved
                ],
            )
            .map_err(|e| crate::utils::err_to_string(module_path!(), line!(), e))?;
            let id = conn.last_insert_rowid();
            Ok(AsrCorrection {
                id: Some(id),
                wrong: correction.wrong,
                normalized_wrong: Some(normalized_wrong),
                correct: correction.correct,
                normalized_correct: Some(normalized_correct),
                language,
                scope: Some(scope),
                confidence: Some(confidence),
                use_count: Some(use_count),
                user_approved: Some(user_approved != 0),
                created_at: None,
                updated_at: None,
            })
        }
    }
}

#[tauri::command]
pub fn asr_correction_delete(app: tauri::AppHandle, id: i64) -> Result<(), String> {
    let conn = open_db(&app)?;
    conn.execute("DELETE FROM asr_corrections WHERE id = ?1", params![id])
        .map_err(|e| crate::utils::err_to_string(module_path!(), line!(), e))?;
    Ok(())
}

#[tauri::command]
pub fn asr_voice_examples_list(
    app: tauri::AppHandle,
    language: Option<String>,
    scopes: Option<Vec<String>>,
) -> Result<Vec<AsrVoiceExample>, String> {
    let conn = open_db(&app)?;
    let scopes = map_scopes_for_query(&scopes.unwrap_or_default());
    let language = normalize_language(language.as_deref());
    let sql = format!(
        "SELECT id, audio_path, expected_text, normalized_expected_text, whisper_output, normalized_whisper_output, language, scope, term_id, correction_id, created_at
         FROM asr_voice_examples
         WHERE ({})
           AND (? IS NULL OR language IS NULL OR language = ?)
         ORDER BY created_at DESC, id DESC",
        query_scope_clause(scopes.len())
    );
    let params = scope_and_language_params(&scopes, &language);
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| crate::utils::err_to_string(module_path!(), line!(), e))?;
    let items = stmt
        .query_map(rusqlite::params_from_iter(params), |row| {
            Ok(AsrVoiceExample {
                id: Some(row.get(0)?),
                audio_path: row.get(1)?,
                expected_text: row.get(2)?,
                normalized_expected_text: Some(row.get(3)?),
                whisper_output: row.get(4)?,
                normalized_whisper_output: row.get(5)?,
                language: row.get(6)?,
                scope: Some(row.get(7)?),
                term_id: row.get(8)?,
                correction_id: row.get(9)?,
                created_at: Some(row.get(10)?),
            })
        })
        .map_err(|e| crate::utils::err_to_string(module_path!(), line!(), e))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| crate::utils::err_to_string(module_path!(), line!(), e))?;
    Ok(items)
}

#[tauri::command]
pub fn asr_voice_example_upsert(
    app: tauri::AppHandle,
    example: AsrVoiceExample,
) -> Result<AsrVoiceExample, String> {
    let conn = open_db(&app)?;
    let normalized_expected_text = normalize_lookup_text(&example.expected_text);
    if normalized_expected_text.is_empty() {
        return Err(crate::utils::err_msg(
            module_path!(),
            line!(),
            "Voice example expected text cannot be empty",
        ));
    }

    let normalized_whisper_output = example
        .whisper_output
        .as_deref()
        .map(normalize_lookup_text)
        .filter(|value| !value.is_empty());
    let language = normalize_language(example.language.as_deref());
    let scope = normalize_scope(example.scope.as_deref());

    match example.id {
        Some(id) => {
            conn.execute(
                "UPDATE asr_voice_examples
                 SET audio_path = ?1,
                     expected_text = ?2,
                     normalized_expected_text = ?3,
                     whisper_output = ?4,
                     normalized_whisper_output = ?5,
                     language = ?6,
                     scope = ?7,
                     term_id = ?8,
                     correction_id = ?9
                 WHERE id = ?10",
                params![
                    example.audio_path,
                    example.expected_text,
                    normalized_expected_text,
                    example.whisper_output,
                    normalized_whisper_output,
                    language,
                    scope,
                    example.term_id,
                    example.correction_id,
                    id
                ],
            )
            .map_err(|e| crate::utils::err_to_string(module_path!(), line!(), e))?;
            Ok(AsrVoiceExample {
                id: Some(id),
                audio_path: example.audio_path,
                expected_text: example.expected_text,
                normalized_expected_text: Some(normalized_expected_text),
                whisper_output: example.whisper_output,
                normalized_whisper_output,
                language,
                scope: Some(scope),
                term_id: example.term_id,
                correction_id: example.correction_id,
                created_at: None,
            })
        }
        None => {
            conn.execute(
                "INSERT INTO asr_voice_examples
                 (audio_path, expected_text, normalized_expected_text, whisper_output, normalized_whisper_output, language, scope, term_id, correction_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    example.audio_path,
                    example.expected_text,
                    normalized_expected_text,
                    example.whisper_output,
                    normalized_whisper_output,
                    language,
                    scope,
                    example.term_id,
                    example.correction_id
                ],
            )
            .map_err(|e| crate::utils::err_to_string(module_path!(), line!(), e))?;
            let id = conn.last_insert_rowid();
            Ok(AsrVoiceExample {
                id: Some(id),
                audio_path: example.audio_path,
                expected_text: example.expected_text,
                normalized_expected_text: Some(normalized_expected_text),
                whisper_output: example.whisper_output,
                normalized_whisper_output,
                language,
                scope: Some(scope),
                term_id: example.term_id,
                correction_id: example.correction_id,
                created_at: None,
            })
        }
    }
}

#[tauri::command]
pub fn asr_voice_example_delete(app: tauri::AppHandle, id: i64) -> Result<(), String> {
    let conn = open_db(&app)?;
    conn.execute("DELETE FROM asr_voice_examples WHERE id = ?1", params![id])
        .map_err(|e| crate::utils::err_to_string(module_path!(), line!(), e))?;
    Ok(())
}

#[tauri::command]
pub fn asr_build_prompt(
    app: tauri::AppHandle,
    language: Option<String>,
    scopes: Option<Vec<String>>,
) -> Result<String, String> {
    let terms = asr_vocabulary_list(app, language, scopes)?;
    let mut prompt_terms = Vec::new();
    let mut seen = HashSet::new();
    let mut current_len = 0usize;

    for term in terms {
        let candidate = normalize_whitespace(&term.term);
        if candidate.is_empty() {
            continue;
        }
        let dedupe_key = normalize_lookup_text(&candidate);
        if dedupe_key.is_empty() || !seen.insert(dedupe_key) {
            continue;
        }

        let separator = if prompt_terms.is_empty() { 0 } else { 2 };
        if prompt_terms.len() >= MAX_PROMPT_TERMS
            || current_len + separator + candidate.len() > MAX_PROMPT_CHARS
        {
            break;
        }

        current_len += separator + candidate.len();
        prompt_terms.push(candidate);
    }

    if prompt_terms.is_empty() {
        Ok(String::new())
    } else {
        Ok(format!("{}.", prompt_terms.join(", ")))
    }
}

fn load_corrections_for_processing(
    app: &tauri::AppHandle,
    language: Option<String>,
    scopes: Option<Vec<String>>,
) -> Result<Vec<StoredCorrection>, String> {
    let conn = open_db(app)?;
    let scopes = map_scopes_for_query(&scopes.unwrap_or_default());
    let language = normalize_language(language.as_deref());
    let sql = format!(
        "SELECT id, wrong, correct
         FROM asr_corrections
         WHERE ({})
           AND (? IS NULL OR language IS NULL OR language = ?)
         ORDER BY LENGTH(normalized_wrong) DESC, confidence DESC, use_count DESC, id DESC",
        query_scope_clause(scopes.len())
    );
    let params = scope_and_language_params(&scopes, &language);
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| crate::utils::err_to_string(module_path!(), line!(), e))?;
    let items = stmt
        .query_map(rusqlite::params_from_iter(params), |row| {
            Ok(StoredCorrection {
                id: row.get(0)?,
                wrong: row.get(1)?,
                correct: row.get(2)?,
            })
        })
        .map_err(|e| crate::utils::err_to_string(module_path!(), line!(), e))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| crate::utils::err_to_string(module_path!(), line!(), e))?;
    Ok(items)
}

#[tauri::command]
pub fn asr_apply_corrections(
    app: tauri::AppHandle,
    raw_text: String,
    language: Option<String>,
    scopes: Option<Vec<String>>,
) -> Result<AsrCorrectionResult, String> {
    let corrections = load_corrections_for_processing(&app, language, scopes)?;
    let mut corrected_text = raw_text.clone();
    let mut applied = Vec::new();

    for correction in corrections {
        let regex = correction_regex_for_phrase(&correction.wrong)?;
        let matches: Vec<String> = regex
            .find_iter(&corrected_text)
            .map(|m| m.as_str().to_string())
            .collect();
        if matches.is_empty() {
            continue;
        }

        corrected_text = regex
            .replace_all(&corrected_text, correction.correct.as_str())
            .to_string();
        for matched in matches {
            applied.push(AsrCorrectionApplication {
                correction_id: correction.id,
                wrong: correction.wrong.clone(),
                correct: correction.correct.clone(),
                match_text: matched,
            });
        }
    }

    Ok(AsrCorrectionResult {
        raw_text,
        corrected_text,
        applied,
    })
}

#[tauri::command]
pub fn asr_suggest_corrections_from_edit(
    before: String,
    after: String,
    language: Option<String>,
    scope: Option<String>,
) -> Result<Vec<AsrLearnedSuggestion>, String> {
    let before_tokens_raw = tokenize_words(&before);
    let after_tokens_raw = tokenize_words(&after);
    let before_tokens: Vec<String> = before_tokens_raw
        .iter()
        .map(|token| normalize_lookup_text(token))
        .collect();
    let after_tokens: Vec<String> = after_tokens_raw
        .iter()
        .map(|token| normalize_lookup_text(token))
        .collect();

    let matches = lcs_matches(&before_tokens, &after_tokens);
    let mut last_before = 0usize;
    let mut last_after = 0usize;
    let mut suggestions = Vec::new();
    let mut seen = HashSet::new();

    for (before_match, after_match) in matches
        .into_iter()
        .chain(std::iter::once((before_tokens.len(), after_tokens.len())))
    {
        if before_match > last_before || after_match > last_after {
            let before_slice = &before_tokens_raw[last_before..before_match];
            let after_slice = &after_tokens_raw[last_after..after_match];

            if !before_slice.is_empty()
                && !after_slice.is_empty()
                && before_slice.len() <= MAX_REPLACEMENT_WORDS
                && after_slice.len() <= MAX_REPLACEMENT_WORDS
                && !is_low_value_replacement(before_slice, after_slice)
            {
                let wrong = before_slice.join(" ");
                let correct = after_slice.join(" ");
                let normalized_wrong = normalize_lookup_text(&wrong);
                let normalized_correct = normalize_lookup_text(&correct);

                if !normalized_wrong.is_empty()
                    && !normalized_correct.is_empty()
                    && normalized_wrong != normalized_correct
                    && seen.insert((normalized_wrong.clone(), normalized_correct.clone()))
                {
                    suggestions.push(AsrLearnedSuggestion {
                        wrong,
                        normalized_wrong,
                        correct,
                        normalized_correct,
                        language: normalize_language(language.as_deref()),
                        scope: normalize_scope(scope.as_deref()),
                        confidence: 0.75,
                    });
                }
            }
        }

        last_before = before_match.saturating_add(1);
        last_after = after_match.saturating_add(1);
    }

    Ok(suggestions)
}

#[tauri::command]
pub fn asr_export_library(
    app: tauri::AppHandle,
    language: Option<String>,
    scopes: Option<Vec<String>>,
) -> Result<AsrExportBundle, String> {
    let vocabulary = asr_vocabulary_list(app.clone(), language.clone(), scopes.clone())?;
    let corrections = asr_corrections_list(app.clone(), language.clone(), scopes.clone(), None)?;
    let voice_examples = asr_voice_examples_list(app, language, scopes)?;
    Ok(AsrExportBundle {
        version: 1,
        vocabulary,
        corrections,
        voice_examples,
    })
}

#[tauri::command]
pub fn asr_import_library(
    app: tauri::AppHandle,
    bundle: AsrExportBundle,
) -> Result<BTreeMap<String, usize>, String> {
    let mut counts = BTreeMap::new();
    counts.insert("vocabulary".to_string(), 0);
    counts.insert("corrections".to_string(), 0);
    counts.insert("voiceExamples".to_string(), 0);

    for term in bundle.vocabulary {
        asr_vocabulary_upsert(app.clone(), AsrVocabularyTerm { id: None, ..term })?;
        *counts.get_mut("vocabulary").expect("count key exists") += 1;
    }

    for correction in bundle.corrections {
        asr_correction_upsert(
            app.clone(),
            AsrCorrection {
                id: None,
                ..correction
            },
        )?;
        *counts.get_mut("corrections").expect("count key exists") += 1;
    }

    for example in bundle.voice_examples {
        asr_voice_example_upsert(
            app.clone(),
            AsrVoiceExample {
                id: None,
                ..example
            },
        )?;
        *counts.get_mut("voiceExamples").expect("count key exists") += 1;
    }

    Ok(counts)
}

#[tauri::command]
pub fn asr_voice_example_suggest_correction(
    whisper_output: String,
    expected_text: String,
    language: Option<String>,
    scope: Option<String>,
) -> Result<Option<AsrLearnedSuggestion>, String> {
    Ok(
        asr_suggest_corrections_from_edit(whisper_output, expected_text, language, scope)?
            .into_iter()
            .next(),
    )
}
