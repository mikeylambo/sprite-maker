use crate::{
    error::{CommandError, CommandResult},
    AppState,
};
use chrono::Utc;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use tauri::State;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SocketPoint {
    pub name: String,
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HitRegion {
    pub name: String,
    pub kind: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FrameEvent {
    pub frame: u32,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AssetProduction {
    pub sockets: Vec<SocketPoint>,
    pub hitboxes: Vec<HitRegion>,
    pub events: Vec<FrameEvent>,
    pub tags: Vec<String>,
}

const HIT_REGION_KINDS: [&str; 3] = ["hitbox", "hurtbox", "collision"];

fn valid_name(name: &str) -> bool {
    let trimmed = name.trim();
    !trimmed.is_empty() && trimmed.len() <= 64
}

fn finite(value: f64) -> bool {
    value.is_finite() && value.abs() <= 100_000.0
}

fn validate_production(production: &AssetProduction) -> CommandResult<()> {
    if production.sockets.len() > 64
        || production.hitboxes.len() > 64
        || production.events.len() > 256
        || production.tags.len() > 32
    {
        return Err(CommandError::new(
            "invalid_production",
            "Too many production entries for one asset",
        ));
    }
    for socket in &production.sockets {
        if !valid_name(&socket.name) || !finite(socket.x) || !finite(socket.y) {
            return Err(CommandError::new(
                "invalid_production",
                "Sockets need a short name and finite coordinates",
            ));
        }
    }
    for region in &production.hitboxes {
        if !valid_name(&region.name)
            || !HIT_REGION_KINDS.contains(&region.kind.as_str())
            || !finite(region.x)
            || !finite(region.y)
            || !finite(region.width)
            || !finite(region.height)
            || region.width <= 0.0
            || region.height <= 0.0
        {
            return Err(CommandError::new(
                "invalid_production",
                "Hit regions need a name, a kind of hitbox/hurtbox/collision, and a positive finite size",
            ));
        }
    }
    for event in &production.events {
        if !valid_name(&event.name) || event.frame > 1024 {
            return Err(CommandError::new(
                "invalid_production",
                "Frame events need a short name and a frame index below 1024",
            ));
        }
    }
    if !production.tags.iter().all(|tag| valid_name(tag)) {
        return Err(CommandError::new(
            "invalid_production",
            "Tags must be short non-empty strings",
        ));
    }
    Ok(())
}

fn parse_list<T: serde::de::DeserializeOwned>(json: &str, label: &str) -> CommandResult<Vec<T>> {
    serde_json::from_str(json)
        .map_err(|error| CommandError::new("invalid_production", format!("{label}: {error}")))
}

pub(crate) fn load_asset_production(
    connection: &rusqlite::Connection,
    asset_id: &str,
) -> CommandResult<AssetProduction> {
    let row: Option<(String, String, String, String)> = connection
        .query_row(
            "SELECT sockets_json, hitboxes_json, events_json, tags_json FROM asset_production WHERE asset_id=?1",
            [asset_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?;
    let Some((sockets, hitboxes, events, tags)) = row else {
        return Ok(AssetProduction::default());
    };
    Ok(AssetProduction {
        sockets: parse_list(&sockets, "sockets")?,
        hitboxes: parse_list(&hitboxes, "hitboxes")?,
        events: parse_list(&events, "events")?,
        tags: parse_list(&tags, "tags")?,
    })
}

#[tauri::command]
pub fn get_asset_production(
    asset_id: String,
    state: State<'_, AppState>,
) -> CommandResult<AssetProduction> {
    let connection = state
        .db
        .lock()
        .map_err(|_| CommandError::new("database_locked", "Database lock was poisoned"))?;
    load_asset_production(&connection, &asset_id)
}

#[tauri::command]
pub fn set_asset_production(
    asset_id: String,
    production: AssetProduction,
    state: State<'_, AppState>,
) -> CommandResult<AssetProduction> {
    validate_production(&production)?;
    let connection = state
        .db
        .lock()
        .map_err(|_| CommandError::new("database_locked", "Database lock was poisoned"))?;
    let asset_exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM assets WHERE id=?1)",
        [&asset_id],
        |row| row.get(0),
    )?;
    if !asset_exists {
        return Err(CommandError::new(
            "asset_not_found",
            "The asset no longer exists",
        ));
    }
    let sockets = serde_json::to_string(&production.sockets)
        .map_err(|error| CommandError::new("serialization_error", error.to_string()))?;
    let hitboxes = serde_json::to_string(&production.hitboxes)
        .map_err(|error| CommandError::new("serialization_error", error.to_string()))?;
    let events = serde_json::to_string(&production.events)
        .map_err(|error| CommandError::new("serialization_error", error.to_string()))?;
    let tags = serde_json::to_string(&production.tags)
        .map_err(|error| CommandError::new("serialization_error", error.to_string()))?;
    connection.execute(
        "INSERT INTO asset_production(asset_id, sockets_json, hitboxes_json, events_json, tags_json, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6) ON CONFLICT(asset_id) DO UPDATE SET sockets_json=excluded.sockets_json, hitboxes_json=excluded.hitboxes_json, events_json=excluded.events_json, tags_json=excluded.tags_json, updated_at=excluded.updated_at",
        params![asset_id, sockets, hitboxes, events, tags, Utc::now().to_rfc3339()],
    )?;
    Ok(production)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> AssetProduction {
        AssetProduction {
            sockets: vec![SocketPoint {
                name: "weapon_tip".into(),
                x: 41.0,
                y: 12.0,
            }],
            hitboxes: vec![HitRegion {
                name: "body".into(),
                kind: "hurtbox".into(),
                x: 10.0,
                y: 8.0,
                width: 22.0,
                height: 40.0,
            }],
            events: vec![FrameEvent {
                frame: 3,
                name: "footstep".into(),
            }],
            tags: vec!["melee".into()],
        }
    }

    #[test]
    fn valid_production_passes() {
        validate_production(&sample()).expect("sample should validate");
    }

    #[test]
    fn unknown_hit_region_kind_is_rejected() {
        let mut production = sample();
        production.hitboxes[0].kind = "trigger".into();
        let error = validate_production(&production).expect_err("must fail");
        assert_eq!(error.code, "invalid_production");
    }

    #[test]
    fn non_finite_socket_is_rejected() {
        let mut production = sample();
        production.sockets[0].x = f64::NAN;
        let error = validate_production(&production).expect_err("must fail");
        assert_eq!(error.code, "invalid_production");
    }

    #[test]
    fn zero_size_hit_region_is_rejected() {
        let mut production = sample();
        production.hitboxes[0].width = 0.0;
        let error = validate_production(&production).expect_err("must fail");
        assert_eq!(error.code, "invalid_production");
    }
}
