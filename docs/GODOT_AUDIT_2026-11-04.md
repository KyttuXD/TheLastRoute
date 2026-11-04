# Godot vs Rust audit, 2026-11-04

Static read-only audit of six areas (nothing built or run). Builds on [PARITY.md](PARITY.md), [GODOT_RESYNC_2026-11-04.md](GODOT_RESYNC_2026-11-04.md) and [TODO_AUDIT_2026-11-04.md](TODO_AUDIT_2026-11-04.md); TODO references below point at [TODO.md](../TODO.md). Line numbers come from the audit reads and may drift while other agents edit. "Todo" column: tracked = already in TODO.md, partly = related item exists but not this detail, new = not tracked.

## 1. Summary

- The player mesh, atlas and scale are a faithful port (bbmodel and atlas byte-identical to Godot; 1.90 m). What is wrong is everything that moves or varies: runners and bots are posed by one sine wave on 4 bones instead of the 13 authored clips, remote moving/sprint/crouch state is dropped, there is no slim-arm mesh, skin validation is header-only, no first-person hands, no name tags, no held item on teammates.
- Hunters: Nyxie, Solyth and the 12 void hunters have no rig (stiff fallback), emotes are decided by the brain but never played, hidden hunters vanish instead of ghosting at 28%, Kathy's scythe and beam are not drawn.
- Movement: taser uses the wrong stun key, sprint/walk is changed on stairs, held Space auto-hops, no hard-landing stun, crouch eye too high, flashlight cone wider than Godot.
- HUD: no battery/exposure/noise meters, no thermal overlay, controls cannot be rebound, pause-on-focus-loss default differs.
- World: the mansion/hotel luxury door loads a different model than Godot and fails silently if missing (likely the invisible mansion door report).
- Co-op: hiding state is not replicated, hunters skip locker checks for remote-hunter sessions, pull-out and enter/leave hiding are instant.
- Stale TODO lines to close: 770 (grab files, all 40 exist), 778 (FOV max is already 105), 348 (reduced motion exists).

Counts by severity (as reported by each area; "medium-high" counted as medium, "visible" items counted by their stated rating):

| Area | High | Medium | Low |
|---|---|---|---|
| 1 Player model | 3 | 8 | 3 |
| 2 Hunters | 3 | 4 | 3 |
| 3 Movement/camera/items | 2 | 4 | 4 |
| 4 Menus/HUD/settings | 3 | 2 | 8 |
| 5 World look/sound (partial) | 2 (door items) | 4 | 2 |
| 6 Hiding/interaction/co-op | 3 | 4 | 1 |

## 2. Ranked fix list

Effort S/M/L. Kind: data (JSON/assets) or code.

### 2.1 Player model chain (full, KT asked for it)

