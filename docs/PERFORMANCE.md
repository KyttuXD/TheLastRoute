# Performance: goals, rules and how to measure

The goals and the rules are KT's and are also in [AGENTS.md](../AGENTS.md); this file adds how to measure and where
the numbers stand.

## Goals (2026-11-12)

| Machine | Goal | State |
|---|---|---|
| Very low-end PCs | at least 30 fps in play | not measured on such a machine |
| KT's dual-core MacBook Airs | 60 to 300 fps | last Intel Air numbers are in [STRESS_TESTS.md](STRESS_TESTS.md) and [GRAPHICS_API.md](GRAPHICS_API.md) |
| KT's Framework Laptop 13 | 1000 fps or more; keep it | release build, stress `mixed`: 0.81 ms a frame (about 1230 fps), 2026-11-12 |

## Rules

1. Frame time comes first. No change may slow the stress run's average frame by more than noise (about 3%) on KT's
   PC, not even to save memory, without KT's go-ahead.
2. Memory is saved on what is out of sight: packed, dropped, or built on first use. Never by adding work to every
   frame. Example: the floor's mesh tiles are kept in an 18-byte vertex, and a tile that is being drawn keeps its
   unpacked vertices too, so drawing costs what it did before ([RAM_PASS.md](RAM_PASS.md)). The first version unpacked
   every drawn tile each frame, saved 16 MB and cost 0.3 ms a frame (1000 fps down to 760); it was replaced.
3. Measure before and after, old and new build back to back, at least twice each (the numbers move by about 3% and
   6 MB between runs, and a busy PC moves them much more).
4. A new cache, table or buffer over about 1 MB gets a reason in the commit message and a row in RAM_PASS.md.
5. No new allocation, sort or full pass over the world in the draw or update loop without a measurement.
6. No new crate without KT's go-ahead; shipped builds keep fat LTO; builds stay free of warnings
   ([COMPILE_SPEED.md](COMPILE_SPEED.md)).

## How to measure

Frame time and memory, on Linux (the stress mode is described in [STRESS_TESTS.md](STRESS_TESTS.md)):

```sh
sh tools/perf/mem.sh 45 target/debug/last_route --offline --stress 40 --stress-scenario mixed
awk -F, 'NR>1{a+=$3;r+=$6;n++}END{print "ms_avg",a/n,"render_ms",r/n}' stress.csv
```

The first line prints the resident memory each second and the peak; the stress run writes `stress.csv` (delete it
afterwards, it is not to be committed). `ms_avg` is the whole frame, `render_ms` the drawing. Frames a second is
1000 / `ms_avg`.

For a release number: `cargo build --release --bin last_route` and the same two lines with `target/release`.

A picture check for anything that touches geometry or shading: `LR_SHOT_RUN=1 LR_SHOT_INF=1 last_route --offline
--screenshot out.png` with the old and the new build (the hooks are listed at the top of `app/mod.rs`). The run's
start fade follows the clock, so two shots differ by one overall brightness factor; compare after scaling it out.

## Where the numbers stand (KT's Framework Laptop 13, Linux, 2026-11-12)

| Build | Stress `mixed`, average frame | Peak memory |
|---|---|---|
| dev (`opt-level = 2`) | 1.00 to 1.04 ms | about 210 MB |
| release (fat LTO) | 0.79 to 0.84 ms | not measured |

The menu sits at about 144 MB. What holds the memory, and everything tried, is in [RAM_PASS.md](RAM_PASS.md); the
triangle budget of props is in [PERF_TRIANGLES.md](PERF_TRIANGLES.md).

## Not known yet

- No number exists for a low-end PC or for the PowerPC and Snow Leopard builds.
- The MacBook Air numbers predate the RAM pass.
