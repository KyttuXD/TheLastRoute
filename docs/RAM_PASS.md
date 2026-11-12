# RAM pass

Goal: less memory before and during play. Started 2026-11-12 on KT-FW13 (Linux x86_64, Mesa, dev build).

## How to measure

- `sh tools/perf/mem.sh 12 target/debug/last_route --offline` prints resident memory each second (menu).
- `sh tools/perf/mem.sh 45 target/debug/last_route --offline --stress 40 --stress-scenario mixed` does it for a run.
- On Linux every `startup timing:` log line now ends with `rss N MB` (`app::perf::rss_mb`).

Set `LR_PROFILE_DIR` to a scratch folder so the real profile is not touched.

## Baseline

| Point | Resident memory |
|---|---|
| Window open, nothing loaded | 89 MB (about 60 MB of it is the Mesa driver: libLLVM 46 MB, libgallium 14 MB) |
| Boot card up, before `Content::load` | 133 to 181 MB (varies between runs) |
| After `Assets::load` | +83 MB |
| Main menu, settled | 210 to 216 MB (heap 78 MB, other anonymous 31 MB, the executable 32 MB) |
| 40 s `mixed` stress run, peak | 371 MB |

## Findings

1. **Prop models are the asset cost, not pictures.** All embedded PNGs decode to 3.1 MB. `load_models`
   (`shared/assets.rs`) parses all 978 `props/*.bbmodel` files (30 MB of JSON) at startup, whether or not the
   route ever shows them. That is most of the +83 MB.
2. **The boot card costs 45 to 90 MB before any game data loads** (`BootLoader::new` and its first frames: two
   system fonts and the baked card). Not yet broken down.
3. **Freed load buffers are a small part.** `app::perf::trim_heap` (glibc `malloc_trim`) after the load gives back
   about 6 MB. It is in, because it is free, but the memory above is live data.
4. Play adds about 160 MB over the menu. Not yet broken down (chunk meshes, hunters, the streaming window).

## Done

| Change | Menu | 40 s `mixed` stress peak | `Assets::load` |
|---|---|---|---|
| Baseline | 210 MB | 371 MB | 250 ms |
| Prop models built on first use (`ModelStore`, `shared/assets.rs`) | 190 MB | 292 MB | 77 ms |
| Fonts: each file loaded once, unused glyphs emptied, caption face on first use | 165 MB | 248 MB | |
| Identical embedded model pictures share one texture; mesh tiles and model parts trimmed to size | 165 MB | 240 MB | |
| Loaded sounds held as 16-bit samples instead of f32 (quad-snd patch) | 160 MB | 240 MB | |
| Glyph atlas row-wrap fix (vendored macroquad) | 144 MB | 217 MB | |
| World mesh tiles kept in an 18-byte vertex while out of sight (`render/packed_mesh.rs`) | 144 MB | 210 MB | |

Average frame time in the stress run did not change (1.24 ms before, 1.17 ms after). A model is built the first time
`Assets::models.get` or `contains_key` asks for it; picking a prop's variant builds that prop type's variants only.

Numbers move by about 6 MB between runs; each row is the middle of two or three.

About 75 MB of every figure is not the game's to save on this PC: the Mesa driver's libraries (libLLVM 46 MB,
libgallium 14 MB) and the pages of the executable in use.

### Fonts (the largest single find)

fontdue builds every glyph's outline when a font loads, about 4 KB a glyph: a 2400-glyph face (Liberation, Courier
New) is about 10 MB. The game loaded five (two for the boot card, three in `Assets`), four of them two files twice.
Now `shared/assets.rs` loads each file once (`system_font`, used by the boot card too), `shared/font_subset.rs`
empties the outlines outside Latin, punctuation, arrows, maths and box drawing before the font is parsed, and the
italic caption face loads when a caption is first drawn. Metrics, kerning and glyph numbers are unchanged.
`LR_FULL_FONTS=1` loads whole fonts (Greek, Cyrillic), at about 8 MB a face.

### Textures

Shrinking the pictures would save almost nothing: every standalone PNG decodes to 3.1 MB in total, and the pictures
embedded in the models are 128x64 or smaller (two 512x512 spider skins and three 256x256 environment sheets aside).
The waste was duplication: 1047 models embed a picture and only 207 differ (26.5 MB decoded against 5.3 MB), so
equal pictures now share one texture (`PropModel::parse`, by content hash), which also joins their draw batches.

### What the heap holds during play (allocation profile, 60 s into a `mixed` stress run, before the last row)

