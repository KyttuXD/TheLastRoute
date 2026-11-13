# RoomCatalog / FacilityGenerator porting notes

Generated from the read-only Godot project:
- `game/config/room_catalog.gd` (all room/prop data, pure `const`s)
- `game/config/prop_model_catalog.gd` (`HUNTER_SCALE`, used only by `surface_height`)
- `game/world/generation/facility_generator.gd` (verbatim excerpts below)

`catalog.json` sits next to this file. The extractor that produced it parses the GDScript consts directly, so nothing in it was typed by hand.

## 0. Answers to the "does anything else feed RoomCatalog" questions

- **ContentRegistry is not involved.** `room_catalog.gd` never references ContentRegistry, PropColliderSizes, RoomSizeProfiles or PropModelCatalog footprints. It holds only `const` literals. Godot 4 makes const arrays and dictionaries read-only, so nothing can change them at runtime. ContentRegistry's `"rooms"` pack kind feeds `room_recipes.gd`, which only the *streamed* generators use, and `facility_generator.gd` never calls it. `data/rooms/music_store.json` in the Godot project likewise has nothing to do with the finite generator. With or without mod packs, `TYPES` is exactly what `catalog.json.types` lists, in the same order.
- The only cross-file dependency is `surface_height()`, which multiplies by `PropModelCatalog.hunter_scale(parent)` (`HUNTER_SCALE.get(type, 1.0)`). `catalog.json.surface_heights[*].value` already contains that product.
- `room_size_profiles.gd`, `prop_collider_sizes.gd` and `prop_collider_boxes.gd` are **not** read by the finite generator. Footprints come only from `RoomCatalog.FOOTPRINTS` (default `Vector2(.32, .32)`).
- 52 room types, and 250 distinct prop types appear in the props lists. Only one prop type, `bunkBed` (masterBedroom, bedroom), is missing from `FOOTPRINTS`, so it uses the default (0.32, 0.32).
- Procedural (`streamed_only == false`) types: 32. Everything marked `streamed_only` is skipped by `_room_types_for_count`.
- Only `grandStairHall` has a walkable structure (`grandFoyerStaircase`).

### `_room_types_for_count` facts worth knowing
- `special` defaults to 0 when the key is missing. `large_only`, `streamed_only` and `landmark` default to false. `catalog.json` normalises these and also lists `explicit_keys`.
- `large_only` only filters **specials**. An ordinary type with `large_only: true` (luxuryKitchen, gothicKitchen) still goes into `ordinary` regardless of `cols`.
- Specials in TYPES order (special value, L = large_only so only when cols >= 61): stage 1, arcade 2, serviceBay 5, prizeGallery 6, arcadeHallway 8, relayControl 10, grandStairHall 9, diningRoom 10, bathroomMale 11, bathroomFemale 12, netherChamber 13 L, beachCove 13 L, cargoDepot 14 L, ballroom 15 L, garden 16 L, library 17 L, circusTent 18 L, masterBedroom 19 L, gothicBedroom 20 L. They are appended in TYPES order, not sorted by `special`.
- Ordinary (special 0, not streamed) in TYPES order: kitchen, luxuryKitchen, gothicKitchen, publicShowersMale, publicShowersFemale, houseBathroom, bedroom, office, maintenance, storage, server, breakroom, lounge (13).
- The `chosen` array can hold the same entry more than once (ordinary wrap-around). Rooms copy `type_entry["props"]` with `.duplicate()`.
- If `chosen.size()` is already >= count after the specials, it is **not** truncated, but only the first `outer_room_count` entries are used. `_shuffle_array(chosen)` still shuffles the whole array, so the RNG draw count is `chosen.size()-1`.

## 1. CRITICAL: `sort_custom` in `populate_rooms` is NOT stable

`prop_types.sort_custom(func(a, b): return area(a) > area(b))` uses Godot's `SortArray` introsort. Arrays of 16 elements or fewer are sorted by plain insertion sort, which is stable for a strict comparator. **Longer arrays get partitioned first, which reorders equal-area items.** Eight types have more than 16 props: bathroomFemale (17), ballroom (21), library (17), masterBedroom (20), gothicBedroom (18), bedroom (21), office (22), lounge (20). For **ballroom, library and lounge** Godot's result differs from a stable descending sort.

The sort input is just the type's own `props` list, and no RNG is involved. That means the result is a pure function of the type, so I precomputed it:

> **`catalog.json.types[i].props_sorted_for_populate`** is the exact array `populate_rooms` iterates. The prop index in ids (`"%s-prop-%d"`) is the index into this sorted array.

