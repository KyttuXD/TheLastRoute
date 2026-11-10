//! Which painted kinds a space gets: the original's district / room-purpose tables and pick rules, plus the mapping
//! from the finite route's regions and room types onto them.
//!
//! Sources (original, read-only):
//! * district wall set + ceiling: data/themes/regions/<id>.json, merged by RefLook._build_region_style
//! * room-purpose wall set: data/themes/rooms/<purpose>.json (RefLook._build_room_style)
//! * hall floors, hall runner, district room default floor, room-purpose floors: game/look/floor_sets.gd
//!   (+ src/bin/last_route/data/regions/hospital.json and mall.json, which supply those two districts' entries)
//! * pick rules: RefLook.wall_kind / RefLook.pick, FloorSets.room_kind, StreamedFacilityView3D.ref_floor_kind /
//!   _runner_kind and its safe-room pad.
//!
//! IMPORTANT: in the original only the INFINITE route (StreamedFacilityView3D) paints these textures. The finite route's
//! FacilityView3D uses two baked PNG tiles (assets/environment/floor_texture.png / wall_texture.png) tinted per region
//! (REGION_FLOOR_TINTS / REGION_WALL_TINTS) and only for custom maps. So `region_set` maps each finite region / room
//! type onto the infinite route's district / purpose that matches it (FINITE_DISTRICT, FINITE_PURPOSE below) and then
//! applies the original's rules unchanged. That mapping is this port's choice; everything after it is the original's.

use super::variant_of;

/// One painted surface: `paint(group, kind, variant)` paints it (kind is the full key, variant its painter variant).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TexRef {
    pub group: &'static str,
    pub kind: &'static str,
    pub variant: i32,
}

impl TexRef {
    pub fn new(group: &'static str, kind: &'static str) -> TexRef {
        TexRef {
            group,
            kind,
            variant: variant_of(group, kind).unwrap_or(0),
        }
    }

    pub fn paint(&self) -> Option<(u16, u16, Vec<u8>)> {
        super::paint(self.group, self.kind, self.variant)
    }
}

/// The surfaces of one space (a room of some type in some region, or a hall of a region).
#[derive(Clone, Debug)]
pub struct TextureChoice {
    /// The original's district id the space was mapped to ("maze", "mansion", "carnival", ...).
    pub district: &'static str,
    /// The original's room purpose id, if the room type maps to one ("dining_room", "arcade_room", ...).
    pub purpose: Option<&'static str>,
    /// First entry of each list (what a room / hall gets when the caller does not vary it).
    pub wall: TexRef,
    pub floor: TexRef,
    pub ceiling: Option<TexRef>,
    /// The full lists the original picks from (`wall_at` / `floor_at` apply the original's pick).
    pub wall_kinds: &'static [&'static str],
    pub floor_kinds: &'static [&'static str],
    /// Inferno: the original varies the floor (cave rock) and the wall kind per cell, not per room / 4-cell patch.
    pub per_cell: bool,
    /// Hall only: the district lays the three-strip carpet runner down the middle of 3-wide halls (`runner_kind`).
    pub runner: bool,
    pub is_hall: bool,
}

impl TextureChoice {
    /// RefLook.wall_kind: the wall kind of a face at world cell (gx, gz). Picked per 4 x 4 cell patch (salt 2) so a
    /// wall changes paper in runs; per cell in the inferno.
    pub fn wall_at(&self, gx: i64, gz: i64) -> TexRef {
        let n = self.wall_kinds.len();
        let i = if self.per_cell {
            pick(gx, gz, 2, n)
        } else {
            pick(gx / 4, gz / 4, 2, n)
        };
        TexRef::new("wall", self.wall_kinds[i])
    }

    /// The floor kind. Rooms: (x, z) = the room rectangle's origin cell (FloorSets.room_kind, salt 7: one kind per
    /// room, never a patchwork). Halls: (x, z) = the chunk coordinate in the original (salt 1: one kind per 45-cell
    /// chunk); the finite route has no chunks, so pass any stable per-floor value. Inferno: (x, z) = the cell (salt 7).
    pub fn floor_at(&self, x: i64, z: i64) -> TexRef {
        let n = self.floor_kinds.len();
        let salt = if self.is_hall && !self.per_cell { 1 } else { 7 };
        TexRef::new("floor", self.floor_kinds[pick(x, z, salt, n)])
    }
}

