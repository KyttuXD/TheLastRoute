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

## Next, in order of expected gain

- [ ] Load prop models on first use instead of all at startup (16 call sites read `Assets::models`). Expected: most of 83 MB at the menu; during play, only the regions visited.
- [ ] Break down the boot card's 45 to 90 MB (log `rss` around the two `load_font` calls and the bake) and free the boot fonts and bake when the card is gone.
- [ ] Break down the 160 MB of play: `rss` at run start, after the first window build, and each minute; then the chunk mesh and collision buffers.
- [ ] Check that the 110 MB dev executable (32 MB resident) shrinks in the release build, and measure release numbers.
- [ ] Repeat on the Intel Macs (no `/proc`; use `footprint` or Activity Monitor).
