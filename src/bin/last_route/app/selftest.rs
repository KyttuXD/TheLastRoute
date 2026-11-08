//! `--self-test <file.json>`: starts a run the way a player would (Standard, a fixed seed),
//! drives the runner with scripted input and reports from the game's own state whether
//! moving, the main actions (flashlight, recovering a relay), the hunters and the HUD work.

use super::{load_world, render_play, targets, World};
use crate::config::*;
use crate::game::{Controls, Focus, Game, Item};
use crate::ui::{hud, style};
use macroquad::prelude::*;

async fn frame(
    g: &mut Game,
    c: &Controls,
    w: &World,
    tg: &super::Targets,
    drawn: &mut hud::HudDrawn,
) {
    g.update(1.0 / 60.0, c, &w.content, &w.catalog, &w.assets, &w.sounds);
    render_play(g, &w.assets, &w.mat, tg);
    *drawn = hud::draw_hud(g, &w.assets, 60, style::ui_scale());
    next_frame().await;
}

pub async fn run(path: &str) {
    super::checkpoint::ENABLED.store(false, std::sync::atomic::Ordering::Relaxed);
    let w = load_world().await;
    let tg = targets(None, "AUTO");
    let mut g = Game::new(
        Difficulty::Standard,
        12345,
        &w.content,
        &w.catalog,
        &w.assets,
    );
    let mut steps: Vec<serde_json::Value> = vec![];
    let mut drawn = hud::HudDrawn::default();

    // move: walk forward 1.5 s, then sprint 1 s
    let start = g.p.pos;
    let stamina0 = g.p.stamina;
    for i in 0..150 {
        let c = Controls {
            fwd: 1.0,
            sprint: i >= 90,
            look: vec2(if i < 10 { 0.02 } else { 0.0 }, 0.0),
            ..Default::default()
        };
        frame(&mut g, &c, &w, &tg, &mut drawn).await;
    }
    let moved = ((g.p.pos.0 - start.0).powi(2) + (g.p.pos.1 - start.1).powi(2)).sqrt();
    steps.push(serde_json::json!({
        "step": "move", "passed": g.p.walked > 1.0 && g.p.stamina < stamina0,
        "detail": format!("walked {:.1} m ({:.1} m from the start); sprinting drained stamina {:.0} -> {:.0}", g.p.walked, moved, stamina0, g.p.stamina)
    }));

    // act: switch the flashlight on, then recover a relay at arm's length
    frame(
        &mut g,
        &Controls {
            flashlight: true,
            ..Default::default()
        },
        &w,
        &tg,
        &mut drawn,
    )
    .await;
    let lamp0 = g.p.lamp;
    for _ in 0..60 {
        frame(&mut g, &Controls::default(), &w, &tg, &mut drawn).await;
    }
    let lamp_on = g.p.flashlight_on && g.p.lamp < lamp0;
    let relay = g.fac.relays[0];
    g.teleport_beside(relay);
    for _ in 0..3 {
        frame(&mut g, &Controls::default(), &w, &tg, &mut drawn).await;
    }
    let focus = g.focus;
    frame(
        &mut g,
        &Controls {
            interact: true,
            ..Default::default()
        },
        &w,
        &tg,
        &mut drawn,
    )
    .await;
    let relay_ok = g.relays_collected == 1 && g.score == RELAY_SCORE;
    steps.push(serde_json::json!({
        "step": "act", "passed": lamp_on && relay_ok,
        "detail": format!("flashlight on (charge {:.1} -> {:.1}); aimed at {:?}, recovered relays {}/{} for score {}",
            lamp0, g.p.lamp, focus, g.relays_collected, g.fac.relays.len(), g.score)
    }));

    // equipment: take a battery and verify it occupies the real hotbar inventory.
    let battery = g.fac.batteries[0];
    g.teleport_beside(battery);
    for _ in 0..3 {
        frame(&mut g, &Controls::default(), &w, &tg, &mut drawn).await;
    }
    let battery_focus = g.focus;
    frame(
        &mut g,
        &Controls {
            interact: true,
            ..Default::default()
        },
        &w,
        &tg,
        &mut drawn,
    )
    .await;
    let battery_ok = battery_focus == Some(Focus::Battery(0))
        && g.batteries_taken[0]
        && g.slots.iter().flatten().any(|s| s.item == Item::Battery);
    steps.push(serde_json::json!({
        "step": "equipment", "passed": battery_ok,
        "detail": format!("aimed at {:?}; battery taken {}; inventory has battery {}", battery_focus, g.batteries_taken[0], g.slots.iter().flatten().any(|s| s.item == Item::Battery))
    }));

    // hunter: the first relay wakes the floor's hunter on Standard
    let h0 = g.hunters[0].pos;
    for _ in 0..120 {
        frame(&mut g, &Controls::default(), &w, &tg, &mut drawn).await;
    }
    let h = &g.hunters[0];
    let hd = ((h.pos.0 - h0.0).powi(2) + (h.pos.1 - h0.1).powi(2)).sqrt();
    let holds = h.gimmick.holds_position(h);
    steps.push(serde_json::json!({
        "step": "hunter", "passed": h.active,
        "detail": format!("{} woke after the first relay ({:?}){}; moved {:.1} m", h.def.name, h.state, if holds { ", holding her post by her rule" } else { "" }, hd)
    }));

    // progression: the service lift stays locked until every relay is recovered, then
    // rebuilds the next finite floor while carrying the score forward.
    g.admin.god = true;
    for i in 1..g.fac.relays.len() {
        let relay = g.fac.relays[i];
        g.teleport_beside(relay);
        frame(
            &mut g,
            &Controls {
                interact: true,
                ..Default::default()
            },
            &w,
            &tg,
            &mut drawn,
        )
        .await;
    }
    g.teleport_beside(g.fac.exit);
    for _ in 0..3 {
        frame(&mut g, &Controls::default(), &w, &tg, &mut drawn).await;
    }
    let lift_focus = g.focus;
    frame(
        &mut g,
        &Controls {
            interact: true,
            ..Default::default()
        },
        &w,
        &tg,
        &mut drawn,
    )
    .await;
    let lift_ok = lift_focus == Some(Focus::Lift)
        && g.depth == 2
        && g.relays_collected == 0
        && g.score == 500;
    steps.push(serde_json::json!({
        "step": "lift", "passed": lift_ok,
        "detail": format!("aimed at {:?}; descended to depth {:02}, new floor has {}/{} relays, carried score {}", lift_focus, g.depth, g.relays_collected, g.fac.relays.len(), g.score)
    }));

    steps.push(serde_json::json!({
        "step": "hud",
        "passed": drawn.depth && drawn.relays && drawn.stamina && drawn.bars >= 3 && drawn.hotbar && drawn.minimap,
        "detail": format!("depth {:02}, {}/{} signal relays, {} status bars, five-slot hotbar and minimap drawn", g.depth, g.relays_collected, g.fac.relays.len(), drawn.bars)
    }));
    let authored = g
        .fac
        .props
        .iter()
        .filter(|p| {
            w.content
                .props
                .get(&p.ptype)
                .map(|d| d.models.iter().any(|m| w.assets.models.contains_key(m)))
                .unwrap_or(false)
        })
        .count();
    let items = w
        .assets
        .models
        .keys()
        .filter(|k| k.starts_with("interactables/"))
        .count();
    steps.push(serde_json::json!({
        "step": "floor", "passed": !g.fac.props.is_empty() && g.fac.rooms.len() == 9 && authored > 0 && items >= 10,
        "detail": format!("{} rooms ({}), {} props ({} drawn with their models; {} models loaded, {} item/fixture models), {} lockers, {} vents", g.fac.rooms.len(),
            g.fac.rooms.iter().skip(1).map(|r| r.rtype.as_str()).collect::<Vec<_>>().join(", "), g.fac.props.len(), authored, w.assets.models.len(), items, g.fac.hiding.len(), g.fix.vents.len())
    }));

    // infinite: CREATE ROOM's route streams 3x3 chunks; take the origin's stairs down a level,
    // then walk into the next chunk and check the window recentres
    let mut g = Game::new_infinite(
        Difficulty::Standard,
        12345,
        &crate::world::route::room_text(12345),
        0,
        &w.content,
        &w.catalog,
        &w.assets,
    );
    let window_ok = g.fac.cols == 135 && g.fac.rows == 135 && !g.fac.has_lift;
    let down = g.fac.stairs.iter().position(|s| s.dir < 0);
    let mut stair_detail = "no stairwell in the start chunk".to_string();
    let mut stairs_ok = false;
    if let Some(i) = down {
        let s = g.fac.stairs[i];
        g.p.pos = s.cell.center();
        g.p.yaw = (-(s.run.0 as f32)).atan2(-(s.run.1 as f32));
        g.p.pitch = -0.45;
        for _ in 0..3 {
            frame(&mut g, &Controls::default(), &w, &tg, &mut drawn).await;
        }
        let focus = g.focus;
        frame(
            &mut g,
            &Controls {
                interact: true,
                ..Default::default()
            },
            &w,
            &tg,
            &mut drawn,
        )
        .await;
        let level = g.inf.as_ref().map(|i| i.level).unwrap_or(0);
        stairs_ok =
            level == -1 && g.depth == 2 && g.fac.body_fits(g.p.pos.0, g.p.pos.1, PLAYER_RADIUS);
        stair_detail = format!(
            "aimed at {:?}; now on level {} (depth {:02}), {} stairwells here",
            focus,
            level,
            g.depth,
            g.fac.stairs.len()
        );
    }
    let east = g
        .fac
        .floor_cells()
        .into_iter()
        .filter(|c| c.x >= 92 && c.x <= 100 && (c.y - 67).abs() < 20)
        .min_by_key(|c| (c.y - 67).abs() * 10 + c.x);
    let mut stream_ok = false;
    let mut stream_detail = String::new();
    if let Some(c) = east {
        let before = g.inf.as_ref().map(|i| i.center).unwrap_or((0, 0));
        g.p.pos = c.center();
        frame(&mut g, &Controls::default(), &w, &tg, &mut drawn).await;
        let after = g.inf.as_ref().map(|i| i.center).unwrap_or((0, 0));
        stream_ok = after == (before.0 + 1, before.1) && g.p.pos.0 < 90.0;
        stream_detail = format!(
            "window centre {:?} -> {:?}, {} chunks charted",
            before,
            after,
            g.chunks_visited()
        );
    }
    steps.push(serde_json::json!({
        "step": "infinite", "passed": window_ok && stairs_ok && stream_ok,
        "detail": format!("135x135 live window, no lift; {stair_detail}; {stream_detail}")
    }));
    let out = serde_json::json!({ "version": 1, "steps": steps });
    if let Some(dir) = std::path::Path::new(path).parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, serde_json::to_string_pretty(&out).unwrap());
}
