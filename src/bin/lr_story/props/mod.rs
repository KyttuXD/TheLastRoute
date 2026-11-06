//! Prop models for LR Story, read from Blockbench (.bbmodel) files in a folder the caller names. Copied from the Last
//! Route render code (render/bbmodel.rs, prop_model.rs, prop_scale.rs, door.rs, builder.rs). The world, app and shader
//! references were replaced; each file's header and comments say what changed. Nothing here reads embedded data.
//!
//! `load_prop(props_dir, "blocky/labStool_0.bbmodel", ..)` reads one prop; `prop_scale::table(assets_dir)` reads the
//! draw scale table from `assets_dir/data/render/prop_draw_scale.json`.

// The copied API is larger than the story game calls so far; the unused parts stay on purpose.
#![allow(dead_code)]

pub mod bbmodel;
pub mod builder;
pub mod door;
pub mod prop_model;
pub mod prop_scale;

use prop_model::{AtlasCache, PropModel};
use std::path::Path;

/// A vertex's `normal.w` shader mode for a lit, textured body (copy of `render/shader.rs` LIT_MODE).
pub const LIT_MODE: f32 = 1.0;
/// The mode of a body that fades by its tint alpha (copy of `render/shader.rs` GHOST_MODE).
pub const GHOST_MODE: f32 = 4.0;

/// Reads one prop model. `rel` is its path under `props_dir` (e.g. "blocky/labStool_0.bbmodel"). The texture is the
/// picture embedded in the model, or a file the model names that sits in the same folder. `cache` shares a found atlas
/// between models of one folder; `soft` rounds the normals the way the blocky props are drawn. Making the texture needs
/// a macroquad graphics context.
pub fn load_prop(props_dir: &Path, rel: &str, cache: &mut AtlasCache, soft: bool) -> Option<PropModel> {
    let path = props_dir.join(rel);
    let text = std::fs::read_to_string(&path).ok()?;
    let dir = path.parent()?.to_path_buf();
    let find = |name: &str| std::fs::read(dir.join(name)).ok();
    PropModel::parse(&text, &find, &dir.to_string_lossy(), cache, soft)
}
