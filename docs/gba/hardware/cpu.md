# ARM7TDMI CPU

The GBA's CPU is an ARM7TDMI, the same core found in many phones and embedded devices of the early 2000s.
It implements version 4T of the ARM architecture and runs at the GBA's 16.78 MHz master clock ([ARM7TDMI TRM, DDI 0029G, chapters 1–3][trm-0029g]).
Evenfall implements that exact CPU, not a modern ARM with its later extensions.

The CPU has two instruction sets, and GBA games use both:

- **ARM** instructions are 32 bits wide and do more per instruction.
  Games copy their hottest routines (sound mixing, decompression, interrupt handlers) into the fast 32-bit internal RAM and run them in ARM state.
- **Thumb** instructions are 16 bits wide.
  The cartridge bus is only 16 bits wide, so fetching a Thumb instruction from the cartridge takes one access instead of two.
  Most game code runs as Thumb straight from the cartridge.

The CPU fetches instructions and data over the same bus as everything else.
It never decides what an access costs.
It only says what it accesses, and the bus charges the cycles ([`memory.md`](memory.md)).
This is why the same instruction takes one cycle in internal RAM and five or more from the cartridge.

## Where it lives

The instruction interpreter is in [`crates/cpu-arm/src/lib.rs`](../../../crates/cpu-arm/src/lib.rs) and [`thumb.rs`](../../../crates/cpu-arm/src/thumb.rs) (`Core::execute_arm`, `Core::execute_thumb`).
It knows nothing about the GBA, so a future system with an ARM CPU can reuse it.
The GBA side, which connects it to the bus and the BIOS, is in [`cpu/mod.rs`](../../../systems/gba/src/cpu/mod.rs).

