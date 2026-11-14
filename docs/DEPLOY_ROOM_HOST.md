# Deploying the Rust room host on VPS 2 (approval list, NOT RUN)

**STATUS: NOTHING IN THIS LIST HAS BEEN RUN.** No server was contacted when it was written. AGENTS.md: never change vps1, vps2 or vps3
without KT's approval of the exact steps. KT approves step by step. Background and evidence: [SERVERS.md](SERVERS.md) ("CREATE ROOM says
BUILD MISMATCH", "Public route"), [KT_BUG_LIST.md](KT_BUG_LIST.md) items 10, 11 and 22.

Scripts in the repo (all default to a dry run): `packaging/room-host/deploy-vps2.sh`, `packaging/room-host/apply-node-patch.sh`,
`packaging/public-route/lastroute-public-route.service` (+ `.env.example`).

Assumptions to VERIFY in step 1 (the repo does not confirm them): the service user `lastroute-godot`, the unit names `lastroute-godot-test`
and `lastroute-region-agent`, the paths `/opt/lastroute-godot/current/backend` and `/etc/lastroute-agent/agent.env`.
Pick one quiet window. Only step 7 (the swap and the restart of `lastroute-godot-test` and `lastroute-region-agent`), the optional step 9 (`RESTART=1`, the region agent only) and step 10 (starts a new service) start or restart anything on vps2; step 11 redeploys vps3 and the web build. Step 8 only checks.
Never put a GitHub token on vps2, never paste secrets in commands, keep `agent.env` root-only (0600).

## A. Read-only checks (can be approved as a group)

1. `ssh vps2 'head -3 /etc/os-release; ldd --version | head -1; ldconfig -p | grep libasound.so.2; systemctl is-active lastroute-godot-test lastroute-region-agent; ls -la /opt/lastroute-godot/current /etc/lastroute-agent/agent.env; id lastroute-godot'`
   Restarts: nothing. Lost: nothing. Rollback: none needed. If `libasound.so.2` is missing, step 3 is needed. If glibc is older than 2.35: stop.
2. On KT's PC: `gh release download nightly -R KyttuXD/TheLastRoute -p last-route-linux-x86_64.tar.gz -D /tmp/lrhost`, then `sha256sum` it and
   `tar -tzf` it to find the path of `last_route` inside. The commit must be the one the live web build was made from (compare the build in
   `https://lastrouterust.games.kyttuslab.org/version.json`). Needs KT's OK to download (name, source, size are shown by the command).
   Restarts: nothing. Rollback: delete `/tmp/lrhost`.

## B. Changes

3. Only if libasound is missing: `ssh vps2 'apt-get install -y libasound2'` (`libasound2t64` on Ubuntu 24.04). Restarts: nothing. Lost: nothing.
   Rollback: `apt-get remove` it.
4. Install the binary (extract `last_route` from the tarball first, then): `DRY_RUN=1 sh packaging/room-host/deploy-vps2.sh /tmp/lrhost/last_route`
   (prints the plan, contacts nothing), then `DRY_RUN=0 sh packaging/room-host/deploy-vps2.sh /tmp/lrhost/last_route`. It uploads to
   `/opt/lastroute-rust/releases/<sha>/last_route`, checks the SHA-256 there, checks `ldd`, keeps the old release as `previous`, flips `current`
   atomically and prints the rollback line. Restarts: nothing. Lost: nothing. Rollback: the printed `ln -sfn <old> ... && mv -T ...` line, or
   (first install) `rm -rf /opt/lastroute-rust`.
5. Smoke test without the agent: `ssh vps2 'timeout 5 /opt/lastroute-rust/current/last_route --headless -- --dedicated-host ZZZZZ --backend=http://127.0.0.1:8205 --idle-minutes 1'`.
   Expect it to start and be refused for lack of a spawn ticket. A missing-library error means step 3. Check that nothing was written under `/root`.
   Restarts: nothing. Lost: nothing.
6. Patch a copy of the Node backend: `scp docs/patches/node-hosted-build.patch packaging/room-host/apply-node-patch.sh vps2:/tmp/`, then
   `ssh vps2 'DRY_RUN=1 sh /tmp/apply-node-patch.sh /opt/lastroute-godot/current/backend /tmp/node-hosted-build.patch'` (dry run on a temporary copy), then `ssh vps2 'DRY_RUN=0 sh /tmp/apply-node-patch.sh /opt/lastroute-godot/current/backend /tmp/node-hosted-build.patch'`.
   It makes `backend.new`, patches it with `patch -p1` (not `git apply`), runs `node --test` there if node is on the PATH (else it says so: run the tests by hand
   with the node under `/opt/node`), and writes `backend.swap.sh` without running it. Stop on any failed hunk or test. Restarts: nothing, the live backend is untouched.
   Rollback: `rm -rf /opt/lastroute-godot/current/backend.new /opt/lastroute-godot/current/backend.swap.sh`.