The precomputation is a line-by-line Python port of `core/templates/sort_array.h`. I fetched it from godotengine/godot `master` on 2026-11-01. The project's `config/features` says `4.7`, and I did not check that 4.7's file is byte-identical to master. Older 4.x versions copied the pivot by value rather than tracking it by index, but with a strict comparator the comparisons and swaps come out the same. The port passed a self-test against Python's sort on random and duplicate-heavy inputs, including the heap fallback. The comparator is `f64(f32(x)) * f64(f32(y))`, compared with `>`. `catalog.json.footprints[*].area_sort_key_f64_of_f32` is that key.

**Other `sort_custom` calls with the same instability:** `connect_facility_rooms` `candidates` (rooms choose 2, e.g. 9 rooms gives 36 entries, which is more than 16, so introsort) and `loop_candidates`, plus `place_safe_zones` `room_candidates` (more than 16 rooms only on huge maps). Ties are unlikely there because of the random `score`, but `place_safe_zones` sorts by integer distance and can tie. **If your Rust port uses `sort_by`, which is stable, it will diverge whenever there are ties in an array longer than 16.** Port the SortArray below and use it for every `sort_custom`. `Array.sort()` on unique strings (the hiding-spot `room_keys`) gives the same result with any algorithm.

### Godot `SortArray` (verbatim, `core/templates/sort_array.h`, master)
```cpp
template <typename T, typename Comparator = Comparator<T>, bool Validate = SORT_ARRAY_VALIDATE_ENABLED>
class SortArray {
	enum {
		INTROSORT_THRESHOLD = 16
	};

public:
	Comparator compare;

	inline int64_t median_of_3_index(const T *p_ptr, int64_t p_a_index, int64_t p_b_index, int64_t p_c_index) const {
		const T &a = p_ptr[p_a_index];
		const T &b = p_ptr[p_b_index];
		const T &c = p_ptr[p_c_index];
		if (compare(a, b)) {
			if (compare(b, c)) {
				return p_b_index;
			} else if (compare(a, c)) {
				return p_c_index;
			} else {
				return p_a_index;
			}
		} else if (compare(a, c)) {
			return p_a_index;
		} else if (compare(b, c)) {
			return p_c_index;
		} else {
			return p_b_index;
		}
	}

	inline int64_t bitlog(int64_t p_n) const {
		int64_t k;
		for (k = 0; p_n != 1; p_n >>= 1) {
			++k;
		}
		return k;
	}

	/* Heap / Heapsort functions */

	inline void push_heap(int64_t p_first, int64_t p_hole_idx, int64_t p_top_index, T p_value, T *p_array) const {
		int64_t parent = (p_hole_idx - 1) / 2;
		while (p_hole_idx > p_top_index && compare(p_array[p_first + parent], p_value)) {
			p_array[p_first + p_hole_idx] = p_array[p_first + parent];
			p_hole_idx = parent;
			parent = (p_hole_idx - 1) / 2;
		}
		p_array[p_first + p_hole_idx] = p_value;
	}

	inline void pop_heap(int64_t p_first, int64_t p_last, int64_t p_result, T p_value, T *p_array) const {
		p_array[p_result] = p_array[p_first];
		adjust_heap(p_first, 0, p_last - p_first, p_value, p_array);
	}
	inline void pop_heap(int64_t p_first, int64_t p_last, T *p_array) const {
		pop_heap(p_first, p_last - 1, p_last - 1, p_array[p_last - 1], p_array);
	}

	inline void adjust_heap(int64_t p_first, int64_t p_hole_idx, int64_t p_len, T p_value, T *p_array) const {
		int64_t top_index = p_hole_idx;
		int64_t second_child = 2 * p_hole_idx + 2;

		while (second_child < p_len) {
			if (compare(p_array[p_first + second_child], p_array[p_first + (second_child - 1)])) {
				second_child--;
			}

			p_array[p_first + p_hole_idx] = p_array[p_first + second_child];
			p_hole_idx = second_child;
			second_child = 2 * (second_child + 1);
		}

		if (second_child == p_len) {
			p_array[p_first + p_hole_idx] = p_array[p_first + (second_child - 1)];
			p_hole_idx = second_child - 1;
		}
		push_heap(p_first, p_hole_idx, top_index, p_value, p_array);
	}

	inline void sort_heap(int64_t p_first, int64_t p_last, T *p_array) const {
		while (p_last - p_first > 1) {
			pop_heap(p_first, p_last--, p_array);
		}
	}

	inline void make_heap(int64_t p_first, int64_t p_last, T *p_array) const {
		if (p_last - p_first < 2) {
			return;
		}
		int64_t len = p_last - p_first;
		int64_t parent = (len - 2) / 2;

		while (true) {
			adjust_heap(p_first, parent, len, p_array[p_first + parent], p_array);
			if (parent == 0) {
				return;
			}
			parent--;
		}
	}

	inline void partial_sort(int64_t p_first, int64_t p_last, int64_t p_middle, T *p_array) const {
		make_heap(p_first, p_middle, p_array);
		for (int64_t i = p_middle; i < p_last; i++) {
			if (compare(p_array[i], p_array[p_first])) {
				pop_heap(p_first, p_middle, i, p_array[i], p_array);
			}
		}
		sort_heap(p_first, p_middle, p_array);
	}

	inline void partial_select(int64_t p_first, int64_t p_last, int64_t p_middle, T *p_array) const {
		make_heap(p_first, p_middle, p_array);
		for (int64_t i = p_middle; i < p_last; i++) {
			if (compare(p_array[i], p_array[p_first])) {
				pop_heap(p_first, p_middle, i, p_array[i], p_array);
			}
		}
	}

	inline int64_t partitioner(int64_t p_first, int64_t p_last, int64_t p_pivot, T *p_array) const {
		const int64_t unmodified_first = p_first;
		const int64_t unmodified_last = p_last;
		const T *pivot_element_location = &p_array[p_pivot];

		while (true) {
			while (p_first != p_pivot && compare(p_array[p_first], *pivot_element_location)) {
				if constexpr (Validate) {
					ERR_BAD_COMPARE(p_first == unmodified_last - 1);
				}
				p_first++;
			}
			p_last--;
			while (p_last != p_pivot && compare(*pivot_element_location, p_array[p_last])) {
				if constexpr (Validate) {
					ERR_BAD_COMPARE(p_last == unmodified_first);
				}
				p_last--;
			}

			if (p_first >= p_last) {
				return p_first;
			}

			if (pivot_element_location == &p_array[p_first]) {
				pivot_element_location = &p_array[p_last];
			} else if (pivot_element_location == &p_array[p_last]) {
				pivot_element_location = &p_array[p_first];
			}
			SWAP(p_array[p_first], p_array[p_last]);
			p_first++;
		}
	}

	inline void introsort(int64_t p_first, int64_t p_last, T *p_array, int64_t p_max_depth) const {
		while (p_last - p_first > INTROSORT_THRESHOLD) {
			if (p_max_depth == 0) {
				partial_sort(p_first, p_last, p_last, p_array);
				return;
			}

			p_max_depth--;

			int64_t cut = partitioner(
					p_first,
					p_last,
					median_of_3_index(p_array, p_first, p_first + (p_last - p_first) / 2, p_last - 1),
					p_array);

			introsort(cut, p_last, p_array, p_max_depth);
			p_last = cut;
		}
	}

	inline void introselect(int64_t p_first, int64_t p_nth, int64_t p_last, T *p_array, int64_t p_max_depth) const {
		while (p_last - p_first > 3) {
			if (p_max_depth == 0) {
				partial_select(p_first, p_nth + 1, p_last, p_array);
				SWAP(p_first, p_nth);
				return;
			}

			p_max_depth--;

			int64_t cut = partitioner(
					p_first,
					p_last,
					median_of_3_index(p_array, p_first, p_first + (p_last - p_first) / 2, p_last - 1),
					p_array);

			if (cut <= p_nth) {
				p_first = cut;
			} else {
				p_last = cut;
			}
		}

		insertion_sort(p_first, p_last, p_array);
	}

	inline void unguarded_linear_insert(int64_t p_last, T p_value, T *p_array) const {
		int64_t next = p_last - 1;
		while (compare(p_value, p_array[next])) {
			if constexpr (Validate) {
				ERR_BAD_COMPARE(next == 0);
			}
			p_array[p_last] = p_array[next];
			p_last = next;
			next--;
		}
		p_array[p_last] = p_value;
	}

	inline void linear_insert(int64_t p_first, int64_t p_last, T *p_array) const {
		T val = p_array[p_last];
		if (compare(val, p_array[p_first])) {
			for (int64_t i = p_last; i > p_first; i--) {
				p_array[i] = p_array[i - 1];
			}

			p_array[p_first] = val;
		} else {
			unguarded_linear_insert(p_last, val, p_array);
		}
	}

	inline void insertion_sort(int64_t p_first, int64_t p_last, T *p_array) const {
		if (p_first == p_last) {
			return;
		}
		for (int64_t i = p_first + 1; i != p_last; i++) {
			linear_insert(p_first, i, p_array);
		}
	}

	inline void unguarded_insertion_sort(int64_t p_first, int64_t p_last, T *p_array) const {
		for (int64_t i = p_first; i != p_last; i++) {
			unguarded_linear_insert(i, p_array[i], p_array);
		}
	}

	inline void final_insertion_sort(int64_t p_first, int64_t p_last, T *p_array) const {
		if (p_last - p_first > INTROSORT_THRESHOLD) {
			insertion_sort(p_first, p_first + INTROSORT_THRESHOLD, p_array);
			unguarded_insertion_sort(p_first + INTROSORT_THRESHOLD, p_last, p_array);
		} else {
			insertion_sort(p_first, p_last, p_array);
		}
	}

	inline void sort_range(int64_t p_first, int64_t p_last, T *p_array) const {
		if (p_first != p_last) {
			introsort(p_first, p_last, p_array, bitlog(p_last - p_first) * 2);
			final_insertion_sort(p_first, p_last, p_array);
		}
	}

	inline void sort(T *p_array, int64_t p_len) const {
		sort_range(0, p_len, p_array);
	}
```
Note that C++ `(p_hole_idx - 1) / 2` truncates toward zero, so `(0-1)/2 == 0`.

