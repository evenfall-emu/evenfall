## What and why

<!-- The visible behavior or developer-facing change, and the reason for it. -->

## Evidence

<!--
Emulator behavior: cite the source (Arm manual, GBATEK section, test ROM,
hardware observation) and name the focused tests you added.
Performance: link the measurement and the binaries measured (docs/performance.md).
Guest behavior must stay exactly the same.
-->

## Checklist

- [ ] `cargo test --workspace` passes in debug and release
- [ ] Affected validators and SDK tests pass
- [ ] Documentation updated where behavior, APIs or state formats change
- [ ] No ROMs, BIOS images, saves or other copyrighted material added
