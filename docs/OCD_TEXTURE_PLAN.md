# OCD texture plan: flat world textures to match the hunters

Based on commit 3ecd51b. All figures come from five investigations and their skeptic checks. I did not re-measure anything myself.

## 1. Bottom line

- **What "OCD-ify" means here.** Remove the shading baked into the world's texture pixels. That means grain, painted bevels, highlight and shadow edges, grime streaks, rivet shading, and the 16 px swatch bevels on props. Replace it with hard-edged flat fills of a few colours each, as the hunter atlases already are. It does not mean making the renderer unlit.
- **The pack's pixels cannot be used. Only its style can.** The zip has no licence, README or credits, only `pack.mcmeta` ("re-OCD pack v26.2 disco/Sanhoshi 2026"), `pack.png` and `assets/`. The pack investigator found that Modrinth lists Apache-2.0 and credits disco_ (FVDisco) as the original author. That is secondhand from a web lookup and I have not checked it myself. Even if accurate, an Apache label cannot grant rights the packager may not hold, either from disco_ or from Mojang. I could not check Mojang's terms. The pack is 16x16 Minecraft block art (stone_bricks, bookshelf and so on). It has no counterpart for LR's 99 painted kinds (concrete0, carpet_tile0, parquet0 and so on), nor for cornice, rail and skirt walls. Treat it as a style spec and do not ship any of it.
- **Effort.**
  - First visible slice: about 1.5 days.
  - World surfaces only (floors, walls, ceilings, ducts): about 5 days.
  - Plus props and icons: about 8-9 days in total.
  - Plus the `lr_story` copy and an optional renderer pass: about 10 days.
  - Verdict: L, edging toward XL if everything is included. The investigators ranged from 3-4 days to 7-11 days. The low estimates covered the wrong files or ignored props and the duplicate painter.
- **Biggest risk: readability of the dark maze, not the code.** Grain and bevels currently carry much of the visible structure. Hunters are bright and saturated (mean luminance 137, chroma 54), while the world is dark and muted (mean luminance 52, chroma 20). Flat dark colours can merge into one value. Plan for a per-kind colour retune.

## 2. What "shading" is in this game

**(1) Baked into pixels. This is the target.**
- The 99 world kinds are painted at runtime and ship zero bytes: 52 floor, 38 wall, 6 ceiling and 3 duct (`render/textures/mod.rs:39-149`). Floors are 32x32 and walls 32x128. They are uploaded once, nearest-sampled, with no mipmaps (`surfaces.rs:13-47`).
- Sources of shading:
  - `canvas.rs:163 noise()`, a three-step grain of ±10-22%, with about 36 call sites. `wall_base` applies ±10% to every wall (`canvas.rs:263-266`).
  - Cornice 0.45x, highlight 1.4x, skirt and grime streaks (`canvas.rs:268-300`).
  - Rivets at 0.35x/1.7x (`canvas.rs:182`).
  - Metal bevels (`floors.rs:34-39`).
  - Per-painter jitter in glass and rockwall (`ceilings.rs`, `walls.rs:432`).
- Prop textures live as base64 inside the .bbmodel files:
  - The shared 128x64 `blocky_atlas.png` has 305 colours.
  - There are about 431 per-model palette strips (16 px high, widths 96-192), with a baked light top-left and dark bottom-right 1 px bevel per 16x16 swatch.
- The 7 inventory icons (96x96) are pre-rendered 3D with baked three-tone face lighting.

**(2) Computed by the renderer. This stays.**
- Lamp: `ndl = max(dot(n,L),0)*0.7+0.3` (`shader.rs:92`). Flashlight: `0.25+0.75*ndl` (`shader.rs:104-105`). Ambient has no normal term.
- World quads are passed WHITE vertex colour (`world_mesh.rs:1617` and others).
- Exceptions: the stairs depth `shade()` (`stairs.rs:137`), the waterfall streak (`scene.rs:957`) and a constant ceiling colour (`world_mesh.rs:1570`).

**(3) Model-level.** None of significance. `prop_model.rs:112-130 soften()` blends normals, but it is applied only to `props/blocky/*` (`assets.rs:172`). No model sets `shade:false`.

**How the hunters avoid it.** There is no material trick. Hunters go through the same lit shader as the world (`bbmodel.rs:406` → `LIT_MODE`, `shader.rs:25`), with the same lamps, flashlight, fog and ambient. They look unshaded only because their atlases are flat: 10-24 colours, with 0-3% gentle gradient pixel pairs. This also means "unshaded" is not literally true for hunters. They still get face-dependent lamp light. That is why a global "ndl=0" would be wrong (see the disagreements below).

## 3. Disagreements between investigators, resolved

