//! The wolf spider's rig and presentation (assets/hunters/anim/white_wolf_spider.json from extract_creature.py, the
//! creature clips of `HunterView`, and the pounce-and-pin grab on a creature rig). The model is the original bbmodel.

use super::clip::{rig, Rig};
use super::grab;
use super::hunter::{CreatureInput, HunterView, Input};
use crate::content::embedded;
use crate::hunters::brain::State;
use crate::hunters::gimmicks::wolf_spider::Phase;
use crate::hunters::secret_creatures::{IDS, WHITE};
use macroquad::prelude::*;
use serde_json::Value;

const CLIPS: [&str; 6] = ["idle", "walk", "sprint", "bark", "attack", "death"];

fn json(path: &str) -> Value {
    serde_json::from_slice(embedded::asset(path).unwrap_or_else(|| panic!("{path} is not embedded"))).unwrap()
}

fn group_names(model: &Value) -> Vec<String> {
    fn walk(nodes: &Value, out: &mut Vec<String>) {
        for n in nodes.as_array().into_iter().flatten() {
            if n.is_object() {
                out.push(n["name"].as_str().unwrap_or("").to_string());
                walk(&n["children"], out);
            }
        }
    }
    let mut out = vec![];
    walk(&model["outliner"], &mut out);
    out
}

#[test]
fn the_model_is_embedded_with_its_palette_and_a_hidden_hitbox() {
    for (id, cubes) in [(WHITE, 90)] {
        let model = json(&format!("hunters/{id}.bbmodel"));
        assert!(crate::render::bbmodel::texture_bytes(&model, &|_: &str| None).is_some(), "{id}: no embedded palette");
        let visible = model["elements"].as_array().unwrap().iter().filter(|e| e["visibility"].as_bool() != Some(false)).count();
        assert_eq!(visible, cubes, "{id}: visible cubes (the hitbox cube is hidden)");
        assert!(model["elements"].as_array().unwrap().iter().any(|e| e["name"] == "hitbox" && e["visibility"].as_bool() == Some(false)), "{id}");
    }
}

#[test]
fn the_rigs_drive_the_models_own_groups_and_carry_the_six_clips() {
    for id in IDS {
        let rig_json = json(&format!("hunters/anim/{id}.json"));
        let names = group_names(&json(&format!("hunters/{id}.bbmodel")));
        let bones: Vec<&str> = rig_json["bones"].as_array().unwrap().iter().map(|b| b[0].as_str().unwrap()).collect();
        for b in &bones {
            assert!(names.iter().any(|n| n == b), "{id}: rig bone {b} is no group of the model");
        }
        for n in names.iter().filter(|n| *n != "hitbox") {
            assert!(bones.contains(&n.as_str()), "{id}: model group {n} has no bone");
        }
        // 0.0525 m per unit: the original 0.035 scaled 1.5x (the spider is drawn 1.5x its original size)
        assert!((rig_json["metres"].as_f64().unwrap() - 0.0525).abs() < 1e-9, "{id}: 0.0525 m per unit (1.5x the original 0.035)");
        for clip in CLIPS {
            let c = &rig_json["clips"][clip];
            assert!(c.is_object(), "{id}: no {clip} clip");
            assert!(!c["tracks"].as_array().unwrap().is_empty(), "{id}: {clip} has no tracks");
            for tr in c["tracks"].as_array().unwrap() {
                assert!((tr[0].as_u64().unwrap() as usize) < bones.len(), "{id}: {clip} drives a missing bone");
                let n = if tr[1] == "t" { 3 } else { 4 };
                assert_eq!(tr[3].as_array().unwrap().len(), tr[2].as_array().unwrap().len() * n, "{id}: {clip} track shape");
                assert!(tr[3].as_array().unwrap().iter().all(|v| v.as_f64().is_some_and(f64::is_finite)));
            }
        }
        for looping in ["idle", "walk", "sprint"] {
            assert_eq!(rig_json["clips"][looping]["loop"], true, "{id}: {looping} loops");
        }
        for once in ["bark", "attack", "death"] {
            assert_eq!(rig_json["clips"][once]["loop"], false, "{id}: {once} plays once");
        }
        for (clip, len) in [("idle", 4.0), ("walk", 1.2), ("sprint", 0.6), ("bark", 0.9), ("attack", 1.1), ("death", 2.6)] {
            assert!((rig_json["clips"][clip]["len"].as_f64().unwrap() - len).abs() < 0.01, "{id}: {clip} length");
        }
        assert!(rig_json["stride"]["walk"]["ref"].as_f64().unwrap() > 0.0 && rig_json["stride"]["sprint"]["ref"].as_f64().unwrap() > rig_json["stride"]["walk"]["ref"].as_f64().unwrap(), "{id}: stride");
    }
}

