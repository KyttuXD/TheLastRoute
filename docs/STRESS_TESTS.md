# Stress tests

The stress mode plays a fixed-length run with a fixed seed and writes the frame timing to CSV. Because the seed is fixed, two runs on the same machine fly the same route, so the numbers can be compared.

## Running the mode

```sh
./target/release/last_route --stress 60 --stress-log run.csv
```

- `--stress <seconds>` sets the run length. The Intel Mac check uses 60.
- `--stress-log <path>` sets the CSV file. The header row names the columns; that row is the reference for them.
- Vsync is off for the run, so the frame rate is not capped at the display refresh rate.

## The flight

The run flies the way the Godot flythrough does. The runner has admin noclip on, so walls do not stop it, and the run speed is 4x (`FLY_SPEED` in `app/stress.rs`). The admin preferences file is not written, and sprint stays off.

The route goes straight for 20 s (`LEG_SECONDS`), then turns 110 degrees (`TURN_DEGREES`) at each leg boundary. Each turn's direction comes from the fixed seed. The route is a function of the elapsed time, so the same seed gives the same flight. A run that a hunter catches ends there, and the summary covers the run up to the catch.

The CSV's last column, `travelled_m`, is the metres the runner has flown so far in the run (1 cell = 1 m). It is counted from the runner's absolute cell, so it keeps counting when the infinite route moves the window.

## Running on KT-MBA

KT-MBA is the Intel MacBook Air (macOS 12.7.6). Its copy comes from git, so commit first: `git archive HEAD` leaves out uncommitted changes.

```sh
ssh kt-mba 'mkdir -p $HOME/lr-stress'
git archive HEAD | ssh kt-mba 'tar -x -C $HOME/lr-stress'
ssh kt-mba 'sh $HOME/lr-stress/tools/stress/run_kt_mba.sh'
```

The script builds the release binary into `$HOME/lr-stress-out/build.log`, runs the 60 second stress mode with `caffeinate -d` to keep the display awake, and prints the CSV path and the last line of the run output. The run output and CSV are both written to `$HOME/lr-stress-out/`.

The game opens a window, so the run needs a logged-in desktop on the Mac. If it fails with no window over ssh, run the script from Terminal on KT-MBA instead.

## Target

100+ FPS on KT-MBA with vsync off. The target and the open items are in the "Intel Mac FPS" section of [TODO.md](../TODO.md). The triangle counts for the Intel Mac work are in [PERF_TRIANGLES.md](PERF_TRIANGLES.md).

The runner script is [tools/stress/run_kt_mba.sh](../tools/stress/run_kt_mba.sh).