| Question | Positions | Resolution |
|---|---|---|
| Are `environment/*.png` what draws? | The pack skeptic said the fallbacks are nearly irrelevant. The tooling skeptic said they are not unused. | Both are right. `assets.rs:212-215` loads them, but `world_mesh.rs:1160-1166` paints the 99 kinds first and uses the PNGs only as a fallback and for custom maps. They are cheap (26 files, 13 KB), so flatten them in Phase 3 and do not rely on them. |
| Do we need shader work? | The renderer area proposed a `Shade` uniform and ndl softening. The skeptics said no shader work is needed. | A texture change alone is enough for the request. The `Shade` uniform would also flatten hunters unless it is per class, and it touches the `mode > 3.5` ghost test and three MSL/GLSL parity tests. Make it an optional Phase 5, gated on screenshots. |
| How much does ndl matter? | The renderer scratch experiment found that removing the flashlight ndl doubles floor luminance (41 → 85). Flat textures alone barely move the means. | Flattening removes noise, not value structure, so keep the ndl terms. Dropping ndl is a lighting redesign, not part of this task. |
| Where does the flatten live? | The textures investigator edits the painters. The tooling investigator wants a post-process stage after `paint()`. | The prototype that kept RNG draws and zeroed `noise()`/grime took painted kinds from 84 heavy to 9 and the mean flatness metric from 0.44 to 0.11. Use that as the base: a flat flag in the painters that still consumes RNG, so layouts do not shuffle. Add a deterministic post-process snap pass, driven by a per-kind override table, for residual bevels. Keep the old path behind the flag so `matches_godot_dump` (`mod.rs:320`, gated on `LR_TEX_DUMP`) still passes with the flag off. |
| Gate metric | Flat share ≤0.10 (textures) versus soft fraction ≤0.03 (tooling). | Different definitions. The pack median is 0.13 on one and 0.028 on the other. Phase 0 must write one measurement script and calibrate it against hunters, then freeze the thresholds. Until then, treat both numbers as indicative. |
| Prop effort | Textures said strips plus atlas take 0.5 days. Models said 2 days. Tooling said 3-5 days. | Use 2.5 days. Only 139 unique textures exist, but they need a visual pass. The tooling skeptic showed the soft-pair gate misses the hard 1-2 px bevel ring on every 16 px swatch, so the gate needs an edge-ring check. |
| Download size | Textures: about 8 MB of base64 from 571 blocky-atlas copies. Models: 3.24 MB. Tooling: about 2.4 MB gzip saving on a 6.67 MB gz wasm. | Unresolved, since the counts conflict (571 versus 220 models; 627 embedded versus 1047 total). Directionally it is a modest win, not the motivation. Phase 0 measures a real build before and after. |
| Is the colour merge usable? | Textures: it over-flattens concrete and vinyl. Tooling: concrete, vinyl, carpet_tile1, rock0/1 and sand all collapse to one colour. | Agreed. Those kinds keep their seams and dunes as hand-set 2-3 tone shapes. Hedge (114 colours) never converges and needs a bespoke painter. |

Other points all five reports agreed on: `ARRAY_TINT` (1.25, 1.22, 1.3, `mod.rs:297`) is unused in the Rust renderer. Painted colours are therefore drawn un-lifted, and flat colours will look darker than the grained ones because grain averages up. Use the mean base colour and expect a brightness pass.

## 4. Options

| | Idea | Effort | Web impact | Visual risk |
|---|---|---|---|---|
| A | Renderer flag only (unlit or ndl=0) | S, 1 day | None | High. Hunters flatten too, the dark maze goes full-bright or loses its corner cues, and it does not remove baked noise. |
| B | Automated flatten only (script every texture) | M, 3 days | Props shrink | Medium. Concrete, vinyl and carpets go blank, and it does not fix runtime-painted kinds unless done as a post-process. |
| C | Hand-remake in the pack's style or import pack pixels | L-XL | Slightly bigger if PNGs are imported | Low visual risk, but blocked on licence for any pixels, and it needs a 16→32 upscale. |
| D | Combination (recommended) | L, about 8-9 days | Zero-to-small win | Controlled by the phase gates below. |

**Recommendation: D.** Do the following in order: a flat flag in the painters plus a snap pass with per-kind overrides, then a Node tool for prop atlases, strips and env tiles, then a re-render of the icons, with an optional per-class lighting nudge held back until the screenshots show it is needed.

## 5. Phased plan

Each phase ends in something KT can look at.

- **Phase 0 (0.5 day): baseline.**
  - One measurement script (flat share, colour count, an edge-ring check for 16 px cells), calibrated on hunters.
  - Contact sheets of all 99 kinds, before.
  - A real wasm build for size.
  - Game-lighting screenshots of two or three regions, before.
- **Phase 1 (1-1.5 days): first slice.**
  - A flag, `LR_FLAT` or a setting, off by default. `canvas::noise` still draws from the RNG but returns the base colour, and grime is a no-op.
  - Apply it to one region. Deliver one before/after screenshot pair at game lighting plus a floor and wall contact sheet. KT judges at this point.
- **Phase 2 (2-3 days): all 99 painted kinds.**
  - Bevels become hard-edged flat bands. Cornice, skirt and rail stay as flat colour bands.
  - Bespoke work for rockwall, glass and hedge. Hand-set seam shapes for concrete, vinyl, carpet_tile, sand and rock.
  - Per-kind override table. Mean-preserving brightness and a tint check.
  - Gate test: every kind below the frozen flatness threshold. Contact sheets per class.
  - Class order proposed: floors first, then walls, then ceilings and ducts.