## 2. CRITICAL: float precision (Vector2 is float32)

Godot ships single-precision builds by default. I found no sign of a double-precision engine build, but that is not something I can confirm from the project files, so verify it. Under that assumption, `Vector2` components are **f32**. GDScript `float` variables, literals like `.12`, and Dictionary float values are **f64**. Reading `v.x` widens f32 to f64, and the arithmetic that follows is f64. What this means for the port:

- Footprints: parse the JSON numbers as `f32`, then widen (`0.38f32 as f64 = 0.3799999952316284`). `_room_prop_fits` tests like `center.x - footprint.x - clearance < room.x + 1.0` sit **exactly on boundaries** for many cells (for example `1.5 - .38 - .12`), so f64 literals would flip results.
- Evaluation order (left to right, all f64): `(center.x - fp.x) - clearance`, `(center.x + fp.x) + clearance`, `float(room.x + room.width) - 1.0` (int add first), `(fp.x + other_fp.x) + clearance`, `fp.x + radius`.
- `center = Vector2(cell) + Vector2(.5,.5)` is exact in f32.
- `Vector2.distance_to` returns **f32**: `sqrtf((x-px)*(x-px)+(y-py)*(y-py))`, all f32, then widened for the comparison. This applies to `_room_prop_fits` (start distance) and `place_hiding_spots` (spacing). In `_room_prop_fits` the threshold is `maxf(1.2, maxf(fp.x, fp.y) + 0.23 + .5)` in f64.
- `Vector2(cell).distance_squared_to(Vector2(center))` (in `_room_prop_preference`) is f32. With small integers it is exact.
- `Vector2(...).length()` (`_far_from_all`, `_pairwise_far`, the exit distance) is f32 sqrt.
- `surface_height` is f64 (`0.71 * 1.5` and so on). `catalog.json` stores the f64 products.

