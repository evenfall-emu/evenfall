# mGBA suite ROM

Builds the pinned [mGBA suite](https://github.com/mgba-emu/suite) ROM and runs
it on ZiGBA and Evenfall through the parity harness. Results and their triage
are in [`docs/compatibility.md`](../../docs/compatibility.md#mgba-suite-rom).

## Build

The suite needs devkitARM and libgba. `build.sh` runs `make` in the
`devkitpro/devkitarm` container image pinned by digest, with podman or
Docker (`CONTAINER_ENGINE` selects one). It exports the pinned suite commit
from the `refs/suite` checkout with `git archive`, so the checkout is only
read, and builds in `tools/mgba-suite/build/`, which is gitignored. The same
image and commit produce the same ROM bytes.

```sh
tools/setup-refs.sh suite zigba cult-gba-bios
tools/mgba-suite/build.sh            # or: tools/mgba-suite/build.sh PATH/TO/suite
```

## Run

```sh
cargo run --release -p evenfall-parity -- oracle build
cargo run --release -p evenfall-parity -- mgba-suite run
cargo run --release -p evenfall-parity -- mgba-suite report --log
```

`mgba-suite run` runs every `tools/parity/scenarios/mgba-suite-*.json`
scenario through the ordinary parity check, ZiGBA against Evenfall under
three slice schedules, then prints each suite's results from Evenfall's
trace: the suite's own passing and total check counts, its passing tests,
and every failing test by index and name. ZiGBA's states use its own format,
so the report reads the list from Evenfall's states only and prints whether
ZiGBA's checkpoint frames, which draw the list, and SRAM logs match. It
exits nonzero when a scenario fails parity, a trace cannot be read, or the
two cores' checkpoints differ;
failing suite tests are results, not errors. `report` re-reads existing
traces, and `--log` adds the SRAM log the suite writes, which holds the
got/expected values of every failing check. In "Misc. edge case tests" the
suite prints the expected value first.

Each scenario boots the ROM (HLE, or the Cult-of-GBA BIOS for the
`-external_bios` variants), presses DOWN to select one suite and A to run it,
waits, then presses RIGHT to page through the result list with a checkpoint
on every page. The report loads each of Evenfall's checkpoint states and reads the
list from the text tilemap at `06000800h`: a tile index is a character minus
`' '`, and palette bank 1 marks a failing line. It checks the list title, the
cursor row expected from the menu's paging rule and a final wrap to the first
test, so a suite that has not finished, or a wrong test count, fails the
report instead of producing results.

`evenfall-parity mgba-suite scenarios` regenerates the manifests from the
suite table in [`mgba_suite.rs`](../parity/src/mgba_suite.rs); run `npm run fmt`
afterwards, as for the other manifests. `run --all`
includes these scenarios, so it needs the ROM built as well.
