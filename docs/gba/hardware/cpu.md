# ARM7TDMI CPU

The GBA's CPU is an ARM7TDMI, an implementation of the ARMv4T architecture,
clocked from the 16.78 MHz master clock. Arm specifies two instruction sets,
32-bit ARM and 16-bit Thumb, a three-stage fetch/decode/execute pipeline and a
single 32-bit von Neumann bus for instructions and data
([ARM7TDMI TRM, DDI 0029G, chapters 1–3][trm-0029g]). ZiGBA implements that
architecture, not a generic modern AArch32 subset.

The machine CPU lives in [`cpu/mod.rs`](../../../systems/gba/src/cpu/mod.rs)
and the interpreter in [`crates/cpu-arm/src/lib.rs`](../../../crates/cpu-arm/src/lib.rs)
and [`thumb.rs`](../../../crates/cpu-arm/src/thumb.rs) (`Core::execute_arm`,
`Core::execute_thumb`). The scalar interpreter is the reference: optimized
execution paths, described in ZiGBA's
[`performance.md`](../../../refs/zigba/docs/performance.md), must
reproduce its register, memory and cycle results exactly. The CPU never prices
its own accesses; it emits them to the bus, which owns time
([`memory.md`](memory.md)).

## States and modes

CPSR bit 5 (T) selects ARM or Thumb state. Only `BX` and an exception return
that restores T from an SPSR change state; ordinary loads of the PC do not
interwork on ARMv4T (see [The visible PC](#the-visible-pc-and-the-pipeline)).

| Mode       | M[4:0] | Banked registers      | SPSR | Entered by ZiGBA on                |
| ---------- | ------ | --------------------- | ---- | ---------------------------------- |
| User       | `10h`  | none                  | no   | guest code only                    |
| FIQ        | `11h`  | r8–r14                | yes  | guest code only                    |
| IRQ        | `12h`  | r13, r14              | yes  | IRQ exception                      |
| Supervisor | `13h`  | r13, r14              | yes  | reset and SWI with a BIOS image    |
| Abort      | `17h`  | r13, r14              | yes  | guest code only                    |
| Undefined  | `1Bh`  | r13, r14              | yes  | Undefined-instruction trap         |
| System     | `1Fh`  | shares User registers | no   | HLE and skip-BIOS boot, HLE resets |

`Core::switch_banked_mode` swaps the banked registers on every mode change, so
`r13`/`r14` (and `r8`–`r12` for FIQ) always hold the current mode's view while
the other banks keep their saved values.

In a normal GBA the FIQ input is tied high, so no hardware raises FIQ
([GBATEK, GBA Interrupt Control][gbatek-fiq]), and nothing on the bus
signals a prefetch or data abort. ZiGBA therefore never raises those
exceptions. Guest code can still enter FIQ or Abort mode with `MSR` and use
their banks.

`MSR` follows the architectural field rules:

- The flags field (`f`) writes CPSR bits 31–28. In User mode a control-field
  (`c`) write is ignored rather than changing mode or masks.
- An `SPSR` write updates only the `f` and `c` fields of the current mode's
  SPSR.
- A CPSR control write that selects an invalid mode encoding, or that changes
  T directly, is UNPREDICTABLE. ZiGBA stops with a typed fault instead
  ([containment](#undefined-versus-unpredictable)).

## The visible PC and the pipeline

`Cpu` keeps the two opcodes already fetched ahead of the executing one in a
two-entry `Pipeline`. Each step executes the oldest entry and fetches the next
opcode at the executing address plus two instruction widths. Because of this
fetch-ahead, an instruction that reads r15 sees an address ahead of its own:

| Use of r15                                                      | Value seen                                      |
| --------------------------------------------------------------- | ----------------------------------------------- |
| ARM operand or address base                                     | instruction address + 8                         |
| ARM `Rn` or `Rm` when operand 2 uses a register-specified shift | instruction address + 12                        |
| ARM `STR pc` and `STM` including `pc`                           | instruction address + 12                        |
| Thumb operand                                                   | instruction address + 4                         |
| Thumb PC-relative `LDR` and `ADD rd, pc`                        | (instruction address + 4) with bits 1–0 cleared |
| Thumb empty `PUSH {}` / `STMIA rb!, {}` stored value            | instruction address + 6                         |

Opcodes already in the pipeline execute as fetched. A store that overwrites
the next two instructions changes only later fetches; the jsmolka NES
edge-case ROM checks this with self-modifying code copied to VRAM
([ZiGBA compatibility](../../../refs/zigba/docs/compatibility.md)).

**Branches and PC writes.** A taken branch, `BX`, a data-processing write to
r15, a load of r15, or exception entry discards the pipeline and refills it:
one non-sequential fetch at the target, then one sequential fetch at the
target plus the instruction width (`Cpu::refill_pipeline`). A pending IRQ is not
taken at the end of an instruction that branched; the first instruction at the
target executes before IRQ entry, because ARM7TDMI ignores exceptions during
the refill. The test "ARM7 defers a pending IRQ until after a taken branch
refill" ([ZiGBA's `machine_test.zig`](../../../refs/zigba/test/machine_test.zig)) pins this.

**No interworking outside `BX`.** On ARMv4T, `LDR pc`, `LDM {…, pc}`, Thumb
`POP {…, pc}`, `MOV pc, …` and Thumb high-register `ADD`/`MOV` to r15 stay in
the current state. The target is aligned for that state: bits 1–0 cleared in
ARM, bit 0 cleared in Thumb. `BX` selects Thumb when bit 0 of the target is
set. A Thumb `BL` pair writes the return address with bit 0 set.

## Cycle classes and timed accesses

Arm defines three bus-cycle classes
([DDI 0029G, chapters 3 and 6][trm-0029g]):

- **N** (non-sequential): an access whose address is unrelated to the
  previous one;
- **S** (sequential): an access to the address following the previous one,
  of the same width;
- **I** (internal): a cycle with no memory access.

The same instruction costs different amounts in IWRAM, EWRAM and Game Pak
ROM, so cycle counts are not baked into opcode handlers. Each instruction
emits its accesses, each carrying address, width, N/S kind and source (code
or data), and internal cycles through `Bus::elapse_cpu_internal`. The bus prices
every access by region, width, sequential state, `WAITCNT` and prefetch state,
and applies the rule that a data access to memory makes the next code fetch
non-sequential
([`memory.md`](memory.md#sequential-and-non-sequential-accesses)).

Besides the step's own fetch-ahead, ZiGBA emits:

| Instruction                                                        | Accesses emitted                     |
| ------------------------------------------------------------------ | ------------------------------------ |
| Data processing, immediate or immediate-shift operand              | none                                 |
| Data processing, register-specified shift                          | 1 I                                  |
| `B`, `BL`, `BX`, data-processing or load write to r15              | N fetch + S fetch at the target      |
| Single load (`LDR`, `LDRB`, `LDRH`, `LDRSB`, `LDRSH`, Thumb loads) | 1 N data read, 1 I                   |
| Single store                                                       | 1 N data write                       |
| `LDM`, `POP` of n words                                            | N, then S for each further word, 1 I |
| `STM`, `PUSH` of n words                                           | N, then S for each further word      |
| `SWP`, `SWPB`                                                      | N read, N write, 1 I                 |
| `MUL` / `MLA`                                                      | m I / m + 1 I                        |
| `UMULL`, `SMULL` / `UMLAL`, `SMLAL`                                | m + 1 I / m + 2 I                    |
| Thumb `MUL`                                                        | m I, sized from Rd                   |
| Exception entry                                                    | N fetch + S fetch at the vector      |
| `LDRD`/`STRD` encodings, ARM `011` space with bit 4 set            | 1 I, then the Undefined refill       |

The multiplier cycle count m is the number of significant bytes of the
multiplier operand, 1 to 4. `MUL`, `MLA`, `SMULL` and `SMLAL` size it as a
signed value, so leading one bytes are insignificant; `UMULL` and `UMLAL` size
it unsigned. When the destination aliases the multiplier (`mul r3, r3, r2`),
timing uses the multiplier's value before the write. Thumb `MUL rd, rs`
computes `rd * rs` and sizes m from `rd`, following NanoBoyAdvance's model and
the mGBA suite rows, which vary with that operand.

### The carry flag after a multiply

Flag-setting multiplies (`MULS`, `MLAS`, the long forms with S, and Thumb
`MUL`, which always sets flags) set N and Z from the result and C from the
multiplier's datapath; V is unchanged. ARM calls C meaningless there, but
on the ARM7TDMI it is deterministic: the multiplier recodes the multiplier
operand in radix-4 Booth digits, sums four addends per cycle through
carry-save stages into a sum vector and a carry vector, stops early when the
remaining multiplier bits only extend its sign, and resolves the two
vectors in the ALU with the carry vector taken through the barrel shifter,
whose carry-out becomes C. `evenfall_cpu_arm::multiply_carry` models that
datapath as far as the carry vector, following zayd's reconstruction from
the multiplier patents and fuzzing on hardware
([Solving the Mystery of ARM7TDMI Multiply Carry Flag][multiply-carry]).
`MUL`, `MLA`, `SMULL` and `SMLAL` recode signed, `UMULL` and `UMLAL`
unsigned; for Thumb `MUL rd, rs`, Rd is the multiplier. The mGBA suite's
multiply-long table, recorded on hardware, pins every `SMULLS` and `UMULLS`
result, N, Z and C
(`flag_setting_long_multiplies_match_the_mgba_suite_hardware_table` in
[`cpu-arm`](../../../crates/cpu-arm/src/lib.rs)); the other forms are pinned
against outputs of the model's published reference implementation
(`flag_setting_multiplies_take_c_from_the_reference_multiplier_model`), not
against hardware directly. ZiGBA leaves C unchanged; this is a deliberate
difference ([`roadmap.md`](../../roadmap.md#mgba-suite-rom-findings)).
Non-flag-setting multiplies change no flags, as jsmolka's ARM test checks.

The TRM's bus request and class signals are pipelined ahead of the accesses
they describe, so a cycle-table row's `SEQ` signal belongs to the next
access, not the address printed on the same row
([DDI 0210C, §3.3, §6.5, Table 6-4, §6.20][trm-0210c]). ZiGBA's timing is the
sum the bus charges for the emitted accesses; the suite tables in
[`memory.md`](memory.md#mgba-suite-timing-tables) check those sums for loads,
stores, block transfers, branches, interworking and multiplies.

## Data processing with r15 as destination

ARMv4T gives a condition-passing data-processing instruction two distinct
PC-write forms. They apply to the twelve result-producing opcodes `AND`,
`EOR`, `SUB`, `RSB`, `ADD`, `ADC`, `SBC`, `RSC`, `ORR`, `MOV`, `BIC` and `MVN`
([ARM DDI 0100I, §A3.4][arm-0100i]).

| Form                  | Architectural effect                                                                                                                                                         |
| --------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `<op> pc, …` (S = 0)  | Write the result to the PC and branch. Every CPSR field is unchanged. `MOV pc, lr` is an ARM-to-ARM return; it does not look at `lr[0]` and does not interwork like `BX lr`. |
| `<op>S pc, …` (S = 1) | Write the result to the PC and copy the current mode's entire SPSR to the CPSR. This is the data-processing exception return; `MOVS pc, lr` is the zero-adjustment case.     |

In the S = 1 form, N, Z, C, V, I, F, T and M[4:0] all come from the SPSR, not
from the ALU or the shifter. Operands, including a banked exception-mode
`lr`, are read before the restored mode changes the visible bank. Restoring
T = 1 resumes Thumb with no `BX`
([DDI 0100I, `MOV`, pp. A4-68–A4-70][arm-0100i];
[DDI 0210C, §§2.6–2.8][trm-0210c]).

The branch target is aligned for the state that runs next: bits 1–0 cleared
for ARM, only bit 0 cleared for a Thumb return, so bit 1 of a Thumb target
survives. An ARM-state PC write whose bits 1–0 are not `00` is UNPREDICTABLE
on ARMv4 ([DDI 0100I, §A2.4.3][arm-0100i]); ZiGBA clears them, which is an
emulator policy, not an architectural claim.

A successful PC write costs `2S + 1N`, or `2S + 1N + 1I` with a
register-specified shift: the old stream's sequential fetch, which is
discarded, the non-sequential fetch at the target, and the sequential fetch
after it. A failed condition performs neither the branch nor the SPSR restore
and costs `1S`. The region's wait states stretch the N and S fetches.

If the SPSR about to be restored holds an invalid mode, the instruction stops
with an `invalid_psr_state` fault before any register changes.

### User and System mode: the PC-write policy

User and System mode have no SPSR, so the S = 1 form is UNPREDICTABLE there
([DDI 0100I, p. A4-68][arm-0100i]; [DDI 0210C, Table 2-2][trm-0210c]).
Commercial code nevertheless issues `MOVS pc, lr` outside exception modes.
ZiGBA executes it as a branch with an ordinary S-bit flag update from the ALU
result, and restores nothing; mGBA does the same. This is a compatibility
policy, separate from the fault containment below, and is pinned by
"user-mode movs pc, lr branches with a flag update and no SPSR restore".

`TST`, `TEQ`, `CMP` and `CMN` have no destination. ZiGBA treats an encoding of
one of them with S = 1 and Rd = r15 as the legacy PSR-restore form
(`legacy_psr_restore` in `Core::execute_arm_data_processing`): in a mode with an SPSR it
copies the SPSR to the CPSR without branching, and in User or System mode it
has no effect.

## Block transfers

`LDM`/`STM` transfer the lowest-numbered register to the lowest address in
every addressing mode. Each word request drives an address with bits 1–0
cleared, while address progression and base writeback keep the original low
bits. This holds for ARM `LDM`/`STM`, Thumb `LDMIA`/`STMIA`, and every Thumb
`PUSH`/`POP` beat including `lr`/`pc`, and follows the ARM7TDMI aligned
word-bus rules rather than the general unaligned-load rotation. Tests with a
deliberately misaligned base check both the transfer addresses and the
low-bit-preserving writeback.

With writeback and the base in the list, `STM` stores the original base if it
is the first register transferred and the written-back base otherwise; `LDM`
keeps the loaded value and skips writeback.

The S bit (`^`) has two meanings. `LDM` with r15 in the list is an exception
return: after the transfer, the CPSR is restored from the SPSR and the target
is aligned for the restored state. Without r15, or for `STM`, the transfer
uses the User-mode registers regardless of the current mode.

### Empty register lists

An empty list is not a useful assembler operation, but GBA software and
conformance tests can execute the raw encoding. GBATEK records the ARMv4
observation that it transfers r15 alone while moving the base by `40h`, as if
sixteen registers had been transferred
([GBATEK, ARM block transfers][gbatek-ldm];
[GBATEK, Thumb multiple transfers][gbatek-thumb-ldm]). NanoBoyAdvance models
the stack form the same way, as one aligned non-sequential word
([NanoBoyAdvance Thumb handler][nba-thumb]). This is GBA/ARM7 behavior, not
general Arm semantics.

| Encoding              | Transfer                                                                                               | Base                   |
| --------------------- | ------------------------------------------------------------------------------------------------------ | ---------------------- |
| ARM `STM rn, {}`      | stores instruction address + 12 at the lowest address of the 16-word block the addressing mode selects | ± `40h` with writeback |
| ARM `LDM rn, {}`      | loads r15 and branches                                                                                 | ± `40h` with writeback |
| Thumb `STMIA rb!, {}` | stores instruction address + 6                                                                         | + `40h`                |
| Thumb `LDMIA rb!, {}` | loads r15 and branches, staying in Thumb                                                               | + `40h`                |
| Thumb `PUSH {}`       | stores instruction address + 6 at `sp − 40h`                                                           | − `40h`                |
| Thumb `POP {}`        | loads r15 from `sp` and branches, staying in Thumb                                                     | + `40h`                |

Each form makes one word access. Pinned by "ARM empty STM uses one PC beat
and sixteen-word addressing in every mode" and the Thumb `PUSH`/`POP`
empty-list tests in [ZiGBA's `machine_test.zig`](../../../refs/zigba/test/machine_test.zig).

## Single, halfword and signed transfers

- **Unaligned loads.** `LDR` reads the aligned word and rotates it right by
  8 × (address bits 1–0). `LDRH` from an odd address rotates the halfword by 8.
  `LDRSH` from an odd address loads the addressed byte sign-extended, as
  `LDRSB` would. `SWP` rotates like `LDR`. Stores clear the low address bits.
  The byte-wide save region (`0E000000h`–`0FFFFFFFh`) receives the exact
  address instead ([`memory.md`](memory.md#region-behavior)).
- **Loads into r15.** A word `LDR pc` branches to the loaded value with bits
  1–0 cleared. `STR pc` stores the instruction address + 12.
- **PC-relative halfword and signed loads.** The extension-space halfword and
  signed transfers permit the pre-indexed, no-writeback form with `Rn = pc`.
  ZiGBA uses the architectural visible PC, instruction address + 8, as the
  base. A destination of r15, a PC offset register, or a PC base with
  post-indexing or writeback stays UNPREDICTABLE and is contained
  ([DDI 0100I][arm-0100i]; mGBA and NanoBoyAdvance as cross-checks).
- **Single loads end with an internal cycle.** Every ARM and Thumb
  single-register load charges one I after its data access, reproducing the
  suite's IWRAM costs of one cycle for `NOP`, two for `STR` and three for
  `LDR`.

### `LDRD` and `STRD` are UNDEFINED

The halfword extension space assigns `L = 0, S/H = 10` and `L = 0, S/H = 11`
to `LDRD` and `STRD` only in the E variants of ARMv5 and later. Applying the
architecture-version decode rule of [DDI 0100I, §A3.16][arm-0100i] leaves both
encodings UNDEFINED on ARMv4T, so ZiGBA takes the Undefined trap with no data
access and no writeback. Following the ARM7TDMI trap sequence
([DDI 0029G, §6.18][trm-0029g]) it charges one internal cycle before the
Undefined-vector refill. Pinned mGBA 0.10.5 agrees: both decode-table slots
map to its illegal-instruction handler. The test "ARMv5 doubleword encodings
take Undefined on ARM7TDMI" covers both encodings, exception state and
return, unchanged registers and memory, and exact timing.

NanoBoyAdvance instead labels these encodings UNPREDICTABLE and models an
ARM7TDMI-S idle/writeback observation. That secondary claim has not been
reproduced on GBA silicon, so it does not override the architectural
classification.

## Exceptions

| Exception      | Vector | Mode       | `lr` on entry        | Return            | Raised by ZiGBA                 |
| -------------- | ------ | ---------- | -------------------- | ----------------- | ------------------------------- |
| Reset          | `00h`  | Supervisor | —                    | —                 | external-BIOS boot              |
| Undefined      | `04h`  | Undefined  | next instruction     | `MOVS pc, lr`     | reserved encodings              |
| SWI            | `08h`  | Supervisor | next instruction     | `MOVS pc, lr`     | `SWI` with an external BIOS     |
| Prefetch abort | `0Ch`  | Abort      | —                    | —                 | never                           |
| Data abort     | `10h`  | Abort      | —                    | —                 | never                           |
| IRQ            | `18h`  | IRQ        | next instruction + 4 | `SUBS pc, lr, #4` | IE & IF & IME with CPSR.I clear |
| FIQ            | `1Ch`  | FIQ        | —                    | —                 | never                           |

"Next instruction" is the address after the trapping instruction: +4 in ARM,
+2 in Thumb. Entry (`Core::enter_exception`) saves the CPSR in the new mode's
SPSR, writes the banked `lr`, switches the bank, sets the mode, clears T, sets
I, leaves F unchanged, and refills at the vector in ARM state. Exception
return may legally restore T from the SPSR; unlike a direct `MSR` T change,
that is not a fault. `LDM sp!, {…, pc}^` is the block-transfer return.

IRQ entry happens at an instruction boundary: after an instruction that did
not branch, when the interrupt line is asserted and CPSR.I is clear. The line
is `IE & IF` gated by `IME`; its sources and HALT wake rules are described in
[`timers-dma-interrupts.md`](timers-dma-interrupts.md). With the HLE BIOS, SWI
and IRQ entry run ZiGBA's own BIOS model instead of vector code
([`bios.md`](bios.md)).

External-BIOS boot starts at the reset vector in Supervisor mode with IRQ and
FIQ masked (CPSR `D3h`). The boot state for HLE mode is in
[`bios.md`](bios.md#post-boot-state).

Focused tests cover an SWI taken from FIQ mode, an IRQ that interrupts
Supervisor mode without losing its bank, an `LDM … {pc}^` return from IRQ
into Thumb System state, ARM coprocessor and reserved Thumb encodings
entering and returning from Undefined mode, and Thumb SWI immediates from
`DF00h` to `DFFFh` reaching the Supervisor vector.

## Undefined versus UNPREDICTABLE

The architecture distinguishes two kinds of invalid instruction, and ZiGBA
handles them differently.

**UNDEFINED** encodings take the Undefined-instruction trap, an architectural
exception the guest can handle. ZiGBA traps on:

- ARM coprocessor space (`CDP`, `LDC`/`STC`, `MCR`/`MRC`), for every
  coprocessor number: GBATEK reports no coprocessor on the GBA apart from a
  dummy CP14 unit ([GBATEK, coprocessor instructions][gbatek-cp]);
- the ARM `011` single-transfer space with bit 4 set, after one internal
  cycle, and only when the condition passes;
- the ARMv5E `LDRD`/`STRD` encodings, after one internal cycle;
- Thumb high bytes `B1`–`B3`, `B6`–`BB`, `BE`–`BF`, the always-undefined
  condition `DE`, and the ARMv5 `BLX` suffix `E8`–`EF`.

**UNPREDICTABLE** encodings have no architectural behavior at all. ZiGBA does
not guess one and does not abort the host: it stops with a typed, sticky
`Machine.Fault` before any register write, memory access, writeback or
pipeline refill beyond the instruction fetch
([ZiGBA architecture](../../../refs/zigba/docs/architecture.md)). Repeated steps return the
same fault without advancing time; a faulted machine refuses to write a save
state. The reasons are:

| Fault reason                 | Payload                                              | Raised for                                                                                                                                                                                                                                                                                                                                         |
| ---------------------------- | ---------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `unpredictable_arm_encoding` | instruction class, 16-bit set of offending registers | `MRS pc`; `MUL`/`MLA` naming r15 as Rd, Rm, Rs, or Rn when accumulating; `SWP` naming r15 anywhere; long multiply naming r15, or with RdHi = RdLo; halfword/signed transfer with Rd = r15, Rm = r15, or `Rn = pc` with post-indexing or writeback; single transfer with `Rn = pc` and post-indexing or writeback, or a byte transfer with Rd = r15 |
| `missing_spsr`               | operation, current mode                              | `MRS`/`MSR` of the SPSR, or `LDM … {pc}^`, in User or System mode                                                                                                                                                                                                                                                                                  |
| `invalid_psr_state`          | operation, kind, offending value                     | `MSR` CPSR selecting an invalid mode or changing T; an exception return (data processing or `LDM`) restoring an SPSR whose mode is invalid                                                                                                                                                                                                         |

Defined uses of r15 stay available: PC-relative pre-indexed loads without
writeback, word transfers with PC data, data-processing PC writes, and
block-transfer PC semantics. The User-mode `MOVS pc` policy above is also not
a fault. `Fault::write_diagnostic` formats each fault as one stable line, for
example `reason=unpredictable_arm_encoding class=multiply
offending_registers=8000`.

Containment is a host-safety diagnostic, not a claim about what silicon does
with these encodings. Tests: "unpredictable ARM register encodings fault
before architectural side effects", "invalid MSR CPSR mode and state
transitions fault before CPU mutation", "invalid LDM exception-return PSR
faults before transfers or writeback", "invalid data-processing
exception-return PSR faults before CPU mutation", "MRS and MSR SPSR without a
bank are typed pre-mutation faults" and "LDM exception return without an SPSR
faults before transfer side effects", all in
[ZiGBA's `machine_test.zig`](../../../refs/zigba/test/machine_test.zig).

## Open questions

- **Misaligned ARM PC writes.** ZiGBA clears bits 1–0 of an ARM-state PC
  write; ARMv4 calls such writes UNPREDICTABLE and no hardware test here
  shows what silicon does.
- **`MOVS pc` in User or System mode.** The branch-plus-flags policy matches
  mGBA and commercial expectations but is not a hardware measurement.
- **Compare/test with Rd = r15 and S = 1.** The SPSR-to-CPSR copy (or no-op
  in User and System mode) is not hardware-tested.
- **Compare/test encodings with S = 0.** Encodings in this space that are not
  `MRS`, `MSR`, `BX`, `SWP` or a multiply execute as ordinary flag-setting
  compares, although the architecture assigns them to the extension space.
- **`LDRD`/`STRD`.** ZiGBA follows the Arm decode rule and mGBA (Undefined);
  NanoBoyAdvance's ARM7TDMI-S observation has not been checked on a GBA.
- **Empty-list timing.** The single-word transfer and `40h` base step come
  from GBATEK and NanoBoyAdvance; their cycle cost has not been measured on
  hardware here.
- **Aborts.** ZiGBA never raises a prefetch or data abort; no GBA source for
  one is known, but no hardware test here rules it out.
- **Multiply carry outside the long forms.** The carry model reproduces
  the recorded `SMULLS`/`UMULLS` table; its `MULS`, `MLAS`, `UMLALS` and
  `SMLALS` results rest on its author's hardware fuzzing, which is published
  as the model rather than as a table of measurements.
- **CP14.** GBATEK mentions a dummy CP14 unit; ZiGBA sends CP14 transfers to
  the Undefined trap like every other coprocessor encoding, and what the
  dummy unit returns is not modeled.

[trm-0029g]: https://documentation-service.arm.com/static/5e8e1323fd977155116a3129
[trm-0210c]: https://documentation-service.arm.com/static/5f4786a179ff4c392c0ff819
[arm-0100i]: https://documentation-service.arm.com/static/5f8dacc8f86e16515cdb865a
[gbatek-ldm]: https://rust-console.github.io/gbatek-gbaonly/#armopcodesmemoryblockdatatransferldmstm
[gbatek-thumb-ldm]: https://rust-console.github.io/gbatek-gbaonly/#thumbopcodesmemorymultipleloadstorepushpopandldmstm
[nba-thumb]: https://github.com/nba-emu/NanoBoyAdvance/blob/master/src/nba/src/arm/handlers/handler16.inl
[multiply-carry]: https://bmchtech.github.io/post/multiply/
[gbatek-fiq]: https://rust-console.github.io/gbatek-gbaonly/#gbainterruptcontrol
[gbatek-cp]: https://rust-console.github.io/gbatek-gbaonly/#armopcodescoprocessorinstructionsmrcmcrldcstccdpmcrrmrrc