| Owner | Live |
|---|---|
| World mesh tiles (`render/mesh_batch.rs::split_into_tiles`; now packed, see below) | 51 MB |
| Fonts (after the change above) | 22 MB |
| Prop model geometry (`PropModel::parse`) | 18 MB |
| One font's glyph atlas, 2048x2048 RGBA (macroquad keeps the CPU copy) | 16 MB |
| The draw batch buffer (`geometry` in `submit`) | 10 MB |
| Decoded sounds (quad-snd keeps f32 samples) | 9.5 MB |
| Rig and model parse caches, chunk grids | about 10 MB |

`tools/perf/alloc_prof.rs` is the profiler that made this table (how to use it is in the file).

### World mesh tiles (2026-11-12)

The floor's meshes (about two million vertices in a stress run) were kept as macroquad's 40-byte vertex. They are now
kept as `PackedMesh` (`render/packed_mesh.rs`), 18 bytes a vertex. A tile that is being drawn keeps its unpacked
vertices as well and is drawn from them as before; a tile the cull has dropped for 1.5 s gives them back. So the
saving is on what is out of sight: the tiles behind the fog and the camera, the kept parts of the last window, the
neighbour levels.

| Field | Before | Now | Cost |
|---|---|---|---|
| Position | 3 x f32 | 3 x u16 on a fixed 1/2048 m grid from the mesh's corner | at most 0.25 mm off; the grid is shared, so a corner two meshes share stays one point |
| UV | 2 x f32 | 2 x u16 on a power-of-two step fitted to the mesh's UV span | 1/32768 for spans up to 2 |
| Colour | 4 x u8 | the same | none |
| Normal and shader mode | 4 x f32 | 9 bits an axis, 5 bits of mode | under 0.25 degrees; exact for axis-aligned faces (84% of vertices) |

Measured before choosing: every mode is a whole number (0, 1, 2), every normal is unit length, the widest mesh is
17.3 m (the grid holds 32 m), the UV span of 99% of meshes is 3 or less (the widest, 187, takes a coarser step).

- Peak in the stress run: 213 to 222 MB before, 210 to 211 MB now (three runs each, back to back). A window recentre
  or a level's height shift now moves one origin per mesh in place of every vertex.
- Picture: two in-game screenshots (a corridor, a spiral stair well) against the build before match to within what
  two runs of one build differ by, once the run's start fade (which follows the clock) is taken out.
- Frame time: unchanged (1.00 to 1.04 ms against 1.00 to 1.01 ms).
- The first version unpacked every drawn tile each frame. It saved more (217 MB to 201 MB) but unpacking costs
  1.6 ns a vertex against 0.7 ns for a plain copy, and the average frame went from 1.01 ms to 1.31 ms. That broke
  the frame-time rule ([PERFORMANCE.md](PERFORMANCE.md)), so drawn tiles keep their unpacked form. A lookup table
  for the normals was slower than the division (2.0 ns against 1.6: the compiler's vector code handles the division).

### Tried, no gain (2026-11-12)

- **Welding duplicate mesh vertices.** Counted over 3200 world meshes in a stress run: 2,063,220 vertices, 2,020,189
  distinct (2% duplicates), 1.5 indices a vertex. The meshes are plain quads with nothing to weld.
- **glibc allocator settings** (`MALLOC_ARENA_MAX`, mmap and trim thresholds): within run-to-run noise.
- **Fewer preloaded levels** (`data/stairs_links.json`). One level each way saved about 9 MB, but
  `the_levels_near_the_runner_are_all_kept_within_the_frame_budget` requires two levels below to stay built, and
  with that kept (one up, two down) the peak did not move. Left as it was.

## Next, in order of expected gain

- [ ] World mesh tiles: the vertex is done (see above). What is left is fewer chunks kept around the player, which changes view distance.
- [ ] The time a drawn tile keeps its unpacked form (`HOT_SECONDS`, 1.5 s) is a guess; a shorter time saves more while moving fast.
- [ ] The glyph atlas still stores one-channel glyphs as RGBA and keeps a CPU copy. Small now that it no longer over-grows (see `vendor/macroquad/LAST_ROUTE_PATCHES.md`); a one-channel atlas would need shader changes.
- [ ] Check that the 110 MB dev executable (32 MB resident) shrinks in the release build, and measure release numbers.
- [ ] Repeat on the Intel Macs (no `/proc`; use `footprint` or Activity Monitor).
