// Injected by the macOS app shell (LastRoute.m) into the game page at document start, before the page's own scripts run.
// It does the things the shell needs from inside the page and nothing the game depends on to work in a normal browser:
//   * LR_API_BASE: the page's net plugin (index.html, lr_net) builds every /api URL and the WebSocket address from it. The page
//     itself lives at lrapp://game/, so without this the game would try lrapp://game/api/ws. @SITE@ is the website the shell
//     was started with (--site-url), by default the live game site, whose /api answers any origin (CORS) and any WebSocket origin.
//   * ready: tells the shell the wasm module is instantiated and has run for a moment without a fatal error (the shell rolls back to an older copy of the
//     web files when this does not happen), and forwards page errors and console warnings to the shell's log.
//   * fullscreen: WKWebView has no element fullscreen on older macOS, so canvas.requestFullscreen() / document.exitFullscreen()
//     ask the shell to put the whole WINDOW into native fullscreen and the page sees document.fullscreenElement as usual.
//   * paste: the shell can hand the game a clipboard text as a synthetic `paste` event (window.__lrPaste) when WKWebView will not
//     fire a real one for a page without an editable element.
// Nothing here may throw into the page.
(function () {
  "use strict";
  var SITE = "@SITE@";
  var VERSION = "@VERSION@";
  try {
    window.LR_API_BASE = SITE;
    window.LR_NATIVE_SHELL = { name: "lastroute-macos-webapp", version: VERSION };
  } catch (e) {}

  var bridge = null;
  try {
    bridge = window.webkit.messageHandlers.lr;
  } catch (e) {}
  function post(message) {
    try {
      bridge.postMessage(message);
    } catch (e) {}
  }
  window.__lrPost = post;

  // ---- the shell's log gets the page's problems ----
  var budget = { log: 40, info: 40, warn: 120, error: 200 };
  function describe(args) {
    try {
      return Array.prototype.map
        .call(args, function (a) {
          if (a instanceof Error) return a.name + ": " + a.message;
          return typeof a === "string" ? a : JSON.stringify(a);
        })
        .join(" ")
        .slice(0, 400);
    } catch (e) {
      return "(unprintable)";
    }
  }
  ["log", "info", "warn", "error"].forEach(function (level) {
    var original = console[level];
    console[level] = function () {
      try {
        if (budget[level] > 0) {
          budget[level] -= 1;
          post({ type: "console", level: level, text: describe(arguments) });
        }
        // miniquad reports a wasm that cannot start with console.error, not with an exception
        if (level === "error" && !ready) {
          var text = describe(arguments);
          if (FATAL.test(text)) problem("console", text);
        }
      } catch (e) {}
      return original.apply(console, arguments);
    };
  });
  var ready = false;
  var FATAL = /RuntimeError|CompileError|LinkError|unreachable|out of memory|Out of memory/;
  function problem(kind, text) {
    post({ type: "error", kind: kind, text: String(text).slice(0, 500), fatal: !ready && FATAL.test(String(text)) });
  }
  window.addEventListener("error", function (e) {
    problem("error", (e && e.message) || "error");
  });
  window.addEventListener("unhandledrejection", function (e) {
    var r = e && e.reason;
    problem("rejection", r && r.message ? r.name + ": " + r.message : String(r));
  });

  // ---- ready: the wasm module is instantiated and nothing fatal has happened for a moment ----
  // A timer, not requestAnimationFrame: a window that is covered, minimised or on another desktop gets no animation frames, and a game
  // that was merely hidden while it started must not count as a game that failed to start. Timers still run (slower) there.
  var upTicks = 0;
  var timer = window.setInterval(function () {
    var up = false;
    try {
      // both are globals of the miniquad bundle (index.html itself reads wasm_memory), set once the module is instantiated
      up = (typeof wasm_exports !== "undefined" && !!wasm_exports) || (typeof wasm_memory !== "undefined" && !!wasm_memory);
    } catch (e) {}
    if (up) upTicks += 1;
    if (up && upTicks >= 6) {
      window.clearInterval(timer);
      ready = true;
      var info = {};
      try {
        info.gl = typeof gl !== "undefined" && gl ? String(gl.getParameter(gl.VERSION)) : "none";
        info.canvas = document.getElementById("glcanvas").width + "x" + document.getElementById("glcanvas").height;
        info.hidden = document.hidden;
      } catch (e) {}
      post({ type: "ready", info: info });
    }
  }, 250);

  // ---- fullscreen through the window ----
  var fullscreen = false;
  try {
    Object.defineProperty(document, "fullscreenElement", {
      configurable: true,
      get: function () {
        return fullscreen ? document.getElementById("glcanvas") : null;
      },
    });
    Object.defineProperty(document, "fullscreenEnabled", {
      configurable: true,
      get: function () {
        return true;
      },
    });
    Element.prototype.requestFullscreen = function () {
      post({ type: "fullscreen", on: true });
      return Promise.resolve();
    };
    document.exitFullscreen = function () {
      post({ type: "fullscreen", on: false });
      return Promise.resolve();
    };
  } catch (e) {}
  window.__lrFullscreen = function (on) {
    fullscreen = !!on;
    try {
      document.dispatchEvent(new Event("fullscreenchange"));
    } catch (e) {}
  };

  // ---- paste handed over by the shell (Cmd+V when WKWebView would not send a paste event itself) ----
  window.__lrPaste = function (text) {
    try {
      var data = new DataTransfer();
      data.setData("text/plain", String(text));
      var event = new ClipboardEvent("paste", { clipboardData: data, bubbles: true, cancelable: true });
      if (!event.clipboardData || event.clipboardData.getData("text/plain") !== String(text)) return -2;
      window.dispatchEvent(event);
      return 1;
    } catch (e) {
      return -1;
    }
  };
})();