/// RefLook.pick: deterministic 0..count-1 from a cell and a salt (GDScript int64 arithmetic, wrapping).
pub fn pick(cell_x: i64, cell_z: i64, salt: i64, count: usize) -> usize {
    let count = count.max(1) as i64;
    let mut h =
        cell_x.wrapping_mul(73856093) ^ cell_z.wrapping_mul(19349663) ^ salt.wrapping_mul(83492791);
    h = (h ^ (h >> 13)).wrapping_mul(1274126177);
    let r = (h ^ (h >> 16)).wrapping_abs() % count;
    // only i64::MIN stays negative; GDScript would then index from the end
    (if r < 0 { r + count } else { r }) as usize
}

// -- the original's tables -------------------------------------------------------------------------------------------

pub struct District {
    pub id: &'static str,
    /// data/themes/regions/<id>.json "wall"
    pub wall: &'static [&'static str],
    /// data/themes/regions/<id>.json "ceiling"
    pub ceiling: &'static str,
    /// FloorSets.HALLS floor / runner
    pub hall_floor: &'static [&'static str],
    pub hall_runner: bool,
    /// FloorSets.ROOM_DEFAULT: rooms whose purpose has no floor entry
    pub room_default: &'static [&'static str],
}

/// RefLook.REGION_STYLE (code-table key order). An id not here (e.g. "office", "industrial") falls back to "maze".
pub const DISTRICTS: &[District] = &[
    District {
        id: "maze",
        wall: &["panel0", "panel1", "panel2"],
        ceiling: "ceil0",
        hall_floor: &["concrete0", "concrete1"],
        hall_runner: false,
        room_default: &["carpet_tile0", "carpet_tile1"],
    },
    District {
        id: "mansion",
        wall: &["paper0", "paper1"],
        ceiling: "ceil1",
        hall_floor: &["parquet0", "parquet1"],
        hall_runner: true,
        room_default: &["parquet0", "parquet1"],
    },
    District {
        id: "hotel",
        wall: &["paper1", "paper0"],
        ceiling: "ceil1",
        hall_floor: &["carpet_teal"],
        hall_runner: false,
        room_default: &["floral0", "floral1"],
    },
    District {
        id: "apartment",
        wall: &["paper0", "office1"],
        ceiling: "ceil1",
        hall_floor: &["parquet0", "parquet1"],
        hall_runner: true,
        room_default: &["parquet1", "floral1"],
    },
    District {
        id: "beach",
        wall: &["seawall0", "whitewash0", "tealpanel0", "seawall1"],
        ceiling: "skylight0",
        hall_floor: &["sand0", "sand1", "wetsand0"],
        hall_runner: false,
        room_default: &["sand0", "sand1"],
    },
    District {
        id: "gothic",
        wall: &["brick0", "brick1", "stone1"],
        ceiling: "ceil1",
        hall_floor: &["stone0", "stone1"],
        hall_runner: true,
        room_default: &["carpet_red"],
    },
    District {
        id: "carnival",
        wall: &["paper1", "panel1"],
        ceiling: "ceil1",
        hall_floor: &["check0", "check1"],
        hall_runner: false,
        room_default: &["star0", "star1"],
    },
    District {
        id: "prison",
        wall: &["corridor0", "corridor1", "panel0"],
        ceiling: "ceil0",
        hall_floor: &["concrete0", "concrete1"],
        hall_runner: false,
        room_default: &["concrete0", "concrete1"],
    },
    District {
        id: "inferno",
        wall: &[
            "rockwall0",
            "rockwall1",
            "rockwall2",
            "rockwall3",
            "rockwall4",
            "rockwall5",
            "rockwall6",
            "rockwall7",
        ],
        ceiling: "rockceil",
        hall_floor: &["hazard", "metal3"],
        hall_runner: false,
        room_default: &["metal3", "stone1"],
    },
    District {
        id: "sewer",
        wall: &["brick0", "brick1", "brick1"],
        ceiling: "vault",
        hall_floor: &["stone0", "stone1"],
        hall_runner: false,
        room_default: &["concrete0", "concrete1"],
    },
    District {
        id: "hospital",
        wall: &["hospital0", "hospital1"],
        ceiling: "ceil0",
        hall_floor: &["vinyl0", "vinyl1"],
        hall_runner: false,
        room_default: &["vinyl0", "vinyl1"],
    },
    District {
        id: "lab",
        wall: &["lab0", "lab0", "lab1"],
        ceiling: "ceil0",
        hall_floor: &["epoxy0", "epoxy0", "epoxy1"],
        hall_runner: false,
        room_default: &["epoxy0"],
    },
    District {
        id: "mall",
        wall: &["mall0", "mall1"],
        ceiling: "ceil1",
        hall_floor: &["terrazzo0", "terrazzo1"],
        hall_runner: false,
        room_default: &["terrazzo0"],
    },
    District {
        id: "greenhouse",
        wall: &["glasshouse0", "glasshouse1"],
        ceiling: "glass",
        hall_floor: &["terracotta0", "terracotta1", "stone2"],
        hall_runner: false,
        room_default: &["terracotta0", "terracotta1"],
    },
];

