# Dispatch diagnostics

Three bare-metal loops that measure what the CPU costs per guest instruction, with nothing else running.
The display is in forced blank, and no interrupt, DMA or timer is enabled.
Each loop body is 32 instructions and a branch, so the number of guest instructions per frame is fixed by the loop's cycle cost.

| ROM             | Loop                                    | Guest instructions per frame |
| --------------- | --------------------------------------- | ---------------------------: |
| `arm-alu.gba`   | ARM ALU operations running from IWRAM   |                     ~265,000 |
| `arm-mem.gba`   | ARM IWRAM loads and stores, from IWRAM  |                     ~123,000 |
| `thumb-rom.gba` | Thumb ALU operations from cartridge ROM |                      ~87,000 |

Build them with `make -C tools/bench/diag` (needs `arm-none-eabi-gcc`), then divide host instructions by guest instructions:

```sh
perf stat -e instructions -- target/release-lto/evenfall-harness \
  tools/bench/diag/build/arm-alu.gba 300
```

Measuring 330 and 30 frames and taking the difference removes start-up costs.
The guest counts come from running each ROM for a fixed number of steps with the command-line runner and dividing by the cycles it reports.
Any other harness that follows the [benchmark protocol](../README.md#the-harness-protocol) can be measured the same way.
