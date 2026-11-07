// Run by the shell for `LastRoute --self-test`, once the game page has reported ready. It collects facts about this WKWebView
// and the running game page and posts them back as one {type:"selftest", result:{...}} message; the shell writes them to the log
// (~/Library/Logs/LastRoute/app.log) and exits. Nothing here changes game data: the only things written are one localStorage key
// ("lr_selftest_counter") and one IndexedDB database ("lr_selftest").
(async function () {
  var r = {};
  var post = window.__lrPost || function () {};
  function timeout(ms, value) {
    return new Promise(function (resolve) {
      setTimeout(function () {
        resolve(value);
      }, ms);
    });
  }
  async function step(name, fn) {
    try {
      r[name] = await fn();
    } catch (e) {
      r[name] = "ERROR " + (e && e.message ? e.message : e);
    }
  }

  await step("page", function () {
    return {
      href: location.href,
      secureContext: window.isSecureContext,
      dpr: window.devicePixelRatio,
      inner: window.innerWidth + "x" + window.innerHeight,
      ua: navigator.userAgent,
      apiBase: window.LR_API_BASE,
    };
  });
  await step("wasm", function () {
    return {
      exports: typeof wasm_exports !== "undefined" && !!wasm_exports ? Object.keys(wasm_exports).length : 0,
      memoryBytes: typeof wasm_memory !== "undefined" ? wasm_memory.buffer.byteLength : 0,
    };
  });
  await step("webgl", function () {
    var info = { gameContext: typeof gl !== "undefined" && !!gl };
    if (info.gameContext) {
      info.version = String(gl.getParameter(gl.VERSION));
      var ext = gl.getExtension("WEBGL_debug_renderer_info");
      if (ext) info.renderer = String(gl.getParameter(ext.UNMASKED_RENDERER_WEBGL));
      info.isWebGL2 = typeof WebGL2RenderingContext !== "undefined" && gl instanceof WebGL2RenderingContext;
    }
    return info;
  });
  await step("frames", async function () {
    var count = 0;
    var start = performance.now();
    await new Promise(function (resolve) {
      function loop() {
        count += 1;
        if (performance.now() - start >= 1000) resolve();
        else requestAnimationFrame(loop);
      }
      requestAnimationFrame(loop);
      setTimeout(resolve, 2500);
    });
    return { rafPerSecond: count, ms: Math.round(performance.now() - start) };
  });
  await step("versionJson", async function () {
    var response = await fetch("version.json?t=" + Date.now(), { cache: "no-store" });
    var text = await response.text();
    return { status: response.status, type: response.headers.get("content-type"), body: text.slice(0, 160) };
  });
  // the scheme handler may only serve plain files of the build folder: every one of these must be refused (400/404), never 200
  await step("traversal", async function () {
    var paths = ["../../state.json", "..%2f..%2fstate.json", "%2e%2e/state.json", "..%5cstate.json", "web/../state.json", ".stage-x", "./index.html/..", "index.html?x=/../a", "state.json", "meta%00.json", "a/b.js", "//etc/passwd", "%2fetc%2fpasswd"];
    var out = {};
    var leaked = [];
    for (var i = 0; i < paths.length; i++) {
      var status;
      try {
        status = (await fetch("lrapp://game/" + paths[i], { cache: "no-store" })).status;
      } catch (e) {
        status = "err";
      }
      out[paths[i]] = status;
      if (status === 200 && paths[i] !== "index.html?x=/../a" && paths[i] !== "./index.html/..") leaked.push(paths[i]);
    }
    out.leaked = leaked.length;
    return out;
  });
  await step("wasmFetch", async function () {
    var response = await fetch("last_route.wasm", { method: "HEAD" });
    return { status: response.status, type: response.headers.get("content-type"), length: response.headers.get("content-length") };
  });
  await step("localStorage", function () {
    var before = window.localStorage.getItem("lr_selftest_counter");
    window.localStorage.setItem("lr_selftest_counter", String((parseInt(before || "0", 10) || 0) + 1));
    return { before: before, after: window.localStorage.getItem("lr_selftest_counter"), keysInStore: window.localStorage.length };
  });
  await step("indexedDB", function () {
    return new Promise(function (resolve) {
      try {
        var req = window.indexedDB.open("lr_selftest", 1);
        req.onupgradeneeded = function () {
          req.result.createObjectStore("s");
        };
        req.onsuccess = function () {
          req.result.close();
          resolve("ok");
        };
        req.onerror = function () {
          resolve("error " + (req.error && req.error.name));
        };
        setTimeout(function () {
          resolve("timeout");
        }, 3000);
      } catch (e) {
        resolve("throws " + e.message);
      }
    });
  });
  await step("apiDirect", async function () {
    var response = await fetch(window.LR_API_BASE + "/api/health", { cache: "no-store" });
    var text = await response.text();
    return { status: response.status, body: text.slice(0, 90) };
  });
  await step("apiViaShell", async function () {
    var response = await fetch("/api/health", { cache: "no-store" });
    var text = await response.text();
    return { status: response.status, body: text.slice(0, 90) };
  });
  await step("apiSyncXhr", function () {
    var xhr = new XMLHttpRequest();
    xhr.open("GET", window.LR_API_BASE + "/api/health", false);
    xhr.send(null);
    return { status: xhr.status };
  });
  await step("webSocket", function () {
    return new Promise(function (resolve) {
      var base = String(window.LR_API_BASE).replace(/^http/, "ws") + "/api/ws";
      var out = { url: base };
      try {
        var ws = new WebSocket(base);
        ws.onopen = function () {
          out.opened = true;
          ws.close();
        };
        ws.onclose = function (e) {
          out.closed = e.code;
          resolve(out);
        };
        setTimeout(function () {
          out.timeout = true;
          resolve(out);
        }, 6000);
      } catch (e) {
        out.error = e.message;
        resolve(out);
      }
    });
  });
  await step("platform", function () {
    var canvas = document.getElementById("glcanvas");
    return {
      pointerLockApi: typeof canvas.requestPointerLock + "/" + ("pointerLockElement" in document),
      fullscreenShim: typeof canvas.requestFullscreen + "/" + document.fullscreenEnabled,
      clipboardApi: typeof navigator.clipboard + "/" + (navigator.clipboard ? typeof navigator.clipboard.readText : "-"),
      gamepads: typeof navigator.getGamepads,
      audioContext: typeof (window.AudioContext || window.webkitAudioContext),
      wasmStreaming: typeof WebAssembly.instantiateStreaming,
      cores: navigator.hardwareConcurrency,
      deviceMemory: navigator.deviceMemory,
    };
  });
  await step("audio", async function () {
    var Ctx = window.AudioContext || window.webkitAudioContext;
    if (!Ctx) return "no AudioContext";
    var ctx = new Ctx();
    var before = ctx.state;
    try {
      await Promise.race([ctx.resume(), timeout(1500)]);
    } catch (e) {}
    var out = { before: before, after: ctx.state, sampleRate: ctx.sampleRate };
    try {
      ctx.close();
    } catch (e) {}
    return out;
  });
  await step("synthPaste", function () {
    var got = null;
    function listener(e) {
      got = e.clipboardData ? e.clipboardData.getData("text/plain") : null;
    }
    window.addEventListener("paste", listener, true);
    var code = window.__lrPaste("selftest-paste");
    window.removeEventListener("paste", listener, true);
    return { code: code, delivered: got };
  });
  await step("pointerLock", async function () {
    var canvas = document.getElementById("glcanvas");
    var out = { api: typeof canvas.requestPointerLock };
    var result = null;
    var events = [];
    var t0 = performance.now();
    var realExit = document.exitPointerLock;
    // the game releases the lock itself whenever its own menus are up; hold that back while the platform is being probed
    document.exitPointerLock = function () {};
    function changed() {
      result = document.pointerLockElement === canvas ? "locked" : "unlocked";
      events.push(result + "@" + Math.round(performance.now() - t0));
    }
    function failed() {
      result = "pointerlockerror";
    }
    document.addEventListener("pointerlockchange", changed);
    document.addEventListener("pointerlockerror", failed);
    var tried = false;
    function attempt() {
      if (tried) return;
      tried = true;
      try {
        var p = canvas.requestPointerLock();
        if (p && p.catch) {
          p.catch(function (e) {
            result = "rejected " + e.name;
          });
        }
      } catch (e) {
        result = "throws " + e.message;
      }
    }
    window.addEventListener("mousedown", attempt, true);
    post({ type: "selftest-click" });
    var waited = 0;
    while (result === null && waited < 3000) {
      await timeout(100);
      waited += 100;
    }
    out.clickSeen = tried;
    out.result = result === null ? "no answer in 3 s" : result;
    out.lockedNow = document.pointerLockElement === canvas;
    if (out.lockedNow) {
      // the shell sends one mouse-moved event with a delta of (17, -5) to the window: a locked pointer must report it as movementX/Y
      var moves = [];
      var onMove = function (e) {
        moves.push(e.movementX + "," + e.movementY);
      };
      window.addEventListener("mousemove", onMove, true);
      post({ type: "selftest-move" });
      await timeout(800);
      window.removeEventListener("mousemove", onMove, true);
      out.moves = moves.slice(0, 4);
    }
    out.events = events;
    document.exitPointerLock = realExit;
    try {
      document.exitPointerLock();
    } catch (e) {}
    window.removeEventListener("mousedown", attempt, true);
    document.removeEventListener("pointerlockchange", changed);
    document.removeEventListener("pointerlockerror", failed);
    return out;
  });
  post({ type: "selftest", result: r });
})();
