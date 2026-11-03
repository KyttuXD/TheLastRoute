//! Level-0 hunter residency for the live Infinite route: which of the layout's hunter spawns are in the world, and which
//! of those are loaded into the live window. Ports the admission and residency rules of Godot's
//! `_refresh_hunter_stream_work` (game/infinite_route/infinite_route_test.gd), with `_hunter_spawn_score`,
//! `_unique_hunter_identity` and `_unused_hunter_identity`, `_all_a_z_hunters_present`, `_void_auto_spawn_allowed` and the
//! office-void roll of its candidate loop.
//!
//! The rules here are pure. `admit` takes the hunters that exist (`Record`s, stored and loaded) and the candidate spawns of
//! the active chunks, and says which hunters are wanted, which to create and which of the wanted ones stand in the
//! runner's window. `game::endless` applies that to its `Brain`s.
//!
//! The live window is the runner's 3x3 chunks, so a wanted hunter outside it stays a stored record and is not simulated.
//! Godot simulates the hunters of every active ring (HUNTER_RESIDENCY_CHUNK_RADIUS); see docs/PARITY.md, "Live level 0".
//!
//! Not ported: the day-scaled population and the public route's population and identities (no day or public route in
//! Rust), `_repair_duplicate_hunter_identities` (saves from earlier generator revisions), the removed-hunter pressure slots
//! (admin removals on the Infinite route), and the Gray Void anti-idle spawn. HUNTER_VERTICAL_STREAM_RADIUS is kept as a
//! constant, but no hunter on another level is in a level-0 window, so the vertical rings add nothing yet.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use crate::world::infinite::CHUNK;
use crate::world::rng::Rng;
use crate::world::route::sewer::hunters::office_void_for;

/// `HUNTER_RESIDENCY_CHUNK_RADIUS` (infinite_route_test.gd): an awake hunter this many chunks from the runner
/// brings its own 3x3 ring into the active chunks.
pub const RESIDENCY_CHUNK_RADIUS: i32 = 3;

/// `HUNTER_VERTICAL_STREAM_RADIUS` (infinite_route_test.gd): see the module note.
pub const VERTICAL_STREAM_RADIUS: i32 = 1;

/// `HunterRoster.A_Z_IDS` (config/hunter_roster.gd).
pub const A_Z_IDS: [&str; 26] = [
    "aki", "briselle", "ceryth", "dovelle", "elise", "ferryn", "glyra", "haelith", "ilyra", "javelle", "kathy", "loxelle",
    "mavrine", "nyxie", "opalie", "porcelle", "quenelle", "ravelle", "solyth", "thessine", "umbriel", "verrine", "wynelle",
    "xivvy", "ysolde", "zellia",
];

/// `HunterRoster.VOID_IDS` (config/hunter_roster.gd).
pub const VOID_IDS: [&str; 12] = [
    "void_black", "void_blue", "void_brown", "void_cyan", "void_gray", "void_green", "void_orange", "void_pink",
    "void_purple", "void_red", "void_white", "void_yellow",
];

/// A hunter that exists: its entity id, its identity, its absolute position (cells) and whether it is awake.
#[derive(Clone, Debug, PartialEq)]
pub struct Record {
    pub id: String,
    pub hunter_id: String,
    pub pos: (f32, f32),
    pub active: bool,
}

/// A layout spawn of an active chunk: its entity id, the identity the layout gives it, and the centre of its absolute cell.
#[derive(Clone, Debug, PartialEq)]
pub struct Cand {
    pub id: String,
    pub hunter_id: String,
    pub pos: (f32, f32),
}

/// A hunter `admit` creates: its entity id, its identity (after the office-void and uniqueness rules) and its spawn position.
#[derive(Clone, Debug, PartialEq)]
pub struct Created {
    pub id: String,
    pub hunter_id: String,
    pub pos: (f32, f32),
}

/// What `admit` decided. `desired` is every hunter that exists and is wanted this pass, in admission order. `resident` is
/// the wanted ones that stand in the runner's window; the other wanted ones stay stored.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Admission {
    pub desired: Vec<String>,
    pub resident: Vec<String>,
    pub created: Vec<Created>,
}

