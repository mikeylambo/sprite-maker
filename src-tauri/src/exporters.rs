use crate::{
    error::{CommandError, CommandResult},
    production::load_asset_production,
    profiles::{workspace_profile, GameProfile},
    workspace::workspace_path,
    AppState,
};
use rusqlite::OptionalExtension;
use serde::Serialize;
use std::path::{Path, PathBuf};
use tauri::State;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineExportResult {
    pub engine: String,
    pub destination: String,
    pub files: Vec<String>,
}

struct SheetRecord {
    project_id: String,
    name: String,
    png_path: PathBuf,
    metadata: serde_json::Value,
}

fn load_sheet(state: &AppState, sheet_id: &str) -> CommandResult<SheetRecord> {
    let row: Option<(String, String, String, String)> = {
        let connection = state
            .db
            .lock()
            .map_err(|_| CommandError::new("database_locked", "Database lock was poisoned"))?;
        connection
            .query_row(
                "SELECT project_id, name, png_path, metadata_path FROM sprite_sheets WHERE id=?1",
                [sheet_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?
    };
    let Some((project_id, name, png_path, metadata_path)) = row else {
        return Err(CommandError::new(
            "sprite_sheet_not_found",
            "The sprite sheet no longer exists",
        ));
    };
    // Same trust rule as deletion: DB paths are data. Only read files that
    // provably live inside the sheet's workspace.
    let root = workspace_path(state, &project_id)?;
    let png_path = Path::new(&png_path).canonicalize()?;
    let metadata_path = Path::new(&metadata_path).canonicalize()?;
    if !png_path.starts_with(&root) || !metadata_path.starts_with(&root) {
        return Err(CommandError::new(
            "unsafe_export",
            "Refusing to export sheet files outside their workspace",
        ));
    }
    let metadata: serde_json::Value = serde_json::from_slice(&std::fs::read(&metadata_path)?)
        .map_err(|error| CommandError::new("invalid_sheet_metadata", error.to_string()))?;
    Ok(SheetRecord {
        project_id,
        name,
        png_path,
        metadata,
    })
}

fn resolve_profile(
    state: &AppState,
    project_id: &str,
    profile_id: Option<&str>,
) -> CommandResult<GameProfile> {
    let connection = state
        .db
        .lock()
        .map_err(|_| CommandError::new("database_locked", "Database lock was poisoned"))?;
    if let Some(profile_id) = profile_id {
        let row = connection
            .query_row(
                "SELECT id, name, profile_json, created_at, updated_at FROM game_profiles WHERE id=?1",
                [profile_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                    ))
                },
            )
            .optional()?;
        let Some((id, name, profile_json, created_at, updated_at)) = row else {
            return Err(CommandError::new(
                "profile_not_found",
                "The game profile no longer exists",
            ));
        };
        let profile = serde_json::from_str(&profile_json)
            .map_err(|error| CommandError::new("invalid_profile", error.to_string()))?;
        return Ok(GameProfile {
            id,
            name,
            profile,
            created_at,
            updated_at,
        });
    }
    workspace_profile(&connection, project_id)?.ok_or_else(|| {
        CommandError::new(
            "no_profile",
            "Assign a game profile to this workspace (or pass one explicitly) before exporting",
        )
    })
}

fn guarded_destination(profile: &GameProfile) -> CommandResult<PathBuf> {
    let destination = profile
        .profile
        .get("export")
        .and_then(|export| export.get("destination"))
        .and_then(|value| value.as_str())
        .ok_or_else(|| {
            CommandError::new(
                "no_export_destination",
                "The game profile does not define export.destination",
            )
        })?;
    let destination = Path::new(destination);
    if !destination.is_absolute() {
        return Err(CommandError::new(
            "invalid_export_destination",
            "export.destination must be an absolute path",
        ));
    }
    if !destination.is_dir() {
        return Err(CommandError::new(
            "invalid_export_destination",
            "export.destination must be an existing directory",
        ));
    }
    let destination = destination.canonicalize()?;
    if destination
        .parent()
        .is_none_or(|parent| parent.parent().is_none())
    {
        return Err(CommandError::new(
            "unsafe_export",
            "Refusing to export into a root-level directory",
        ));
    }
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .and_then(|home| home.canonicalize().ok());
    if home.as_deref() == Some(destination.as_path()) {
        return Err(CommandError::new(
            "unsafe_export",
            "Refusing to export directly into the home directory",
        ));
    }
    Ok(destination)
}