| # | Title | Why the player notices | Godot vs Rust evidence | Fix | Effort | Kind | Todo |
|---|---|---|---|---|---|---|---|
| P1 | Runner and bot bodies use a sine wave, not the player GLB's 13 clips | Teammates and bots walk stiff-legged; arms over-swing (about 31 deg vs Godot 16 deg), legs about 25% short, no knee/elbow bend, no pelvis bob, no sprint lean, no idle breathing; stopped bots are statues | Godot: `player_mannequin.glb` clips idle 2.4s, walk 1.0s, chase 0.5s, sprint 0.67s, crouch, stun, stun_recover, tpose, sit, lay, jump, fall, climb; chosen at `player_controller_3d.gd:1923-1935`, played via `model_slot.gd:424`. Rust: `net/runner_figure.rs:112-133` rotates only arms and thighs about X; `render/anim/player.rs` and `anim/clip.rs` exist but nothing loads the player GLB (`assets/hunters/anim` has 24 hunter JSONs only) | Run `extract.py` on the player GLB into `assets/player/anim/player_mannequin.json`, load as a Rig, drive each figure with an AnimPlayer like hunters; delete the sine path and `models.json` bones | M | code + one generated JSON | partly (TODO 924, 319); clip list and over-swing new |
| P2 | Remote runners always draw standing still | A sprinting/crouching friend looks like a gliding statue; crouch not ghosted; chase tells lost | Godot: `remote_players.gd:572-573, 736-763` plays the sender's clip, crouch = idle at 0.55 fade (`:755`, `model_slot.gd:444`). Rust: `net/world_sync.rs:70-85` Sample has moving/sprinting/crouching; `:1061-1067` RemoteView copies only peer,x,y,yaw,skin; `runner_figure.rs:196-198` uses (0,0) swing | Add moving/sprinting/crouching and previous position to RemoteView, map to a clip name, crouch plays idle at 0.55 alpha | S after P1 (M alone) | code | partly (TODO 319); sprint/crouch/fade new |
| P3 | No slim (Alex) arm model; ARMS setting not saved | Slim skins smear across 4 px arms, transparent 4th column shows; ARMS button does nothing | Godot: `player_mannequin_slim/` GLB, `player_skin.gd` is_slim_layout (~196) and set_arms (~395), `Settings.effective_slim()`, `remote_players.gd:1265`. Rust: missing; `shared/skins.rs` has no slim detection; `settings_panel.rs:350` "Not saved by this build yet" | Copy Godot's slim model, add is_slim_layout and a slim flag in the wire profile, pick mesh per figure, save the setting | M | code + copied asset | new |
| P4 | Skin validation is header-only | Old 64x32 skins refused; transparent body pixels punch holes or make runner partly invisible | Godot: `player_skin.gd:~55-85` decodes, accepts 64x32 (upgrade_legacy), forces alpha 1.0 outside overlay rects (hat 32,0,32,16; jacket 16,32,24,16; sleeves 40,32 / 48,48; pants 0,32 / 0,48), re-encodes, hashes canonical bytes. Rust: `shared/skins.rs:29-37` IHDR check only, `:513-530` no decode, hashes raw bytes; shader discards alpha < 0.5 (`render/shader.rs:55`) | Decode, upgrade 64x32, clamp base alpha, re-encode, hash canonical bytes; match the server check | S-M | code | new |
| P5 | No first-person hands | Held items float with no arm; skin never seen in first person; no hands on ladder rungs | Godot: `player_controller_3d.gd:2549-2615` (FP_HAND_SCALE 0.52, tilt 118 deg, forearm at R-HandProp), `:1448-1485` climb hands (scale 0.3, tilt 105, offset 0.2/-0.06/-0.45, alternating per 0.3 m rung). Rust: `render/viewmodel.rs:99-167` item only; `game/ladder.rs` no hands | Build forearm+hand from the mannequin with skin texture, draw in `draw_viewmodel`; add two mirrored hands for ladders | M | code | partly (TODO 433) |
| P6 | Remote runners and bots snap, sit at y=0, no turn limit | 20 Hz jitter; teammates never rise on stairs/jump/fall/climb; instant turns | Godot: `remote_players.gd:35-36` INTERPOLATION_SPEED 14, MAX_TURN_RATE 4 pi, `:722-725`, `:834-871`, Y at `:713-715`. Rust: `runner_figure.rs:195` fixed y 0 and raw yaw; `draw_bots :238` same; Sample has no height | Interpolate x,y,yaw with the Godot constants, add height to Sample/RemoteView | M | code | partly (TODO 741); height/turn limit new |
| P7 | No 3D name tag over teammates | Cannot tell which co-op player is which; peer colour missing | Godot: `remote_players.gd:966-981` Label3D at 1.92 m, billboard, no depth test, font 24, outline 5, upper-case max 32 chars, colour lightened 0.35; "z Z" sleep tag 2.25 m (`:809-818`). Rust: missing (`runner_figure.rs:196-222`) | Project (x,1.92,y) to screen and draw name in peer colour; drop unused `skin_colour` palette (`world_sync.rs:66-69, 1478`) | S | code | partly (TODO 320) |
| P8 | No hand anchor or held item on teammates | Teammate's torch/crowbar not visible to others | Godot: GLB L/R-HandProp and Eye-Attachment nodes, `remote_players.gd:995-1180` HeldItemPoses third-person transform. Rust: no locators in bbmodel, nothing drawn | Add R-HandProp point at R-Hand origin, draw item via `item_model()` with the third-person transform; send selected slot | M | code + wire field | new |
| P9 | Per-state poses and ghost look missing (grab/carry, ladder, sit/lay, stun, downed, ghost fade) | Grabbed friend keeps walking in place; downed/dead friends do not lie down or go ghostly | Godot: `remote_players.gd:872-920` grab and climb poses, SIT_CLIP_SEAT 0.498, `player_controller_3d.gd:944-955, 1655` GHOST_FADE 0.55. Rust: missing; `render/anim/grab.rs` notes runner body not ported | After P1/P2 add a state enum on the wire, route as Godot, use grab.rs runner pose, add ghost fade uniform | L | code | partly (TODO 125, 428, 433, 703, 807) |
| P10 | Walk/sprint clips not paced by ground speed | Feet skate over the floor | Godot: `player_stride.gd:1-24`, `remote_players.gd:1192-1215`, `stride_table.json`. Rust: fixed 2.4 rad/m in `models.json`; stride chooser exists only in `render/anim/hunter.rs` | Add the `player_mannequin` stride entry and reuse the hunter chooser per figure | S after P1 | code + small data | new |
| P11 | Skin menu preview is flat 2D, no TEMPLATE save | Cannot see back/side/overlay layers or get the template to paint on | Godot: `skin_panel.gd` rotating 3D preview, `player_skin.gd:~223` template_png. Rust: `settings_panel.rs:589-601` draw_front, `~:676` TEMPLATE "not in this build yet" | Render the runner model to a target while rotating; write the template by clearing overlay rects | M preview / S template | code | tracked (TODO 268) |
| P12 | No shadows or contact blob under runners/bots | Figures look pasted on the floor | Godot: remote flashlight shadows (`remote_players.gd:28-33, 1049-1085`). Rust: no shadow code in render/ | Blob quad under each figure (S); real shadow pass later (L) | S / L | code | new |
| P13 | Runner body about 1.5% too small | Barely visible, about 3 cm | Godot: head top 113.92 units x SCALE 1.067 = 1.899 m (`player_models.gd:11-14`). Rust: `shared/assets.rs:66-72, 359-373` scales the hat shell (115.7) to 1.90, head top 1.871 | Use scale 1.067/64 or measure the head cube | S | code | new |
| P14 | Viewmodel lacks lockpick and revive-kit poses | Nothing in hand for those tools once they exist | Godot: `held_item_poses.gd:20-30` (lockpick view 0.25,-0.26,-0.52 scale 0.56; revive_kit 0.24,-0.40,-0.56 scale 0.55). Rust: `viewmodel.rs:20-29`, `anim/held.rs` six items | Add the two ids and pose rows | S | code table | new |