## 3. RoomCatalog functions the generator calls (verbatim)

`type_by_id` (room_catalog.gd:608-612):
```gdscript
static func type_by_id(id: String) -> Dictionary:
	for entry in TYPES:
		if String(entry["id"]) == id:
			return entry
	return {}
```

`type_for_region (custom-map path only)` (room_catalog.gd:615-616):
```gdscript
static func type_for_region(region: String) -> Dictionary:
	return type_by_id(String(REGION_DEFAULT_TYPE.get(region, "")))
```

`footprint` (room_catalog.gd:619-621):
```gdscript
static func footprint(type: String, rotation: int = 0) -> Vector2:
	var value: Vector2 = FOOTPRINTS.get(type, Vector2(.32, .32))
	return Vector2(value.y, value.x) if absi(rotation) % 2 == 1 else value
```

`is_wall_backed` (room_catalog.gd:637-638):
```gdscript
static func is_wall_backed(type: String) -> bool:
	return WALL_BACKED_TYPES.has(type)
```

`is_surface_prop` (room_catalog.gd:641-642):
```gdscript
static func is_surface_prop(type: String) -> bool:
	return SURFACE_PARENT_TYPES.has(type)
```

`surface_parent_types` (room_catalog.gd:645-646):
```gdscript
static func surface_parent_types(type: String) -> Array:
	return SURFACE_PARENT_TYPES.get(type, [])
```

`surface_height` (room_catalog.gd:665-667):
```gdscript
## The authored (person-sized) top, raised with the parent's hunter scale (PropModelCatalog.HUNTER_SCALE).
static func surface_height(parent_type: String) -> float:
	return float(SURFACE_HEIGHTS.get(parent_type, 0.8)) * PropModelCatalog.hunter_scale(parent_type)
```