The interpreter is the reference.
Evenfall also has a faster execution engine that runs pre-decoded code, and it must produce exactly the same registers, memory and cycle counts ([`performance.md`](../../performance.md#where-the-speed-comes-from)).

## A few terms

- **Register.** The CPU has sixteen 32-bit registers, r0 to r15.
  r13 is the stack pointer (`sp`), r14 the link register (`lr`) that holds a return address, and r15 the program counter (`pc`).
- **CPSR.** The current program status register holds the condition flags (N, Z, C, V), the interrupt masks (I, F), the instruction set (T: 1 for Thumb) and the mode.
- **Mode and SPSR.** A mode is the context the CPU runs in (normal code, an interrupt handler, a system call).
  Most modes have their own copy of some registers and a saved program status register (SPSR) that keeps the CPSR from before the mode was entered.
- **Exception.** An event that makes the CPU jump to a fixed address (a vector) in the BIOS and switch mode: an interrupt, a system call (`SWI`), or an undefined instruction.
- **UNDEFINED and UNPREDICTABLE.** Arm's manuals mark some encodings UNDEFINED, which means they trap in a defined way, and others UNPREDICTABLE, which means the manual promises nothing.

## States and modes

CPSR bit 5 (T) selects ARM or Thumb state.
Only `BX` and an exception return that restores T from an SPSR switch between them.
On this CPU, ordinary loads into the program counter stay in the current state ([below](#the-visible-pc-and-the-pipeline)).

| Mode       | M[4:0] | Own registers         | SPSR | When Evenfall enters it                  |
| ---------- | ------ | --------------------- | ---- | ---------------------------------------- |
| User       | `10h`  | none                  | no   | only when guest code switches to it      |
| FIQ        | `11h`  | r8–r14                | yes  | only when guest code switches to it      |
| IRQ        | `12h`  | r13, r14              | yes  | on an interrupt                          |
| Supervisor | `13h`  | r13, r14              | yes  | at reset, and on `SWI` with a real BIOS  |
| Abort      | `17h`  | r13, r14              | yes  | only when guest code switches to it      |
| Undefined  | `1Bh`  | r13, r14              | yes  | on an undefined instruction              |
| System     | `1Fh`  | shares User registers | no   | after HLE and skip-BIOS boot, HLE resets |

On every mode change, `Core::switch_banked_mode` swaps the mode's own registers in, so r13 and r14 (and r8–r12 in FIQ) always show the current mode's copy.

The GBA wires the FIQ input so that it never fires ([GBATEK, GBA Interrupt Control][gbatek-fiq]), and nothing on its bus signals an abort.
Evenfall therefore never raises FIQ or abort exceptions.
Guest code can still switch into those modes with `MSR` and use their registers.

`MSR` writes the program status registers by field:

- The flags field writes CPSR bits 31–28.
  In User mode, a write to the control field (mode and interrupt masks) is ignored.
- An SPSR write updates only the flags and control fields of the current mode's SPSR.
- A CPSR write that selects an invalid mode, or changes T directly, is UNPREDICTABLE.
  Evenfall stops the machine with a fault instead of guessing ([below](#undefined-versus-unpredictable)).

## The visible PC and the pipeline

The ARM7TDMI works on three instructions at once: it executes one, decodes the next and fetches the one after.
`Cpu` models this with a two-entry `Pipeline` of instructions already fetched.
Each step executes the oldest entry and fetches the next instruction two instruction widths ahead.

Because of this, an instruction that reads r15 sees an address ahead of its own:

| Use of r15                                                      | Value seen                                     |
| --------------------------------------------------------------- | ---------------------------------------------- |
| ARM operand or address base                                     | instruction address + 8                        |
| ARM `Rn` or `Rm` when operand 2 uses a register-specified shift | instruction address + 12                       |
| ARM `STR pc` and `STM` including `pc`                           | instruction address + 12                       |
| Thumb operand                                                   | instruction address + 4                        |
| Thumb PC-relative `LDR` and `ADD rd, pc`                        | instruction address + 4, with bits 1–0 cleared |
| Thumb empty `PUSH {}` or `STMIA rb!, {}` stored value           | instruction address + 6                        |

Instructions already in the pipeline run as they were fetched.
Code that overwrites the next two instructions only changes later fetches.
The jsmolka NES edge-case ROM checks this with self-modifying code copied to VRAM ([compatibility](../../compatibility.md)).

**Branches.** A taken branch, `BX`, any write to r15, or an exception throws the pipeline away and refills it: one non-sequential fetch at the target, then one sequential fetch after it (`Cpu::refill_pipeline`).
The CPU checks for an interrupt at the end of every instruction, a taken branch included, so a pending interrupt is taken once the branch has refilled the pipeline, before the target runs.
The mGBA suite's timer count-up table, recorded on hardware, requires it: no instruction at an interrupt's return address runs before the next interrupt of a timer that overflowed while the handler ran.

**Switching instruction sets.** On this CPU, only `BX` switches between ARM and Thumb.
`LDR pc`, `LDM {…, pc}`, Thumb `POP {…, pc}`, `MOV pc, …` and Thumb `ADD` or `MOV` to r15 all stay in the current state.
The target is aligned for that state: bits 1–0 cleared in ARM, bit 0 cleared in Thumb.
`BX` selects Thumb when bit 0 of the target is set.
A Thumb `BL` pair writes the return address with bit 0 set.

## Cycle classes and timed accesses

Arm sorts every cycle into three classes ([DDI 0029G, chapters 3 and 6][trm-0029g]):

- **N** (non-sequential): an access to an address unrelated to the previous one.
- **S** (sequential): an access to the address right after the previous one, with the same width.
- **I** (internal): a cycle with no memory access, for example while the multiplier works.

The same instruction costs different amounts in IWRAM, EWRAM and cartridge ROM, so no instruction has a fixed cycle count in Evenfall.
Each instruction reports its accesses (address, width, N or S, code or data) and its internal cycles (`Bus::elapse_cpu_internal`), and the bus prices them by region, wait states and prefetch state.
The bus also applies the rule that a data access, I/O included, makes the next instruction fetch non-sequential ([`memory.md`](memory.md#sequential-and-non-sequential-accesses)).

Besides the step's own instruction fetch, each instruction reports:

| Instruction                                                        | Accesses                                                                 |
| ------------------------------------------------------------------ | ------------------------------------------------------------------------ |
| Data processing, immediate or immediate-shift operand              | none                                                                     |
| Data processing, register-specified shift                          | 1 I                                                                      |
| `B`, `BL`, `BX`, or a data-processing or load write to r15         | N fetch + S fetch at the target                                          |
| Single load (`LDR`, `LDRB`, `LDRH`, `LDRSB`, `LDRSH`, Thumb loads) | 1 N data read, 1 I                                                       |
| Single store                                                       | 1 N data write                                                           |
| `LDM` or `POP` of n words                                          | N, then S for each further word, 1 I                                     |
| `STM` or `PUSH` of n words                                         | N, then S for each further word                                          |
| `SWP`, `SWPB`                                                      | N read, N write, 1 I                                                     |
| `MUL` / `MLA`                                                      | m I / m + 1 I                                                            |
| `UMULL`, `SMULL` / `UMLAL`, `SMLAL`                                | m + 1 I / m + 2 I                                                        |
| Thumb `MUL`                                                        | m I, sized from Rd                                                       |
| Exception entry                                                    | N fetch + S fetch at the vector                                          |
| Interrupt entry                                                    | discarded fetch of the next opcode, then N fetch + S fetch at the vector |
| `LDRD`/`STRD` encodings, ARM `011` space with bit 4 set            | 1 I, then the Undefined-vector refill                                    |

The multiplier finishes early on small numbers.
Its cycle count m is the number of significant bytes of the multiplier operand, 1 to 4.
`MUL`, `MLA`, `SMULL` and `SMLAL` treat the operand as signed, so leading `FF` bytes don't count.
`UMULL` and `UMLAL` treat it as unsigned.
When the destination is also the multiplier (`mul r3, r3, r2`), timing uses the multiplier's value from before the write.
Thumb `MUL rd, rs` computes `rd * rs` and sizes m from `rd`, as NanoBoyAdvance does and as the mGBA suite's measurements show.

### The carry flag after a multiply

Flag-setting multiplies (`MULS`, `MLAS`, the long forms with S, and Thumb `MUL`, which always sets flags) set N and Z from the result and C from the multiplier's datapath.
V is unchanged.
Arm calls C meaningless there, but on the ARM7TDMI it is deterministic.
The multiplier recodes its operand in radix-4 Booth digits and sums four addends per cycle through carry-save stages into a sum vector and a carry vector.
It stops early when the remaining multiplier bits only extend its sign.
The ALU then adds the two vectors, taking the carry vector through the barrel shifter, whose carry-out becomes C.
`evenfall_cpu_arm::multiply_carry` models that datapath as far as the carry vector, following zayd's reconstruction from the multiplier patents and fuzzing on hardware ([Solving the Mystery of ARM7TDMI Multiply Carry Flag][multiply-carry]).
`MUL`, `MLA`, `SMULL` and `SMLAL` recode the operand as signed, `UMULL` and `UMLAL` as unsigned.
For Thumb `MUL rd, rs`, Rd is the multiplier.

The mGBA suite's multiply-long table, recorded on hardware, pins N, Z and C after every `SMULLS` and `UMULLS` it runs (`flag_setting_long_multiplies_match_the_mgba_suite_hardware_table` in [`cpu-arm`](../../../crates/cpu-arm/src/lib.rs)).
The other forms are pinned against the model's published reference implementation (`flag_setting_multiplies_take_c_from_the_reference_multiplier_model`), not against hardware directly.
Non-flag-setting multiplies change no flags, as jsmolka's ARM test checks.

Arm's timing tables are easy to misread.
The bus signals that describe an access are sent one cycle ahead of it, so the `SEQ` column on a row belongs to the next access, not to the address on the same row ([DDI 0210C, §3.3, §6.5, Table 6-4, §6.20][trm-0210c]).
Evenfall's timing is simply the sum the bus charges for the reported accesses.
The mGBA suite tables in [`memory.md`](memory.md#mgba-suite-timing-tables) check those sums for loads, stores, block transfers, branches, interworking and multiplies.

## Writing r15 from a data-processing instruction

An ARM data-processing instruction (`ADD`, `MOV`, `SUB` and the others that produce a result) can write its result to r15.
There are two forms ([ARM DDI 0100I, §A3.4][arm-0100i]):

- **Without S** (`<op> pc, …`), it is a plain jump.
  The CPSR doesn't change.
  `MOV pc, lr` returns from an ARM function, and unlike `BX lr` it never switches to Thumb.
- **With S** (`<op>S pc, …`), it is an exception return.
  The instruction jumps and copies the whole SPSR back into the CPSR, restoring the flags, masks, instruction set and mode from before the exception.
  `MOVS pc, lr` is the usual form.

In the S form, operands are read before the mode changes, so a banked `lr` is the exception mode's.
Restoring T = 1 resumes Thumb code without a `BX` ([DDI 0100I, `MOV`, pp. A4-68–A4-70][arm-0100i], [DDI 0210C, §§2.6–2.8][trm-0210c]).

The target is aligned for the state that runs next: bits 1–0 cleared for ARM, only bit 0 for Thumb.
An ARM-state target with bits 1–0 set is UNPREDICTABLE on ARMv4 ([DDI 0100I, §A2.4.3][arm-0100i]).
Evenfall clears them, and that is Evenfall's choice, not a documented behavior.

A write to r15 costs `2S + 1N`, plus `1I` with a register-specified shift: the sequential fetch of the old stream, which is thrown away, then the refill at the target.
When the condition fails, nothing happens and the instruction costs `1S`.
If the SPSR about to be restored holds an invalid mode, the machine stops with an `invalid_psr_state` fault before any register changes.

### `MOVS pc, lr` without an SPSR

User and System mode have no SPSR, so the S form is UNPREDICTABLE there ([DDI 0100I, p. A4-68][arm-0100i], [DDI 0210C, Table 2-2][trm-0210c]).
Commercial games still use `MOVS pc, lr` outside exception modes.
Evenfall runs it as a jump that sets the flags from the result and restores nothing, as mGBA does.

`TST`, `TEQ`, `CMP` and `CMN` have no destination register.
An encoding of one of them with S set and Rd = r15 is an old form of PSR restore (`legacy_psr_restore` in `Core::execute_arm_data_processing`).
In a mode with an SPSR it copies the SPSR to the CPSR without jumping.
In User or System mode it does nothing.

## Block transfers

`LDM` and `STM` load or store a list of registers.
In every addressing mode, the lowest-numbered register goes to the lowest address.
Each word access clears address bits 1–0, while the address sequence and the base writeback keep the original low bits.
This applies to ARM `LDM` and `STM`, Thumb `LDMIA` and `STMIA`, and every Thumb `PUSH` and `POP`, including `lr` and `pc`.

When the base register is in the list and writeback is on:

- `STM` stores the original base if it is the first register stored, and the updated base otherwise.
- `LDM` keeps the loaded value and skips writeback.

The S bit (`^`) means two different things.
`LDM` with r15 in the list is an exception return: after the transfer, the SPSR is copied to the CPSR and the target is aligned for the restored state.
Without r15, or for `STM`, the transfer uses the User-mode registers whatever the current mode.

### Empty register lists

An assembler won't produce an empty list, but GBA software and test ROMs can run the raw encoding.
GBATEK records that on this CPU it transfers r15 alone and moves the base by `40h`, as if all sixteen registers had been transferred ([GBATEK, ARM block transfers][gbatek-ldm], [GBATEK, Thumb multiple transfers][gbatek-thumb-ldm]).
NanoBoyAdvance models the stack form the same way ([Thumb handler][nba-thumb]).
This is specific to the ARM7, not general ARM behavior.

| Encoding              | Transfer                                                                                        | Base                   |
| --------------------- | ----------------------------------------------------------------------------------------------- | ---------------------- |
| ARM `STM rn, {}`      | stores instruction address + 12 at the lowest address of the 16-word block the mode would cover | ± `40h` with writeback |
| ARM `LDM rn, {}`      | loads r15 and jumps                                                                             | ± `40h` with writeback |
| Thumb `STMIA rb!, {}` | stores instruction address + 6                                                                  | + `40h`                |
| Thumb `LDMIA rb!, {}` | loads r15 and jumps, staying in Thumb                                                           | + `40h`                |
| Thumb `PUSH {}`       | stores instruction address + 6 at `sp − 40h`                                                    | − `40h`                |
| Thumb `POP {}`        | loads r15 from `sp` and jumps, staying in Thumb                                                 | + `40h`                |

Each form makes one word access.

## Single, halfword and signed transfers

- **Unaligned loads.** `LDR` reads the aligned word and rotates it right by 8 * (address bits 1–0).
  `LDRH` from an odd address rotates the halfword by 8.
  `LDRSH` from an odd address loads the addressed byte, sign-extended, as `LDRSB` would.
  `SWP` rotates like `LDR`.
  Stores clear the low address bits.
  The save chip window (`0E000000h`–`0FFFFFFFh`) is the exception and gets the exact address ([`memory.md`](memory.md#region-behavior)).
- **Loads into r15.** `LDR pc` jumps to the loaded value with bits 1–0 cleared.
  `STR pc` stores the instruction address + 12.
- **PC-relative halfword loads.** Halfword and signed loads may use `pc` as the base, pre-indexed and without writeback.
  The base is then the instruction address + 8.
  Other uses of r15 in these instructions are UNPREDICTABLE and stop the machine ([DDI 0100I][arm-0100i], with mGBA and NanoBoyAdvance as cross-checks).
- **Loads end with an internal cycle.** Every single-register load charges one I after its data access.
  This is what gives the mGBA suite's IWRAM costs of one cycle for `NOP`, two for `STR` and three for `LDR`.

### `LDRD` and `STRD`

Two halfword-transfer encodings became `LDRD` and `STRD` on later ARM CPUs (ARMv5E).
On this CPU, Arm's decoding rules make them UNDEFINED ([DDI 0100I, §A3.16][arm-0100i]).
Evenfall takes the Undefined trap without touching memory or the base register, after one internal cycle, as the ARM7TDMI's trap sequence does ([DDI 0029G, §6.18][trm-0029g]).
mGBA 0.10.5 agrees.

NanoBoyAdvance treats them as UNPREDICTABLE instead, based on an observation of a related core, the ARM7TDMI-S.
Nobody has reproduced that on a GBA, so the architectural rule wins.

## Exceptions

| Exception      | Vector | Mode       | `lr` on entry        | Return            | When Evenfall raises it                    |
| -------------- | ------ | ---------- | -------------------- | ----------------- | ------------------------------------------ |
| Reset          | `00h`  | Supervisor | —                    | —                 | booting with a real BIOS                   |
| Undefined      | `04h`  | Undefined  | next instruction     | `MOVS pc, lr`     | reserved encodings                         |
| SWI            | `08h`  | Supervisor | next instruction     | `MOVS pc, lr`     | `SWI` with a real BIOS                     |
| Prefetch abort | `0Ch`  | Abort      | —                    | —                 | never                                      |
| Data abort     | `10h`  | Abort      | —                    | —                 | never                                      |
| IRQ            | `18h`  | IRQ        | next instruction + 4 | `SUBS pc, lr, #4` | an enabled interrupt while CPSR.I is clear |
| FIQ            | `1Ch`  | FIQ        | —                    | —                 | never                                      |

"Next instruction" is the address after the trapping one: +4 in ARM, +2 in Thumb.
On entry (`Core::enter_exception`), the CPU saves the CPSR in the new mode's SPSR, writes the new mode's `lr`, switches mode, clears T, sets I, leaves F alone, and refills the pipeline at the vector in ARM state.
Interrupt entry takes 2S + 1N, as GBATEK's cycle table gives for a trap: its first cycle fetches the interrupted code's next opcode, which is discarded, and the refill at the vector is the rest (`Cpu::enter_irq`).
An exception return may restore T from the SPSR.
That is legal, unlike changing T directly with `MSR`.
`LDM sp!, {…, pc}^` is the block-transfer form of the return.

An interrupt is taken between instructions, after any instruction, when an enabled interrupt has reached the CPU and CPSR.I is clear.
The request is `IE & IF`, gated by `IME`, and it reaches the CPU four cycles after it rises ([`timers-dma-interrupts.md`](timers-dma-interrupts.md#interrupts)).
Its sources, and how interrupts wake the CPU from HALT, are described in [`timers-dma-interrupts.md`](timers-dma-interrupts.md).
With the HLE BIOS, `SWI` and interrupt entry run Evenfall's own BIOS code instead of the vectors ([`bios.md`](bios.md)).

Booting with a real BIOS starts at the reset vector in Supervisor mode, with both interrupt masks set (CPSR `D3h`).
The state after HLE boot is in [`bios.md`](bios.md#state-after-hle-boot).

## Undefined versus UNPREDICTABLE

Arm's manuals describe two kinds of invalid instruction, and Evenfall handles them differently.

**UNDEFINED** encodings trap to the Undefined vector, which a game can handle like any other exception.
Evenfall traps on:

- the coprocessor instructions (`CDP`, `LDC`, `STC`, `MCR`, `MRC`) for every coprocessor number, since the GBA has no coprocessor apart from a dummy CP14 unit ([GBATEK, coprocessor instructions][gbatek-cp]),
- the ARM `011` single-transfer space with bit 4 set, after one internal cycle and only when the condition passes,
- the ARMv5E `LDRD` and `STRD` encodings, after one internal cycle,
- the Thumb first bytes `B1`–`B3`, `B6`–`BB` and `BE`–`BF`, the never-true condition `DE`, and the ARMv5 `BLX` suffix `E8`–`EF`.

**UNPREDICTABLE** encodings have no defined behavior at all.
Evenfall doesn't guess one, and doesn't crash the host.
The machine stops with a typed fault before any register write, memory access or pipeline refill ([architecture](../../architecture.md#faults)).
Stepping again returns the same fault without advancing time, and a stopped machine refuses to write a save state.
The fault reasons are:

- **`unpredictable_arm_encoding`**, with the instruction class and the set of offending registers.
  Raised for:
  - `MRS pc`,
  - `MUL` or `MLA` naming r15 as Rd, Rm, Rs, or Rn when accumulating,
  - `SWP` naming r15 anywhere,
  - a long multiply naming r15, or with RdHi = RdLo,
  - a halfword or signed transfer with Rd = r15, Rm = r15, or `Rn = pc` with post-indexing or writeback,
  - a single transfer with `Rn = pc` and post-indexing or writeback, or a byte transfer with Rd = r15.
- **`missing_spsr`**, with the operation and the current mode.
  Raised for `MRS` or `MSR` of the SPSR, or `LDM … {pc}^`, in User or System mode.
- **`invalid_psr_state`**, with the operation, the kind and the offending value.
  Raised for an `MSR` to the CPSR that selects an invalid mode or changes T, and for an exception return that restores an SPSR with an invalid mode.

The well-defined uses of r15 all still work: PC-relative loads without writeback, word transfers of the PC, data-processing writes to the PC, and block transfers of the PC.
`MOVS pc, lr` in User mode isn't a fault either.
`Fault::write_diagnostic` prints each fault as one stable line, for example `reason=unpredictable_arm_encoding class=multiply offending_registers=8000`.

Stopping protects the host and makes the problem visible.
It says nothing about what a real GBA does with these encodings.

## How it is checked

- [`tests/machine_a.rs`](../../../systems/gba/tests/machine_a.rs) holds the focused CPU tests: the interrupt taken after a branch refill, empty register lists, `MOVS pc, lr` in User mode, `LDRD` and `STRD`, exceptions taken and returned from every mode, and one test per fault reason, each checking that nothing changed before the fault.
- The execution engine is compared with the interpreter on random code ([`performance.md`](../../performance.md#execution-engine)).
- The [jsmolka ARM and Thumb test ROMs](../../compatibility.md) run every instruction class and pass.
- The mGBA suite tables in [`memory.md`](memory.md#mgba-suite-timing-tables) check instruction timing against hardware.

## Open questions

- **Misaligned ARM jumps.** Evenfall clears bits 1–0 of an ARM-state PC write.
  The architecture leaves it UNPREDICTABLE, and no hardware test here shows what the GBA does.
- **`MOVS pc` in User or System mode.** The jump-and-set-flags behavior matches mGBA and what games expect, but hasn't been measured.
- **Compare instructions with Rd = r15 and S set.** The SPSR-to-CPSR copy, and doing nothing in User and System mode, haven't been tested on hardware.
- **Compare encodings with S clear.** Encodings in this space that aren't `MRS`, `MSR`, `BX`, `SWP` or a multiply run as ordinary compares, although the architecture reserves them.
- **`LDRD` and `STRD`.** Evenfall follows Arm's decoding rule and mGBA.
  NanoBoyAdvance's ARM7TDMI-S observation hasn't been checked on a GBA.
- **Empty-list timing.** The single-word transfer and the `40h` base step come from GBATEK and NanoBoyAdvance.
  Their cycle cost hasn't been measured.
- **Multiply carry outside the long forms.** The carry model reproduces the recorded `SMULLS` and `UMULLS` table.
  Its `MULS`, `MLAS`, `UMLALS` and `SMLALS` results rest on its author's hardware fuzzing, which is published as the model rather than as measurements.
- **Aborts.** Evenfall never raises one.
  No GBA source of an abort is known, but no hardware test rules one out.
- **CP14.** GBATEK mentions a dummy CP14 unit.
  Evenfall sends CP14 instructions to the Undefined trap like every other coprocessor, and doesn't model what the dummy unit returns.

[trm-0029g]: https://documentation-service.arm.com/static/5e8e1323fd977155116a3129
[trm-0210c]: https://documentation-service.arm.com/static/5f4786a179ff4c392c0ff819
[arm-0100i]: https://documentation-service.arm.com/static/5f8dacc8f86e16515cdb865a
[gbatek-ldm]: https://rust-console.github.io/gbatek-gbaonly/#armopcodesmemoryblockdatatransferldmstm
[gbatek-thumb-ldm]: https://rust-console.github.io/gbatek-gbaonly/#thumbopcodesmemorymultipleloadstorepushpopandldmstm
[nba-thumb]: https://github.com/nba-emu/NanoBoyAdvance/blob/master/src/nba/src/arm/handlers/handler16.inl
[gbatek-fiq]: https://rust-console.github.io/gbatek-gbaonly/#gbainterruptcontrol
[gbatek-cp]: https://rust-console.github.io/gbatek-gbaonly/#armopcodescoprocessorinstructionsmrcmcrldcstccdpmcrrmrrc
[multiply-carry]: https://bmchtech.github.io/post/multiply/