/// The chunk an absolute position stands in.
pub fn chunk_of(p: (f32, f32)) -> (i32, i32) {
    ((p.0 / CHUNK as f32).floor() as i32, (p.1 / CHUNK as f32).floor() as i32)
}

/// Whether a chunk is in the runner's 3x3 window (the live window is centred on the runner's chunk).
pub fn in_window(chunk: (i32, i32), runner: (i32, i32)) -> bool {
    (chunk.0 - runner.0).abs() <= 1 && (chunk.1 - runner.1).abs() <= 1
}

/// The active chunks of a level (`_refresh_hunter_stream_work`, the active coordinates): the runner's chunk, and the
/// chunk of each awake hunter within RESIDENCY_CHUNK_RADIUS of it. Each of those centres brings its 3x3 ring.
pub fn active_chunks(runner: (i32, i32), recs: &[Record]) -> HashSet<(i32, i32)> {
    let mut centres = vec![runner];
    for r in recs.iter().filter(|r| r.active) {
        let c = chunk_of(r.pos);
        let near = (c.0 - runner.0).abs() <= RESIDENCY_CHUNK_RADIUS && (c.1 - runner.1).abs() <= RESIDENCY_CHUNK_RADIUS;
        if near && !centres.contains(&c) {
            centres.push(c);
        }
    }
    let mut out = HashSet::new();
    for (cx, cy) in centres {
        for dy in -1..=1 {
            for dx in -1..=1 {
                out.insert((cx + dx, cy + dy));
            }
        }
    }
    out
}

/// `_hunter_spawn_score` on level 0: the distance from a spawn to the centre of the runner's chunk (`(c + .5) * CHUNK`).
pub fn spawn_score(pos: (f32, f32), runner: (i32, i32)) -> f32 {
    let centre = (
        (runner.0 as f32 + 0.5) * CHUNK as f32,
        (runner.1 as f32 + 0.5) * CHUNK as f32,
    );
    ((pos.0 - centre.0).powi(2) + (pos.1 - centre.1).powi(2)).sqrt()
}

/// `_all_a_z_hunters_present`: every A-Z identity is held by a hunter that exists.
pub fn all_a_z(used: &HashSet<String>) -> bool {
    A_Z_IDS.iter().all(|id| used.contains(*id))
}

/// `_void_auto_spawn_allowed`: a Void identity is admitted only once the A-Z roster is complete, except Gray.
pub fn void_allowed(hunter_id: &str, all_a_z: bool) -> bool {
    !hunter_id.starts_with("void_") || hunter_id == "void_gray" || all_a_z
}

/// `_unique_hunter_identity`: the proposed identity, or, when a hunter already holds it, an unused A-Z or Void identity
/// picked by `"<seed>:UNIQUE-HUNTER:<entity>"`. The proposed one stays when every identity is taken.
pub fn unique_identity(seed: &str, entity_id: &str, proposed: &str, used: &HashSet<String>) -> String {
    if !used.contains(proposed) {
        return proposed.to_string();
    }
    let available: Vec<&str> = A_Z_IDS
        .iter()
        .chain(VOID_IDS.iter())
        .copied()
        .filter(|id| !used.contains(*id))
        .collect();
    if available.is_empty() {
        return proposed.to_string();
    }
    let mut rng = Rng::new(&format!("{seed}:UNIQUE-HUNTER:{entity_id}"));
    available[rng.next_index(available.len())].to_string()
}

