# Compile speed

Local iteration profile: `cargo build --profile fast` (inherits dev; opt-level 1,
dependencies at 2, line-tables-only debug, incremental on). `profile.release` and
`profile.ci` are unchanged.

`.cargo/config.toml` only sets macOS `split-debuginfo=unpacked`. Windows and Linux use
the default linker; add lld only after confirming it is installed.

## Measuring

Cold: `cargo clean` then `cargo build --profile fast --timings`.
Warm: touch one source file, then `cargo build --profile fast --timings`.
Repeat with `--profile dev` to compare. Reports land in `target/cargo-timings/`.

| Profile | Cold (s) | Warm (s) |
|---------|----------|----------|
| dev     |          |          |
| fast    |          |          |