- **Phase 3 (2.5-3 days): props.**
  - `blocky_atlas`, per 16x16 tile (305 → about 32 colours; plank, brick and window tiles need a per-tile preset).
  - The 26 env tiles.
  - The 139 unique palette strips and the 161 strip PNGs, cell by cell with ring-fix, rewriting the bbmodels. A Node tool under `tools/`, deterministic, with a manifest of the preset used per texture. Dimensions must stay unchanged (NPOT widths such as 144, 160 and 112 are already handled on web; `prop_model.rs:662-675`).
  - Add the flattening to the converters (`tools/props/glb_to_bbmodel.mjs`, `tools/prop_convert`), or gate their output in CI, so a re-run does not bring the shading back.
  - Eyeball the prop textures: signs, books and paintings contain real pixel art, not shading.
- **Phase 4 (0.5-1 day): the rest.**
  - Re-render the 7 inventory icons flat from their models. Quantising does not help, because the face shading stays.
  - Player mannequin atlas and held-item atlas.
  - Class-level constants: stairs `shade`/`dim` (`stairs.rs:137`), waterfall streak, ceiling colour.
- **Phase 5 (optional, about 1 day): lighting nudge.** Only if the Phase 2 screenshots show lost depth cues.
  - Per-class tints through vertex colour (ceiling about 0.55, floor 1.0, wall about 0.8), a fixed axis shade (N/S 0.9, E/W 0.8), and compressed floor ndl (0.3 → about 0.6), applied to world and props only so hunters are untouched.
  - Keep 1 px flat boundary bands at cornice, skirting and rail.
- **Phase 6 (1-1.5 days): tune and ship.**
  - Lamp and ambient QA in the playtest bridge, then flip the default.
  - Retarget or retire `matches_godot_dump` (and the `lr_story` twin), and update `NOTES.md` and `CREDITS.md`.
- **`lr_story` copy (+0.5-1 day):** `src/bin/lr_story/textures/` is a full duplicate of the painter stack, and `lr_story` has its own prop code and about 220 blocky-atlas bbmodels. The change must land twice, or the painters get deduped first.

**QA throughout:** the flatness metric above; contact sheets before and after each phase; screenshot diffs at game lighting; a check that the converter output is stable on re-run (the autotune presets must live in a manifest, because 22 of 99 results changed on a second pass without one).

**Rollback:** the flag is off by default until Phase 6. Original assets stay untouched, and the Node tool writes from archived originals. Every phase is its own commit. Mods and skin packs keep their own shaded textures.

## 6. Decisions KT must make

1. Should "flat" mean solid fills with the existing patterns kept as 2-3 tone shapes (brick joints, locker panels, rails), or truly blank surfaces? This drives the effort in Phase 2 more than anything else.
2. Keep the dark moody palette, or lift it toward the hunters' brightness and saturation? Hunters are about 2.6x brighter and 2.7x more saturated than the world.
3. Keep the lamp/flashlight face shading as it is (recommended), or soften it on world and props only (Phase 5)? Never globally.
4. Which classes first? I suggest floors, then walls, then props. Is the UI in scope? Portraits are already flat, the HUD is procedural, and the icons are the only UI-ish item that needs work.
5. Palette size per texture: 8-24 colours like the hunters, or 2-4 like the pack? I recommend the hunters' range.
6. Is `lr_story` in scope, and should the duplicate painter stack be unified first?
7. Does "byte-identical to Godot" stay as a project goal behind the flag, or retire once flat becomes the default?
8. Licence stance: style reference only (recommended), or contact the pack author and disco_ in writing before using any pixels?
9. The stairs depth shade and the hand-tuned minority of kinds: do they get flattened too, or stay as exceptions?

## 7. What not to do

- Do not use the unlit shader mode. It ignores lamps and ambient, so the maze goes full-bright, and hunters would flatten too.
- Do not ship or import any pack pixels, and do not copy Minecraft-recognisable tiles.
- Do not run a global colour merge on the painted kinds (concrete, vinyl, carpet, sand and rock collapse to one colour).
- Do not flatten the `environment/*.png` files and call it done. They are a fallback, and nearly all in-game walls and floors would stay unchanged.
- Do not change atlas dimensions or merge atlases in this effort (UV remap plus NPOT risk on WebGL1).
- Do not edit the originals, and do not flatten portraits, skins or the hunter atlases (already flat).
- Do not trust the flatness gate alone. It misses hard 1-2 px bevel rings, and swatches and the dark maze need a human look.

## Uncertainty

- The investigators' numbers conflict on embedded size and effort.
- Inventory icons, portraits and the player atlas were measured but not viewed.
- The ceiling and duct painters were not all viewed.
- The 431 palette-strip props were judged by sampling UV rectangles, not by rendered images.
- The Modrinth licence line is unverified, and Mojang's terms were not checked.

VERDICT: feasible-with-work (L; about 8-9 days all-in, first slice about 1.5 days)
