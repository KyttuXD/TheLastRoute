// What the browser says about this machine, for the game's AUTO quality tier (app/quality.rs).
//
// It is left in localStorage under "last_route.hw" as JSON, and the wasm reads it through the same storage bridge as
// the settings (app/web_storage.rs), so the game needs no extra wasm import: a page without this script, or with blocked
// storage, simply reports nothing and AUTO falls back to timing the first seconds of play.
//
//   cores      navigator.hardwareConcurrency (0 when the browser hides it)
//   memory_gb  navigator.deviceMemory, a power of two up to 8 in Chrome and Edge (0 in Firefox and Safari)
//   mobile     a phone or tablet browser
//   software   WebGL is drawn by a software renderer (SwiftShader, llvmpipe), which is far too slow for the 3D view
//
// Nothing here leaves the page: it is read by this game only and is not sent anywhere.
(function () {
  try {
    var nav = window.navigator || {};
    var software = false;
    try {
      var canvas = document.createElement("canvas");
      var gl = canvas.getContext("webgl") || canvas.getContext("experimental-webgl");
      if (gl) {
        var info = gl.getExtension("WEBGL_debug_renderer_info");
        var renderer = String(info ? gl.getParameter(info.UNMASKED_RENDERER_WEBGL) : "");
        software = /swiftshader|llvmpipe|software|softpipe/i.test(renderer);
        var lose = gl.getExtension("WEBGL_lose_context");
        if (lose) lose.loseContext();
      }
    } catch (e) {
      // no WebGL probe: the flag stays false and the timing decides
    }
    var report = {
      cores: nav.hardwareConcurrency > 0 ? Math.floor(nav.hardwareConcurrency) : 0,
      memory_gb: nav.deviceMemory > 0 ? Number(nav.deviceMemory) : 0,
      mobile: /android|iphone|ipad|ipod|mobile/i.test(String(nav.userAgent || "")) || (nav.maxTouchPoints > 1 && /macintosh/i.test(String(nav.userAgent || ""))),
      software: software,
    };
    window.localStorage.setItem("last_route.hw", JSON.stringify(report));
  } catch (e) {
    // storage is blocked: the game reports nothing and times the first seconds instead
  }
})();
