# LR Story pack format and commands

LR Story (`src/bin/lr_story`) plays a story pack: a folder of JSON and PNG files that the engine reads, merges and plays
in first person.

## Layout

```text
<pack>/
  pack.json           manifest (required)
  scenes/*.json       lr.story.scenes/1: dialogue scenes
  items/*.json        lr.story.items/1: item definitions
  maps/*.json         lr.story.map/1: a map's name and legend
  maps/*.png          the floor picture (a whole multiple of the mask) and the mask (one pixel per tile)
```

The core pack defaults to `src/bin/last_route/data/lr_story` (`--pack DIR` or `LR_STORY_PACK` replaces it). Mods are read
from `mods/` beside the executable, in the working folder and per-user, plus `--mod DIR`, merged in id order.

## pack.json

```json
{ "schema": "lr.pack/1", "id": "my_story", "name": "My story", "version": "0.1.0" }
```

| Field | Rule |
| --- | --- |
| `schema` | Must be `lr.pack/1`. |
| `id` | Required. 3 to 32 characters: lower-case letters, digits and `_`, starting with a letter. The folder name must equal it. |
| `name`, `version`, `author`, `description` | Optional. `name` defaults to the id, `version` to `0.0.0`. |
| `overrides` | Optional list of `scenes/<id>`, `items/<id>`, `maps/<id>` or `start`. See below. |
| `start` | `{"map", "spawn"}`. Required in the core pack. A mod may set it only if `overrides` lists `start`. |
| `start_scene` | Optional scene id that runs before play starts. |

## Ids and overrides

- A mod's scene, item and map ids must start with `<pack id>_`, so they never collide. The core pack has no prefix rule.
- A mod may replace an id an earlier pack defined only if it lists `"<kind>/<id>"` in `overrides`. A second definition
  without that entry is an error, and so is an override of an id no pack defines.
- Scene targets, item ids, exits and spawn names are all checked at load.

## Scenes (`lr.story.scenes/1`)

```json
{ "schema": "lr.story.scenes/1", "scenes": { "my_story_intro": {
    "once": true,
    "lines": ["Plain text is narration.", { "speaker": "Felix", "text": "Hello." }],
    "choices": [{ "text": "Take the card", "if": { "not_flag": "took_card" },
                  "do": [{ "op": "set_flag", "flag": "took_card" }], "next": "my_story_after" }],
    "next": "my_story_after" } } }
```

- `lines`: a string, or `{speaker, text}`. No speaker means narration. `once`: run only the first time (default false).
- `choices`: offered once the lines are done. Each has `text`, an optional `if` (hidden when false), `do` and `next`.
  Picking one runs its `do` and starts its `next`; with no `next` the scene ends. While choices are offered, the
  scene's own `do` and `next` are not used.
- `do` and `next`: when no choice is offered, the scene's `do` runs and `next` starts. A run of scenes stops after 32
  in a row without the player acting.

Conditions (`if`, `cond`): `{"flag": "f"}`, `{"not_flag": "f"}`, `{"has": "item"}`, `{"not": C}`,
`{"all": [C, ...]}`, `{"any": [C, ...]}`.

Ops (`do`): `{"op": "set_flag", "flag": "f", "value": true}` (value defaults to true), `{"op": "give", "item": "id"}`,
`{"op": "take", "item": "id"}`, `{"op": "exit", "map": "id", "spawn": "name"}`, `{"op": "toast", "text": "..."}`.

Flags are kept between runs in `lr_story_flags.json` in the user folder, under the core pack's id.

## Items and maps

`items/*.json`: `{"schema": "lr.story.items/1", "items": {"keycard": {"name": "Keycard", "description": "..."}}}`.

`maps/*.json`: `{"schema": "lr.story.map/1", "id", "name", "art", "mask", "legend"}`. `id` defaults to the file name and
`mask` to the art. Each legend key is a 6-digit hex colour (or `clear`); a mask colour with no entry is a load error.

| Kind | Fields |
| --- | --- |
| `floor` | none |
| `wall` | `height` (metres, 0 to 16, default 3), `color` |
| `spawn` | `name`: where the player can arrive |
| `door` | `if` (opens when true), `locked` (message), `height`, `color` |
| `item` | `item`: picked up once when the player steps on it |
| `npc` | `name`, `scene` (talk with E) |
| `trigger` | `scene`, `once` (default true): runs when the player steps on it |
| `exit` | `map`, `spawn`: moves the player to another map |

## LORE choices (KyttusLab-LORE import)

`lore.rs` turns each LORE file into scenes. Each `Story` chapter becomes one scene, `<pack>_<file>_ch<N>`, chained with
`next`. A `???` speaker is narration. The text is copied as written; `Title` is not copied.

A chapter may add a `Choices:` list:

```yaml
    Chapter 1:
      Summary:
        - ???: The corridor splits.
      Choices:
        - Text: Go left
          Needs: [lantern]        # all listed flags must be set for the choice to show
          Sets: [went_left]       # flags set when it is picked
          Goto: Chapter 2         # optional; without it the choice goes to the next chapter
```

Flag names are lower-case letters, digits and `_`. `Goto` must name a chapter in the same file. A chapter with choices
has no `next` of its own, and the last chapter's choice without `Goto` ends the scene.

## Commands

Run from the repo root with `cargo run --bin lr_story -- <command>` (or the built binary). `--pack` and `--mod` apply
to every command.

```text
lr_story                              open the game window on the pack
lr_story --shot out.png [--scene ID]  save a few frames to out.png and exit
lr_story check                        load and validate (same as --check; status 1 on problems)
lr_story init <id> [parent]           write a starter mod to <parent or ./mods>/<id>, then check it
lr_story pack <dir>                   print the pack's id, version, file count, bytes and sha256
lr_story import-lore <files...> --id <id> [--into DIR]   LORE files to scenes (default DIR ./mods/<id>), then check
```

`check` prints one OK line when the pack loads, e.g. `OK: 'Dive Lab (sample pack)' - 2 map(s), 6 scene(s), 1 item(s)`.

`init` writes `pack.json`, `scenes/main.json` (one scene, `<id>_intro`, with one narration line) and an empty
`items/items.json`. It refuses an invalid id or an existing folder, then runs the check:

```text
$ lr_story init my_story
created mods\my_story
OK: 'Dive Lab (sample pack)' - 2 map(s), 6 scene(s), 1 item(s)
```

`pack` prints a fingerprint. The sha256 covers every file in sorted path order (`/` separators): for each file it hashes
the relative path, a NUL, the bytes and a NUL. Changing, adding or renaming any file changes it:

```text
$ lr_story pack mods/demo_mod
id: demo_mod
version: 0.1.0
files: 3
bytes: 316
sha256: 677e74e71c779a873a51bd41a09f36c2fc71bc065e3a90fe611b5a280fea5f8e
```

`import-lore` takes one or more `.yml` files and refuses an invalid id, suggesting a valid one:

```text
$ lr_story import-lore fork.yml --id my_fork
imported 2 scene(s) into mods\my_fork
```
