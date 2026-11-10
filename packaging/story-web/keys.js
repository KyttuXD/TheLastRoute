"use strict";
// Stuck keys. The browser delivers keyup to whatever element has focus, so a key released after focus moved (Escape and
// the pointer lock, a click on a menu, another tab, an alert) never reaches the game and its movement key stays down:
// the runner keeps walking with no key held. This tracks the keys that are down and releases them in the game when the
// page loses focus or the pointer lock ends, and forwards a keyup the canvas did not see.
//
// Cmd on a Mac. Two more things the stock miniquad page script gets wrong, and the text fields' Cmd+V / C / X / A need:
//  - it names the Windows/Command key "OSLeft" / "OSRight", the old Firefox spelling; every current browser says "MetaLeft" /
//    "MetaRight", which it does not know, so the game never saw Cmd held. This forwards those two as the Super keys (343, 347).
//  - a browser on macOS sends no keyup for a key released while Cmd is held (Cmd+V: the V is never reported up), which would
//    leave V "down" in the game. The keys pressed under Cmd are released when Cmd comes up.
(function () {
  const META_KEYS = { MetaLeft: 343, MetaRight: 347 };
  const underMeta = new Set(); // KeyboardEvent.code of keys pressed while Cmd was held
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
      underMeta.clear();
      return;
    }
    for (const key of down.values()) sendUp(key);
    down.clear();
    underMeta.clear();
  }
  function modifiers(e) {
    // SAPP_MODIFIER_* of the bundle: shift 1, ctrl 2, alt 4, super 8
    return (e.shiftKey ? 1 : 0) | (e.ctrlKey ? 2 : 0) | (e.altKey ? 4 : 0) | (e.metaKey ? 8 : 0);
  }
  window.addEventListener(
    "keydown",
    function (e) {
      const meta = META_KEYS[e.code];
      if (meta !== undefined) {
        if (gameReady() && typeof wasm_exports.key_down === "function") {
          try {
            wasm_exports.key_down(meta, modifiers(e), e.repeat);
          } catch (err) {}
        }
        if (!e.repeat) down.set(e.code, meta);
        return;
      }
      if (e.metaKey) underMeta.add(e.code);
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
      if (META_KEYS[e.code] !== undefined) {
        // Cmd came up: it releases itself and every key that was pressed under it
        const codes = Array.from(underMeta);
        underMeta.clear();
        for (const code of codes.concat([e.code])) {
          const held = down.get(code);
          down.delete(code);
          if (held !== undefined && gameReady()) sendUp(held);
        }
        return;
      }
      underMeta.delete(e.code);
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
