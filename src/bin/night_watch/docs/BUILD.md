# Night Watch: building and releases

Night Watch is the `night_watch` binary in this repo (`src/bin/night_watch/`).

## Local build

```bash
cargo build --release --bin night_watch
```

On KT's PC use `CARGO_BUILD_JOBS=4` and one cargo build at a time (see `AGENTS.md`).

## CI build and release

- Workflow: `.github/workflows/nightly-nw.yml`. It runs only by hand (Actions tab, `nightly-nw`, Run workflow, or `gh workflow run nightly-nw.yml`).
- It builds Windows x86_64, Linux x86_64, macOS Intel and macOS Apple Silicon, and publishes them to the rolling pre-release tag `nightly-nw`, replaced in place on each run.
- Files: `night_watch-windows-x86_64.exe`, `night_watch-linux-x86_64.tar.gz`, `night_watch-macos-x86_64.zip`, `night_watch-macos-arm64.zip`.

The release rules shared with the other games are in [docs/RELEASES.md](../../../../docs/RELEASES.md).
