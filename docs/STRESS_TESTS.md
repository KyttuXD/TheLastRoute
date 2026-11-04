# Stress tests

The stress mode plays a fixed-length run with a fixed seed and writes the frame timing to CSV. Because the seed is fixed, two runs on the same machine fly the same route, so the numbers can be compared.

## Running the mode

```sh
./target/release/last_route --stress 60 --stress-log run.csv
```

- `--stress <seconds>` sets the run length. The Intel Mac check uses 60.
- `--stress-log <path>` sets the CSV file. The header row names the columns; that row is the reference for them.
- `--stress-scenario <name>` picks what the run does: fly (default), npcs, hunters, interact or mixed. See below.
- `--stress-vsync on|off` sets vsync for the window and the live setting (default off, so the frame rate is not capped at the display refresh rate). The saved settings file is never written.
- On macOS, `--stress-vsync off` also drops the run loop's display-link wait, so it is not capped at the refresh rate (see docs/GRAPHICS_API.md). `LR_GFX=metal` or `LR_GFX=gl` (or `--gfx metal|gl`) picks the graphics API; the default is Metal on Apple Silicon and OpenGL on Intel.
- The summary line ends with `scenario=` and `vsync=`. The CSV columns are the same for every scenario.

## Scenarios

- fly: noclip, 4x speed, no extras (the flight below).
- npcs: noclip off, walks the same route at normal speed, 8 NPCs kept near the runner (spawned with the admin spawn when fewer, the farthest removed when more).
- hunters: noclip off, walks the route, 6 active hunters kept up with the admin summon placement. A catch still ends the run.
- interact: noclip off, walks the route and presses interact every 3 s when the crosshair is on something. The tool swing is not driven: the swing in `game/items.rs` only hits hunters, there is no barrier or prop swing to call.
- mixed: fly for 20 s, then npcs, hunters and interact in turn, 10 s each. The player bots (NPCs) and hunters stay once spawned.

## The flight

The fly scenario flies the way the Godot flythrough does. The runner has admin noclip on, so walls do not stop it, and the run speed is 4x (`FLY_SPEED` in `app/stress.rs`). The admin preferences file is not written, and sprint stays off.

The route goes straight for 20 s (`LEG_SECONDS`), then turns 110 degrees (`TURN_DEGREES`) at each leg boundary. Each turn's direction comes from the fixed seed. The route is a function of the elapsed time, so the same seed gives the same flight. A run that a hunter catches ends there, and the summary covers the run up to the catch.

The CSV's last column, `travelled_m`, is the metres the runner has flown so far in the run (1 cell = 1 m). It is counted from the runner's absolute cell, so it keeps counting when the infinite route moves the window.

## Running on KT-MBA

KT-MBA is the Intel MacBook Air (macOS 12.7.6). Its copy comes from git, so commit first: `git archive HEAD` leaves out uncommitted changes.

```sh
ssh kt-mba 'mkdir -p $HOME/lr-stress'
git archive HEAD | ssh kt-mba 'tar -x -C $HOME/lr-stress'
ssh kt-mba 'sh $HOME/lr-stress/tools/stress/run_kt_mba.sh'
```

The script builds the release binary into `$HOME/lr-stress-out/build.log`, then runs every scenario in both vsync modes (10 runs of 60 s, with `caffeinate -d` when present) and prints each run's summary line. Each run writes `run-<scenario>-vsync<on|off>-<stamp>.csv` and a matching `.out` to `$HOME/lr-stress-out/`. Limit the list with `SCENARIOS` and `VSYNC`, e.g. `ssh kt-mba 'SCENARIOS="fly mixed" VSYNC=off sh $HOME/lr-stress/tools/stress/run_kt_mba.sh'`.

The game opens a window, so the run needs a logged-in desktop on the Mac. If it fails with no window over ssh, run the script from Terminal on KT-MBA instead.

## Target

100+ FPS on KT-MBA with vsync off (the fly scenario). The target and the open items are in the "Intel Mac FPS" section of [TODO.md](../TODO.md). The triangle counts for the Intel Mac work are in [PERF_TRIANGLES.md](PERF_TRIANGLES.md).

The runner script is [tools/stress/run_kt_mba.sh](../tools/stress/run_kt_mba.sh).
