//! The Inferno's noisy floor through the game (owner decision, `world/inferno_terrain.rs`): the body stands on the
//! surface the renderer draws, walks it without hopping, bots follow it, and the flat stairwell pads keep the flights'
//! rules.

use super::player::stair_height_at;
use super::level_stack::{floor_y_of, ground_y, local_ground, LevelStack};
use super::{Controls, Game};
use crate::audio::Sounds;
use crate::config::{Difficulty, PLAYER_RADIUS};
use crate::content::Content;
use crate::world::catalog::Catalog;
use crate::world::facility::Cell;
use crate::world::infinite::INFERNO_LEVEL;

const FRAME: f32 = 1.0 / 60.0;

fn inferno(n: usize, content: &Content, catalog: &Catalog) -> Game {
    let g = Game::new_infinite_sim(Difficulty::Standard, 7, &format!("ROOM-INFERNO-{n}"), INFERNO_LEVEL, content, catalog);
    assert_eq!(g.fac.level, INFERNO_LEVEL);
    assert!(g.fac.terrain().is_some(), "an Inferno window carries the terrain");
    g
}

/// A cell a body fits on that is no flight tread, well or drop (the flights keep their own surface rules).
fn on_plain_floor(g: &Game, c: Cell) -> bool {
    let (x, z) = (c.x as f32 + 0.5, c.y as f32 + 0.5);
    g.fac.body_fits(x, z, PLAYER_RADIUS)
        && stair_height_at(&g.fac, (x, z)).is_none()
        && !g.fac.is_hole(c.x, c.y)
        && !g.fac.is_drop(c)
}

/// The absolute y of the feet (the jump offset included).
fn feet_abs(g: &Game) -> f32 {
    g.p.y() + g.p.feet_y
}

fn ground_abs(g: &Game) -> f32 {
    floor_y_of(INFERNO_LEVEL) + local_ground(&g.fac, g.p.pos.0, g.p.pos.1)
}

#[test]
fn the_body_stands_on_the_noisy_surface_at_sampled_cells() {
    let (content, catalog) = (Content::load(), Catalog::load());
    let mut checked = 0;
    let mut spread = (f32::MAX, f32::MIN);
    for n in 0..3 {
        let mut g = inferno(n, &content, &catalog);
        let cells: Vec<Cell> = g
            .fac
            .floor_cells()
            .into_iter()
            .filter(|c| on_plain_floor(&g, *c))
            .collect();
        assert!(cells.len() > 500, "the window has floor");
        for c in cells.iter().step_by(cells.len() / 20) {
            for (ox, oz) in [(0.5, 0.5), (0.2, 0.8)] {
                g.p.pos = (c.x as f32 + ox, c.y as f32 + oz);
                g.p.floor_y = floor_y_of(INFERNO_LEVEL);
                g.p.height = local_ground(&g.fac, g.p.pos.0, g.p.pos.1);
                g.p.feet_y = 0.0;
                for _ in 0..3 {
                    g.step(FRAME, &Controls::default(), &content, &catalog, &Sounds::default());
                }
                assert!(
                    g.p.grounded && !g.p.falling(),
                    "{c:?}: not standing (feet_y {} height {} ground {} pos {:?} flight {:?})",
                    g.p.feet_y,
                    g.p.height,
                    ground_abs(&g),
                    g.p.pos,
                    stair_height_at(&g.fac, g.p.pos)
                );
                assert!((feet_abs(&g) - ground_abs(&g)).abs() < 1e-3, "{c:?}: feet {} ground {}", feet_abs(&g), ground_abs(&g));
                let under = ground_y(&g.fac, &LevelStack::EMPTY, g.p.pos.0, g.p.pos.1, feet_abs(&g));
                assert!((under.y - feet_abs(&g)).abs() < 1e-3);
                spread = (spread.0.min(feet_abs(&g)), spread.1.max(feet_abs(&g)));
                checked += 1;
            }
        }
    }
    assert!(checked >= 100);
    assert!(spread.1 - spread.0 > 0.2, "the sampled floor is not noisy ({spread:?})");
}

#[test]
fn walking_the_caves_follows_the_surface_without_hops_or_sinking() {
    let (content, catalog) = (Content::load(), Catalog::load());
    let mut g = inferno(0, &content, &catalog);
    // a clear run of at least 24 cells along +x
    let run = g
        .fac
        .floor_cells()
        .into_iter()
        .find(|c| (0..240).all(|i| g.fac.body_fits(c.x as f32 + 0.5 + i as f32 * 0.1, c.y as f32 + 0.5, PLAYER_RADIUS + 0.05)))
        .expect("a clear run on the Inferno floor");
    g.p.pos = (run.x as f32 + 0.5, run.y as f32 + 0.5);
    g.p.yaw = -std::f32::consts::FRAC_PI_2;
    g.p.floor_y = floor_y_of(INFERNO_LEVEL);
    g.p.height = local_ground(&g.fac, g.p.pos.0, g.p.pos.1);
    g.p.feet_y = 0.0;
    let start = g.p.pos;
    let c = Controls { fwd: 1.0, ..Default::default() };
    let (mut worst, mut lo, mut hi) = (0.0_f32, f32::MAX, f32::MIN);
    for _ in 0..180 {
        g.step(FRAME, &c, &content, &catalog, &Sounds::default());
        assert!(!g.p.falling(), "fell on the noisy floor at {:?}", g.p.pos);
        worst = worst.max((feet_abs(&g) - ground_abs(&g)).abs());
        lo = lo.min(feet_abs(&g));
        hi = hi.max(feet_abs(&g));
    }
    assert!(g.p.pos.0 - start.0 > 3.0, "the body walked ({:?} to {:?})", start, g.p.pos);
    assert!(worst < 0.02, "feet {worst} m off the surface while walking");
    assert!(hi - lo > 0.01, "the surface under the walk is flat");
}

