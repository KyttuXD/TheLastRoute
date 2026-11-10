//! Mixed Shift's rotation (port of `mixed_skin.gd`): nine skins, one for each mode's map. Mixed's rules run on one fixed
//! shape (nine camera rooms, two door halls, three vents, the music box); a skin gives that shape another map's names,
//! palette, light, props and minimap labels. `SkinBag` is the rotation: a shuffled bag of all nine, never the same skin
//! twice in a row, refilled when empty. Pure logic.
#![allow(dead_code)]

use crate::core::Mulberry32;

/// The map rooms in camera order (camera n = CAMS[n - 1]).
pub const CAMS: [&str; 9] = ["platform", "concourse", "ticket_hall", "west_arcade", "lost_property", "cafe", "east_arcade", "newsstand", "duct"];

#[derive(Clone, Copy, Debug)]
pub struct Skin {
    pub id: &'static str,
    pub name: &'static str,
    /// The mode whose map this skin dresses the hub as.
    pub mode: &'static str,
    /// The prop kit (world.rs places the props of the kit).
    pub kit: &'static str,
    /// Multiplies floors and walls.
    pub tint: [f32; 3],
    /// Ambient light colour.
    pub light: [f32; 3],
    /// Background colour.
    pub sky: [f32; 3],
    /// Minimap and camera labels, in `CAMS` order.
    pub labels: [&'static str; 9],
    pub theme: &'static str,
}

pub const SKINS: [Skin; 9] = [
    Skin {
        id: "transit",
        name: "TRANSIT HUB",
        mode: "mixed",
        kit: "bench",
        tint: [1.0, 1.0, 1.0],
        light: [0.38, 0.40, 0.55],
        sky: [0.02, 0.02, 0.03],
        labels: ["PLATFORM", "CONCOURSE", "TICKET HALL", "WEST ARCADE", "LOST PROPERTY", "CAFE", "EAST ARCADE", "NEWSSTAND", "VENT RUN"],
        theme: "the station as it is",
    },
    Skin {
        id: "pizzeria",
        name: "THE PIZZERIA",
        mode: "classic",
        kit: "pizzeria",
        tint: [1.3, 1.1, 0.9],
        light: [0.5, 0.38, 0.34],
        sky: [0.03, 0.02, 0.02],
        labels: ["SHOW STAGE", "DINING AREA", "BACKSTAGE", "WEST HALL", "SUPPLY CLOSET", "KITCHEN", "EAST HALL", "CORNER", "VENT RUN"],
        theme: "Classic's pizzeria: checker floor, party tables, a show stage",
    },
    Skin {
        id: "station",
        name: "RADIO STATION",
        mode: "lure",
        kit: "studio",
        tint: [0.9, 1.0, 1.25],
        light: [0.28, 0.42, 0.5],
        sky: [0.01, 0.03, 0.04],
        labels: ["ON-AIR STUDIO", "NEWSROOM", "RECORDS", "WEST CORRIDOR", "TAPE VAULT", "BREAK ROOM", "EAST CORRIDOR", "TRANSMITTER", "CABLE RUN"],
        theme: "Dead Air: mic stands, speaker stacks, a blinking on-air light",
    },
    Skin {
        id: "party",
        name: "PARTY FLOOR",
        mode: "mask",
        kit: "party",
        tint: [1.4, 1.1, 1.5],
        light: [0.48, 0.36, 0.55],
        sky: [0.03, 0.01, 0.04],
        labels: ["PRIZE STAGE", "PARTY ROOM", "ARCADE", "LEFT HALL", "PARTS CLOSET", "KITCHEN", "RIGHT HALL", "GAME AREA", "AIR DUCT"],
        theme: "Masked Hours: balloons, tables, streamers, arcade cabinets",
    },
    Skin {
        id: "house",
        name: "THE HOUSE WING",
        mode: "bedroom",
        kit: "house",
        tint: [1.25, 1.05, 0.85],
        light: [0.42, 0.34, 0.3],
        sky: [0.03, 0.02, 0.01],
        labels: ["NURSERY", "LANDING", "PLAYROOM", "WEST PASSAGE", "LINEN CLOSET", "STUDY", "EAST PASSAGE", "TOY ROOM", "CRAWLSPACE"],
        theme: "Bedtime: beds, wardrobes, toy chests, a night light",
    },
    Skin {
        id: "facility",
        name: "THE FACILITY",
        mode: "story",
        kit: "lab",
        tint: [0.85, 1.15, 1.0],
        light: [0.3, 0.46, 0.4],
        sky: [0.01, 0.03, 0.02],
        labels: ["CONTROL ROOM", "ATRIUM", "ARCHIVE", "WEST CORRIDOR", "SPECIMEN STORE", "STAFF ROOM", "EAST CORRIDOR", "LAB", "SERVICE PIPE"],
        theme: "the Facility: lab benches, consoles, pipe racks, a green status light",
    },
    Skin {
        id: "yard",
        name: "SALVAGE YARD",
        mode: "dayshift",
        kit: "yard",
        tint: [1.3, 1.0, 0.8],
        light: [0.5, 0.4, 0.3],
        sky: [0.04, 0.025, 0.01],
        labels: ["CRANE DECK", "MAIN YARD", "SCRAP HEAP", "PORT ALLEY", "PARTS CAGE", "SITE OFFICE", "STARBOARD ALLEY", "CARGO BAY", "BILGE RUN"],
        theme: "Salvage Run: containers, scrap, barrels, a crane arm",
    },
    Skin {
        id: "shaft",
        name: "THE SHAFT",
        mode: "descent",
        kit: "shaft",
        tint: [0.8, 0.85, 1.0],
        light: [0.26, 0.3, 0.46],
        sky: [0.005, 0.01, 0.03],
        labels: ["LIFT HEAD", "MID LANDING", "MACHINE ROOM", "LEFT GANTRY", "COUNTERWEIGHTS", "CONTROL CAB", "RIGHT GANTRY", "CABLE DECK", "SHAFT WALL"],
        theme: "the Descent: pillars, hanging cables, lift frames",
    },
    Skin {
        id: "camp",
        name: "THE CAMPSITE",
        mode: "extra",
        kit: "camp",
        tint: [0.9, 1.15, 0.85],
        light: [0.28, 0.4, 0.34],
        sky: [0.01, 0.025, 0.02],
        labels: ["BUS STOP", "FIRE CIRCLE", "SUPPLY SHED", "WEST TRAIL", "WOODPILE", "MESS TENT", "EAST TRAIL", "LOOKOUT", "DRAIN PIPE"],
        theme: "Open Air: tents, logs, lanterns",
    },
];

pub fn ids() -> Vec<&'static str> {
    SKINS.iter().map(|s| s.id).collect()
}

