"use strict";
// Stuck keys. The browser delivers keyup to whatever element has focus, so a key released after focus moved (Escape and
// the pointer lock, a click on a menu, another tab, an alert) never reaches the game and its movement key stays down:
// the runner keeps walking with no key held. This tracks the keys that are down and releases them in the game when the
// page loses focus or the pointer lock ends, and forwards a keyup the canvas did not see.
(function () {
  const down = new Map(); // KeyboardEvent.code -> miniquad key code
  function gameReady() {
    return typeof wasm_exports !== "undefined" && wasm_exports && typeof wasm_exports.key_up === "function";
  }
  function sendUp(key) {
    try {
      wasm_exports.key_up(key, 0);
    } catch (e) {
      // the game is not running (yet): nothing to release
    }
  }
  function releaseAll() {
    if (!gameReady()) {
      down.clear();
      return;
    }
    for (const key of down.values()) sendUp(key);
    down.clear();
  }
  window.addEventListener(
    "keydown",
    function (e) {
      if (e.repeat) return;
      try {
        down.set(e.code, into_sapp_keycode(e.code));
      } catch (err) {
        // an unknown key: the game ignores it too
      }
    },
    true
  );
  window.addEventListener(
    "keyup",
    function (e) {
      const key = down.get(e.code);
      if (key === undefined) return;
      down.delete(e.code);
      // the canvas handler sends its own keyup when it has focus; a second one is harmless, a missing one is the bug
      if (gameReady() && document.activeElement !== document.getElementById("glcanvas")) sendUp(key);
    },
    true
  );
  window.addEventListener("blur", releaseAll);
  document.addEventListener("visibilitychange", function () {
    if (document.hidden) releaseAll();
  });
  document.addEventListener("pointerlockchange", function () {
    // the lock ended (Escape, focus): the player let go of the keys somewhere the game could not see
    if (document.pointerLockElement === null) releaseAll();
  });
})();
