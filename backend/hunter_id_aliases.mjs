// Four hunters were renamed to their lore names and their internal ids followed. Ids that outlive a build (room settings from an older
// client, stored account data) can still be the old ones: normalise them on the way in. Mirrors game/config/hunter_id_aliases.gd, the
// only other place the old ids live (docs/HUNTER_ID_ALIASES.md).
export const HUNTER_ID_ALIASES = Object.freeze({
  eirys: "elise",
  jyvra: "javelle",
  quirith: "quenelle",
  zephyra: "zellia",
});

export function canonicalHunterId(id) {
  return Object.prototype.hasOwnProperty.call(HUNTER_ID_ALIASES, id) ? HUNTER_ID_ALIASES[id] : id;
}
