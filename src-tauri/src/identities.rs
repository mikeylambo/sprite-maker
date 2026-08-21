//! Identity Bible: persistent character objects that outlive a single
//! workspace or chat. An identity records who a character is (summary,
//! proportions, palette, forbidden changes) plus canonical imagery, so a
//! later asset — a portrait, a sequel sprite sheet, an eight-direction set —
//! can start from an established identity instead of re-deriving it.

use crate::{
    assets::get_asset,
    error::{CommandError, CommandResult},
    AppState,
};
use chrono::Utc;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tauri::{Manager, State};
use uuid::Uuid;

pub const IMAGE_KINDS: [&str; 4] = ["canonical", "silhouette", "portrait", "reference"];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityImage {
    pub id: String,
    pub identity_id: String,
    pub path: String,
    pub kind: String,
    pub label: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Identity {
    pub id: String,
    pub name: String,
    pub summary: String,
    pub proportions: String,
    pub scale_px: Option<u32>,
    pub palette: Vec<String>,
    pub forbidden: Vec<String>,
    pub vocabulary: Vec<String>,
    pub tags: Vec<String>,
    pub images: Vec<IdentityImage>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityInput {
    pub id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub proportions: String,
    #[serde(default)]
    pub scale_px: Option<u32>,
    #[serde(default)]
    pub palette: Vec<String>,
    #[serde(default)]
    pub forbidden: Vec<String>,
    #[serde(default)]
    pub vocabulary: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

fn clean_list(values: &[String], limit: usize) -> CommandResult<Vec<String>> {
    let cleaned: Vec<String> = values
        .iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect();
    if cleaned.len() > limit || cleaned.iter().any(|value| value.len() > 200) {
        return Err(CommandError::new(
            "invalid_identity",
            "Identity lists are limited to short entries",
        ));
    }
    Ok(cleaned)
}

fn parse_list(json: &str) -> Vec<String> {
    serde_json::from_str(json).unwrap_or_default()
}

fn load_images(
    connection: &rusqlite::Connection,
    identity_id: &str,
) -> CommandResult<Vec<IdentityImage>> {
    let mut statement = connection.prepare(
        "SELECT id, identity_id, path, kind, label, created_at FROM identity_images WHERE identity_id=?1 ORDER BY created_at",
    )?;
    let rows = statement.query_map([identity_id], |row| {
        Ok(IdentityImage {
            id: row.get(0)?,
            identity_id: row.get(1)?,
            path: row.get(2)?,
            kind: row.get(3)?,
            label: row.get(4)?,
            created_at: row.get(5)?,
        })
    })?;
    Ok(rows.filter_map(Result::ok).collect())
}

fn load_identity(connection: &rusqlite::Connection, id: &str) -> CommandResult<Identity> {
    let row = connection
        .query_row(
            "SELECT id, name, summary, proportions, scale_px, palette_json, forbidden_json, vocabulary_json, tags_json, created_at, updated_at FROM identities WHERE id=?1",
            [id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, String>(9)?,
                    row.get::<_, String>(10)?,
                ))
            },
        )
        .optional()?
        .ok_or_else(|| CommandError::new("identity_not_found", "The identity no longer exists"))?;
    Ok(Identity {
        id: row.0,
        name: row.1,
        summary: row.2,
        proportions: row.3,
        scale_px: row.4.and_then(|value| u32::try_from(value).ok()),
        palette: parse_list(&row.5),
        forbidden: parse_list(&row.6),
        vocabulary: parse_list(&row.7),
        tags: parse_list(&row.8),
        images: load_images(connection, id)?,
        created_at: row.9,
        updated_at: row.10,
    })
}

#[tauri::command]
pub fn list_identities(state: State<'_, AppState>) -> CommandResult<Vec<Identity>> {
    let connection = state
        .db
        .lock()
        .map_err(|_| CommandError::new("database_locked", "Database lock was poisoned"))?;
    let ids: Vec<String> = {
        let mut statement =
            connection.prepare("SELECT id FROM identities ORDER BY name COLLATE NOCASE")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        rows.filter_map(Result::ok).collect()
    };
    ids.iter()
        .map(|id| load_identity(&connection, id))
        .collect()
}

#[tauri::command]
pub fn save_identity(input: IdentityInput, state: State<'_, AppState>) -> CommandResult<Identity> {
    let name = input.name.trim();
    if name.is_empty() || name.len() > 120 {
        return Err(CommandError::new(
            "invalid_identity",
            "Identity name must be 1-120 characters",
        ));
    }
    if input.summary.len() > 4000 || input.proportions.len() > 2000 {
        return Err(CommandError::new(
            "invalid_identity",
            "Identity notes are too long",
        ));
    }
    if input.scale_px.is_some_and(|value| value == 0 || value > 4096) {
        return Err(CommandError::new(
            "invalid_identity",
            "Identity scale must be between 1 and 4096 pixels",
        ));
    }
    let palette = clean_list(&input.palette, 64)?;
    let forbidden = clean_list(&input.forbidden, 64)?;
    let vocabulary = clean_list(&input.vocabulary, 64)?;
    let tags = clean_list(&input.tags, 32)?;
    let now = Utc::now().to_rfc3339();
    let connection = state
        .db
        .lock()
        .map_err(|_| CommandError::new("database_locked", "Database lock was poisoned"))?;
    let id = match input.id {
        Some(id) => {
            let changed = connection.execute(
                "UPDATE identities SET name=?1, summary=?2, proportions=?3, scale_px=?4, palette_json=?5, forbidden_json=?6, vocabulary_json=?7, tags_json=?8, updated_at=?9 WHERE id=?10",
                params![
                    name,
                    input.summary.trim(),
                    input.proportions.trim(),
                    input.scale_px,
                    serde_json::to_string(&palette).unwrap_or_else(|_| "[]".into()),
                    serde_json::to_string(&forbidden).unwrap_or_else(|_| "[]".into()),
                    serde_json::to_string(&vocabulary).unwrap_or_else(|_| "[]".into()),
                    serde_json::to_string(&tags).unwrap_or_else(|_| "[]".into()),
                    now,
                    id
                ],
            )?;
            if changed == 0 {
                return Err(CommandError::new(
                    "identity_not_found",
                    "The identity no longer exists",
                ));
            }
            id
        }
        None => {
            let id = Uuid::new_v4().to_string();
            connection.execute(
                "INSERT INTO identities(id, name, summary, proportions, scale_px, palette_json, forbidden_json, vocabulary_json, tags_json, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)",
                params![
                    id,
                    name,
                    input.summary.trim(),
                    input.proportions.trim(),
                    input.scale_px,
                    serde_json::to_string(&palette).unwrap_or_else(|_| "[]".into()),
                    serde_json::to_string(&forbidden).unwrap_or_else(|_| "[]".into()),
                    serde_json::to_string(&vocabulary).unwrap_or_else(|_| "[]".into()),
                    serde_json::to_string(&tags).unwrap_or_else(|_| "[]".into()),
                    now
                ],
            )?;
            id
        }
    };
    load_identity(&connection, &id)
}

fn identity_storage(app: &tauri::AppHandle, identity_id: &str) -> CommandResult<PathBuf> {
    let directory = app
        .path()
        .app_data_dir()
        .map_err(|error| {
            CommandError::new("app_data_unavailable", error.to_string())
        })?
        .join("identities")
        .join(identity_id);
    std::fs::create_dir_all(&directory)?;
    Ok(directory)
}

#[tauri::command]
pub fn delete_identity(
    id: String,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> CommandResult<()> {
    {
        let connection = state
            .db
            .lock()
            .map_err(|_| CommandError::new("database_locked", "Database lock was poisoned"))?;
        connection.execute("DELETE FROM identities WHERE id=?1", [&id])?;
    }
    // Identity imagery lives in a directory this module owns, keyed by id.
    let directory = identity_storage(&app, &id)?;
    if directory.is_dir() {
        std::fs::remove_dir_all(directory)?;
    }
    Ok(())
}

fn insert_image(
    state: &AppState,
    identity_id: &str,
    stored: &Path,
    kind: &str,
    label: &str,
) -> CommandResult<Identity> {
    let connection = state
        .db
        .lock()
        .map_err(|_| CommandError::new("database_locked", "Database lock was poisoned"))?;
    connection.execute(
        "INSERT INTO identity_images(id, identity_id, path, kind, label, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            Uuid::new_v4().to_string(),
            identity_id,
            stored.to_string_lossy(),
            kind,
            label,
            Utc::now().to_rfc3339()
        ],
    )?;
    load_identity(&connection, identity_id)
}

fn validated_kind(kind: &str) -> CommandResult<&str> {
    if IMAGE_KINDS.contains(&kind) {
        Ok(kind)
    } else {
        Err(CommandError::new(
            "invalid_identity_image",
            format!("Image kind must be one of: {}", IMAGE_KINDS.join(", ")),
        ))
    }
}

#[tauri::command]
pub fn add_identity_image_from_asset(
    identity_id: String,
    asset_id: String,
    kind: String,
    label: String,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> CommandResult<Identity> {
    let kind = validated_kind(kind.trim())?;
    let asset = get_asset(&state, &asset_id)?;
    let source = Path::new(&asset.path);
    if !source.is_file() {
        return Err(CommandError::new(
            "asset_missing",
            "The asset file is no longer on disk",
        ));
    }
    // Copy rather than reference: identities must survive the workspace that
    // produced them being moved, restored, or deleted.
    let extension = source
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("png")
        .to_ascii_lowercase();
    let stored = identity_storage(&app, &identity_id)?
        .join(format!("{}.{extension}", Uuid::new_v4()));
    std::fs::copy(source, &stored)?;
    let label = if label.trim().is_empty() {
        asset.name.clone()
    } else {
        label.trim().to_string()
    };
    insert_image(&state, &identity_id, &stored, kind, &label)
}

#[tauri::command]
pub fn delete_identity_image(
    id: String,
    state: State<'_, AppState>,
) -> CommandResult<Option<Identity>> {
    let connection = state
        .db
        .lock()
        .map_err(|_| CommandError::new("database_locked", "Database lock was poisoned"))?;
    let row: Option<(String, String)> = connection
        .query_row(
            "SELECT identity_id, path FROM identity_images WHERE id=?1",
            [&id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((identity_id, path)) = row else {
        return Ok(None);
    };
    connection.execute("DELETE FROM identity_images WHERE id=?1", [&id])?;
    // Only files this module wrote into the identity's own directory.
    let expected_parent = Path::new(&path).parent().map(Path::to_path_buf);
    if expected_parent.is_some_and(|parent| parent.ends_with(&identity_id)) {
        let _ = std::fs::remove_file(&path);
    }
    load_identity(&connection, &identity_id).map(Some)
}

/// Render an identity as prompt-ready text. This is the payload that keeps a
/// character consistent across chats, asset types, and sessions.
pub fn brief_text(identity: &Identity) -> String {
    let mut lines = vec![format!("IDENTITY: {}", identity.name)];
    if !identity.summary.trim().is_empty() {
        lines.push(format!("Summary: {}", identity.summary.trim()));
    }
    if !identity.proportions.trim().is_empty() {
        lines.push(format!("Proportions: {}", identity.proportions.trim()));
    }
    if let Some(scale) = identity.scale_px {
        lines.push(format!("Sprite scale: {scale}px tall at base unit"));
    }
    if !identity.palette.is_empty() {
        lines.push(format!("Locked palette: {}", identity.palette.join(", ")));
    }
    if !identity.vocabulary.is_empty() {
        lines.push(format!(
            "Animation vocabulary: {}",
            identity.vocabulary.join(", ")
        ));
    }
    if !identity.tags.is_empty() {
        lines.push(format!("Tags: {}", identity.tags.join(", ")));
    }
    if !identity.forbidden.is_empty() {
        lines.push("Never change:".to_string());
        for rule in &identity.forbidden {
            lines.push(format!("- {rule}"));
        }
    }
    let canonical = identity
        .images
        .iter()
        .filter(|image| image.kind == "canonical")
        .count();
    if canonical > 0 {
        lines.push(format!(
            "{canonical} canonical reference image(s) define this identity; match them exactly."
        ));
    }
    lines.join("\n")
}

#[tauri::command]
pub fn get_identity_brief(id: String, state: State<'_, AppState>) -> CommandResult<String> {
    let connection = state
        .db
        .lock()
        .map_err(|_| CommandError::new("database_locked", "Database lock was poisoned"))?;
    Ok(brief_text(&load_identity(&connection, &id)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity() -> Identity {
        Identity {
            id: "i1".into(),
            name: "Jo".into(),
            summary: "Scrappy courier in a patched jacket".into(),
            proportions: "5 heads tall, oversized boots".into(),
            scale_px: Some(64),
            palette: vec!["#5b3a8e".into(), "#e0a458".into()],
            forbidden: vec!["scar on left cheek".into(), "jacket patch colors".into()],
            vocabulary: vec!["idle".into(), "run".into()],
            tags: vec!["protagonist".into()],
            images: vec![IdentityImage {
                id: "img1".into(),
                identity_id: "i1".into(),
                path: "/tmp/jo.png".into(),
                kind: "canonical".into(),
                label: "Master".into(),
                created_at: "now".into(),
            }],
            created_at: "now".into(),
            updated_at: "now".into(),
        }
    }

    #[test]
    fn brief_includes_every_locked_rule() {
        let brief = brief_text(&identity());
        assert!(brief.starts_with("IDENTITY: Jo"));
        assert!(brief.contains("Locked palette: #5b3a8e, #e0a458"));
        assert!(brief.contains("Sprite scale: 64px"));
        assert!(brief.contains("- scar on left cheek"));
        assert!(brief.contains("1 canonical reference image(s)"));
    }

    #[test]
    fn brief_skips_empty_sections() {
        let sparse = Identity {
            summary: String::new(),
            proportions: String::new(),
            scale_px: None,
            palette: vec![],
            forbidden: vec![],
            vocabulary: vec![],
            tags: vec![],
            images: vec![],
            ..identity()
        };
        let brief = brief_text(&sparse);
        assert_eq!(brief, "IDENTITY: Jo");
    }

    #[test]
    fn unknown_image_kind_is_rejected() {
        let error = validated_kind("thumbnail").expect_err("must fail");
        assert_eq!(error.code, "invalid_identity_image");
    }

    #[test]
    fn overlong_list_entries_are_rejected() {
        let error = clean_list(&["x".repeat(201)], 64).expect_err("must fail");
        assert_eq!(error.code, "invalid_identity");
    }

    #[test]
    fn blank_list_entries_are_dropped() {
        let cleaned = clean_list(&["  ".into(), " keep ".into()], 8).expect("should clean");
        assert_eq!(cleaned, vec!["keep".to_string()]);
    }
}