`is_solid` (room_catalog.gd:723-745):
```gdscript
static func is_solid(type: String) -> bool:
	return type not in ["arcadeCarpetCircle", "arcadeCarpetRunner",
		"diningChandelier", "ceilingLight", "ceilingFan",
		"infernoChainLantern", "bathroomMirror",
		"mansionAreaRug", "framedWallPortrait", "normalPictureFrameEmpty",
		"normalPaintingFrameEmpty", "luxuryPictureFrameEmpty",
		"luxuryPaintingFrameEmpty", "gothicPictureFrameEmpty",
		"gothicPaintingFrameEmpty", "wallSconce",
		"heavyMansionDrapes", "flowerCenterpiece", "tableCandelabra",
		"bedroomRug", "bedroomWallClock", "bedsideLamp", "tableLamp",
		"retroComputerCompact", "retroComputerTower",
		"retroComputerDesktop", "retroComputerTerminal",
		"officePrinter",
		"gothicBedroomRug",
		"gothicBedsideCandelabra", "publicHandDryer",
		"publicSoapDispenser", "publicPaperTowelDispenser",
		"publicMensRoomSign", "publicWomensRoomSign", "houseMedicineCabinet",
		"houseTowelRack", "houseBathMat", "houseBathroomShelf",
		"grandStaircase", "grandFoyerStaircase",
		"surgicalLight", "hangingVines", "overgrowthPatch", "labMicroscope",
		"xrayLightbox", "hospitalWardSign", "monitorWall", "labWhiteboard", "diveHeadsetShelf",
		"hazardSign", "menuBoard", "donutSign", "cinemaScreen",
		"deskMic", "studioMonitors", "cableCoil", "cableRun", "acousticPanel", "guitarWallHanger", "onAirSign"]
```

`is_walkable_structure` (room_catalog.gd:748-749):
```gdscript
static func is_walkable_structure(type: String) -> bool:
	return WALKABLE_STRUCTURE_TYPES.has(type)
```

`room_has_walkable_structure` (room_catalog.gd:752-757):
```gdscript
static func room_has_walkable_structure(type: String) -> bool:
	var entry := type_by_id(type)
	for prop_type in entry.get("props", []):
		if is_walkable_structure(String(prop_type)):
			return true
	return false
```

`PropModelCatalog.hunter_scale` (game/config/prop_model_catalog.gd):
```gdscript
static func hunter_scale(type: String) -> float:
	return float(HUNTER_SCALE.get(type, 1.0))
```

Data tables used by the functions above are all in `catalog.json`: `TYPES`, `FOOTPRINTS` (both `footprints` for room props and `footprint_table_full`), `non_solid_types`, `wall_backed` (`WALL_BACKED_TYPES`), `surface_props` (`SURFACE_PARENT_TYPES`, insertion order kept), `surface_heights_table_raw` (`SURFACE_HEIGHTS`), `walkable_structures`, `centerpiece_types` and `region_default_type` (custom-map path only).

Implementation notes:
- `footprint(type, rotation)`: `absi(rotation) % 2 == 1` swaps x and y. Rotations are 0 to 3 in the generator.
- `is_solid`: `true` unless the type is in `non_solid_types`. Every surface child (lamps, computers, printer, candelabras, microscope, centerpiece) is non-solid.
- `room_has_walkable_structure("lobby")` and `room_has_walkable_structure("")` return false (`type_by_id` gives `{}`).
- `type_by_id` returns the first match. Ids are unique (checked).

## 4. Generator-side details from the procedural path

- `populate_rooms` skips rooms whose `prop_types` is empty (the lobby) or whose width or height is less than 4. **A skipped room consumes no RNG.**
- For each kept room: `_room_prop_candidates` builds the interior cells (`x+1..x+w-2`, `y+1..y+h-2`, row-major y then x). Unless the room type is in `CENTERPIECE_TYPES`, it keeps only cells with `|dx|>1 AND |dy|>1` from `room.center`. Then it shuffles, which draws `n-1` values. Procedural rooms have no `"cells"` key.
- Per prop in `props_sorted_for_populate`:
  - Surface prop (`is_surface_prop`): **no RNG draw**. Scan the global `props` in order for the first entry with the same `room_id`, `type` in `surface_parent_types(type)` and an id not in `used_surfaces`. If found, append with the parent's `x`, `y` and `rotation`, `mount_y = surface_height(parent.type)` and `surface_id`. If not found, skip. Surface props never mark `used`. A child whose area is bigger than its parent's sorts **before** the parent and gets skipped. Example: `retroComputerTower` .62*.33 is bigger than `officeDesk` .42*.28, so in `office` the tower always finds no desk.
  - Otherwise: `rotation_start = next_index(4)` is drawn **even if nothing fits**. For each candidate cell in shuffled order that is not in `used`, try `offset 0..3`, `rotation = (rotation_start+offset)%4`. Keep the strict `<` minimum of `_room_prop_preference`. Ties keep the first one found.
- `_room_prop_fits` checks against **all** `props` so far (every room, solid ones only, each with its own rotation). It also rejects `spawn_points` and `exit_cell`. At populate time `exit_cell == start` (set in `_init`) and `spawn_points == [start]`.
- `_room_prop_preference` uses the literal list `["carnivalCarousel", "giantTree", "diningTable"]` together with `CENTERPIECE_TYPES` of the room type. Wall-backed desired rotation: bottom gives 2, then left gives 1, then right gives 3, otherwise 0. Checked in that order, with ties going to the earlier check.
- `can_occupy` (and therefore `distance_map`, relays, batteries, lift and hiding) treats only `is_solid` props as blockers, using the rotated footprint. Walkable structures are non-solid.