/// `_refresh_hunter_stream_work`'s admission. `recs` are the hunters that exist (stored and loaded), `active` the active
/// chunks from `active_chunks`, `cands` the layout spawns of those chunks, and `limit` the population. A hunter that exists
/// and is in an active chunk is wanted. Then the candidates, by score (ties by id), are admitted while the wanted count
/// is under the limit: a candidate whose entity exists is wanted again, and a new one is created with its identity.
/// `seed` is the world seed text (the generator's `world_seed`).
pub fn admit(
    seed: &str,
    runner: (i32, i32),
    recs: &[Record],
    active: &HashSet<(i32, i32)>,
    cands: &[Cand],
    limit: i32,
) -> Admission {
    let mut adm = Admission::default();
    let mut known: HashMap<String, (f32, f32)> = recs.iter().map(|r| (r.id.clone(), r.pos)).collect();
    let mut used: HashSet<String> = recs.iter().map(|r| r.hunter_id.clone()).collect();
    let mut desired: HashSet<String> = HashSet::new();
    for r in recs {
        if active.contains(&chunk_of(r.pos)) && desired.insert(r.id.clone()) {
            adm.desired.push(r.id.clone());
        }
    }

    let mut pool: Vec<&Cand> = cands.iter().filter(|c| !desired.contains(&c.id)).collect();
    pool.sort_by(|a, b| {
        spawn_score(a.pos, runner)
            .partial_cmp(&spawn_score(b.pos, runner))
            .unwrap_or(Ordering::Equal)
            .then_with(|| a.id.cmp(&b.id))
    });
    for c in pool {
        if adm.desired.len() as i32 >= limit {
            break;
        }
        if known.contains_key(&c.id) {
            if desired.insert(c.id.clone()) {
                adm.desired.push(c.id.clone());
            }
            continue;
        }
        let all = all_a_z(&used);
        // a candidate's own identity is never a Void, so this holds; kept as Godot states it
        if !void_allowed(&c.hunter_id, all) {
            continue;
        }
        // the office-void roll: a rare Void identity, more likely once the A-Z roster is complete
        let chance = if all { 0.25 } else { 0.015 };
        let mut hunter_id = c.hunter_id.clone();
        let mut office = Rng::new(&format!("{seed}:OFFICE-ADMIT:{}", c.id));
        if office.next_float() < chance {
            let office_id = office_void_for(&c.id);
            if void_allowed(office_id, all) {
                hunter_id = office_id.to_string();
            }
        }
        let hunter_id = unique_identity(seed, &c.id, &hunter_id, &used);
        used.insert(hunter_id.clone());
        known.insert(c.id.clone(), c.pos);
        desired.insert(c.id.clone());
        adm.desired.push(c.id.clone());
        adm.created.push(Created {
            id: c.id.clone(),
            hunter_id,
            pos: c.pos,
        });
    }

    adm.resident = adm
        .desired
        .iter()
        .filter(|id| known.get(id.as_str()).is_some_and(|p| in_window(chunk_of(*p), runner)))
        .cloned()
        .collect();
    adm
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(id: &str, hunter_id: &str, pos: (f32, f32), active: bool) -> Record {
        Record {
            id: id.to_string(),
            hunter_id: hunter_id.to_string(),
            pos,
            active,
        }
    }

    fn cand(id: &str, hunter_id: &str, pos: (f32, f32)) -> Cand {
        Cand {
            id: id.to_string(),
            hunter_id: hunter_id.to_string(),
            pos,
        }
    }

    /// The centre of the cell at `x` in chunk `cx` on an axis.
    fn at(c: i32, x: i32) -> f32 {
        (c * CHUNK + x) as f32 + 0.5
    }

    #[test]
    fn the_runners_ring_is_active_and_an_awake_hunter_near_it_adds_its_own() {
        let runner = (0, 0);
        assert_eq!(active_chunks(runner, &[]).len(), 9);
        // a sleeping hunter three chunks east does not add a ring
        let sleeping = [rec("a", "dovelle", (at(3, 5), at(0, 5)), false)];
        assert_eq!(active_chunks(runner, &sleeping).len(), 9);
        // an awake one at chunk 3 adds the columns 2..=4: x -1..=4 over three rows
        let awake = [rec("a", "dovelle", (at(3, 5), at(0, 5)), true)];
        let set = active_chunks(runner, &awake);
        assert_eq!(set.len(), 18);
        assert!(set.contains(&(4, 0)));
        // an awake one at chunk 4 is past the radius
        let far = [rec("a", "dovelle", (at(4, 5), at(0, 5)), true)];
        assert_eq!(active_chunks(runner, &far).len(), 9);
    }

    #[test]
    fn the_population_admits_the_nearest_spawns_first() {
        let runner = (0, 0);
        let active = active_chunks(runner, &[]);
        let cands = vec![
            cand("far", "elise", (at(1, 40), at(0, 40))),
            cand("near", "aki", (at(0, 10), at(0, 10))),
            cand("mid", "xivvy", (at(0, 25), at(0, 25))),
        ];
        let adm = admit("7", runner, &[], &active, &cands, 2);
        // mid is about 4 cells from the runner's chunk centre, near about 17, far about 65
        assert_eq!(adm.desired, vec!["mid".to_string(), "near".to_string()]);
        assert_eq!(adm.created.len(), 2);
        assert_eq!(adm.created[0].hunter_id, "xivvy");
        // the runner's own window holds both
        assert_eq!(adm.resident.len(), 2);
    }

    #[test]
    fn an_existing_hunter_is_wanted_again_and_not_created_twice() {
        let runner = (0, 0);
        let active = active_chunks(runner, &[]);
        let recs = [rec("near", "aki", (at(0, 10), at(0, 10)), false)];
        let cands = vec![cand("near", "aki", (at(0, 10), at(0, 10)))];
        let adm = admit("7", runner, &recs, &active, &cands, 4);
        assert_eq!(adm.desired, vec!["near".to_string()]);
        assert!(adm.created.is_empty());
        assert_eq!(adm.resident, vec!["near".to_string()]);
    }

    #[test]
    fn a_stored_hunter_outside_the_window_is_wanted_but_not_resident() {
        let runner = (0, 0);
        // the sleeper two chunks west is wanted through the ring of an awake hunter three chunks west, but it
        // is outside the runner's 3x3 window
        let recs = [
            rec("walker", "aki", (at(-3, 2), at(0, 2)), true),
            rec("sleeper", "elise", (at(-2, 2), at(0, 2)), false),
        ];
        let active = active_chunks(runner, &recs);
        let adm = admit("7", runner, &recs, &active, &[], 4);
        assert!(adm.desired.contains(&"sleeper".to_string()));
        assert!(!adm.resident.contains(&"sleeper".to_string()));
        assert!(!adm.resident.contains(&"walker".to_string()));
    }

    #[test]
    fn a_duplicate_identity_is_replaced_by_an_unused_one() {
        let used: HashSet<String> = ["dovelle".to_string()].into_iter().collect();
        let got = unique_identity("7", "7:0:0:HUNTER:1", "dovelle", &used);
        assert_ne!(got, "dovelle");
        assert!(A_Z_IDS.contains(&got.as_str()) || VOID_IDS.contains(&got.as_str()), "{got}");
        // a free identity is kept
        assert_eq!(unique_identity("7", "e", "aki", &used), "aki");
    }

    #[test]
    fn a_void_needs_the_full_roster_except_gray() {
        assert!(void_allowed("dovelle", false));
        assert!(void_allowed("void_gray", false));
        assert!(!void_allowed("void_red", false));
        assert!(void_allowed("void_red", true));
    }

    #[test]
    fn the_a_z_roster_is_complete_only_with_all_twenty_six() {
        let mut used: HashSet<String> = A_Z_IDS.iter().map(|s| s.to_string()).collect();
        assert!(all_a_z(&used));
        used.remove("zellia");
        assert!(!all_a_z(&used));
    }

    #[test]
    fn the_spawn_score_is_the_distance_to_the_runners_chunk_centre() {
        let centre = ((0.5) * CHUNK as f32, (0.5) * CHUNK as f32);
        assert!(spawn_score(centre, (0, 0)).abs() < 1e-4);
        assert!((spawn_score((centre.0 + 3.0, centre.1 + 4.0), (0, 0)) - 5.0).abs() < 1e-4);
    }
}