#[test]
fn a_creature_rig_is_drawn_at_its_own_scale_and_loops_close() {
    for id in IDS {
        let r: std::rc::Rc<Rig> = rig(id).expect("the rig loads");
        assert!(r.is_creature());
        // 0.0525 = the original 0.035 x 1.5, so the spider is drawn 1.5x bigger
        assert!((r.draw_scale() - 0.0525).abs() < 1e-6, "{id}: {}", r.draw_scale());
        for clip in ["idle", "walk", "sprint"] {
            let c = &r.clips[clip];
            assert!(c.looping && (c.length - c.times[c.times.len() - 1]).abs() < 1e-4);
            // the loop closes: the last key of every track is the first
            for tr in &c.tracks {
                assert!((tr.keys[0] - tr.keys[tr.keys.len() - 1]).abs().max_element() < 1e-3 || tr.kind == super::clip::Kind::Rot, "{id}: {clip} does not close");
            }
        }
    }
    // a humanoid keeps the humanoid scale
    let aki = rig("aki").unwrap();
    assert!(!aki.is_creature() && (aki.draw_scale() - super::hunter::MODEL_SCALE / aki.unit).abs() < 1e-9);
}

#[test]
fn the_grab_pool_is_the_pounce_and_the_creature_rig_falls_back_to_it() {
    for id in IDS {
        assert_eq!(grab::pool_of(id), vec![("pounce_pin".to_string(), 6.0)], "{id}");
        // no room: a creature rig falls back to the pounce, never to a humanoid style
        let cramped = grab::Room { ceiling: 0.5, back: 0.1, hunter_back: 0.1 };
        assert_eq!(grab::pick(id, 0.5, &cramped), "pounce_pin", "{id}");
        assert_ne!(grab::pick("aki", 0.5, &cramped), "pounce_pin", "a humanoid never pounces");
        let open = grab::Room { ceiling: 3.0, back: 3.0, hunter_back: 3.0 };
        assert_eq!(grab::pick(id, 0.5, &open), "pounce_pin");
    }
    let pose = grab::pose("pounce_pin", grab::duration("pounce_pin"), &grab::Ctx { eye: 1.6, shoulder_side: -0.45, shoulder_fwd: 0.0, shoulder_y: 1.8, arm: 0.66, back: 3.0 });
    assert!(pose.tilt > 1.0, "the runner is knocked nearly flat: {}", pose.tilt);
    assert!(pose.arms.is_empty(), "a creature holds with its body, not arms");
}

fn input(pos: (f32, f32), creature: Option<CreatureInput>, state: State) -> Input<'static> {
    Input {
        pos,
        brain_facing: (1.0, 0.0),
        active: true,
        stun: 0.0,
        heavy_stun: false,
        state,
        direct_visible: false,
        winded: false,
        airborne: false,
        lift: 0.0,
        sway: (0.0, 0.0),
        charging: false,
        look_target: None,
        path_ahead: None,
        chase_style: "pump",
        camera: vec3(0.0, 1.6, 0.0),
        grabbing: false,
        peek_lean: None,
        creature,
        mechanic: None,
    }
}

fn creature(phase: Phase, bark_seq: u32, reach: bool) -> Option<CreatureInput> {
    Some(CreatureInput { phase, bark_seq, target_in_reach: reach })
}

/// Runs `n` frames at 30 fps, the body moving `speed` m/s along +X from `x`; returns the clip of the last frame and the new x.
fn run(view: &mut HunterView, mut x: f32, speed: f32, n: usize, c: Option<CreatureInput>, state: State, clock: &mut f32) -> (String, f32) {
    for _ in 0..n {
        x += speed / 30.0;
        *clock += 1.0 / 30.0;
        view.update(&input((x, 5.0), c, state), 1.0 / 30.0, *clock);
    }
    (view.clip().to_string(), x)
}

