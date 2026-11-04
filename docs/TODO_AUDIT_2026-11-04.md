# TODO audit, 2026-11-04

Items skipped in TODO.md because they need a design decision or hand verification.

## Needs a decision from KT
- Markdown placement: README.md, TODO.md, MECHANICS.md and CREDITS.md sit at the root, and five NOTES.md or README.md files sit under src/. The rule says all except AGENTS.md live under docs/. Move them, or relax the rule?
- Search rules: no doc owns them. Which doc should?
- NPC task scope: only the door task exists, and Godot has no relay or locker task either. Are relay and locker tasks in scope?

## Needs hand verification (not changed)
- TODO "Admin tab order and keyboard navigation" is ticked; the audit found no matching code. Not unticked until checked.
- TODO "WORLD tab: regenerate the current floor" is ticked; same.
- TODO "Parallel Linux and macOS CI jobs" is ticked; not verified.
- TODO sewer geometry item (Infinite route and levels) may be stale after commits eb9904f and 8bc16f8; left open.
- Git history was not scanned for previously committed secrets.
