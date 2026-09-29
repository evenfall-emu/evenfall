# Dispatch diagnostics

Three bare-metal loops that measure what the CPU interpreter costs per guest
instruction, with nothing else running: the display is in forced blank and no
interrupt, DMA or timer is enabled. Each loop body is 32 instructions and a
branch, so guest instructions per frame are fixed by the loop's cycle cost.

| ROM             | Loop                                   | Guest instructions per frame |
| --------------- | -------------------------------------- | ---------------------------: |
| `arm-alu.gba`   | ARM ALU operations running from IWRAM  |                     ~265,000 |
| `arm-mem.gba`   | ARM IWRAM loads and stores, from IWRAM |                     ~123,000 |
| `thumb-rom.gba` | Thumb ALU operations from Game Pak ROM |                      ~87,000 |

Build with `make -C tools/bench/diag` (needs `arm-none-eabi-gcc`), then divide
host instructions by guest instructions:

```sh
perf stat -e instructions -- target/release-lto/evenfall-harness \
  tools/bench/diag/build/arm-alu.gba 300
perf stat -e instructions -- target/bench/zigba-harness \
  tools/bench/diag/build/arm-alu.gba 300
```

The guest counts come from running each ROM for a fixed number of steps with
the command-line runner and dividing by the cycles it reports.