Not compared in the player area: eye height from Eye-Attachment nodes (PARITY.md:65), doorway pose, revive/downed visuals, VR pose sync.

### 2.2 Other areas, ranked (visible/gameplay first)

| # | Area | Title | Why the player notices | Godot vs Rust evidence | Fix | Effort | Kind | Todo |
|---|---|---|---|---|---|---|---|---|
| H1 | Hunters | Nyxie, Solyth and 12 void hunters have no rig | Stiff swinging block; no stun slump, chase reach, head turn, face change | Godot: `assets/hunters_blocky/<id>/<id>.glb`, `hunter_view_3d.gd:1708-1829`. Rust: `anim/` has 24 JSONs, none for these; `render/scene.rs:549-577` fallback, `bbmodel.rs:459-466` | Run `extract.py` on the 14 GLBs into `anim/<id>.json`; check fallback never reached | M | data + small code | partly (TODO 580, 581 cover pounce/incursion only) |
| H2 | Hunters | Hunter emotes decided but never shown | No waves, laughs, shrugs; only sound radius | Godot: `hunter_view_3d.gd:1426-1460`, emotes in every GLB (17-19 clips), `emotes.json` 41 KB. Rust: `hunters/brain.rs:649-676` sets emote; `render/anim/hunter.rs` Input has no emote; clips absent; `data/hunters/emotes.json` 6.7 KB | Re-extract with emote_* clips, add emote_clip/emote_face to Input, play per Godot rules | M | code + data | partly (TODO 769: catalogue done, playback missing) |
| H3 | Hunters | Hidden hunters vanish instead of ghosting at 28% | Void Blue/Gray/Green and Verrine in ducts fully invisible; Ilyra drawn opaque dark | Godot: `hunter_view_3d.gd:1090, 1112-1135` MIN_VISIBLE_ALPHA 0.28. Rust: `render/scene.rs:524-528, 540-541, 676-679`; `gimmicks/mod.rs:360-371` | Draw 28% alpha for every hidden_from_view hunter except dispersed Ceryth; drop Ilyra special case | S-M | code | new |
| H4 | Hunters | Unrigged hunters drawn at wrong height | Nyxie (3.14 m), Solyth (3.19 m) about 18-19% short; void about 8% short | Godot: `hunter_roster.gd:185, 212` MODEL_SCALE 1.5. Rust: `shared/assets.rs:210`, `config/mod.rs:124`, `bbmodel.rs:337-339` normalise to 2.58 | Moot after H1; meanwhile scale by MODEL_SCALE/65 | S | code | new |
| H5 | Hunters | Void emissive glow layer missing | Void hunters look flat in the dark | Godot: `void_<colour>_emissive.png` x12. Rust: none; shader has unlit mode 2 | Add emissive PNGs and a second additive pass | M | data + code | new |
| H6 | Hunters | Kathy's scythe prop and clips unused | Armed-carry pose and weapon cue missing | Godot: `hunter_view_3d.gd:747-762, 1777-1784`, SCYTHE_CHASE_SPEED 159. Rust: `anim/kathy.json` has scythe_* clips, no code chooses them or draws the prop | Seeded carry flag, switch to scythe_* clips, draw model at R-HandProp | M | code | new |
| H7 | Hunters | Kathy's eye beam not drawn | Cannot see where she aims | Godot: `hunter_view_3d.gd:771-870, 956`. Rust: `hunters/gimmicks/kathy.rs` rules only; no beam in `render/scene.rs` | Add lamp and additive frustum mesh from eye bone | M | code | new |
| H8 | Hunters | Wolf spiders have no model/rig/draw path | Inferno spiders do not exist | Godot: `assets/city/*_wolf_spider/*.glb`, `creature_view.gd`. Rust: only grab JSON | Port GLBs and a creature clip chooser | L | code + data | partly (TODO 633; visuals not) |
| H9 | Hunters | Ceryth draws cube eyes Godot removed | Two floating coloured blocks | Godot: `hunter_view_3d.gd:778-792`. Rust: `render/scene.rs:676-695` | Remove cubes | S | code | new |
| H10 | Hunters | Addon system and Night Watch pose table absent | Nothing yet; vent/hide/sit/sleep poses will lack look later | Godot: `hunter_addons.gd`, `night_watch_poses.json` (21 poses). Rust: none | Port pose table when those features land | M | data + code | partly (TODO 458-461, 607) |
| M1 | Movement | Taser stun uses escape_stun | Stun 0.2 s too long; message hides duration | Godot: `main_3d.gd:1868, 1880`, `game_config.gd:26-56`. Rust: `game/items.rs:154-155` | Use `prof.taser_stun`, format "STUNNED %.1fs" | S | code | partly (TODO 594; PARITY.md:80) |
| M2 | Movement | Sprint/walk speed changed on stairs | Cannot sprint on stairs; held to walk pace | Godot: `player_controller_3d.gd:1875-1884`. Rust: `game/player.rs:330-333` | Remove on_stair terms | S | code | new |
| M3 | Movement | Held Space auto-hops | Hops on every landing | Godot: `player_controller_3d.gd:2093-2118` just_pressed. Rust: `app/mod.rs:167`, `game/player.rs:406` | Derive a jump edge in read_controls | S | code | new |
| M4 | Movement | No hard-landing fall stun | Long drops cost nothing | Godot: `player_controller_3d.gd:2109-2126`, threshold 8.25 m/s, `game_config.gd:107-110`. Rust: `game/player.rs:403-411` | Track peak vy, apply scaled stun and message | M | code | new |
| M5 | Movement | Crouch eye not capped under crouch hitbox | Crouched camera about 10 cm higher | Godot: `player_controller_3d.gd:1001-1002, 41`. Rust: `config/mod.rs:19-21`, `game/player.rs:208` | Add min() with CROUCH_COLLISION_HEIGHT - 0.08 (about 1.072 m); confirm standing height first | S | code | new |
| M6 | Movement | Flashlight cone wider than Godot | Beam wider and softer | Godot: `main_3d.tscn:61-63` spot_angle 30, attenuation 1.4. Rust: `render/shader.rs:76-77` hard-coded | Move cosines into config, compare screenshots (check FP flashlight node angle first) | S | code | new |
| M7 | Movement | Mouse look scaled to one display | Turn rate differs per monitor | Godot: `player_controller_3d.gd:1115-1116`. Rust: `config/mod.rs:27-35`, `app/mod.rs:209` | Use 1.0 or derive from DPI; KT decision (deliberately measured) | S | code/config | new |
| M8 | Movement | Bob phase held, not eased | Small eye jump on restart | Godot: `player_controller_3d.gd:1009-1011`. Rust: `game/player.rs:237-241, 417-419` | Ease phase at 10 rad/s | S | code | partly (TODO 14) |
| M9 | Movement | Bob rates, sprint drain, troll/wading (confirmed open) | Bob cadence and overheat drain differ | See PARITY items 10-12, 15-18 | Per PARITY | M | code | tracked (TODO 14, 760, 761) |
| M10 | Movement | Vent lamp sprint surcharge (confirmed open) | Lamp drains faster in vents | Godot: `player_controller_3d.gd:2407-2420`. Rust: `player.rs:382-385` | Skip surcharge in vents | S | code | tracked (TODO 885) |
| U1 | HUD | Battery, exposure, noise meters missing | No lamp charge, exposure, noise feedback | Godot: `hud/hud.gd:748-751, 780, 786, 144-149`. Rust: `ui/hud.rs:200-300, 424-437` | Add three captioned meter rows in Godot order | M | code | new |
| U2 | HUD | Thermal warning overlay missing | No visual heat/cold warning | Godot: `hud/hud.gd:26-40, 259-290`. Rust: only `game/meters.rs:58-68` | Screen-edge tint/wobble from heat/cold | S-M | code | new |
| U3 | Settings | Controls tab read-only, no rebinding | Hint promises rebind that does nothing; some labelled keys have no effect | Godot: `settings_panel.gd:701-757`, `settings.gd:70-78`. Rust: `ui/settings_panel.rs:52, 55-90, 803`; `app/mod.rs:1135` | Bindings store with defaults, CHANGE/+/RESET rows, route input through it | L | code | new |
| U4 | Settings | pause_on_focus_loss default differs | Alt-tab pauses by default in Rust | Godot: `settings.gd:117` false. Rust: `app/settings.rs:63` true | Default false; check test/migration | S | data | new |
| U5 | Settings | Waypoint guidance settings absent | No mode/distance/scale/opacity/chime | Godot: `settings.gd:168-178`, `settings_panel.gd:656-681`. Rust: only TEAMMATE WAYPOINTS (`settings_panel.rs:850`) | Add rows and values; wire when overlay exists | M | code + data | partly (RESYNC 34) |
| U6 | HUD | Player list PULSE column missing | No pulse indicator | Godot: `player_list_panel.gd:124`. Rust: `ui/menus.rs:1699` | Add column | S | code | new |
| U7 | Menus | Pause/dossier labels differ | Different wording; missing chart/secret lines | Godot: `pause_menu.gd:419, 465, 536`. Rust: `ui/pause.rs:296` | Copy strings, add chart if in scope | S | data | partly (TODO 16, 274) |
| U8 | Settings | Category tabs differ; no SFX VOLUME row | Different grouping | Godot: `settings_panel.gd:57, 69-70, 181`. Rust: `ui/settings_panel.rs:23` | Rename/regroup, add row | S | code | new |
| U9 | Settings | Pixel effect and player hue absent | Cannot change them | Godot: `settings.gd:32, 88, 149, 446-447` (no UI row found). Rust: `app/settings.rs:15-45` | Add fields; rows only if Godot shows them | S | data | new |
| U10 | Settings | Touch controls absent | Phone options missing | Godot: `settings.gd:95-102`, `settings_panel.gd:349-352, 583-590` | With touch work | L | code | tracked (TODO 264) |
| U11 | Settings | Subtitles/voice toggles do nothing | Dead toggles | `app/settings.rs:1-7`, `settings_panel.rs:929-966` | Hide rows until voice chat exists | S | code | partly (TODO 116) |
| U12 | Docs | Reduced motion TODO stale | None | `app/settings.rs:45-46`, `settings_panel.rs:861`; TODO 348 | Close TODO 348 and confirm HUD honours it | S | docs | stale |
| W1 | World | Luxury mansion/hotel doors use a different model; silent failure | Mansion door may draw as nothing (open doorway) | Godot: `blocky_prop_models.gd:301-302` (blocky GLBs), `streamed_route_generator.gd:252-261`. Rust: `data/props/p04.json:97-99, 115-117` point at `assets/props/mansion/*.bbmodel`; `world/route/grid.rs:53-54`; manifest has separate `props/luxuryRoomDoor.bbmodel` (`manifest.json:4090-4110`); `shared/assets.rs:118-142` keys by path; `render/world_mesh.rs:739-746` returns None silently; no mansion path in manifest | Pick canonical door, point p04 at it, add a startup validator and a warning when `prop_model_key` is None for a door; then run a mansion level | S | data + code | new (TODO 659 is thresholds only) |
| W2 | World | Lava loop kind missing | No lava hum | `audio/cues/ambience.rs:7`; Godot `emitter_loops.gd:29, 44-45` | Add kind when lava cells land | S-M | code | tracked (TODO 804, 815) |
| W3 | World | Sewer bed missing | Silent sewer | Godot `level_ambience.gd`; Rust `ambience.rs` | Map a sewer bed | S | data | tracked (TODO 797-802) |
| W4 | World | Flood wading tone/slowdown | Flood feels dry | Godot `player_controller_3d.gd:336-338, 365-372, 1237-1256` | Depth-based speed, splash, noise | M | code | tracked (RESYNC 2) |
| W5 | World | Ambient 0.35 vs Godot 0.035 | Dark corridors brighter | TODO 276, 282 | Keep only if KT confirms | S | data | tracked, deliberate |
| W6 | World | Fog/far-plane look not compared | Unknown | TODO 196 (cost only) | Compare Godot environment to shader | S | data | partly |
| W7 | World | Per-area room tones | Rooms sound alike | `ambience.rs`; Godot `level_ambience.gd` | Per-region tone table | M | data | tracked (TODO 72) |
| C1 | Co-op | Remote hiding state not replicated | Hidden runner visible in corridor or vanishes | Godot: `remote_players.gd:501-504, 556, 602-604, 737-746`. Rust: `net/protocol.rs:112` HostHidden only; no hide fields in `net/` | Add hidden, hiding_spot, hiding_style, hide_slot; skip drawing while hidden | M | code | partly (TODO 270) |
| C2 | Co-op | Hunter locker check skipped for remote hunters | Hiding is free in those sessions (scope unverified) | Godot: `infinite_route_test.gd:3626-3657, 3699-3704`. Rust: `game/hunting.rs:~376` early return on `remote_hunters` | Confirm what `remote_hunters` gates, route through host hide state, port front-slot rule | M | code | new |
| C3 | Co-op | Hunter pull-out is an instant ejection | Snapped out in one frame | Godot: `infinite_route_test.gd:3690-3704`, `route_interactable_3d.gd:800`, `player_controller_3d.gd:764-805`. Rust: `game/hunting.rs:~375`, `game/interact.rs:457-463` | Forced leave through a short move timed to the door animation | M | code | partly (TODO 403, 270) |
| C4 | Co-op | Enter/leave hiding instant | Camera jumps | Godot: `player_controller_3d.gd:707-830`. Rust: `game/interact.rs:447-448, 457-463` | Port `_begin_hide_move` curve, durations in data | M | code | tracked (TODO 403) |
| C5 | Co-op | Locker capacity/slot absent | No two-hider case | Godot: `route_interactable_3d.gd:161-162, 487-505, 715, 800`. Rust: `game/player.rs:72`, `game/interact.rs:430-449` | Slot in `hidden`, read hide_capacity, occupancy check | M | code | tracked (TODO 23, 410) |
| C6 | Co-op | Search tell not visible | No warning that a locker is checked | Godot: `route_interactable_3d.gd:518-560`, `infinite_route_test.gd:5676`. Rust: `game/hunting.rs:~420-428`, `render/scene.rs:326-351` | Add shake offset and creak, broadcast | S | code | partly (TODO 606, 20) |
| C7 | Co-op | Map pings not drawn | Ping only in chat | `net/chat.rs:37, 93-104, 160-165`; no draw in `ui/map.rs` | Draw on minimap and floor | S | code | tracked (TODO 255) |
| C8 | Co-op | Nametag | Same as P7 | Same as P7 | Same as P7 | S | code | duplicate of P7 |

