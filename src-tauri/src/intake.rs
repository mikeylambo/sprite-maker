//! Asset intake: turn an arbitrary generated or imported image into a
//! rig-ready master. Deterministic image work only — trim, normalize, place —
//! plus profile-driven socket and hit-region seeding the user then adjusts.

use crate::{
    assets::{self, get_asset},
    error::{CommandError, CommandResult},
    models::Asset,
    production::{AssetProduction, HitRegion, SocketPoint},
    profiles::workspace_profile,
    workspace::workspace_path,
    AppState,
};
use chrono::Utc;
use image::RgbaImage;
use rusqlite::OptionalExtension;
use serde::Serialize;
use std::path::Path;
use tauri::State;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreparedAsset {
    pub asset: Asset,
    pub production: AssetProduction,
    pub notes: Vec<String>,
}

/// Tight bounding box of pixels above the alpha threshold.
fn opaque_bounds(image: &RgbaImage) -> Option<(u32, u32, u32, u32)> {
    let (mut left, mut top, mut right, mut bottom) = (u32::MAX, u32::MAX, 0u32, 0u32);
    for (x, y, pixel) in image.enumerate_pixels() {
        if pixel.0[3] <= 8 {
            continue;
        }
        left = left.min(x);
        top = top.min(y);
        right = right.max(x);
        bottom = bottom.max(y);
    }
    if left == u32::MAX {
        return None;
    }
    Some((left, top, right - left + 1, bottom - top + 1))
}

fn seeded_socket(name: &str, content: (u32, u32, u32, u32)) -> SocketPoint {
    let (left, top, width, height) = content;
    let center_x = left as f64 + width as f64 / 2.0;
    let center_y = top as f64 + height as f64 / 2.0;
    let bottom = (top + height) as f64;
    let right = (left + width) as f64;
    let lowered = name.to_ascii_lowercase();
    let (x, y) = if lowered.contains("feet") || lowered.contains("foot") || lowered.contains("ground")
    {
        (center_x, bottom)
    } else if lowered.contains("head") || lowered.contains("overhead") || lowered.contains("crown") {
        (center_x, top as f64)
    } else if lowered.contains("muzzle")
        || lowered.contains("weapon")
        || lowered.contains("hand")
        || lowered.contains("tip")
    {
        (right, center_y)
    } else {
        (center_x, center_y)
    };
    SocketPoint {
        name: name.to_string(),
        x: x.round(),
        y: y.round(),
    }
}

#[derive(Debug)]
struct Normalized {
    image: RgbaImage,
    content: (u32, u32, u32, u32),
    notes: Vec<String>,
}

/// Trim to content, optionally scale down to fit the profile's base unit, then
/// place on a square canvas using the profile's pivot convention.
fn normalize(source: &RgbaImage, base_unit: Option<u32>, pivot: (f64, f64)) -> CommandResult<Normalized> {
    let mut notes = Vec::new();
    let bounds = opaque_bounds(source).ok_or_else(|| {
        CommandError::new(
            "empty_asset",
            "This image is fully transparent, so there is nothing to prepare",
        )
    })?;
    let (left, top, width, height) = bounds;
    if width != source.width() || height != source.height() {
        notes.push(format!(
            "Trimmed {}×{} of transparent margin",
            source.width() - width,
            source.height() - height
        ));
    }
    let cropped = image::imageops::crop_imm(source, left, top, width, height).to_image();

    let canvas_size = base_unit.unwrap_or_else(|| width.max(height).max(1));
    let (content_width, content_height) = if width > canvas_size || height > canvas_size {
        let ratio = (canvas_size as f64 / width as f64).min(canvas_size as f64 / height as f64);
        let scaled_width = ((width as f64 * ratio).floor() as u32).max(1);
        let scaled_height = ((height as f64 * ratio).floor() as u32).max(1);
        notes.push(format!(
            "Scaled {width}×{height} down to {scaled_width}×{scaled_height} for the {canvas_size}px base unit"
        ));
        (scaled_width, scaled_height)
    } else {
        (width, height)
    };
    let content = if (content_width, content_height) == (width, height) {
        cropped
    } else {
        image::imageops::resize(
            &cropped,
            content_width,
            content_height,
            image::imageops::FilterType::Nearest,
        )
    };

    let mut canvas = RgbaImage::new(canvas_size, canvas_size);
    // Horizontal placement follows pivot.x; vertical placement follows pivot.y,
    // so a bottom-center convention plants the sprite on the canvas floor.
    let offset_x = ((canvas_size as f64 - content_width as f64) * pivot.0).round().max(0.0) as u32;
    let offset_y = ((canvas_size as f64 - content_height as f64) * pivot.1).round().max(0.0) as u32;
    let offset_x = offset_x.min(canvas_size.saturating_sub(content_width));
    let offset_y = offset_y.min(canvas_size.saturating_sub(content_height));
    image::imageops::overlay(&mut canvas, &content, offset_x as i64, offset_y as i64);
    notes.push(format!(
        "Placed on a {canvas_size}×{canvas_size} canvas at pivot ({}, {})",
        pivot.0, pivot.1
    ));
    Ok(Normalized {
        image: canvas,
        content: (offset_x, offset_y, content_width, content_height),
        notes,
    })
}