#[test]
fn a_jump_lands_back_on_the_surface() {
    let (content, catalog) = (Content::load(), Catalog::load());
    let mut g = inferno(1, &content, &catalog);
    let c = g.fac.floor_cells().into_iter().find(|c| g.fac.body_fits(c.x as f32 + 0.5, c.y as f32 + 0.5, PLAYER_RADIUS + 0.3)).unwrap();
    g.p.pos = (c.x as f32 + 0.5, c.y as f32 + 0.5);
    g.p.floor_y = floor_y_of(INFERNO_LEVEL);
    g.p.height = local_ground(&g.fac, g.p.pos.0, g.p.pos.1);
    g.p.feet_y = 0.0;
    g.step(FRAME, &Controls::default(), &content, &catalog, &Sounds::default());
    let jump = Controls { jump: true, ..Default::default() };
    g.step(FRAME, &jump, &content, &catalog, &Sounds::default());
    let mut peak = f32::MIN;
    for _ in 0..180 {
        g.step(FRAME, &Controls::default(), &content, &catalog, &Sounds::default());
        peak = peak.max(feet_abs(&g) - ground_abs(&g));
    }
    assert!(peak > 0.2, "the jump left the ground ({peak})");
    assert!(g.p.grounded);
    assert!((feet_abs(&g) - ground_abs(&g)).abs() < 1e-3);
}

#[test]
fn bots_read_the_same_ground_and_every_walkable_cell_has_headroom() {
    let (content, catalog) = (Content::load(), Catalog::load());
    let g = inferno(2, &content, &catalog);
    let t = g.fac.terrain().expect("terrain");
    let mut seen = (f32::MAX, f32::MIN);
    for c in g.fac.floor_cells().into_iter().filter(|c| on_plain_floor(&g, *c)).step_by(7) {
        let (x, z) = (c.x as f32 + 0.5, c.y as f32 + 0.5);
        let local = local_ground(&g.fac, x, z);
        assert_eq!(local, g.fac.ground_height(x, z));
        seen = (seen.0.min(local), seen.1.max(local));
        assert!(g.fac.ceiling_height(x, z).unwrap() - local >= 2.4);
        assert!(t.ceiling_at(x, z) <= crate::world::inferno_terrain::TerrainRules::slab_line() + 1e-4);
    }
    assert!(seen.1 - seen.0 > 0.2);
}

mod lava_damage {
    use crate::world::infinite::{ChunkKey, INFERNO_LEVEL};
    use crate::world::inferno::{InfernoChunk, InfernoRules};

    fn lava_chunks() -> Vec<InfernoChunk> {
        let mut v = Vec::new();
        for cx in 0..4 {
            for cy in 0..4 {
                let inf = InfernoChunk::carve("LAVA-DMG", ChunkKey { cx, cy, level: INFERNO_LEVEL });
                if !inf.lava_cells().is_empty() {
                    v.push(inf);
                }
            }
        }
        assert!(!v.is_empty(), "the sample has lava");
        v
    }

    #[test]
    fn the_damage_cells_are_exactly_the_carved_lava_cells() {
        let dps = InfernoRules::builtin().lava.damage_per_second;
        assert!(dps > 0.0);
        for inf in lava_chunks() {
            let marked = inf.lava_damage_cells();
            assert_eq!(marked.iter().map(|m| m.0).collect::<Vec<_>>(), inf.lava_cells());
            assert!(marked.iter().all(|m| m.1 == dps));
            for c in inf.open_cells() {
                assert!(!marked.iter().any(|m| m.0 == c), "no walkable cell burns");
            }
        }
    }

    #[test]
    fn standing_in_or_at_the_edge_of_lava_burns_and_open_ground_does_not() {
        let r = InfernoRules::builtin();
        for inf in lava_chunks() {
            let l = inf.lava_cells()[0];
            assert_eq!(inf.lava_damage_at(l.x as f32 + 0.5, l.y as f32 + 0.5), r.lava.damage_per_second);
            // a walkable cell with no lava within two cells is safe
            let safe = inf.open_cells().into_iter().find(|c| (-2..=2).all(|dy| (-2..=2).all(|dx| !inf.is_lava(crate::world::Cell::new(c.x + dx, c.y + dy)))));
            if let Some(c) = safe {
                assert_eq!(inf.lava_damage_at(c.x as f32 + 0.5, c.y as f32 + 0.5), 0.0);
            }
        }
    }

    #[test]
    fn the_lava_data_reads_through_the_rules() {
        let mut text: serde_json::Value = serde_json::from_str(include_str!("../data/inferno.json")).unwrap();
        text["lava"]["damage_per_second"] = serde_json::json!(5.0);
        let r = InfernoRules::parse(&text.to_string());
        assert_eq!(r.lava.damage_per_second, 5.0);
        assert!(r.lava.glow_radius_m > 0.0 && r.lava.glow_color.iter().all(|c| (0.0..=1.0).contains(c)));
    }
}
