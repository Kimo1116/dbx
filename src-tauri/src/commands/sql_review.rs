use std::path::PathBuf;
use std::sync::Arc;
use tauri::State;

use crate::commands::connection::AppState;
use dbx_core::sql_review::rules::{list_builtin_rules, RuleMeta};
use dbx_core::sql_review::{self, ReviewSettings, SqlReviewReport};

/// Managed state pointing at the review settings JSON file on disk.
pub struct ReviewSettingsState {
    pub settings_path: PathBuf,
}

fn read_settings_file(path: &std::path::Path) -> Option<ReviewSettings> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn write_settings_file(path: &std::path::Path, settings: &ReviewSettings) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| e.to_string())
}

/// Run SQL review (rule engine + optional AI) on the given SQL.
#[tauri::command]
pub async fn sql_review_run(
    _state: State<'_, Arc<AppState>>,
    sql: String,
    dialect: String,
    connection_id: Option<String>,
    database: Option<String>,
    settings: Option<ReviewSettings>,
    ai_response: Option<String>,
) -> Result<SqlReviewReport, String> {
    let settings = settings.unwrap_or_default();

    // Resolve database type from dialect string via serde
    let database_type: dbx_core::models::connection::DatabaseType =
        serde_json::from_value(serde_json::Value::String(dialect.clone()))
            .unwrap_or(dbx_core::models::connection::DatabaseType::Mysql);

    let report = sql_review::run_review(
        &sql,
        &dialect,
        database_type,
        connection_id.as_deref(),
        database.as_deref(),
        None,
        &settings,
        ai_response.as_deref(),
    );

    Ok(report)
}

/// List all built-in review rules with their metadata.
#[tauri::command]
pub async fn sql_review_list_rules(
    _state: State<'_, Arc<AppState>>,
) -> Result<Vec<RuleMeta>, String> {
    Ok(list_builtin_rules())
}

/// Load review settings (persisted to a JSON file; falls back to defaults).
#[tauri::command]
pub async fn sql_review_load_settings(
    review_state: State<'_, ReviewSettingsState>,
) -> Result<ReviewSettings, String> {
    Ok(read_settings_file(&review_state.settings_path).unwrap_or_default())
}

/// Save review settings to the JSON file.
#[tauri::command]
pub async fn sql_review_save_settings(
    review_state: State<'_, ReviewSettingsState>,
    settings: ReviewSettings,
) -> Result<(), String> {
    write_settings_file(&review_state.settings_path, &settings)
}