7. **THE RESTART.** Add the binary to the agent's env: `ssh vps2 'set -e; E=/etc/lastroute-agent/agent.env; if [ ! -e $E.bak ]; then cp -a $E $E.bak; fi; if ! grep -q ^LASTROUTE_RUST_BIN= $E; then echo LASTROUTE_RUST_BIN=/opt/lastroute-rust/current/last_route >> $E; fi'`
   (the backup is made once and never overwritten; a failed backup stops before the append; a rerun appends nothing twice)
   (the token in that file is never printed), then run the swap script: `ssh vps2 'sh /opt/lastroute-godot/current/backend.swap.sh'`.
   Restarts: the Node directory (port 8205, shared by the Godot AND the Rust site) and the region agent. Lost: every open hosted room and running
   dedicated host on both sites, and open web tabs may be asked to reload (`reloadId`); the Godot site is down for a few seconds. `lastroute-room@AKIRI` is not touched.
   Rollback: `ssh vps2 'B=/opt/lastroute-godot/current/backend; mv $B $B.new && mv $B.old $B && cp -a /etc/lastroute-agent/agent.env.bak /etc/lastroute-agent/agent.env && systemctl restart lastroute-godot-test lastroute-region-agent'`.
   The script refuses to write the swap script if `backend.old` or `backend.bak-<date>` already exists (a second swap would nest folders): move those aside by hand first.
8. Check: sign in on the Rust site, CREATE ROOM, then `curl -H 'Accept: application/json' https://lastrouterust.games.kyttuslab.org/api/enet/rooms/CODE` must show
   `"dedicated":true` and the Rust protocol (the 8 digits after `YOU` in any mismatch line, `09923496` at the time of writing). The Godot site must still create a Godot room.
   `ssh vps2 'journalctl -u lastroute-region-agent -n 50 --no-pager'` shows the spawn of the Rust path. Old refused rooms hold up to 9 region slots for 10 minutes.
   Restarts: nothing.
9. (Optional, instead of step 7's restart for the binary alone) `RESTART=1` with step 4 restarts only `lastroute-region-agent` and prints the warning
   that its open rooms drop. Not needed when step 7 follows.

## C. Public route process (after A and B work)

10. Install the unit: `scp packaging/public-route/lastroute-public-route.service vps2:/etc/systemd/system/`, then
    `ssh vps2 'id lr-publicroute >/dev/null 2>&1 || useradd --system --home-dir /var/lib/lastroute-public --shell /usr/sbin/nologin lr-publicroute; systemctl daemon-reload; systemctl enable --now lastroute-public-route; sleep 5; curl -fsS http://127.0.0.1:8205/api/enet/rooms/PUBLC'`.
    Needs KT's approval for the new system user. Restarts: nothing existing. Lost: nothing (the stand-in logic keeps working). Check it holds `PUBLC` for a few
    minutes and comes back after `kill` (`Restart=always`). Rollback: `systemctl disable --now lastroute-public-route; rm /etc/systemd/system/lastroute-public-route.service; systemctl daemon-reload; userdel lr-publicroute; rm -rf /var/lib/lastroute-public`.
11. Only after step 10 has held `PUBLC` for a while: on a branch set `client_host_fallback` to `false` in `src/bin/last_route/data/net/public_route.json`, run the tests, merge. The live Godot backend has no `/api/public/settings`, so this
    is the only way to flip it. Merging to `main` also redeploys vps3 (deploy-server.yml) and the web build. Old cached tabs keep the stand-in until reloaded.
    Rollback: revert the commit and redeploy.

## D. Stale GitHub secrets and the old upload account (approval needed)

12. From KT's PC: `gh secret delete NIGHTLY_SSH_KEY --repo KyttuXD/TheLastRoute` and `gh secret delete NIGHTLY_KNOWN_HOSTS --repo KyttuXD/TheLastRoute`.
    They are used only by `packaging/publish-vps.sh`, which no workflow calls (checked 2026-11-13). Deleting is permanent: the value cannot be read back
    (a key can be made again). Check no manual process still uses them. Rollback: none. A separate approval: lock the vps3 account (`ssh vps3 'passwd -l nightly-upload'`
    or remove its authorized key; rollback: restore the key line). `packaging/publish-vps.sh` and `packaging/update-manifest.mjs` stay until the vps3 update route is retired.

## Follow-ups (not done)

- `last_route --print-protocol` (prints `net::protocol::fingerprint()` and exits; a source change and a build) and a check script that compares it with a live room, so the
  host binary cannot drift from the wasm unnoticed.
- Optional, with KT's approval of a restricted `lr-roomhost` user and a sudoers rule like `lr-server-swap`: a workflow job that uploads the binary on the same push as deploy-web.
- Without the patch, `build: "rust"` is ignored and the Godot host starts, which is today's behaviour, so a skipped or failed patch is not worse than now.