fn slug_of(name: &str) -> String {
    let slug: String = name
        .chars()
        .map(|value| {
            if value.is_ascii_alphanumeric() {
                value.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let slug = slug.trim_matches('-').to_string();
    if slug.is_empty() {
        "sprite".into()
    } else {
        slug
    }
}

#[derive(Debug)]
struct FrameRect {
    x: u64,
    y: u64,
    width: u64,
    height: u64,
    duration_ms: u64,
}

fn sheet_frames(metadata: &serde_json::Value) -> CommandResult<Vec<FrameRect>> {
    let frames = metadata
        .get("frames")
        .and_then(|value| value.as_array())
        .ok_or_else(|| {
            CommandError::new("invalid_sheet_metadata", "Sheet metadata has no frames array")
        })?;
    let mut rects = Vec::with_capacity(frames.len());
    for frame in frames {
        let read = |key: &str| frame.get(key).and_then(|value| value.as_u64());
        let (Some(x), Some(y), Some(width), Some(height)) =
            (read("x"), read("y"), read("width"), read("height"))
        else {
            return Err(CommandError::new(
                "invalid_sheet_metadata",
                "Sheet frame entries need x, y, width, and height",
            ));
        };
        rects.push(FrameRect {
            x,
            y,
            width,
            height,
            duration_ms: read("durationMs").unwrap_or(100).max(1),
        });
    }
    if rects.is_empty() {
        return Err(CommandError::new(
            "invalid_sheet_metadata",
            "Sheet metadata contains no frames",
        ));
    }
    Ok(rects)
}

fn first_frame_asset_id(metadata: &serde_json::Value) -> Option<String> {
    metadata
        .get("frames")?
        .as_array()?
        .first()?
        .get("assetId")?
        .as_str()
        .map(str::to_string)
}

fn build_manifest(
    sheet: &SheetRecord,
    profile: &GameProfile,
    production: &crate::production::AssetProduction,
    slug: &str,
) -> serde_json::Value {
    serde_json::json!({
        "schema": 1,
        "name": sheet.name,
        "slug": slug,
        "image": format!("{slug}.png"),
        "profile": { "id": profile.id, "name": profile.name },
        "engine": engine_of(profile),
        "sheet": sheet.metadata,
        "sockets": production.sockets,
        "hitboxes": production.hitboxes,
        "events": production.events,
        "tags": production.tags,
    })
}

fn engine_of(profile: &GameProfile) -> String {
    profile
        .profile
        .get("engine")
        .and_then(|value| value.as_str())
        .unwrap_or("generic")
        .to_string()
}

fn fps_and_loop(metadata: &serde_json::Value) -> (f64, bool) {
    let fps = metadata
        .get("fps")
        .and_then(|value| value.as_f64())
        .filter(|value| value.is_finite() && *value >= 1.0)
        .unwrap_or(10.0);
    let looping = metadata
        .get("loop")
        .and_then(|value| value.as_bool())
        .unwrap_or(true);
    (fps, looping)
}

fn godot_tres(
    slug: &str,
    frames: &[FrameRect],
    fps: f64,
    looping: bool,
    res_prefix: &str,
) -> String {
    let mut output = String::new();
    let load_steps = frames.len() + 2;
    output.push_str(&format!(
        "[gd_resource type=\"SpriteFrames\" load_steps={load_steps} format=3]\n\n"
    ));
    output.push_str(&format!(
        "[ext_resource type=\"Texture2D\" path=\"{}/{slug}.png\" id=\"1\"]\n\n",
        res_prefix.trim_end_matches('/')
    ));
    for (index, frame) in frames.iter().enumerate() {
        output.push_str(&format!(
            "[sub_resource type=\"AtlasTexture\" id=\"AtlasTexture_{index}\"]\natlas = ExtResource(\"1\")\nregion = Rect2({}, {}, {}, {})\n\n",
            frame.x, frame.y, frame.width, frame.height
        ));
    }
    output.push_str("[resource]\nanimations = [{\n\"frames\": [");
    let frame_entries: Vec<String> = frames
        .iter()
        .enumerate()
        .map(|(index, frame)| {
            // Godot expresses per-frame duration as a speed multiplier.
            let base_ms = 1000.0 / fps;
            let duration = (frame.duration_ms as f64 / base_ms).max(0.01);
            format!(
                "{{\n\"duration\": {duration},\n\"texture\": SubResource(\"AtlasTexture_{index}\")\n}}"
            )
        })
        .collect();
    output.push_str(&frame_entries.join(", "));
    output.push_str(&format!(
        "],\n\"loop\": {},\n\"name\": &\"{slug}\",\n\"speed\": {fps}\n}}]\n",
        looping
    ));
    output
}

fn phaser_atlas(slug: &str, sheet: &SheetRecord, frames: &[FrameRect]) -> serde_json::Value {
    let pivot = sheet
        .metadata
        .get("pivot")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({"x": 0.5, "y": 1.0}));
    let mut frame_map = serde_json::Map::new();
    for (index, frame) in frames.iter().enumerate() {
        frame_map.insert(
            format!("{slug}_{index}"),
            serde_json::json!({
                "frame": {"x": frame.x, "y": frame.y, "w": frame.width, "h": frame.height},
                "rotated": false,
                "trimmed": false,
                "sourceSize": {"w": frame.width, "h": frame.height},
                "spriteSourceSize": {"x": 0, "y": 0, "w": frame.width, "h": frame.height},
                "pivot": pivot,
            }),
        );
    }
    serde_json::json!({
        "frames": frame_map,
        "meta": {
            "app": "sprite-studio",
            "image": format!("{slug}.png"),
            "format": "RGBA8888",
            "scale": "1"
        }
    })
}

fn phaser_anim(slug: &str, frames: &[FrameRect], fps: f64, looping: bool) -> serde_json::Value {
    let frame_names: Vec<serde_json::Value> = (0..frames.len())
        .map(|index| serde_json::json!({"key": slug, "frame": format!("{slug}_{index}")}))
        .collect();
    serde_json::json!({
        "key": slug,
        "type": "frame",
        "frames": frame_names,
        "frameRate": fps,
        "repeat": if looping { -1 } else { 0 }
    })
}

fn write_json(path: &Path, value: &serde_json::Value) -> CommandResult<()> {
    std::fs::write(
        path,
        serde_json::to_vec_pretty(value)
            .map_err(|error| CommandError::new("serialization_error", error.to_string()))?,
    )?;
    Ok(())
}

#[tauri::command]
pub fn export_sprite_sheet_to_engine(
    sheet_id: String,
    profile_id: Option<String>,
    state: State<'_, AppState>,
) -> CommandResult<EngineExportResult> {
    export_sprite_sheet_internal(&state, &sheet_id, profile_id.as_deref())
}

pub(crate) fn export_sprite_sheet_internal(
    state: &AppState,
    sheet_id: &str,
    profile_id: Option<&str>,
) -> CommandResult<EngineExportResult> {
    let sheet = load_sheet(state, sheet_id)?;
    let profile = resolve_profile(state, &sheet.project_id, profile_id)?;
    let destination = guarded_destination(&profile)?;
    let engine = engine_of(&profile);
    let slug = slug_of(&sheet.name);
    let frames = sheet_frames(&sheet.metadata)?;
    let (fps, looping) = fps_and_loop(&sheet.metadata);

    let production = {
        let connection = state
            .db
            .lock()
            .map_err(|_| CommandError::new("database_locked", "Database lock was poisoned"))?;
        match first_frame_asset_id(&sheet.metadata) {
            Some(asset_id) => load_asset_production(&connection, &asset_id)?,
            None => Default::default(),
        }
    };

    let mut files = Vec::new();
    let png_destination = destination.join(format!("{slug}.png"));
    std::fs::copy(&sheet.png_path, &png_destination)?;
    files.push(png_destination);

    let manifest_path = destination.join(format!("{slug}.manifest.json"));
    write_json(
        &manifest_path,
        &build_manifest(&sheet, &profile, &production, &slug),
    )?;
    files.push(manifest_path);

    match engine.as_str() {
        "godot" => {
            let res_prefix = profile
                .profile
                .get("export")
                .and_then(|export| export.get("godotResPrefix"))
                .and_then(|value| value.as_str())
                .ok_or_else(|| {
                    CommandError::new(
                        "no_export_destination",
                        "Godot export requires export.godotResPrefix in the game profile",
                    )
                })?;
            let tres_path = destination.join(format!("{slug}.tres"));
            std::fs::write(
                &tres_path,
                godot_tres(&slug, &frames, fps, looping, res_prefix),
            )?;
            files.push(tres_path);
        }
        "phaser" => {
            let atlas_path = destination.join(format!("{slug}.atlas.json"));
            write_json(&atlas_path, &phaser_atlas(&slug, &sheet, &frames))?;
            files.push(atlas_path);
            let anim_path = destination.join(format!("{slug}.anim.json"));
            write_json(&anim_path, &phaser_anim(&slug, &frames, fps, looping))?;
            files.push(anim_path);
        }
        _ => {}
    }

    Ok(EngineExportResult {
        engine,
        destination: destination.to_string_lossy().into_owned(),
        files: files
            .into_iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frames() -> Vec<FrameRect> {
        vec![
            FrameRect {
                x: 0,
                y: 0,
                width: 64,
                height: 64,
                duration_ms: 100,
            },
            FrameRect {
                x: 64,
                y: 0,
                width: 64,
                height: 64,
                duration_ms: 200,
            },
        ]
    }

    #[test]
    fn godot_tres_references_every_frame_and_prefix() {
        let output = godot_tres("hero-run", &frames(), 10.0, true, "res://assets/sprites");
        assert!(output.contains("load_steps=4"));
        assert!(output.contains("path=\"res://assets/sprites/hero-run.png\""));
        assert!(output.contains("Rect2(64, 0, 64, 64)"));
        assert!(output.contains("\"name\": &\"hero-run\""));
        assert!(output.contains("\"loop\": true"));
        // 200ms at 10 fps (100ms base) is a 2x duration multiplier.
        assert!(output.contains("\"duration\": 2"));
    }

    #[test]
    fn phaser_atlas_names_frames_by_slug_index() {
        let sheet = SheetRecord {
            project_id: "p".into(),
            name: "Hero Run".into(),
            png_path: PathBuf::from("/tmp/x.png"),
            metadata: serde_json::json!({"pivot": {"x": 0.5, "y": 1.0}}),
        };
        let atlas = phaser_atlas("hero-run", &sheet, &frames());
        assert!(atlas["frames"]["hero-run_1"]["frame"]["x"] == 64);
        assert_eq!(atlas["meta"]["image"], "hero-run.png");
    }

    #[test]
    fn phaser_anim_repeats_forever_when_looping() {
        let anim = phaser_anim("hero-run", &frames(), 12.0, true);
        assert_eq!(anim["repeat"], -1);
        assert_eq!(anim["frameRate"], 12.0);
        assert_eq!(anim["frames"].as_array().map(Vec::len), Some(2));
    }

    #[test]
    fn sheet_frames_requires_rects() {
        let error = sheet_frames(&serde_json::json!({"frames": [{"x": 1}]}))
            .expect_err("must fail");
        assert_eq!(error.code, "invalid_sheet_metadata");
    }

    #[test]
    fn slug_of_collapses_everything_else() {
        assert_eq!(slug_of("Hero Run!! (v2)"), "hero-run----v2");
        assert_eq!(slug_of("---"), "sprite");
    }

    #[test]
    fn exports_godot_files_end_to_end() {
        use crate::database;
        use std::{collections::HashMap, sync::Mutex};

        let root = std::env::temp_dir().join(format!("sprite-export-test-{}", uuid::Uuid::new_v4()));
        let workspace = root.join("workspace");
        let game_repo = root.join("game-repo").join("assets").join("sprites");
        std::fs::create_dir_all(workspace.join("exports/sprite-sheets"))
            .expect("workspace dirs should create");
        std::fs::create_dir_all(&game_repo).expect("game repo dirs should create");
        let workspace = workspace.canonicalize().expect("workspace should canonicalize");

        let png_path = workspace.join("exports/sprite-sheets/hero-run.png");
        std::fs::write(&png_path, b"fake png bytes").expect("png should write");
        let metadata_path = workspace.join("exports/sprite-sheets/hero-run.json");
        let metadata = serde_json::json!({
            "fps": 12.0,
            "loop": true,
            "pivot": {"x": 0.5, "y": 1.0},
            "frames": [
                {"index": 0, "assetId": "asset-1", "x": 0, "y": 0, "width": 64, "height": 64, "durationMs": 83},
                {"index": 1, "assetId": "asset-1", "x": 64, "y": 0, "width": 64, "height": 64, "durationMs": 83}
            ]
        });
        std::fs::write(&metadata_path, serde_json::to_vec(&metadata).unwrap())
            .expect("metadata should write");

        let connection = database::open(&root.join("app.sqlite3")).expect("database should open");
        connection.execute(
            "INSERT INTO projects(id,name,path,created_at,last_opened_at) VALUES ('p1','Test',?1,'now','now')",
            [workspace.to_string_lossy().as_ref()],
        ).expect("project should insert");
        connection.execute(
            "INSERT INTO animations(id,workspace_id,name,fps,looping,frames_json,created_at,updated_at) VALUES ('a1','p1','Hero Run',12,1,'[]','now','now')",
            [],
        ).expect("animation should insert");
        connection.execute(
            "INSERT INTO sprite_sheets(id,project_id,animation_id,name,layout,frame_width,frame_height,rows,columns,pivot_x,pivot_y,png_path,metadata_path,width,height,frame_count,created_at,updated_at) VALUES ('s1','p1','a1','Hero Run','grid',64,64,1,2,0.5,1.0,?1,?2,128,64,2,'now','now')",
            [png_path.to_string_lossy().as_ref(), metadata_path.to_string_lossy().as_ref()],
        ).expect("sheet should insert");
        let profile = serde_json::json!({
            "schema": 1,
            "engine": "godot",
            "export": {
                "destination": game_repo.to_string_lossy(),
                "godotResPrefix": "res://assets/sprites"
            }
        });
        connection.execute(
            "INSERT INTO game_profiles(id,name,profile_json,created_at,updated_at) VALUES ('gp1','Test Game',?1,'now','now')",
            [profile.to_string()],
        ).expect("profile should insert");
        connection.execute(
            "INSERT INTO settings(key,value_json,updated_at) VALUES ('game-profile:p1','\"gp1\"','now')",
            [],
        ).expect("assignment should insert");

        let state = AppState {
            db: Mutex::new(connection),
            cancellers: Mutex::new(HashMap::new()),
        };
        let result = export_sprite_sheet_internal(&state, "s1", None)
            .expect("export should succeed");
        assert_eq!(result.engine, "godot");
        assert_eq!(result.files.len(), 3);

        let game_repo = game_repo.canonicalize().expect("destination should canonicalize");
        assert!(game_repo.join("hero-run.png").is_file());
        let manifest: serde_json::Value = serde_json::from_slice(
            &std::fs::read(game_repo.join("hero-run.manifest.json")).expect("manifest should exist"),
        )
        .expect("manifest should parse");
        assert_eq!(manifest["slug"], "hero-run");
        assert_eq!(manifest["engine"], "godot");
        assert!(manifest["sockets"].as_array().is_some());
        let tres = std::fs::read_to_string(game_repo.join("hero-run.tres"))
            .expect("tres should exist");
        assert!(tres.contains("res://assets/sprites/hero-run.png"));
        assert!(tres.contains("Rect2(64, 0, 64, 64)"));
        assert!(tres.contains("\"speed\": 12"));

        // A profile without a destination fails loudly instead of writing.
        let missing = export_sprite_sheet_internal(&state, "missing", None)
            .expect_err("unknown sheet must fail");
        assert_eq!(missing.code, "sprite_sheet_not_found");

        std::fs::remove_dir_all(root).expect("temporary fixture should be removable");
    }
}
