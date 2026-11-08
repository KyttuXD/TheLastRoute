import assert from "node:assert/strict";
import test from "node:test";
import { HUNTER_ID_ALIASES, canonicalHunterId } from "./hunter_id_aliases.mjs";

test("old hunter ids map to the lore-name ids, everything else is unchanged", () => {
  assert.deepEqual({ ...HUNTER_ID_ALIASES }, { eirys: "elise", jyvra: "javelle", quirith: "quenelle", zephyra: "zellia" });
  for (const [oldId, newId] of Object.entries(HUNTER_ID_ALIASES)) {
    assert.equal(canonicalHunterId(oldId), newId);
    assert.equal(canonicalHunterId(newId), newId);
  }
  assert.equal(canonicalHunterId("kathy"), "kathy");
  assert.equal(canonicalHunterId("constructor"), "constructor");
  assert.equal(canonicalHunterId(undefined), undefined);
});
