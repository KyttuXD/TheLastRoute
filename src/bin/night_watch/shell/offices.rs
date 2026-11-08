#![allow(dead_code)]
//! The office registry (port of `night_watch_office_registry.gd` and the `required_stations` of each office spec): one
//! office per mode, every id different, with the stations each one must offer. Pure data, no drawing.

use crate::core::data::MODE_ID_LIST;

/// A mode's office: its id, whether the Rust port builds it yet, and the stations it must list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OfficeSpec {
    pub mode: &'static str,
    pub office_id: &'static str,
    pub built: bool,
    pub required_stations: &'static [&'static str],
}

const DOORS: &[&str] = &["monitor", "doorL_door", "doorR_door", "doorL_light", "doorR_light", "lamp", "chair"];

/// One row per mode, in mode order (night_watch_office_registry.gd 12-26). Bedroom and Open Air have no office in Rust yet.
pub const OFFICES: [OfficeSpec; 9] = [
    OfficeSpec { mode: "classic", office_id: "classic_security", built: true, required_stations: DOORS },
    OfficeSpec { mode: "mask", office_id: "front_office", built: true, required_stations: &["monitor", "chair", "lamp", "mask", "musicbox", "ventL", "ventR", "hall_light"] },
    OfficeSpec { mode: "lure", office_id: "broadcast_booth", built: true, required_stations: DOORS },
    OfficeSpec { mode: "bedroom", office_id: "child_bedroom", built: false, required_stations: &["closet_door", "hall_light", "bed_light"] },
    OfficeSpec { mode: "story", office_id: "control_room", built: true, required_stations: &["terminal", "hide_spot", "door_control_L", "door_control_R", "door_control_C"] },
    OfficeSpec { mode: "descent", office_id: "lift_landing", built: true, required_stations: &["panel", "door"] },
    OfficeSpec {
        mode: "dayshift",
        office_id: "cargo_bay",
        built: true,
        required_stations: &["doorL_door", "doorR_door", "doorL_light", "doorR_light", "lamp", "chair", "ledger", "rack0", "rack1", "rack2", "rack3"],
    },
    OfficeSpec { mode: "extra", office_id: "watch_post", built: false, required_stations: &["lamp", "can0", "can1", "can2"] },
    OfficeSpec { mode: "mixed", office_id: "hub_office", built: true, required_stations: &["monitor", "doorL_door", "doorR_door", "lamp", "chair", "ventL", "ventR", "ventT"] },
];

pub fn spec_for(mode: &str) -> Option<&'static OfficeSpec> {
    OFFICES.iter().find(|o| o.mode == mode)
}

pub fn office_id(mode: &str) -> &'static str {
    spec_for(mode).map_or("", |o| o.office_id)
}

pub fn built(mode: &str) -> bool {
    spec_for(mode).map_or(false, |o| o.built)
}

/// Problems with the table: a mode without a row, a shared office id, a built office with no stations, a monitor mode without it.
pub fn problems() -> Vec<String> {
    let mut p = Vec::new();
    for id in MODE_ID_LIST.iter() {
        if spec_for(id).is_none() {
            p.push(format!("offices: {id} has no office"));
        }
    }
    for (i, a) in OFFICES.iter().enumerate() {
        if a.required_stations.is_empty() {
            p.push(format!("offices: {} lists no stations", a.mode));
        }
        for b in OFFICES.iter().skip(i + 1) {
            if a.office_id == b.office_id {
                p.push(format!("offices: {} and {} share {}", a.mode, b.mode, a.office_id));
            }
        }
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_mode_has_its_own_office() {
        assert_eq!(OFFICES.len(), MODE_ID_LIST.len());
        for (row, id) in OFFICES.iter().zip(MODE_ID_LIST.iter()) {
            assert_eq!(row.mode, *id);
        }
        assert!(problems().is_empty(), "{:?}", problems());
    }

    #[test]
    fn no_two_modes_share_an_office_id() {
        let mut ids: Vec<&str> = OFFICES.iter().map(|o| o.office_id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 9);
    }

    #[test]
    fn unbuilt_offices_are_bedroom_and_open_air() {
        let unbuilt: Vec<&str> = OFFICES.iter().filter(|o| !o.built).map(|o| o.mode).collect();
        assert_eq!(unbuilt, vec!["bedroom", "extra"]);
        assert_eq!(office_id("classic"), "classic_security");
        assert!(!built("nope") && office_id("nope").is_empty());
    }

    #[test]
    fn classic_stations_match_the_built_room() {
        let o = crate::gfx::office::Office::parts(&crate::gfx::office::OfficeConfig::default());
        for s in spec_for("classic").unwrap().required_stations {
            assert!(o.stations.iter().any(|d| d.id == *s), "classic office lacks {s}");
        }
    }
}