## 5. Verbatim generator functions requested

### room_at / _cell_in_walkable_structure_room / place_hiding_spots / _walls_around / hiding_exit_position
```gdscript
# facility_generator.gd:1287-1433
var hiding_spots: Array = []


## HUNTER_ROSTER.md's "trivial gap": a point -> room query nothing needed
## until Loxelle, whose passive only ever destroys spots in her OWN room.
func room_at(x: float, y: float) -> Dictionary:
	var cell := Vector2i(floori(x), floori(y))
	for room in rooms:
		if room.has("cells"):
			var room_cells = room["cells"]
			if room_cells is Dictionary and room_cells.has(cell):
				return room
			if room_cells is Array and room_cells.has(cell):
				return room
			continue
		if x >= room["x"] and x < room["x"] + room["width"] \
				and y >= room["y"] and y < room["y"] + room["height"]:
			return room
	return {}


func _cell_in_walkable_structure_room(cell: Vector2i) -> bool:
	var room := room_at(cell.x + 0.5, cell.y + 0.5)
	return RoomCatalog.room_has_walkable_structure(String(room.get("type", "")))


func place_hiding_spots(count: int = 6) -> Array:
	hiding_spots = []
	var rng := SeededRandom.new("%s:HIDING-SPOTS" % seed_text)
	var taken := {exit_cell: true}
	for spawn_cell in spawn_points:
		taken[spawn_cell] = true
	for group in [relays, batteries, safe_zones]:
		for entry in group:
			taken[Vector2i(entry["x"], entry["y"])] = true

	# Room cells preferred over corridor ones: a locker in a room is furniture,
	# a locker wedged in a corridor wall is not somewhere real. It also gives
	# every spot a real room_id, which Loxelle's passive depends on -- she only
	# ever burns a spot in her OWN room, and a spot with no room could never be
	# hers or anyone else's. Corridor cells are the fallback for a layout with
	# too little room floor to seat every spot, not the first choice.
	var in_room: Array = []
	var in_corridor: Array = []
	for y in range(1, rows - 1):
		for x in range(1, cols - 1):
			var cell := Vector2i(x, y)
			if not can_occupy(x + 0.5, y + 0.5) or taken.has(cell):
				continue
			if _cell_in_walkable_structure_room(cell):
				continue
			# Backed onto a wall, and not in a doorway: a spot in the middle of a
			# corridor is not somewhere you could hide, and a spot in a doorway
			# blocks the way through.
			if _walls_around(cell) != 1:
				continue
			if room_at(cell.x, cell.y).is_empty():
				in_corridor.append(cell)
			else:
				in_room.append(cell)
	if in_room.is_empty() and in_corridor.is_empty():
		return hiding_spots

	# Spread them out. Six lockers clustered in one room is one hiding place
	# with six doors, not six hiding places.
	# Increase spacing bias so hiding spots cluster by room and avoid sparse
	# single-cell scatter across the chunk. Use room-aware grouping by preferring
	# spots that share the same room_id until exhausted, falling back to the
	# original spaced scatter algorithm. This reduces carnival-like sparse
	# placement where individual props sit isolated across the chunk.
	var spacing := maxf(4.0, float(mini(cols, rows)) / float(maxi(1, count)))
	# Build room buckets to prefer multiple picks from the same room
	var room_buckets: Dictionary = {}
	for c: Vector2i in in_room:
		var r: String = room_at(c.x, c.y).get("id", "")
		if not room_buckets.has(r):
			room_buckets[r] = []
		room_buckets[r].append(c)
	# Prefer selecting multiple spots within the same room to create bundled
	# locker areas. Iterate room buckets first, drawing at most one per room in
	# a pass to keep distribution fair, then fall back to corridor candidates.
	var room_keys: Array = room_buckets.keys()
	room_keys.sort()
	for i in range(room_keys.size() - 1, 0, -1):
		var j := rng.next_index(i + 1)
		var tmp = room_keys[i]
		room_keys[i] = room_keys[j]
		room_keys[j] = tmp
	while hiding_spots.size() < count and not room_keys.is_empty():
		for rk: String in room_keys.duplicate():
			if hiding_spots.size() >= count:
				break
			var bucket: Array = room_buckets.get(rk, [])
			if bucket.is_empty():
				room_keys.erase(rk)
				continue
			var pick: Vector2i = bucket[rng.next_index(bucket.size())]
			# enforce spacing from already chosen spots
			var clear := true
			for spot in hiding_spots:
				if Vector2(pick).distance_to(Vector2(spot["x"], spot["y"])) < spacing:
					clear = false
					break
			if not clear:
				# Remove this candidate to avoid infinite loop; try others in bucket
				bucket.erase(pick)
				continue
			bucket.erase(pick)
			var room := room_at(pick.x, pick.y)
			hiding_spots.append({"id": "hide-%d" % hiding_spots.size(),
				"x": pick.x, "y": pick.y,
				"room_id": String(room.get("id", ""))})

	# If still short, fall back to corridors using original spacing logic
	for candidates in [in_corridor]:
		while hiding_spots.size() < count and not candidates.is_empty():
			var pick: Vector2i = candidates[rng.next_index(candidates.size())]
			var clear := true
			for spot in hiding_spots:
				if Vector2(pick).distance_to(Vector2(spot["x"], spot["y"])) < spacing:
					clear = false
					break
			candidates.erase(pick)
			if not clear:
				continue
			var room := room_at(pick.x, pick.y)
			hiding_spots.append({"id": "hide-%d" % hiding_spots.size(),
				"x": pick.x, "y": pick.y,
				"room_id": String(room.get("id", ""))})
	return hiding_spots


func _walls_around(cell: Vector2i) -> int:
	var walls := 0
	for step in [Vector2i(0, 1), Vector2i(0, -1), Vector2i(1, 0), Vector2i(-1, 0)]:
		var at: Vector2i = cell + step
		if at.x < 0 or at.y < 0 or at.x >= cols or at.y >= rows or grid[at.y][at.x] != 0:
			walls += 1
	return walls


## An open cell beside a spot, for stepping out of it.
func hiding_exit_position(spot: Dictionary) -> Vector2:
	var open := nearest_open_position(spot["x"] + 0.5, spot["y"] + 0.5, GameConfig.PLAYER_RADIUS)
	if open.is_empty():
		return Vector2(spot["x"] + 0.5, spot["y"] + 0.5)
	return Vector2(open["x"], open["y"])
```
`GameConfig.PLAYER_RADIUS := 0.24` (game_config.gd:173). `DEFAULT_AGENT_RADIUS := 0.23`.

