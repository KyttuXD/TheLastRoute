//! Open Air's admin panel (port of `openair_admin.gd`, plus the generic hooks the Godot admin menu offered every mode): the
//! controls of the watch that is up (rooftop: lantern, oil, the door she is at; bus stop: warmth, fuel, chill, a canister, the
//! bus; campfire: fire, logs, Verrine's mood) and the weather; then god mode, freeze, reveal, no scare, the hour, the hunters'
//! speed and where each hunter stands. Opens only for an admin account (`hud::admin::AdminGate`); any use marks the run DEBUG.
//!
//! Register it with `admin_reg.register(Box::new(crate::modes::openair::OpenAirAdmin));`. Rows of other watches do nothing.
#![allow(dead_code)]

use super::game::OpenAirMode;
use super::sim::Side;
use crate::core::{HOUR_MS, NIGHT_MS};
use crate::hud::admin::{AdminAction, ModeAdmin};

pub struct OpenAirAdmin;

impl ModeAdmin for OpenAirAdmin {
    fn mode_id(&self) -> &'static str {
        super::data::MODE_ID
    }

    fn title(&self) -> String {
        "OPEN AIR SPECIFIC".to_string()
    }

    fn actions(&self) -> Vec<AdminAction> {
        let mut v: Vec<AdminAction> = Vec::new();
        let mut add = |label: &str, id: &str| v.push(AdminAction::new(label, id));
        // ROOFTOP
        add("ROOFTOP: LIGHT THE LANTERN", "lantern:1");
        add("ROOFTOP: BLOW IT OUT", "lantern:0");
        add("ROOFTOP: OIL 100%", "oil:100");
        add("ROOFTOP: OIL 50%", "oil:50");
        add("ROOFTOP: OIL 10%", "oil:10");
        add("ROOFTOP: OIL 0% (WINCHES STOP)", "oil:0");
        add("ROOFTOP: HER DOOR THE STAIR DOOR (LEFT)", "side:L");
        add("ROOFTOP: HER DOOR THE HATCH (RIGHT)", "side:R");
        // BUS STOP
        add("BUS STOP: WARMTH 100%", "warm:100");
        add("BUS STOP: WARMTH 50%", "warm:50");
        add("BUS STOP: WARMTH 10% (BITING)", "warm:10");
        add("BUS STOP: FUEL 100%", "fuel:100");
        add("BUS STOP: FUEL 50%", "fuel:50");
        add("BUS STOP: FUEL 0% (LAMP DRY)", "fuel:0");
        add("BUS STOP: CHILL 0%", "chill:0");
        add("BUS STOP: CHILL 50%", "chill:50");
        add("BUS STOP: CHILL 90% (ABOUT TO FREEZE)", "chill:90");
        add("BUS STOP: DROP A CANISTER NOW", "can:1");
        add("BUS STOP: CALL THE BUS (LAST 18 SECONDS)", "bus:1");
        // CAMPFIRE
        add("CAMPFIRE: FIRE 100%", "fire:100");
        add("CAMPFIRE: FIRE 50%", "fire:50");
        add("CAMPFIRE: FIRE 10% (LOW: THEY GET BOLDER)", "fire:10");
        add("CAMPFIRE: FIRE 0% (OUT)", "fire:0");
        add("CAMPFIRE: NO LOGS", "logs:0");
        add("CAMPFIRE: 6 LOGS", "logs:6");
        add("CAMPFIRE: 12 LOGS", "logs:12");
        add("CAMPFIRE: VERRINE CALM", "mood:0");
        add("CAMPFIRE: VERRINE BOLD (QUICKER)", "mood:1");
        add("CAMPFIRE: VERRINE SHY (SLOWER)", "mood:2");
        // WEATHER
        add("WEATHER NOW", "rain:1");
        add("WEATHER OFF", "rain:0");
        // GENERIC
        add("GOD MODE ON", "god:1");
        add("GOD MODE OFF", "god:0");
        add("FREEZE THE WATCH", "freeze:1");
        add("RUN THE WATCH", "freeze:0");
        add("REVEAL HUNTERS", "reveal:1");
        add("HIDE HUNTERS", "reveal:0");
        add("NO DEATH SCENE ON", "noscare:1");
        add("NO DEATH SCENE OFF", "noscare:0");
        for h in 0..6 {
            add(&format!("SET THE HOUR {}", crate::core::hour_label(h)), &format!("hour:{h}"));
        }
        add("HUNTER SPEED 1X", "speed:1");
        add("HUNTER SPEED 2X", "speed:2");
        add("HUNTER SPEED 4X", "speed:4");
        for who in ["verrine", "wynelle", "ysolde", "quenelle"] {
            for place in ["far", "step 2", "step 1", "at the edge"] {
                add(&format!("PLACE {}: {}", who.to_uppercase(), place.to_uppercase()), &format!("place:{who}:{place}"));
            }
        }
        v
    }
}