/// The inferno's drawn floor: StreamedFacilityView3D paints every inferno cell (rooms too) with cave rock,
/// "rock%d" % posmod(pick(gx, gz, 7, 3), 3); only the chain bridge / ramp walkways are "hazard".
pub const INFERNO_ROCK: &[&str] = &["rock0", "rock1", "rock2"];

pub fn district(id: &str) -> Option<&'static District> {
    DISTRICTS.iter().find(|d| d.id == id)
}

/// REGION_STYLE.get(region, REGION_STYLE["maze"]).
pub fn district_or_maze(id: &str) -> &'static District {
    district(id).unwrap_or(&DISTRICTS[0])
}

/// data/themes/rooms/<purpose>.json: the purpose's own wall set (overrides the district's).
pub const ROOM_WALLS: &[(&str, &[&str])] = &[
    ("arcade_room", &["panel1", "paper1"]),
    ("backstage", &["panel2", "panel0"]),
    ("beach_hut", &["whitewash0", "whitewash1"]),
    ("bedroom", &["paper0", "paper1"]),
    ("big_top", &["paper1"]),
    ("boardwalk_room", &["whitewash1", "tealpanel1"]),
    ("cactus_room", &["glasshouse0"]),
    ("chapel", &["stone0", "brick0"]),
    ("cold_storage", &["lab0"]),
    ("containment", &["lab1"]),
    ("cove", &["seawall0", "seawall1"]),
    ("crypt", &["brick1", "stone1"]),
    ("cubicle_room", &["office1", "office0"]),
    ("decon", &["lab1"]),
    ("dining_room", &["paper1", "paper0"]),
    ("drawing_room", &["paper0", "paper1"]),
    ("family_dining", &["paper0", "office1"]),
    ("forge", &["inferno0", "panel0"]),
    ("guest_room", &["paper1", "paper0"]),
    ("hotel_bathroom", &["office0"]),
    ("house_bathroom", &["office0"]),
    ("irrigation_room", &["glasshouse0", "panel2"]),
    ("kitchen", &["office0", "corridor0"]),
    ("lab_office", &["office0", "lab0"]),
    ("living_room", &["paper0", "office1"]),
    ("luxury_bathroom", &["paper0"]),
    ("morgue", &["corridor0", "hospital0"]),
    ("office_job", &["office0", "office1"]),
    ("office_reception", &["office0", "office1"]),
    ("office_suite", &["office0"]),
    ("operating_theatre", &["hospital0"]),
    ("overgrown_room", &["glasshouse1", "hedge1"]),
    ("potting_shed", &["glasshouse0"]),
    ("prison_cafeteria", &["corridor0", "office0"]),
    ("prison_cell", &["corridor1", "panel0"]),
    ("private_office", &["paper0", "office0"]),
    ("public_restroom", &["corridor0", "office0"]),
    ("restaurant_dining", &["paper0", "paper1"]),
    ("server_hall", &["lab0", "panel2"]),
    ("stairwell", &["paper0"]),
    ("storage", &["panel2", "corridor1"]),
    ("supply_closet", &["panel0", "hospital0"]),
    ("thermal_chamber", &["inferno0", "panel2"]),
    ("tide_pool", &["seawall1", "tealpanel0"]),
    ("waiting_room", &["hospital1"]),
];

