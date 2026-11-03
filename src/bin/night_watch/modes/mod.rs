//! One module per night type (Godot `night_watch_mode_registry.gd`). `start` opens a night by its hub id and
//! `Session` ticks it; the mode files hold the rules, and the shell plays the events they return.

pub mod bedtime;
pub mod classic;
pub mod dead_air;
pub mod descent;
pub mod facility;
pub mod masked_hours;
pub mod mixed_shift;
pub mod open_air;
pub mod salvage_run;

use crate::data::CallsData;
use crate::framework::Mulberry;

/// A started night of one type. Every variant holds its mode's own state.
pub enum Session {
    Classic(classic::ClassicNight),
    Mask(masked_hours::MaskedHours),
    Lure(dead_air::DeadAir),
    Bedroom(bedtime::Bedtime),
    Story(facility::Facility),
    Descent(descent::DescentShift),
    Dayshift(salvage_run::SalvageRun),
    Extra(open_air::OpenAir),
    Mixed(mixed_shift::MixedShift),
}

/// Opens the first night of the hub entry `mode_id`; None for an id the hub does not list.
pub fn start(mode_id: &str, seed: i64, calls: &CallsData) -> Option<Session> {
    Some(match mode_id {
        "classic" => Session::Classic(classic::start(calls, 1, seed, &[], false)),
        "mask" => Session::Mask(masked_hours::MaskedHours::new(1, Mulberry::new(seed), masked_hours::MaskOptions::default())),
        "lure" => Session::Lure(dead_air::DeadAir::new(dead_air::DeadAirSetup::night(1, seed))),
        "bedroom" => Session::Bedroom(bedtime::Bedtime::new(bedtime::BedtimeOptions::campaign(1, seed))),
        "story" => Session::Story(facility::Facility::start(facility::FacilityOptions::new(1, seed))),
        "descent" => Session::Descent(descent::DescentShift::start(1, seed, None, calls)),
        "dayshift" => Session::Dayshift(salvage_run::SalvageRun::new(1, seed, salvage_run::Options::default())),
        "extra" => Session::Extra(open_air::OpenAir::new(open_air::Watch::from_level(1), seed, open_air::Levels::LISTED, false)),
        "mixed" => Session::Mixed(mixed_shift::MixedShift::start(seed, 0)),
        _ => return None,
    })
}

impl Session {
    /// Advances the night by `dt_ms` with no player input. The shell's controls will feed each mode's input later.
    pub fn tick(&mut self, dt_ms: f64) {
        match self {
            Session::Classic(night) => {
                night.step(dt_ms, classic::Input::default());
            }
            Session::Mask(night) => {
                night.step(dt_ms, masked_hours::MaskInput::default());
            }
            Session::Lure(night) => {
                night.step(dt_ms, None);
            }
            Session::Bedroom(night) => {
                night.step(dt_ms, bedtime::BedtimeInput::default());
            }
            Session::Story(night) => {
                night.step(dt_ms, facility::Input::default());
            }
            Session::Descent(night) => {
                night.step(dt_ms, false);
            }
            Session::Dayshift(night) => {
                night.step(dt_ms, salvage_run::Input::default());
            }
            Session::Extra(night) => {
                night.step(dt_ms, open_air::Input::default());
            }
            Session::Mixed(night) => {
                night.step(dt_ms);
            }
        }
    }

    /// True once the night has a result (won, caught, or scored).
    pub fn finished(&self) -> bool {
        match self {
            Session::Classic(night) => night.outcome().is_some(),
            Session::Mask(night) => night.outcome().is_some(),
            Session::Lure(night) => night.outcome().is_some(),
            Session::Bedroom(night) => night.score().is_some(),
            Session::Story(night) => night.over.is_some(),
            Session::Descent(night) => night.outcome() != descent::Outcome::Playing,
            Session::Dayshift(night) => night.outcome().is_some(),
            Session::Extra(night) => night.outcome() != open_air::Outcome::Playing,
            Session::Mixed(night) => night.outcome() != mixed_shift::Outcome::Ongoing,
        }
    }
}