Porting gotchas for `place_hiding_spots`:
- It uses its own RNG `SeededRandom("%s:HIDING-SPOTS" % seed_text)`. `taken` covers the exit, the spawn points, relays, batteries and safe zones. Props are not in `taken`, but `can_occupy` excludes cells covered by solid props.
- `room_at(cell.x, cell.y)` takes integer coordinates (no +0.5). With half-open integer bounds the result is the same. `room_at` walks `rooms` in order (lobby first).
- `room_buckets` is a Dictionary in insertion order. `room_keys.sort()` is lexicographic String order (`"room-10" < "room-2"`, and `"room-lobby"` comes after the digit keys). The keys are then Fisher-Yates shuffled with the hiding RNG.
- `room_buckets.get(rk, [])` returns the **same Array reference**, so `bucket.erase(pick)` mutates the bucket. `erase` removes the first occurrence. Cells are unique.
- When a pick fails the spacing test, it is erased and the loop moves on to the **next room key**. It does not retry the same room.
- The inner `for rk in room_keys.duplicate()` iterates a snapshot while `room_keys.erase(rk)` removes empty buckets.
- The corridor fallback mutates `in_corridor` directly.
- Spacing is `maxf(4.0, float(mini(cols, rows)) / float(maxi(1, count)))` in f64. The distance is f32 `distance_to`.

### nearest_open_position (used by hiding_exit_position)
```gdscript
# facility_generator.gd:935-953
func nearest_open_position(origin_x: float, origin_y: float, radius: float, blocked_cells = null) -> Dictionary:
	var start_cell := Vector2i(int(floor(origin_x)), int(floor(origin_y)))
	var visited := {start_cell: true}
	var queue: Array = [start_cell]
	var qi := 0
	while qi < queue.size():
		var current: Vector2i = queue[qi]
		qi += 1
		var world_x: float = current.x + 0.5
		var world_y: float = current.y + 0.5
		if can_occupy(world_x, world_y, radius, blocked_cells):
			return {"x": world_x, "y": world_y, "cell": current}
		for dir in DIRECTIONS4:
			var next_cell: Vector2i = current + dir
			if visited.has(next_cell) or not is_floor(next_cell.x + 0.5, next_cell.y + 0.5):
				continue
			visited[next_cell] = true
			queue.append(next_cell)
	return {}
```

