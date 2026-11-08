# LR Story: building and releases

LR Story is the `lr_story` binary in this repo (`src/bin/lr_story/`). Its web build is packaged separately (`packaging/story-web/`).

## Local build

```bash
cargo build --release --bin lr_story
```

On KT's PC use `CARGO_BUILD_JOBS=4` and one cargo build at a time (see `AGENTS.md`).

## CI build and release

- Workflow: `.github/workflows/nightly-lrs.yml`. It runs only by hand (Actions tab, `nightly-lrs`, Run workflow, or `gh workflow run nightly-lrs.yml`).
- It builds Windows x86_64, Linux x86_64, macOS Intel and macOS Apple Silicon, and publishes them to the rolling pre-release tag `nightly-lrs`, replaced in place on each run.
- Files: `lr_story-windows-x86_64.exe`, `lr_story-linux-x86_64.tar.gz`, `lr_story-macos-x86_64.zip`, `lr_story-macos-arm64.zip`.

The release rules shared with the other games are in [docs/RELEASES.md](../../../../docs/RELEASES.md).
