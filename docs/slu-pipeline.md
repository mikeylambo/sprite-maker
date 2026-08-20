# SLU Production Pipeline (fork additions)

Custom Tier-1 features layered on upstream Sprite Studio. Everything here is
additive (new modules, new tables, new files) so upstream merges stay cheap.

## Goal

Open Sprite Studio → pick a game profile → generate or drag in art → prepare,
rig, animate → export — and the result drops into the target game ready to use,
with pivots, sockets, hitboxes, and animation data intact. Engine-agnostic by
design: engine choice has historically been downstream of the sprite problem,
so no adapter is privileged.

## Concepts

### Game Profile (`game_profiles` table, `profiles.rs`)

A per-title production contract. Stored app-wide (not per workspace) so one
game's profile can serve several workspaces. A workspace selects its profile
through the existing `settings` table under `game-profile:<workspace_id>`.

Profile JSON (schema 1):

```json
{
  "schema": 1,
  "engine": "godot" | "phaser" | "generic",
  "baseUnitPx": 64,
  "outlinePx": 2,
  "fps": { "default": 10, "overrides": { "idle": 6 } },
  "pivot": { "x": 0.5, "y": 1.0 },
  "palette": { "name": "Rift Violet", "colors": ["#5b3a8e", "..."] },
  "shadow": "soft-ellipse",
  "socketNames": ["core", "feet", "muzzle", "overhead"],
  "export": {
    "destination": "/abs/path/into/game/repo/assets/sprites",
    "godotResPrefix": "res://assets/sprites"
  }
}
```

`engine` selects the export adapter. Unknown fields are preserved (forward
compatibility); unknown `engine` values are rejected. More engines (Unity,
RPG Maker MV) are added as adapters without touching the schema.

### Production metadata (`asset_production` table, `production.rs`)

Gameplay-facing annotations on an asset, kept out of upstream's `assets` table
so merges never conflict:

- `sockets`: named attachment points `{ "name": "weapon_tip", "x": 41, "y": 12 }`
  (pixel coords on the master canvas; per-frame overrides later)
- `hitboxes`: `{ "name": "hurt", "kind": "hurtbox" | "hitbox" | "collision", "x": .., "y": .., "width": .., "height": .. }`
- `events`: `{ "frame": 3, "name": "footstep" }` (interpretation is per-animation)
- `tags`: free strings (`"melee"`, `"boss"`, ...)

### Canonical export manifest (`exporters.rs`)

Every export writes `<slug>.manifest.json` — the engine-agnostic truth — plus
whatever the adapter emits. Manifest contents: sheet geometry (frame rects,
layout), pivot (normalized), fps, loop flag, sockets, hitboxes, events, tags,
profile name/engine, and source ids for traceability.

Adapters:

- **godot** — `<slug>.tres` (SpriteFrames, format 3) referencing the copied
  PNG at `godotResPrefix`, one animation with fps/loop from the sheet.
- **phaser** — `<slug>.atlas.json` (TexturePacker hash format) +
  `<slug>.anim.json` (Phaser 3 anim config).
- **generic** — PNG + manifest only.

Export destination comes from the profile, is user-chosen by design (it points
into a game repo checkout so the other machine gets assets via git pull), and
is guarded: must exist, be a directory, not be the home directory or a
root-level path. Files are overwritten deterministically by slug so re-export
updates in place.

## Command surface (all new)

- `list_game_profiles`, `save_game_profile`, `delete_game_profile`
- `assign_game_profile`, `get_workspace_profile`
- `get_asset_production`, `set_asset_production`
- `export_sprite_sheet_to_engine`

## Status

- [x] Game profiles: model, storage, CRUD, workspace assignment
- [x] Production metadata: sockets/hitboxes/events/tags storage + validation
- [x] Export: canonical manifest + godot/phaser/generic adapters
- [ ] Frontend: profile editor, socket editor overlay, export action
- [ ] Profile-aware generation defaults (seed canvas/fps from profile)
- [ ] Identity Bible (persistent character objects) — next after this lands
- [ ] Batch/roster runs (Tier 2)