/// FloorSets.ROOMS: room purpose -> floor kinds.
pub const ROOM_FLOORS: &[(&str, &[&str])] = &[
    ("office_reception", &["carpet_tile0", "carpet_tile1"]),
    ("cubicle_room", &["carpet_tile1", "carpet_tile0"]),
    ("office_job", &["carpet_tile0"]),
    ("office_suite", &["carpet_tile0", "carpet_tile1"]),
    ("private_office", &["carpet_plum"]),
    ("storage", &["concrete1", "hazard"]),
    ("kitchen", &["check0", "check1"]),
    ("restaurant_dining", &["check0", "parquet0"]),
    ("prison_cafeteria", &["check0", "check1"]),
    ("public_restroom", &["vinyl0", "vinyl1"]),
    ("hotel_bathroom", &["check0", "check1"]),
    ("house_bathroom", &["check1", "vinyl0"]),
    ("luxury_bathroom", &["check0"]),
    ("bedroom", &["floral0", "parquet1"]),
    ("guest_room", &["floral0", "floral1"]),
    ("living_room", &["floral1", "parquet0"]),
    ("apartment_suite", &["parquet1", "floral1"]),
    ("family_dining", &["floral1", "parquet0"]),
    ("stairwell", &["floral0"]),
    ("dining_room", &["parquet0", "parquet1"]),
    ("drawing_room", &["carpet_red"]),
    ("archive", &["carpet_green"]),
    ("mourning_room", &["carpet_plum"]),
    ("chapel", &["stone1", "stone0"]),
    ("crypt", &["stone1"]),
    ("arcade_room", &["star0", "star1"]),
    ("big_top", &["star0", "star1"]),
    ("midway", &["check0", "check1"]),
    ("game_stall", &["star1", "check1"]),
    ("backstage", &["concrete1", "grate"]),
    ("cove", &["sand1", "sand0"]),
    ("tide_pool", &["wetsand0"]),
    ("beach_hut", &["driftwood0"]),
    ("boardwalk_room", &["driftwood0", "driftwood1"]),
    ("prison_cell", &["concrete0"]),
    ("infirmary", &["vinyl0", "vinyl1"]),
    ("thermal_chamber", &["hazard", "metal3"]),
    ("forge", &["hazard", "metal3"]),
    ("altar_room", &["stone1", "metal3"]),
    ("lava_gallery", &["metal3", "hazard"]),
    ("pump_room", &["concrete0", "hazard"]),
    ("valve_room", &["concrete1", "concrete0"]),
    ("overflow_chamber", &["stone1", "concrete1"]),
    ("ward", &["vinyl0", "vinyl1"]),
    ("nurses_station", &["vinyl1"]),
    ("waiting_room", &["vinyl1", "carpet_tile0"]),
    ("operating_theatre", &["check1", "vinyl0"]),
    ("icu", &["vinyl0"]),
    ("morgue", &["concrete1", "check1"]),
    ("pharmacy", &["vinyl1"]),
    ("imaging", &["vinyl0"]),
    ("supply_closet", &["concrete1"]),
    ("clean_room", &["epoxy0"]),
    ("containment", &["epoxy1", "hazard"]),
    ("server_hall", &["grate", "epoxy0"]),
    ("control_room", &["carpet_tile1", "epoxy0"]),
    ("decon", &["grate", "epoxy1"]),
    ("chem_lab", &["epoxy0", "epoxy1"]),
    ("bio_lab", &["epoxy0"]),
    ("pod_room", &["epoxy0"]),
    ("music_stage", &["parquet0", "parquet1"]),
    ("practice_room", &["carpet_tile0", "carpet_tile1"]),
    ("instrument_storage", &["concrete1"]),
    ("recording_booth", &["carpet_tile1"]),
    ("music_store", &["parquet1", "parquet0"]),
    ("carnival_stage", &["parquet0", "parquet1"]),
    ("cold_storage", &["metal1", "epoxy0"]),
    ("lab_office", &["carpet_tile0", "carpet_tile1"]),
    ("glasshouse", &["terracotta0", "terracotta1"]),
    ("palm_house", &["terracotta1", "stone2"]),
    ("cactus_room", &["stone2"]),
    ("potting_shed", &["parquet1", "terracotta1"]),
    ("irrigation_room", &["grate", "terracotta0"]),
    ("overgrown_room", &["stone2", "terracotta0"]),
    ("dome_atrium", &["terracotta0", "terracotta1"]),
    ("donut_shop", &["check0", "check1"]),
    ("cinema", &["carpet_red"]),
    ("corner_store", &["vinyl1"]),
    ("food_court", &["terrazzo1", "check0"]),
    ("bakery", &["check1", "parquet1"]),
    ("laundromat", &["vinyl0", "check1"]),
    ("electronics", &["carpet_tile1"]),
    ("night_diner", &["check0"]),
    ("drugstore", &["vinyl0"]),
    ("florist", &["terracotta1", "parquet1"]),
    ("barber", &["check1"]),
    ("pawn_shop", &["parquet0"]),
    ("stationery", &["carpet_green", "parquet1"]),
];

fn lookup(
    table: &'static [(&'static str, &'static [&'static str])],
    id: &str,
) -> Option<&'static [&'static str]> {
    table.iter().find(|e| e.0 == id).map(|e| e.1)
}

/// The wall set RefLook.wall_kind picks from: the purpose's own walls if it has any, else the district's.
pub fn wall_kinds(district_id: &str, purpose: Option<&str>) -> &'static [&'static str] {
    purpose
        .and_then(|p| lookup(ROOM_WALLS, p))
        .unwrap_or(district_or_maze(district_id).wall)
}