## 3. Proposed work batches

Disjoint file ownership. Hub files (`net/world_sync.rs`, `net/protocol.rs`, `app/mod.rs`, `app/settings.rs`, `render/scene.rs`, `config/mod.rs`, `TODO.md`) go through one integrator pass at the end; batches send the integrator small patches or notes instead of editing hubs. Naming follows the other passes (edit pass, then test-fixer).

1. **Batch 1, player-model chain** (P1, P2, P3, P4, P6, P7, P8, P10, P13). Owns `net/runner_figure.rs`, `render/anim/player.rs`, `render/anim/clip.rs` (runner parts only), `shared/skins.rs`, `shared/assets.rs` (runner load only), `assets/player/**`, `data/player/models.json`. Sub-order: extract the player GLB clips and slim model first (data), then clip driver, stride, then RemoteView fields, interpolation, name tag. Wire fields and `RemoteView` edits via the integrator.
2. **Batch 2, first-person and held items** (P5, P12 blob, P14, P11). Owns `render/viewmodel.rs`, `render/anim/held.rs`, `game/ladder.rs` (draw only), `ui/settings_panel.rs` skin preview part. Depends on Batch 1 for the skin texture and slim mesh.
3. **Batch 3, hunter rigs and emotes** (H1, H2, H4, H9). Owns `assets/hunters/anim/**`, `render/anim/hunter.rs`, `render/anim/extract.py`, `data/hunters/emotes.json`. Scene hook for emote Input goes through the integrator.
4. **Batch 4, hunter gimmick visuals** (H3, H5, H6, H7). Owns `hunters/gimmicks/**` draw hooks, void emissive assets, scythe/beam draw code. `render/scene.rs` edits via the integrator.
5. **Batch 5, movement feel** (M1 to M8, M10). Owns `game/player.rs`, `game/items.rs`, `game/meters.rs`. Hub edits (`app/mod.rs` jump edge, `config/mod.rs` crouch eye and mouse scale) via the integrator.
6. **Batch 6, HUD and settings** (U1, U2, U4 to U9, U11). Owns `ui/hud.rs`, `ui/menus.rs`, `ui/pause.rs`, `ui/settings_panel.rs` (non-skin parts), `ui/style.rs`. `app/settings.rs` fields via the integrator. U3 (rebinding) as a follow-up L batch.
7. **Batch 7, world assets and validators** (W1, W3, W6, W7). Owns `data/props/p04.json`, `assets/props/manifest.json`, `world/route/grid.rs`, `audio/cues/ambience.rs`, a new door-model validator. Run a mansion level after (needs an in-game run).
8. **Batch 8, co-op hiding** (C1, C2, C3, C4, C5, C6). Owns `game/hunting.rs`, `game/interact.rs`, hide-related parts of `net/`. Protocol fields via the integrator; runs after Batch 1 lands the RemoteView changes.
9. **Batch 9, integrator** (hubs listed above, TODO.md tick-offs of stale lines 348, 770, 778, test-fixer pass).

Not batched (large, need scoping): P9 per-state poses (L, after Batches 1 and 8), H8 wolf spiders (L), U3 rebinding (L), U10 touch (L).

## 4. Decisions needed from KT

1. Which luxury door is canonical for hotel/mansion: the Godot blocky `luxuryRoomDoor` set, or the Rust `mansion/*.bbmodel` set (W1)?
2. Mouse look multiplier: keep the measured 1.75/3.0 or switch to Godot's 1.0 / DPI-derived (M7)?
3. Ambient light 0.35 vs Godot 0.035: keep as a deliberate choice (W5)?
4. Stairs: should sprint be blocked or slowed on stairs as a rule, or match Godot (no stair penalty) (M2)?
5. Slim arms: copy Godot's slim mesh and add a wire flag now, or defer until server-synced skins (TODO 914) (P3)?
6. Ghost look for hidden hunters: draw all at 28% as Godot, including Ilyra (H3)?
7. Runner shadows: accept a blob quad, or wait for a real shadow pass (P12)?
8. Pause-on-focus-loss default: switch to Godot's false (U4)?
9. Subtitles/voice toggles: hide until voice chat exists (U11)?
10. Whether TODO 770, 778, 348 may be closed as stale.
