# Last Route patch to quad-rand 0.2.3

The crates.io source with one change, marked "Last Route patch" in `src/lib.rs`: the generator's 64-bit state is an
`AtomicU64` where the target has one, and two `AtomicU32` halves where it does not (32-bit PowerPC, `docs/POWERPC.md`).
The numbers are the same on every target. Upstream already loads and stores the state in separate steps, so the two
halves change nothing about what concurrent callers can see.