/// Run a row of the panel on the mode. Unknown ids and rows of another watch change nothing.
pub fn run(m: &mut OpenAirMode, id: &str) {
    let mut it = id.splitn(3, ':');
    let kind = it.next().unwrap_or("");
    let a = it.next().unwrap_or("");
    let b = it.next().unwrap_or("");
    match kind {
        "lantern" | "oil" | "warm" | "fuel" | "chill" | "fire" | "logs" | "mood" | "rain" => {
            let v = a.parse::<f64>().unwrap_or(0.0);
            m.sim.admin_set(kind, v);
        }
        "can" | "bus" => {
            m.sim.admin_set(kind, 1.0);
        }
        "side" => m.sim.admin_set_side(if a == "R" { Side::R } else { Side::L }),
        "god" => m.sim.god = a == "1",
        "speed" => m.sim.speed = a.parse::<f64>().unwrap_or(1.0).clamp(0.1, 10.0),
        "freeze" => m.admin_set_frozen(a == "1"),
        "reveal" => m.admin_set_reveal(a == "1"),
        "noscare" => m.admin_set_no_scare(a == "1"),
        "hour" => {
            let h = a.parse::<i32>().unwrap_or(0).clamp(0, 5);
            m.admin_set_hour(h, (h as f64 * HOUR_MS).clamp(0.0, NIGHT_MS - 1000.0));
        }
        "place" => m.sim.admin_place(a, b),
        _ => {}
    }
    m.admin_resync();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::Session;
    use std::collections::BTreeMap;

    fn mode(level: i32) -> OpenAirMode {
        let mut s = Session::new();
        s.start_mode("extra", level, BTreeMap::new(), Some(5));
        OpenAirMode::new(&s, (level - 1) as usize, 0, false, 3)
    }

    #[test]
    fn the_panel_belongs_to_open_air_and_its_ids_are_unique() {
        let p = OpenAirAdmin;
        assert_eq!(p.mode_id(), "extra");
        let acts = p.actions();
        assert!(acts.len() > 40);
        let mut ids: Vec<&str> = acts.iter().map(|a| a.id.as_str()).collect();
        ids.sort();
        let n = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), n, "every row has its own id");
    }

    #[test]
    fn rooftop_rows_change_the_rooftop() {
        let mut m = mode(1);
        run(&mut m, "oil:10");
        assert_eq!(m.sim.oil, 10.0);
        run(&mut m, "lantern:0");
        assert!(!m.sim.lantern);
        run(&mut m, "side:L");
        assert_eq!(m.sim.climber.side, Side::L);
        run(&mut m, "place:verrine:at the edge");
        assert_eq!(m.sim.climber.dist, 0);
        run(&mut m, "fire:0");
        assert_eq!(m.sim.fire, 100.0, "a campfire row does nothing on the rooftop");
    }

    #[test]
    fn the_generic_rows_set_god_speed_and_the_hour() {
        let mut m = mode(3);
        run(&mut m, "god:1");
        assert!(m.sim.god);
        run(&mut m, "speed:2");
        assert_eq!(m.sim.speed, 2.0);
        run(&mut m, "hour:3");
        assert_eq!(m.sim.elapsed, 3.0 * HOUR_MS);
        run(&mut m, "logs:12");
        assert_eq!(m.sim.logs, 12);
        run(&mut m, "mood:1");
        assert_eq!(m.sim.mood.as_str(), "bold");
        run(&mut m, "place:ysolde:step 1");
        assert_eq!(m.sim.y.dist, 1);
        run(&mut m, "nonsense");
    }

    #[test]
    fn a_placed_hunter_stands_where_the_rules_put_her() {
        let mut m = mode(3);
        run(&mut m, "place:verrine:step 1");
        let want = m.cast.nav().pos("V1").expect("V1");
        let got = m.cast.agent("verrine").expect("verrine").walker.pos;
        assert!(got.distance(want) < 0.01);
    }
}