### place_safe_zones (full) and helpers it calls
```gdscript
# facility_generator.gd:981-1042
func place_safe_zones(from_start: Dictionary) -> void:
	safe_zones = []
	if safe_zone_count <= 0:
		return
	if safe_zone_at_spawn:
		safe_zones = [{"id": "safe-0", "x": start.x, "y": start.y, "room_id": "room-lobby", "spawn": true}]
		return

	var room_candidates: Array = rooms.duplicate()
	room_candidates.sort_custom(func(a, b):
		var ad: int = from_start.get(a["center"], 0)
		var bd: int = from_start.get(b["center"], 0)
		return ad > bd
	)
	var alternatives: Array = room_candidates.filter(func(r):
		return r["id"] != "room-lobby" and not RoomCatalog.room_has_walkable_structure(
			String(r.get("type", ""))))
	var wanted: int = mini(safe_zone_count, alternatives.size())
	if wanted <= 0:
		return

	var stride: int = maxi(1, alternatives.size() / wanted)
	var preferred: Array = []
	var preferred_ids := {}
	var idx := 0
	while preferred.size() < wanted and idx < alternatives.size():
		var room = alternatives[idx]
		preferred.append(room)
		preferred_ids[room["id"]] = true
		idx += stride
	while preferred.size() < wanted:
		var found = null
		for r in alternatives:
			if not preferred_ids.has(r["id"]):
				found = r
				break
		if found == null:
			break
		preferred.append(found)
		preferred_ids[found["id"]] = true

	safe_zones = _make_safe_zones(preferred)
	if safe_zones.size() == wanted and _keeps_hunter_routes_connected():
		return

	var fallback_pool: Array = alternatives.slice(0, mini(alternatives.size(), 12))
	var count := wanted
	while count >= 1:
		for combo in _room_combinations(fallback_pool, count):
			safe_zones = _make_safe_zones(combo)
			if _keeps_hunter_routes_connected():
				return
		count -= 1
	safe_zones = []


static func _make_safe_zones(rooms_subset: Array) -> Array:
	var zones: Array = []
	for i in rooms_subset.size():
		var room = rooms_subset[i]
		zones.append({"id": "safe-%d" % i, "x": room["center"].x, "y": room["center"].y, "room_id": room["id"]})
	return zones
```
```gdscript
# facility_generator.gd:1076-1117
func get_safe_zone_blocked_cells() -> Dictionary:
	var blocked := {}
	for zone in safe_zones:
		if zone.get("spawn", false):
			continue
		var center_x: float = zone["x"] + 0.5
		var center_y: float = zone["y"] + 0.5
		var extent := int(ceil(SAFE_ZONE_BLOCK_RADIUS))
		for y in range(zone["y"] - extent, zone["y"] + extent + 1):
			for x in range(zone["x"] - extent, zone["x"] + extent + 1):
				if not is_floor(x + 0.5, y + 0.5):
					continue
				if Vector2(x + 0.5 - center_x, y + 0.5 - center_y).length() < SAFE_ZONE_BLOCK_RADIUS:
					blocked[Vector2i(x, y)] = true
	return blocked


func _keeps_hunter_routes_connected() -> bool:
	var blocked := get_safe_zone_blocked_cells()
	var component := occupiable_component(start, 0.25, blocked)
	for cell in floor_cells():
		if can_occupy(cell.x + 0.5, cell.y + 0.5, 0.25, blocked) and not component.has(cell):
			return false
	return true


func _room_combinations(source_rooms: Array, size: int) -> Array:
	var result: Array = []
	if size <= 0 or size > source_rooms.size():
		return result
	_walk_combinations(source_rooms, size, 0, [], result)
	return result


func _walk_combinations(source_rooms: Array, size: int, start_index: int, current: Array, result: Array) -> void:
	if current.size() == size:
		result.append(current.duplicate())
		return
	for index in range(start_index, source_rooms.size()):
		current.append(source_rooms[index])
		_walk_combinations(source_rooms, size, index + 1, current, result)
		current.pop_back()
```
Notes:
- With the shipping options (`RoomState.generator_options`), `safe_zone_at_spawn = safe_zone_count > 0`. So whenever there is any safe zone, the function returns early with the spawn zone, and the combination search only runs when someone passes `safe_zone_at_spawn=false` explicitly.
- `alternatives.size() / wanted` is integer division.
- `sort_custom` here sorts descending by BFS distance (ties are possible), so see section 1.
- `from_start.get(a["center"], 0)` defaults to 0 for an unreachable centre.

## 6. Things that cannot be determined statically
- Whether the shipped engine is a single- or double-precision build. I assumed the default single-precision build. With `precision=double`, Vector2 would be f64 and section 2 would not apply.
- Whether Godot 4.7's `sort_array.h` matches the master copy quoted above. I expect it does, and the outcome is equivalent for strict comparators.
- Nothing else. All catalog data is const and was parsed mechanically from source.

## Removed props (`removed_props`, Rust only)

`catalog.json` `removed_props` lists prop types the Rust game never shows (the mirrors: `bathroomMirror`, `standingMirror`). The generators still place them, so the seeded placement stays equal to Godot's; `Facility::remove_listed_props` drops them when a finite floor (`Facility::generate`), a live level-0 chunk (`route::live::from_layout`) or a legacy chunk (`infinite::legacy_chunk`) is finished. Take a type out of the list to bring it back.