pub fn index_of(id: &str) -> usize {
    SKINS.iter().position(|s| s.id == id).unwrap_or(0)
}

pub fn skin(index: usize) -> &'static Skin {
    &SKINS[index.min(SKINS.len() - 1)]
}

impl Skin {
    /// The label of a map room on this skin.
    pub fn label(&self, map_room: &str) -> &'static str {
        match CAMS.iter().position(|c| *c == map_room) {
            Some(i) => self.labels[i],
            None => "",
        }
    }

    /// The camera label for the HUD: "CAM n  LABEL".
    pub fn cam_title(&self, map_room: &str) -> String {
        match CAMS.iter().position(|c| *c == map_room) {
            Some(i) => format!("CAM {}  {}", i + 1, self.labels[i]),
            None => String::new(),
        }
    }
}

/// The rotation: the skins not yet visited, and the one just used. One per game (not process wide), so a seed always
/// gives the same sequence.
#[derive(Clone, Debug, Default)]
pub struct SkinBag {
    bag: Vec<usize>,
    last: Option<usize>,
}

impl SkinBag {
    pub fn new() -> SkinBag {
        SkinBag::default()
    }

    pub fn len(&self) -> usize {
        self.bag.len()
    }

    pub fn reset(&mut self) {
        self.bag.clear();
        self.last = None;
    }

    /// The next skin of the rotation, as an index: from a seeded shuffle of the not-yet-visited skins; a fresh bag never
    /// starts with the skin just used.
    pub fn pick(&mut self, seed_number: u32) -> usize {
        let used = (SKINS.len() - self.bag.len()) as u32;
        let mut rng = Mulberry32::new(seed_number.wrapping_add(17u32.wrapping_mul(used)));
        if self.bag.is_empty() {
            self.bag = (0..SKINS.len()).collect();
            let mut i = self.bag.len() - 1;
            while i > 0 {
                let j = ((rng.next() * (i + 1) as f64).floor() as usize).min(i);
                self.bag.swap(i, j);
                i -= 1;
            }
            if self.bag.len() > 1 && Some(self.bag[0]) == self.last {
                let k = 1 + ((rng.next() * (self.bag.len() - 1) as f64).floor() as usize).min(self.bag.len() - 2);
                self.bag.swap(0, k);
            }
        }
        let next = self.bag.remove(0);
        self.last = Some(next);
        next
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nine_skins_with_nine_labels_each() {
        assert_eq!(SKINS.len(), 9);
        for s in SKINS.iter() {
            assert_eq!(s.labels.len(), 9);
            assert!(s.labels.iter().all(|l| !l.is_empty()), "{}", s.id);
            assert!(s.tint.iter().chain(s.light.iter()).all(|c| *c > 0.0));
        }
        assert_eq!(index_of("pizzeria"), 1);
        assert_eq!(index_of("nope"), 0);
        assert_eq!(SKINS[0].label("platform"), "PLATFORM");
        assert_eq!(SKINS[1].cam_title("platform"), "CAM 1  SHOW STAGE");
        assert_eq!(SKINS[0].cam_title("duct"), "CAM 9  VENT RUN");
        assert_eq!(SKINS[0].cam_title("nowhere"), "");
    }

    #[test]
    fn a_bag_visits_every_skin_before_it_repeats() {
        let mut bag = SkinBag::new();
        let mut seen = Vec::new();
        for _ in 0..9 {
            seen.push(bag.pick(77));
        }
        let mut sorted = seen.clone();
        sorted.sort();
        assert_eq!(sorted, (0..9).collect::<Vec<usize>>());
    }

    #[test]
    fn the_same_skin_never_comes_twice_in_a_row() {
        for seed in 0..40u32 {
            let mut bag = SkinBag::new();
            let mut last = usize::MAX;
            for _ in 0..40 {
                let s = bag.pick(seed);
                assert_ne!(s, last, "seed {seed}");
                last = s;
            }
        }
    }

    #[test]
    fn the_rotation_is_deterministic() {
        let run = |seed| {
            let mut b = SkinBag::new();
            (0..12).map(|_| b.pick(seed)).collect::<Vec<_>>()
        };
        assert_eq!(run(5), run(5));
    }
}