/// FloorSets.room: the purpose's floors, else the district default.
pub fn room_floor_kinds(district_id: &str, purpose: Option<&str>) -> &'static [&'static str] {
    purpose
        .and_then(|p| lookup(ROOM_FLOORS, p))
        .unwrap_or(district_or_maze(district_id).room_default)
}

// -- finite route -> original district / purpose ----------------------------------------------------------------------

/// Finite-route region (RoomCatalog "region"; "" = halls and the lobby) -> the infinite route's district.
///
/// DECIDED (lr_story, 2026-11-05): "office" maps to "maze" (the office/maze set, which uses the office props in
/// props/maze), and "lab" maps to "lab" (the lab set). This is deliberate. last_route's `world/mod.rs` `region_texture`
/// maps "office" to "lab" (line 33); lr_story does not follow that, so the two games disagree on purpose here.
pub const FINITE_DISTRICT: &[(&str, &str)] = &[
    ("", "maze"), // the facility proper: bare concrete halls, riveted panel walls (maze = industrial)
    ("arcade", "carnival"), // stage / arcade / prize gallery: star carpet, checker halls, damask + brass panels
    ("circus", "carnival"),
    ("maintenance", "prison"), // service bays: hazard-striped corridor walls, concrete
    ("office", "maze"), // the original's "office" theme is accent-only, so offices live in the maze style
    ("mansion", "mansion"),
    ("bedroom", "mansion"),
    ("nether", "inferno"),
    ("beach", "beach"),
    ("garden", "greenhouse"),
    ("prison", "prison"),
    ("hospital", "hospital"),
    ("lab", "lab"),
    ("greenhouse", "greenhouse"),
    ("mall", "mall"),
];

/// Finite-route room type id -> (the original's room purpose or None for the district default, district override).
pub const FINITE_PURPOSE: &[(&str, Option<&str>, Option<&str>)] = &[
    ("lobby", None, None),
    ("stage", Some("carnival_stage"), None),
    ("arcade", Some("arcade_room"), None),
    ("prizeGallery", Some("game_stall"), None),
    ("arcadeHallway", Some("midway"), None),
    ("serviceBay", Some("backstage"), None),
    ("relayControl", Some("control_room"), None),
    ("grandStairHall", Some("stairwell"), None),
    ("diningRoom", Some("dining_room"), None),
    ("kitchen", Some("kitchen"), None),
    ("luxuryKitchen", Some("kitchen"), None),
    ("gothicKitchen", Some("kitchen"), Some("gothic")),
    ("bathroomMale", Some("luxury_bathroom"), None),
    ("bathroomFemale", Some("luxury_bathroom"), None),
    ("publicShowersMale", Some("public_restroom"), None),
    ("publicShowersFemale", Some("public_restroom"), None),
    ("houseBathroom", Some("house_bathroom"), None),
    ("netherChamber", Some("thermal_chamber"), None),
    ("prisonCellBlock", Some("prison_cell"), None),
    ("prisonCafeteria", Some("prison_cafeteria"), None),
    ("beachCove", Some("cove"), None),
    ("cargoDepot", Some("storage"), None),
    ("ballroom", None, None), // mansion default: parquet under damask
    ("garden", Some("overgrown_room"), None), // ivy hedge + glazing, glass roof
    ("library", Some("archive"), None), // library-green carpet
    ("circusTent", Some("big_top"), None),
    ("masterBedroom", Some("bedroom"), None),
    ("gothicBedroom", Some("mourning_room"), Some("gothic")),
    ("bedroom", Some("bedroom"), None),
    ("office", Some("cubicle_room"), None),
    ("maintenance", None, None), // prison default: concrete, corridor / panel walls
    ("storage", Some("storage"), None),
    ("server", Some("server_hall"), None),
    ("breakroom", Some("family_dining"), None),
    ("lounge", Some("living_room"), None),
    ("hospitalWard", Some("ward"), None),
    ("operatingTheatre", Some("operating_theatre"), None),
    ("hospitalMorgue", Some("morgue"), None),
    ("hospitalPharmacy", Some("pharmacy"), None),
    ("hospitalImaging", Some("imaging"), None),
    ("labContainment", Some("containment"), None),
    ("labServerHall", Some("server_hall"), None),
    ("labControlRoom", Some("control_room"), None),
    ("labCleanRoom", Some("clean_room"), None),
    ("labChemistry", Some("chem_lab"), None),
    ("domeAtrium", Some("dome_atrium"), None),
    ("palmHouse", Some("palm_house"), None),
    ("cactusHouse", Some("cactus_room"), None),
    ("pottingShed", Some("potting_shed"), None),
    ("donutShop", Some("donut_shop"), None),
    ("mallCinema", Some("cinema"), None),
    ("foodCourt", Some("food_court"), None),
    ("laundromat", Some("laundromat"), None),
];

