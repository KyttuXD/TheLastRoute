// The web build's update check (app/web_update.rs, ui/update_screen.rs), registered as the lr_update miniquad plugin.
//
// The deploy writes version.json beside the page: {"build":"<sha256 of last_route.wasm>","built":"<iso time>"}
// (packaging/web/deploy-vps2.sh, uploaded last). This script reads it once at load to learn which build this page runs,
// then asks again every 30 s plus up to 10 s of random jitter (so many open tabs do not all ask in the same second).
// A different "build" means a newer deploy: lr_update_available() answers 1 from then on and polling stops.
//
// Nothing here may disturb the game: a missing file, a status other than 200, text that is not JSON, a JSON without a
// "build" string and a network error all mean "no update" and are swallowed.
(function () {
  const POLL_MS = 30000;
  const JITTER_MS = 10000;
  const FILE = "version.json";

  let loadedBuild = null; // the build this page started with (null until the first good answer)
  let changed = false; // a different build was seen: sticky, and polling has stopped

  // version.json in the same folder as the page, never from a cache (the ?t= stamp also defeats caching proxies)
  function versionUrl() {
    const url = new URL(FILE, document.baseURI || window.location.href);
    url.searchParams.set("t", String(Date.now()));
    return url.toString();
  }

  // the build text from version.json, or null for any kind of failure
  async function fetchBuild() {
    try {
      const response = await fetch(versionUrl(), { cache: "no-store" });
      if (response.status !== 200) return null;
      const data = await response.json();
      if (data && typeof data.build === "string" && data.build.length > 0) return data.build;
      return null;
    } catch (e) {
      return null;
    }
  }

  async function check() {
    if (changed) return;
    const build = await fetchBuild();
    if (build === null) return;
    if (loadedBuild === null) {
      loadedBuild = build;
    } else if (build !== loadedBuild) {
      changed = true;
    }
  }

  function schedule() {
    if (changed) return;
    setTimeout(function () {
      check().then(schedule, schedule);
    }, POLL_MS + Math.random() * JITTER_MS);
  }

  // the first read starts at once, before the wasm has finished downloading, so the build it names is the page's own
  check().then(schedule, schedule);

  if (typeof miniquad_add_plugin === "function") {
    miniquad_add_plugin({
      register_plugin: function (importObject) {
        importObject.env.lr_update_available = function () {
          return changed ? 1 : 0;
        };
        importObject.env.lr_update_reload = function () {
          try {
            window.location.reload();
          } catch (e) {}
        };
      },
      version: 1,
      name: "lr_update",
    });
  }
})();