fn existing_asset_id(state: &AppState, path: &str) -> CommandResult<Option<String>> {
    let connection = state
        .db
        .lock()
        .map_err(|_| CommandError::new("database_locked", "Database lock was poisoned"))?;
    Ok(connection
        .query_row("SELECT id FROM assets WHERE path = ?1", [path], |row| {
            row.get(0)
        })
        .optional()?)
}

#[tauri::command]
pub fn prepare_asset_for_rigging(
    asset_id: String,
    state: State<'_, AppState>,
) -> CommandResult<PreparedAsset> {
    let asset = get_asset(&state, &asset_id)?;
    let root = workspace_path(&state, &asset.workspace_id)?;
    let source_path = Path::new(&asset.path).canonicalize()?;
    if !source_path.starts_with(&root) {
        return Err(CommandError::new(
            "asset_outside_workspace",
            "Refusing to prepare an asset outside its workspace",
        ));
    }

    let profile = {
        let connection = state
            .db
            .lock()
            .map_err(|_| CommandError::new("database_locked", "Database lock was poisoned"))?;
        workspace_profile(&connection, &asset.workspace_id)?
    };
    let base_unit = profile
        .as_ref()
        .and_then(|profile| profile.profile.get("baseUnitPx"))
        .and_then(|value| value.as_u64())
        .and_then(|value| u32::try_from(value).ok())
        .filter(|value| *value > 0);
    let pivot = profile
        .as_ref()
        .and_then(|profile| profile.profile.get("pivot"))
        .and_then(|pivot| {
            Some((pivot.get("x")?.as_f64()?, pivot.get("y")?.as_f64()?))
        })
        .filter(|(x, y)| (0.0..=1.0).contains(x) && (0.0..=1.0).contains(y))
        .unwrap_or((0.5, 1.0));
    let socket_names: Vec<String> = profile
        .as_ref()
        .and_then(|profile| profile.profile.get("socketNames"))
        .and_then(|value| value.as_array())
        .map(|names| {
            names
                .iter()
                .filter_map(|name| name.as_str())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_else(|| vec!["core".into(), "feet".into()]);

    let source = image::open(&source_path)?.to_rgba8();
    let normalized = normalize(&source, base_unit, pivot)?;

    let stem = source_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("asset");
    let output_directory = source_path.parent().unwrap_or(&root).to_path_buf();
    let output = output_directory.join(format!("{stem}-prepared.png"));
    normalized.image.save(&output)?;

    let output_string = output.to_string_lossy().into_owned();
    let registered = assets::inspect(
        &asset.workspace_id,
        &root,
        &output,
        existing_asset_id(&state, &output_string)?,
    )?;
    assets::upsert(&state, &registered, "prepared")?;

    let (content_x, content_y, content_width, content_height) = normalized.content;
    let production = AssetProduction {
        sockets: socket_names
            .iter()
            .map(|name| seeded_socket(name, normalized.content))
            .collect(),
        hitboxes: vec![HitRegion {
            name: "body".into(),
            kind: "collision".into(),
            x: content_x as f64,
            y: content_y as f64,
            width: content_width as f64,
            height: content_height as f64,
        }],
        events: Vec::new(),
        tags: Vec::new(),
    };
    {
        let connection = state
            .db
            .lock()
            .map_err(|_| CommandError::new("database_locked", "Database lock was poisoned"))?;
        connection.execute(
            "INSERT INTO asset_production(asset_id, sockets_json, hitboxes_json, events_json, tags_json, updated_at) VALUES (?1, ?2, ?3, ?4, '[]', ?5) ON CONFLICT(asset_id) DO UPDATE SET sockets_json=excluded.sockets_json, hitboxes_json=excluded.hitboxes_json, updated_at=excluded.updated_at",
            rusqlite::params![
                registered.id,
                serde_json::to_string(&production.sockets)
                    .map_err(|error| CommandError::new("serialization_error", error.to_string()))?,
                serde_json::to_string(&production.hitboxes)
                    .map_err(|error| CommandError::new("serialization_error", error.to_string()))?,
                serde_json::to_string(&production.events)
                    .map_err(|error| CommandError::new("serialization_error", error.to_string()))?,
                Utc::now().to_rfc3339()
            ],
        )?;
    }

    let mut notes = normalized.notes;
    if profile.is_none() {
        notes.push(
            "No game profile assigned, so defaults were used. Assign one for exact scale and sockets."
                .into(),
        );
    }
    notes.push(format!(
        "Seeded {} socket(s) and a body collision box — adjust them in the inspector",
        production.sockets.len()
    ));
    Ok(PreparedAsset {
        asset: registered,
        production,
        notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    fn sprite(width: u32, height: u32, filled: (u32, u32, u32, u32)) -> RgbaImage {
        let mut image = RgbaImage::new(width, height);
        let (left, top, box_width, box_height) = filled;
        for y in top..top + box_height {
            for x in left..left + box_width {
                image.put_pixel(x, y, Rgba([200, 100, 40, 255]));
            }
        }
        image
    }

    #[test]
    fn bounds_ignore_near_transparent_pixels() {
        let mut image = sprite(16, 16, (4, 4, 4, 4));
        image.put_pixel(0, 0, Rgba([255, 255, 255, 4]));
        assert_eq!(opaque_bounds(&image), Some((4, 4, 4, 4)));
    }

    #[test]
    fn fully_transparent_input_fails_loudly() {
        let error = normalize(&RgbaImage::new(8, 8), Some(16), (0.5, 1.0))
            .expect_err("empty input must fail");
        assert_eq!(error.code, "empty_asset");
    }

    #[test]
    fn normalize_bottom_centers_on_the_base_unit_canvas() {
        let image = sprite(32, 32, (10, 6, 8, 10));
        let result = normalize(&image, Some(64), (0.5, 1.0)).expect("normalize should work");
        assert_eq!(result.image.dimensions(), (64, 64));
        let (x, y, width, height) = result.content;
        assert_eq!((width, height), (8, 10));
        assert_eq!(x, 28, "content should be horizontally centered");
        assert_eq!(y, 54, "content should sit on the canvas floor");
        assert_eq!(result.image.get_pixel(28, 63).0[3], 255);
    }

    #[test]
    fn normalize_scales_oversized_content_down_to_the_base_unit() {
        let image = sprite(200, 200, (0, 0, 200, 100));
        let result = normalize(&image, Some(64), (0.5, 1.0)).expect("normalize should work");
        assert_eq!(result.image.dimensions(), (64, 64));
        let (_, _, width, height) = result.content;
        assert!(width <= 64 && height <= 64);
        assert_eq!(width, 64);
        assert!(result.notes.iter().any(|note| note.contains("Scaled")));
    }

    #[test]
    fn sockets_seed_from_their_names() {
        let content = (10, 20, 40, 60);
        let feet = seeded_socket("feet", content);
        assert_eq!((feet.x, feet.y), (30.0, 80.0));
        let head = seeded_socket("overhead", content);
        assert_eq!((head.x, head.y), (30.0, 20.0));
        let muzzle = seeded_socket("muzzle", content);
        assert_eq!((muzzle.x, muzzle.y), (50.0, 50.0));
        let core = seeded_socket("core", content);
        assert_eq!((core.x, core.y), (30.0, 50.0));
    }

    #[test]
    fn top_left_pivot_places_content_in_the_corner() {
        let image = sprite(32, 32, (8, 8, 8, 8));
        let result = normalize(&image, Some(48), (0.0, 0.0)).expect("normalize should work");
        assert_eq!(result.content.0, 0);
        assert_eq!(result.content.1, 0);
    }
}
