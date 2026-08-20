use crate::{
    error::{CommandError, CommandResult},
    AppState,
};
use chrono::Utc;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use tauri::State;
use uuid::Uuid;

pub const SUPPORTED_ENGINES: [&str; 3] = ["godot", "phaser", "generic"];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameProfile {
    pub id: String,
    pub name: String,
    pub profile: serde_json::Value,
    pub created_at: String,
    pub updated_at: String,
}

fn row_to_profile(row: &rusqlite::Row<'_>) -> rusqlite::Result<(String, String, String, String, String)> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
    ))
}

fn parse_profile(
    (id, name, profile_json, created_at, updated_at): (String, String, String, String, String),
) -> CommandResult<GameProfile> {
    let profile = serde_json::from_str(&profile_json)
        .map_err(|error| CommandError::new("invalid_profile", error.to_string()))?;
    Ok(GameProfile {
        id,
        name,
        profile,
        created_at,
        updated_at,
    })
}

fn finite_in_range(value: &serde_json::Value, low: f64, high: f64) -> bool {
    value
        .as_f64()
        .is_some_and(|number| number.is_finite() && number >= low && number <= high)
}

/// Validate the profile document. Unknown fields are allowed (forward
/// compatibility); known fields must be well-formed so downstream consumers
/// (generation defaults, validation, exporters) can trust them.
pub fn validate_profile(value: &serde_json::Value) -> CommandResult<()> {
    let object = value
        .as_object()
        .ok_or_else(|| CommandError::new("invalid_profile", "Profile must be a JSON object"))?;
    if let Some(engine) = object.get("engine") {
        let engine = engine
            .as_str()
            .ok_or_else(|| CommandError::new("invalid_profile", "engine must be a string"))?;
        if !SUPPORTED_ENGINES.contains(&engine) {
            return Err(CommandError::new(
                "invalid_profile",
                format!(
                    "Unsupported engine '{engine}'. Supported: {}",
                    SUPPORTED_ENGINES.join(", ")
                ),
            ));
        }
    }
    for key in ["baseUnitPx", "outlinePx"] {
        if let Some(value) = object.get(key) {
            if !value.as_u64().is_some_and(|number| number <= 4096) {
                return Err(CommandError::new(
                    "invalid_profile",
                    format!("{key} must be an integer between 0 and 4096"),
                ));
            }
        }
    }
    if let Some(fps) = object.get("fps") {
        let fps_object = fps
            .as_object()
            .ok_or_else(|| CommandError::new("invalid_profile", "fps must be an object"))?;
        if let Some(default) = fps_object.get("default") {
            if !finite_in_range(default, 1.0, 60.0) {
                return Err(CommandError::new(
                    "invalid_profile",
                    "fps.default must be between 1 and 60",
                ));
            }
        }
    }
    if let Some(pivot) = object.get("pivot") {
        let pivot_object = pivot
            .as_object()
            .ok_or_else(|| CommandError::new("invalid_profile", "pivot must be an object"))?;
        for axis in ["x", "y"] {
            let Some(value) = pivot_object.get(axis) else {
                return Err(CommandError::new(
                    "invalid_profile",
                    "pivot requires x and y between 0 and 1",
                ));
            };
            if !finite_in_range(value, 0.0, 1.0) {
                return Err(CommandError::new(
                    "invalid_profile",
                    "pivot coordinates must be between 0 and 1",
                ));
            }
        }
    }
    if let Some(socket_names) = object.get("socketNames") {
        let names = socket_names.as_array().ok_or_else(|| {
            CommandError::new("invalid_profile", "socketNames must be an array of strings")
        })?;
        if names.len() > 64
            || !names.iter().all(|name| {
                name.as_str()
                    .is_some_and(|name| !name.trim().is_empty() && name.len() <= 64)
            })
        {
            return Err(CommandError::new(
                "invalid_profile",
                "socketNames must be at most 64 non-empty strings",
            ));
        }
    }
    if let Some(export) = object.get("export") {
        let export_object = export
            .as_object()
            .ok_or_else(|| CommandError::new("invalid_profile", "export must be an object"))?;
        if let Some(destination) = export_object.get("destination") {
            if !destination
                .as_str()
                .is_some_and(|value| !value.trim().is_empty())
            {
                return Err(CommandError::new(
                    "invalid_profile",
                    "export.destination must be a non-empty path",
                ));
            }
        }
        if let Some(prefix) = export_object.get("godotResPrefix") {
            if !prefix
                .as_str()
                .is_some_and(|value| value.starts_with("res://"))
            {
                return Err(CommandError::new(
                    "invalid_profile",
                    "export.godotResPrefix must start with res://",
                ));
            }
        }
    }
    Ok(())
}

#[tauri::command]
pub fn list_game_profiles(state: State<'_, AppState>) -> CommandResult<Vec<GameProfile>> {
    let connection = state
        .db
        .lock()
        .map_err(|_| CommandError::new("database_locked", "Database lock was poisoned"))?;
    let mut statement = connection.prepare(
        "SELECT id, name, profile_json, created_at, updated_at FROM game_profiles ORDER BY name COLLATE NOCASE",
    )?;
    let rows = statement.query_map([], row_to_profile)?;
    rows.filter_map(Result::ok).map(parse_profile).collect()
}

