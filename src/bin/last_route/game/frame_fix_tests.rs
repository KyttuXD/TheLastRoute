//! The game frame's sound picks: the level layer's ambience bed (sewer, Inferno) and the floor kind under the runner
//! that its step sound reads (`Game::ambience_layer`, `Game::sync_runner_floor`).

use super::{Controls, Game};
use crate::audio::Sounds;
use crate::config::Difficulty;
use crate::content::Content;
use crate::world::catalog::Catalog;

fn infinite(room: &str) -> (Game, Content, Catalog) {
    let (content, catalog) = (Content::load(), Catalog::load());
    let g = Game::new_infinite_sim(Difficulty::Standard, 7, room, 0, &content, &catalog);
    (g, content, catalog)
}

#[test]
fn the_sewer_and_inferno_levels_name_their_layer_and_the_rest_none() {
    let (mut g, _, _) = infinite("LAYER-BED-7");
    assert_eq!(g.ambience_layer(), None, "level 0 plays the region bed");
    for (level, want) in [(-1, Some("sewer")), (-2, Some("sewer")), (-3, Some("inferno")), (-4, None), (1, None)] {
        g.inf.as_mut().unwrap().level = level;
        assert_eq!(g.ambience_layer(), want, "level {level}");
    }
}

#[test]
fn a_layer_bed_wins_over_the_room_region() {
    let sewer = crate::audio::ambient::bed_id_for(Some("sewer"), Some("lab"));
    let lab = crate::audio::ambient::bed_id_for(None, Some("lab"));
    assert!(sewer.is_some() && lab.is_some());
    assert_ne!(sewer, lab, "the sewer layer has its own bed");
}

#[test]
fn a_finite_game_has_no_layer() {
    let (content, catalog) = (Content::load(), Catalog::load());
    let g = Game::new_sim(Difficulty::Standard, 4242, &content, &catalog);
    assert_eq!(g.ambience_layer(), None);
}

#[test]
fn the_frame_sets_the_runners_floor_kind_from_the_world_mesh() {
    let (mut g, content, catalog) = infinite("STEP-FLOOR-7");
    g.hunters.clear();
    g.step(1.0 / 60.0, &Controls::default(), &content, &catalog, &Sounds::default());
    let (x, y) = (g.p.pos.0.floor() as i32, g.p.pos.1.floor() as i32);
    let mut cache = std::collections::HashMap::new();
    let want = crate::render::world_mesh::floor_kind_at(&g.fac, x, y, &mut cache);
    assert_eq!(g.p.floor_kind, want, "the runner steps on the kind the mesh draws");
    assert!(g.runner_floor_at.is_some());
}

#[test]
fn the_runners_floor_is_worked_out_only_when_its_cell_or_floor_changes() {
    let (mut g, _, _) = infinite("STEP-CACHE-7");
    g.sync_runner_floor();
    let real = g.p.floor_kind;
    g.p.floor_kind = "MARK";
    g.sync_runner_floor();
    assert_eq!(g.p.floor_kind, "MARK", "the same cell is not looked up again");
    g.depth += 1;
    g.sync_runner_floor();
    assert_eq!(g.p.floor_kind, real, "a new floor works it out again");
}

#[test]
fn a_dedicated_room_has_no_runner_floor() {
    let (mut g, _, _) = infinite("STEP-DEDICATED-7");
    g.no_local_runner = true;
    g.sync_runner_floor();
    assert_eq!(g.p.floor_kind, "");
    assert!(g.runner_floor_at.is_none());
}