fn intern_purpose(id: &str) -> Option<&'static str> {
    ROOM_FLOORS
        .iter()
        .map(|e| e.0)
        .chain(ROOM_WALLS.iter().map(|e| e.0))
        .find(|p| *p == id)
}

/// The district a finite region maps to (an original district id passes through; unknown -> "maze").
pub fn finite_district(region: &str) -> &'static str {
    if let Some(e) = FINITE_DISTRICT.iter().find(|e| e.0 == region) {
        return e.1;
    }
    district(region).map(|d| d.id).unwrap_or("maze")
}

/// (purpose, district override) of a finite room type (an original purpose id passes through).
pub fn finite_purpose(room_type: &str) -> (Option<&'static str>, Option<&'static str>) {
    if let Some(e) = FINITE_PURPOSE.iter().find(|e| e.0 == room_type) {
        return (e.1, e.2);
    }
    (intern_purpose(room_type), None)
}

/// The floor kind names of the floor painter table, so a data name becomes the table's own `&'static str`.
fn intern_floor(name: &str) -> Option<&'static str> {
    super::FLOOR_PAINTERS.iter().map(|e| e.0).find(|k| *k == name)
}

/// A maze variant's district: the plain maze's walls and ceiling, the variant's floors. Names the painter table
/// lacks are dropped; a list left empty falls back to the maze's own.
pub fn district_of_variant(
    maze: &District,
    hall_floor: &[String],
    hall_runner: bool,
    room_default: &[String],
) -> District {
    let list = |names: &[String], fallback: &'static [&'static str]| -> &'static [&'static str] {
        let kinds: Vec<&'static str> = names.iter().filter_map(|n| intern_floor(n)).collect();
        if kinds.is_empty() {
            fallback
        } else {
            Box::leak(kinds.into_boxed_slice())
        }
    };
    District {
        id: "maze",
        wall: maze.wall,
        ceiling: maze.ceiling,
        hall_floor: list(hall_floor, maze.hall_floor),
        hall_runner,
        room_default: list(room_default, maze.room_default),
    }
}

/// The floor data of one level 0 maze variant (the fields of `src/bin/last_route/data/regions/maze_<name>.json` that the painter needs).
/// Copied by value from last_route's `world::route::maze_regions` (which parses the embedded JSON). Only the four
/// built-in variants are listed; if a file under `src/bin/last_route/data/regions/maze_*.json` changes, change its row here too.
pub struct MazeVariantFloors {
    pub id: &'static str,
    pub hall_floor: &'static [&'static str],
    pub hall_runner: bool,
    pub room_default_floor: &'static [&'static str],
}

/// maze_boiler, maze_courtyard, maze_library, maze_warehouse, in id order (as `maze_regions::variants()` sorts them).
pub const MAZE_VARIANTS: &[MazeVariantFloors] = &[
    MazeVariantFloors {
        id: "maze_boiler",
        hall_floor: &["concrete0", "epoxy0"],
        hall_runner: false,
        room_default_floor: &["epoxy0", "concrete0"],
    },
    MazeVariantFloors {
        id: "maze_courtyard",
        hall_floor: &["stone0", "concrete0"],
        hall_runner: false,
        room_default_floor: &["terracotta0", "stone0"],
    },
    MazeVariantFloors {
        id: "maze_library",
        hall_floor: &["parquet0", "carpet_tile0"],
        hall_runner: true,
        room_default_floor: &["parquet0", "carpet_tile0"],
    },
    MazeVariantFloors {
        id: "maze_warehouse",
        hall_floor: &["concrete0", "epoxy0"],
        hall_runner: false,
        room_default_floor: &["concrete0", "epoxy0"],
    },
];

/// The maze variants as districts, built once from `MAZE_VARIANTS`.
fn variant_districts() -> &'static [(String, District)] {
    use std::sync::OnceLock;
    static CELL: OnceLock<Vec<(String, District)>> = OnceLock::new();
    CELL.get_or_init(|| {
        let maze = district_or_maze("maze");
        MAZE_VARIANTS
            .iter()
            .map(|v| {
                let hall: Vec<String> = v.hall_floor.iter().map(|s| s.to_string()).collect();
                let room: Vec<String> = v.room_default_floor.iter().map(|s| s.to_string()).collect();
                (v.id.to_string(), district_of_variant(maze, &hall, v.hall_runner, &room))
            })
            .collect()
    })
}