#[tauri::command]
pub fn save_game_profile(
    id: Option<String>,
    name: String,
    profile: serde_json::Value,
    state: State<'_, AppState>,
) -> CommandResult<GameProfile> {
    let name = name.trim();
    if name.is_empty() || name.len() > 120 {
        return Err(CommandError::new(
            "invalid_profile",
            "Profile name must be 1-120 characters",
        ));
    }
    validate_profile(&profile)?;
    let now = Utc::now().to_rfc3339();
    let profile_json = profile.to_string();
    let connection = state
        .db
        .lock()
        .map_err(|_| CommandError::new("database_locked", "Database lock was poisoned"))?;
    let id = match id {
        Some(id) => {
            let changed = connection.execute(
                "UPDATE game_profiles SET name=?1, profile_json=?2, updated_at=?3 WHERE id=?4",
                params![name, profile_json, now, id],
            )?;
            if changed == 0 {
                return Err(CommandError::new(
                    "profile_not_found",
                    "The game profile no longer exists",
                ));
            }
            id
        }
        None => {
            let id = Uuid::new_v4().to_string();
            connection.execute(
                "INSERT INTO game_profiles(id, name, profile_json, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?4)",
                params![id, name, profile_json, now],
            )?;
            id
        }
    };
    let row = connection
        .query_row(
            "SELECT id, name, profile_json, created_at, updated_at FROM game_profiles WHERE id=?1",
            [&id],
            row_to_profile,
        )?;
    parse_profile(row)
}

#[tauri::command]
pub fn delete_game_profile(id: String, state: State<'_, AppState>) -> CommandResult<()> {
    let connection = state
        .db
        .lock()
        .map_err(|_| CommandError::new("database_locked", "Database lock was poisoned"))?;
    connection.execute("DELETE FROM game_profiles WHERE id=?1", [&id])?;
    // Clear any workspace assignments that pointed at the deleted profile.
    connection.execute(
        "DELETE FROM settings WHERE key LIKE 'game-profile:%' AND value_json=?1",
        [serde_json::Value::String(id).to_string()],
    )?;
    Ok(())
}

#[tauri::command]
pub fn assign_game_profile(
    workspace_id: String,
    profile_id: Option<String>,
    state: State<'_, AppState>,
) -> CommandResult<()> {
    let connection = state
        .db
        .lock()
        .map_err(|_| CommandError::new("database_locked", "Database lock was poisoned"))?;
    let workspace_exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM projects WHERE id=?1)",
        [&workspace_id],
        |row| row.get(0),
    )?;
    if !workspace_exists {
        return Err(CommandError::new(
            "workspace_not_found",
            "Workspace is no longer registered",
        ));
    }
    let key = format!("game-profile:{workspace_id}");
    match profile_id {
        Some(profile_id) => {
            let profile_exists: bool = connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM game_profiles WHERE id=?1)",
                [&profile_id],
                |row| row.get(0),
            )?;
            if !profile_exists {
                return Err(CommandError::new(
                    "profile_not_found",
                    "The game profile no longer exists",
                ));
            }
            connection.execute(
                "INSERT INTO settings(key, value_json, updated_at) VALUES (?1, ?2, ?3) ON CONFLICT(key) DO UPDATE SET value_json=excluded.value_json, updated_at=excluded.updated_at",
                params![key, serde_json::Value::String(profile_id).to_string(), Utc::now().to_rfc3339()],
            )?;
        }
        None => {
            connection.execute("DELETE FROM settings WHERE key=?1", [&key])?;
        }
    }
    Ok(())
}

pub(crate) fn workspace_profile(
    connection: &rusqlite::Connection,
    workspace_id: &str,
) -> CommandResult<Option<GameProfile>> {
    let key = format!("game-profile:{workspace_id}");
    let assigned: Option<String> = connection
        .query_row("SELECT value_json FROM settings WHERE key=?1", [&key], |row| {
            row.get(0)
        })
        .optional()?;
    let Some(assigned) = assigned else {
        return Ok(None);
    };
    let profile_id: String = serde_json::from_str(&assigned)
        .map_err(|error| CommandError::new("invalid_setting", error.to_string()))?;
    let row = connection
        .query_row(
            "SELECT id, name, profile_json, created_at, updated_at FROM game_profiles WHERE id=?1",
            [&profile_id],
            row_to_profile,
        )
        .optional()?;
    row.map(parse_profile).transpose()
}

#[tauri::command]
pub fn get_workspace_profile(
    workspace_id: String,
    state: State<'_, AppState>,
) -> CommandResult<Option<GameProfile>> {
    let connection = state
        .db
        .lock()
        .map_err(|_| CommandError::new("database_locked", "Database lock was poisoned"))?;
    workspace_profile(&connection, &workspace_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn valid_profile_passes() {
        let profile = json!({
            "schema": 1,
            "engine": "godot",
            "baseUnitPx": 64,
            "outlinePx": 2,
            "fps": {"default": 10.0},
            "pivot": {"x": 0.5, "y": 1.0},
            "socketNames": ["core", "feet"],
            "export": {"destination": "/tmp/game", "godotResPrefix": "res://assets"}
        });
        validate_profile(&profile).expect("profile should validate");
    }

    #[test]
    fn unknown_engine_is_rejected() {
        let error = validate_profile(&json!({"engine": "unreal"})).expect_err("must fail");
        assert_eq!(error.code, "invalid_profile");
    }

    #[test]
    fn out_of_range_pivot_is_rejected() {
        let error = validate_profile(&json!({"pivot": {"x": 1.5, "y": 0.0}}))
            .expect_err("must fail");
        assert_eq!(error.code, "invalid_profile");
    }

    #[test]
    fn non_res_godot_prefix_is_rejected() {
        let error = validate_profile(&json!({"export": {"godotResPrefix": "assets"}}))
            .expect_err("must fail");
        assert_eq!(error.code, "invalid_profile");
    }

    #[test]
    fn unknown_fields_are_preserved_and_allowed() {
        validate_profile(&json!({"futureField": {"anything": true}}))
            .expect("unknown fields must not fail validation");
    }
}