#[test]
fn the_clip_follows_the_gimmick_phase() {
    let mut view = HunterView::new(WHITE, rig(WHITE).unwrap());
    let mut clock = 0.0;
    // lurking: idle; creeping to its lair: walk; hunting at speed: sprint; standing while barking: idle
    let (clip, x) = run(&mut view, 0.0, 0.0, 30, creature(Phase::Lurk, 0, false), State::Idle, &mut clock);
    assert_eq!(clip, "idle");
    let (clip, x) = run(&mut view, x, 0.8, 30, creature(Phase::Retreat, 0, false), State::Wander, &mut clock);
    assert_eq!(clip, "walk");
    let (clip, x) = run(&mut view, x, 4.9, 30, creature(Phase::Hunt, 0, false), State::Chase, &mut clock);
    assert_eq!(clip, "sprint");
    let (clip, x) = run(&mut view, x, 0.0, 30, creature(Phase::Hunt, 0, false), State::Chase, &mut clock);
    assert_eq!(clip, "idle", "a hunting spider that stands still stands");
    // a new bark number plays the bark clip once, then it is idle again
    let (clip, x) = run(&mut view, x, 0.0, 2, creature(Phase::Bark, 1, false), State::Idle, &mut clock);
    assert_eq!(clip, "bark");
    let (clip, x) = run(&mut view, x, 0.0, 20, creature(Phase::Bark, 1, false), State::Idle, &mut clock);
    assert_eq!(clip, "bark", "the bark clip is 0.9 s long");
    let (clip, x) = run(&mut view, x, 0.0, 30, creature(Phase::Bark, 1, false), State::Idle, &mut clock);
    assert_eq!(clip, "idle");
    // a runner within reach on the hunt gets the lunge, once per cooldown
    let (clip, _) = run(&mut view, x, 4.9, 3, creature(Phase::Hunt, 1, true), State::Chase, &mut clock);
    assert_eq!(clip, "attack");
}

#[test]
fn the_first_sight_of_a_spider_already_barking_plays_the_bark() {
    let mut view = HunterView::new(WHITE, rig(WHITE).unwrap());
    let mut clock = 0.0;
    let (clip, _) = run(&mut view, 0.0, 0.0, 2, creature(Phase::Bark, 1, false), State::Idle, &mut clock);
    assert_eq!(clip, "bark");
}

#[test]
fn the_posed_creature_stands_on_the_ground_at_its_size() {
    for id in IDS {
        let r = rig(id).unwrap();
        let mut view = HunterView::new(id, r.clone());
        let mut clock = 0.0;
        run(&mut view, 0.0, 0.0, 10, creature(Phase::Lurk, 0, false), State::Idle, &mut clock);
        let (_, out) = view.update(&input((0.0, 5.0), creature(Phase::Lurk, 0, false), State::Idle), 1.0 / 30.0, clock);
        // the model-space root bone sits at the origin and the whole body is finite
        assert!(out.bones.iter().all(|m| m.to_cols_array().iter().all(|v| v.is_finite())), "{id}");
        let top = r.bones.iter().enumerate().map(|(k, _)| out.root.transform_point3(out.bones[k].w_axis.truncate()).y).fold(f32::MIN, f32::max);
        assert!(top > 1.0 && top < 3.0, "{id}: the highest joint is {top} m up (about 2 m at the head)");
        let low = r.bones.iter().enumerate().map(|(k, _)| out.root.transform_point3(out.bones[k].w_axis.truncate()).y).fold(f32::MAX, f32::min);
        assert!(low > -0.5, "{id}: the lowest joint is {low} m");
    }
}

#[test]
fn a_catch_starts_the_pounce_and_pin_with_a_finite_camera() {
    let fac = crate::world::Facility::generate("SPIDER-GRAB", &crate::world::facility::Options::default(), &crate::world::catalog::Catalog::load());
    let at = fac.start.center();
    for id in IDS {
        grab::abandon();
        let mut view = HunterView::new(id, rig(id).unwrap());
        let mut clock = 0.0;
        let runner_feet = vec3(at.0 + 1.5, 0.0, at.1);
        let camera = (vec3(at.0 + 1.5, 1.6, at.1), vec3(-1.0, 0.0, 0.0));
        let mut used_style = String::new();
        for frame in 0..90 {
            clock += 1.0 / 30.0;
            let inp = Input { grabbing: true, ..input(at, creature(Phase::Hunt, 1, true), State::Chase) };
            let (pose, normal) = view.update(&inp, 1.0 / 30.0, clock);
            let out = grab::drive(0, true, &view, pose, normal, clock, 1.0 / 30.0, 7, runner_feet, 1.6, camera, &fac, 3.0);
            assert!(out.bones.iter().all(|m| m.to_cols_array().iter().all(|v| v.is_finite())), "{id}: frame {frame} bones");
            let cam = grab::camera().unwrap_or_else(|| panic!("{id}: no usable grab camera on frame {frame}"));
            assert!(grab::usable_camera(cam), "{id}: frame {frame} camera {cam:?}");
            used_style = super::grab::GRAB.with(|g| g.borrow().grab.as_ref().map(|x| x.style.clone()).unwrap_or_default());
        }
        assert_eq!(used_style, "pounce_pin", "{id}");
        grab::abandon();
    }
}