/// The district of a maze variant named by `region` (its id, with or without a `maze_` prefix), if there is one.
pub fn variant_district(region: &str) -> Option<&'static District> {
    let id = region.strip_prefix("maze_").unwrap_or(region);
    variant_districts().iter().find(|(v, _)| v.strip_prefix("maze_").unwrap_or(v) == id).map(|(_, d)| d)
}

/// The textures of a finite-route space. `region` is the room's RoomCatalog region ("" for halls / the lobby),
/// `room_type` its type id ("" for a hall), `is_hall` true for corridors and junctions (doorway cells take the hall's
/// floor in the original: the room floor starts inside the door line).
pub fn region_set(region: &str, room_type: &str, is_hall: bool) -> TextureChoice {
    let (purpose, override_district) = if is_hall {
        (None, None)
    } else {
        finite_purpose(room_type)
    };
    let d = district_or_maze(override_district.unwrap_or_else(|| finite_district(region)));
    // a level 0 maze variant (src/bin/last_route/data/regions/maze_*.json) brings its own floors; walls and ceiling stay the maze's
    let variant = variant_district(region).filter(|_| override_district.is_none());
    let inferno = d.id == "inferno";
    let wall_kinds = if is_hall {
        d.wall
    } else {
        wall_kinds(d.id, purpose)
    };
    let floor_kinds = if inferno {
        INFERNO_ROCK
    } else if is_hall {
        variant.map_or(d.hall_floor, |v| v.hall_floor)
    } else {
        match (variant, purpose.and_then(|p| lookup(ROOM_FLOORS, p))) {
            (Some(v), None) => v.room_default,
            _ => room_floor_kinds(d.id, purpose),
        }
    };
    TextureChoice {
        district: d.id,
        purpose,
        wall: TexRef::new("wall", wall_kinds[0]),
        floor: TexRef::new("floor", floor_kinds[0]),
        ceiling: Some(TexRef::new("ceiling", d.ceiling)),
        wall_kinds,
        floor_kinds,
        per_cell: inferno,
        runner: is_hall && variant.map_or(d.hall_runner, |v| v.hall_runner),
        is_hall,
    }
}

// -- hall runner and safe-room pad (StreamedFacilityView3D) ----------------------------------------------------------

/// StreamedFacilityView3D._runner_kind, for a district with `runner`: the runner strip kind of open cell (x, z), or
/// None for the hall's ordinary floor. `open(x, z)` = walkable cell; `width` = the hall width the runner is for
/// (StreamedRouteGenerator.CORRIDOR_WIDTH = 3). A hall running along x (cells across it vary in z) gets runner_l at
/// the -z edge, runner_c in the middle, runner_r at +z; a hall along z gets runnerz_l / _c / _r (l at -x).
/// A junction cell next to an exactly-one-hall-wide cell carries "runner_c" so the carpet runs through openings.
pub fn runner_kind(
    open: impl Fn(i32, i32) -> bool,
    x: i32,
    z: i32,
    width: i32,
) -> Option<&'static str> {
    let mut runs = [0, 0];
    for axis in 0..2 {
        let (sx, sz) = if axis == 0 { (0, 1) } else { (1, 0) };
        let mut before = 0;
        while before < 6 && open(x - sx * (before + 1), z - sz * (before + 1)) {
            before += 1;
        }
        let mut after = 0;
        while after < 6 && open(x + sx * (after + 1), z + sz * (after + 1)) {
            after += 1;
        }
        runs[axis] = before + after + 1;
        if before + after + 1 != width {
            continue;
        }
        let offset = before - width / 2;
        if offset.abs() > 1 {
            return None;
        }
        return Some(match (axis, offset.signum()) {
            (0, -1) => "runner_l",
            (0, 0) => "runner_c",
            (0, _) => "runner_r",
            (_, -1) => "runnerz_l",
            (_, 0) => "runnerz_c",
            _ => "runnerz_r",
        });
    }
    if runs[0] >= width && runs[1] >= width {
        for (sx, sz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            for reach in [1, 2] {
                let (nx, nz) = (x + sx * reach, z + sz * reach);
                if !open(nx, nz) {
                    break;
                }
                if exact_hall_cell(&open, nx, nz, width) {
                    return Some("runner_c");
                }
            }
        }
    }
    None
}

/// StreamedFacilityView3D._exact_hall_cell: the open cell lies in a run of exactly `width` along either axis.
fn exact_hall_cell(open: &impl Fn(i32, i32) -> bool, x: i32, z: i32, width: i32) -> bool {
    for axis in 0..2 {
        let (dx, dz) = if axis == 0 { (0, 1) } else { (1, 0) };
        let mut run = 1;
        for sign in [-1, 1] {
            let mut k = 1;
            while k < 8 && open(x + dx * k * sign, z + dz * k * sign) {
                run += 1;
                k += 1;
            }
        }
        if run == width {
            return true;
        }
    }
    false
}

/// The safe-room pad (StreamedFacilityView3D: a chunk with safe_room): by Chebyshev distance from the pad's centre
/// cell, 0 -> "safe_mark" (teal cross), 1..=2 -> "safe_plain", 3 -> "safe" (the ring with the inlay line), farther ->
/// None (the space's own floor). A 7 x 7 pad; it overrides the room / hall floor of open cells only.
pub fn safe_pad_kind(dx: i32, dz: i32) -> Option<&'static str> {
    match dx.abs().max(dz.abs()) {
        0 => Some("safe_mark"),
        1 | 2 => Some("safe_plain"),
        3 => Some("safe"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_maze_variant_district_takes_its_data_floors_and_the_maze_walls() {
        let maze = district_or_maze("maze");
        let names = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let d = district_of_variant(maze, &names(&["terracotta0", "no_such_floor", "stone0"]), true, &names(&["parquet0"]));
        assert_eq!(d.hall_floor, &["terracotta0", "stone0"], "unknown names are dropped, known ones kept in order");
        assert_eq!(d.room_default, &["parquet0"]);
        assert!(d.hall_runner);
        assert_eq!(d.wall, maze.wall);
        assert_eq!(d.ceiling, maze.ceiling);
        // a list with no known name falls back to the maze's
        let d = district_of_variant(maze, &names(&["nope"]), false, &names(&[]));
        assert_eq!(d.hall_floor, maze.hall_floor);
        assert_eq!(d.room_default, maze.room_default);
    }

    #[test]
    fn every_maze_variant_resolves_to_painted_floors() {
        assert_eq!(MAZE_VARIANTS.len(), 4);
        for v in MAZE_VARIANTS {
            let d = variant_district(v.id).expect("variant has a district");
            assert!(!d.hall_floor.is_empty() && !d.room_default.is_empty());
            for k in d.hall_floor.iter().chain(d.room_default) {
                assert!(variant_of("floor", k).is_some(), "{} uses unpainted floor {k}", v.id);
            }
            let hall = region_set(v.id, "", true);
            assert_eq!(hall.floor_kinds, d.hall_floor);
        }
    }

    #[test]
    fn every_kind_is_painted() {
        for d in DISTRICTS {
            for k in d.wall {
                assert!(variant_of("wall", k).is_some(), "{k}");
            }
            assert!(variant_of("ceiling", d.ceiling).is_some());
            for k in d.hall_floor.iter().chain(d.room_default) {
                assert!(variant_of("floor", k).is_some(), "{k}");
            }
        }
        for (_, ks) in ROOM_WALLS {
            for k in *ks {
                assert!(variant_of("wall", k).is_some(), "{k}");
            }
        }
        for (_, ks) in ROOM_FLOORS {
            for k in *ks {
                assert!(variant_of("floor", k).is_some(), "{k}");
            }
        }
        for (_, p, _) in FINITE_PURPOSE {
            if let Some(p) = p {
                assert!(
                    lookup(ROOM_FLOORS, p).is_some() || lookup(ROOM_WALLS, p).is_some(),
                    "{p}"
                );
            }
        }
    }

    #[test]
    fn examples() {
        let hall = region_set("", "", true);
        assert_eq!(
            (hall.wall.kind, hall.floor.kind, hall.ceiling.unwrap().kind),
            ("panel0", "concrete0", "ceil0")
        );
        let dining = region_set("mansion", "diningRoom", false);
        assert_eq!(
            (dining.wall.kind, dining.floor.kind),
            ("paper1", "parquet0")
        );
        let mansion_hall = region_set("mansion", "", true);
        assert!(mansion_hall.runner);
        let nether = region_set("nether", "netherChamber", false);
        assert_eq!(
            (
                nether.wall.kind,
                nether.floor.kind,
                nether.ceiling.unwrap().kind
            ),
            ("inferno0", "rock0", "rockceil")
        );
        assert_eq!(safe_pad_kind(0, 0), Some("safe_mark"));
        assert_eq!(safe_pad_kind(-3, 2), Some("safe"));
        // a 3-wide hall along x: rows z = 0..2 open
        let open = |x: i32, z: i32| (0..3).contains(&z) && (-20..20).contains(&x);
        assert_eq!(runner_kind(open, 0, 0, 3), Some("runner_l"));
        assert_eq!(runner_kind(open, 0, 1, 3), Some("runner_c"));
        assert_eq!(runner_kind(open, 0, 2, 3), Some("runner_r"));
    }
}
