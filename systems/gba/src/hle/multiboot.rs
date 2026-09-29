//! Suspendable HLE BIOS MultiBoot serial protocol.

use crate::bus::{Access, AccessKind, AccessSource, AccessWidth, Bus};
use crate::cpu::{
    Cpu, HleMultiBootContinuation, HleMultiBootPhase, HleService, InstructionSet, StepResult,
};
use crate::multiboot::{Encoder, Mode, Parameters};

const INITIAL_DELAY: u64 = 1 << 20;
const POLL_CYCLES: u64 = 64;
const INTER_TRANSFER_DELAY: u64 = 604;
const TIMEOUT_CYCLES: u64 = 1 << 24;

fn read(bus: &mut Bus, address: u32, width: AccessWidth) -> u32 {
    bus.read(Access {
        address,
        width,
        kind: AccessKind::Nonsequential,
        source: AccessSource::CpuData,
    })
    .value
}

fn write(bus: &mut Bus, address: u32, width: AccessWidth, value: u32) {
    bus.write(
        Access { address, width, kind: AccessKind::Nonsequential, source: AccessSource::CpuData },
        value,
    );
}

fn step_result(continuation: &HleMultiBootContinuation) -> StepResult {
    StepResult {
        instruction_address: continuation.instruction_address,
        instruction: if continuation.instruction_set == InstructionSet::Thumb {
            0xDF25
        } else {
            0xEF25_0000
        },
        instruction_set: continuation.instruction_set,
    }
}

fn launch(bus: &mut Bus, mode: Mode, outgoing: u32) {
    if mode.is_multiplayer() {
        write(bus, 0x0400_0128, AccessWidth::Half, 0x2003);
        write(bus, 0x0400_012A, AccessWidth::Half, outgoing);
        write(bus, 0x0400_0128, AccessWidth::Half, 0x2083);
        return;
    }
    let control = 0x1001 | if mode == Mode::Normal2Mhz { 2 } else { 0 };
    write(bus, 0x0400_0128, AccessWidth::Half, control);
    write(bus, 0x0400_0120, AccessWidth::Word, outgoing);
    write(bus, 0x0400_0128, AccessWidth::Half, control | 0x0080);
}

fn responses(bus: &mut Bus, mode: Mode) -> [u16; 3] {
    if !mode.is_multiplayer() {
        return [(read(bus, 0x0400_0120, AccessWidth::Word) >> 16) as u16, 0xFFFF, 0xFFFF];
    }
    [
        read(bus, 0x0400_0122, AccessWidth::Half) as u16,
        read(bus, 0x0400_0124, AccessWidth::Half) as u16,
        read(bus, 0x0400_0126, AccessWidth::Half) as u16,
    ]
}

fn collect_random(continuation: &mut HleMultiBootContinuation, responses: [u16; 3]) -> bool {
    for (index, response) in responses.into_iter().enumerate() {
        if continuation.parameters.client_bit & (1 << (index + 1)) == 0 {
            continue;
        }
        if response & 0xFF00 != 0x7300 {
            return false;
        }
        continuation.random_data[index] = response as u8;
    }
    true
}

fn data_responses_valid(continuation: &HleMultiBootContinuation, responses: [u16; 3]) -> bool {
    let expected = (0xC0
        + continuation.source_offset
        + if continuation.multiplayer_high_half { 2 } else { 0 }) as u16;
    for (index, response) in responses.into_iter().enumerate() {
        if continuation.parameters.client_bit & (1 << (index + 1)) != 0 && response != expected {
            return false;
        }
    }
    true
}

fn responses_equal(
    continuation: &HleMultiBootContinuation,
    responses: [u16; 3],
    expected: u16,
) -> bool {
    for (index, response) in responses.into_iter().enumerate() {
        if continuation.parameters.client_bit & (1 << (index + 1)) != 0 && response != expected {
            return false;
        }
    }
    true
}

impl Cpu {
    pub fn execute_hle_multi_boot(&mut self, bus: &mut Bus) {
        let parameter_address = self.registers[0];
        let Ok(mode) = Mode::parse(self.registers[1]) else {
            self.registers[0] = 1;
            bus.elapse(45);
            return;
        };
        let parameters = Parameters {
            handshake_data: read(bus, parameter_address.wrapping_add(0x14), AccessWidth::Byte)
                as u8,
            client_data: [
                read(bus, parameter_address.wrapping_add(0x19), AccessWidth::Byte) as u8,
                read(bus, parameter_address.wrapping_add(0x1A), AccessWidth::Byte) as u8,
                read(bus, parameter_address.wrapping_add(0x1B), AccessWidth::Byte) as u8,
            ],
            palette_data: read(bus, parameter_address.wrapping_add(0x1C), AccessWidth::Byte) as u8,
            client_bit: read(bus, parameter_address.wrapping_add(0x1E), AccessWidth::Byte) as u8,
            source: read(bus, parameter_address.wrapping_add(0x20), AccessWidth::Word),
            end: read(bus, parameter_address.wrapping_add(0x24), AccessWidth::Word),
        };
        if parameters.validate().is_err() {
            self.registers[0] = 1;
            bus.elapse(45);
            return;
        }
        write(bus, 0x0400_0134, AccessWidth::Half, 0);
        self.hle_service = Some(HleService::MultiBoot(HleMultiBootContinuation {
            parameters,
            mode,
            encoder: Encoder::init(mode, parameters.palette_data, parameters.client_data),
            random_data: [0xFF; 3],
            instruction_address: self.next_instruction_address,
            instruction_set: if self.is_thumb() {
                InstructionSet::Thumb
            } else {
                InstructionSet::Arm
            },
            ..Default::default()
        }));
    }

    pub fn continue_hle_multi_boot(&mut self, bus: &mut Bus) -> StepResult {
        let Some(HleService::MultiBoot(mut continuation)) = self.hle_service.clone() else {
            unreachable!("MultiBoot continuation")
        };
        match continuation.phase {
            HleMultiBootPhase::InitialDelay => {
                bus.elapse(INITIAL_DELAY);
                continuation.phase = HleMultiBootPhase::LaunchLength;
            }
            HleMultiBootPhase::LaunchLength => {
                launch(bus, continuation.mode, u32::from(continuation.parameters.length_command()));
                continuation.wait_started_at = bus.now();
                continuation.phase = HleMultiBootPhase::WaitLength;
            }
            HleMultiBootPhase::WaitLength => {
                if let Some(result) = self.hle_multi_boot_still_waiting(bus, &continuation) {
                    return result;
                }
                let replies = responses(bus, continuation.mode);
                if !collect_random(&mut continuation, replies) {
                    return self.finish_hle_multi_boot(false);
                }
                bus.elapse(INTER_TRANSFER_DELAY);
                continuation.phase = HleMultiBootPhase::LaunchData;
            }
            HleMultiBootPhase::LaunchData => {
                if continuation.source_offset == continuation.parameters.payload_length() {
                    continuation.wait_started_at = 0;
                    continuation.phase = HleMultiBootPhase::LaunchCrcRequest;
                } else {
                    if !continuation.multiplayer_high_half {
                        let plain = read(
                            bus,
                            continuation.parameters.source.wrapping_add(continuation.source_offset),
                            AccessWidth::Word,
                        );
                        continuation.encoded_word = continuation
                            .encoder
                            .encode_word(plain, 0xC0 + continuation.source_offset);
                    }
                    let outgoing = if continuation.mode.is_multiplayer() {
                        if continuation.multiplayer_high_half {
                            continuation.encoded_word >> 16
                        } else {
                            continuation.encoded_word & 0xFFFF
                        }
                    } else {
                        continuation.encoded_word
                    };
                    launch(bus, continuation.mode, outgoing);
                    continuation.wait_started_at = bus.now();
                    continuation.phase = HleMultiBootPhase::WaitData;
                }
            }
            HleMultiBootPhase::WaitData => {
                if let Some(result) = self.hle_multi_boot_still_waiting(bus, &continuation) {
                    return result;
                }
                let replies = responses(bus, continuation.mode);
                if !data_responses_valid(&continuation, replies) {
                    return self.finish_hle_multi_boot(false);
                }
                bus.elapse(INTER_TRANSFER_DELAY);
                if continuation.mode.is_multiplayer() && !continuation.multiplayer_high_half {
                    continuation.multiplayer_high_half = true;
                } else {
                    continuation.multiplayer_high_half = false;
                    continuation.source_offset += 4;
                }
                continuation.phase = HleMultiBootPhase::LaunchData;
            }
            HleMultiBootPhase::LaunchCrcRequest => {
                launch(bus, continuation.mode, 0x65);
                if continuation.wait_started_at == 0 {
                    continuation.wait_started_at = bus.now();
                }
                continuation.phase = HleMultiBootPhase::WaitCrcRequest;
            }
            HleMultiBootPhase::WaitCrcRequest => {
                if let Some(result) = self.hle_multi_boot_still_waiting(bus, &continuation) {
                    return result;
                }
                if bus.now() - continuation.wait_started_at >= TIMEOUT_CYCLES {
                    return self.finish_hle_multi_boot(false);
                }
                let replies = responses(bus, continuation.mode);
                bus.elapse(INTER_TRANSFER_DELAY);
                continuation.phase = if responses_equal(&continuation, replies, 0x75) {
                    HleMultiBootPhase::LaunchCrcSignal
                } else {
                    HleMultiBootPhase::LaunchCrcRequest
                };
            }
            HleMultiBootPhase::LaunchCrcSignal => {
                launch(bus, continuation.mode, 0x66);
                continuation.wait_started_at = bus.now();
                continuation.phase = HleMultiBootPhase::WaitCrcSignal;
            }
            HleMultiBootPhase::WaitCrcSignal => {
                if let Some(result) = self.hle_multi_boot_still_waiting(bus, &continuation) {
                    return result;
                }
                let replies = responses(bus, continuation.mode);
                if !responses_equal(&continuation, replies, 0x75) {
                    return self.finish_hle_multi_boot(false);
                }
                bus.elapse(INTER_TRANSFER_DELAY);
                continuation.final_checksum = continuation
                    .encoder
                    .finish(continuation.parameters.handshake_data, continuation.random_data);
                continuation.phase = HleMultiBootPhase::LaunchCrc;
            }
            HleMultiBootPhase::LaunchCrc => {
                launch(bus, continuation.mode, u32::from(continuation.final_checksum));
                continuation.wait_started_at = bus.now();
                continuation.phase = HleMultiBootPhase::WaitCrc;
            }
            HleMultiBootPhase::WaitCrc => {
                if let Some(result) = self.hle_multi_boot_still_waiting(bus, &continuation) {
                    return result;
                }
                let replies = responses(bus, continuation.mode);
                if !responses_equal(&continuation, replies, continuation.final_checksum) {
                    return self.finish_hle_multi_boot(false);
                }
                bus.elapse(INTER_TRANSFER_DELAY);
                return self.finish_hle_multi_boot(true);
            }
        }
        let result = step_result(&continuation);
        self.hle_service = Some(HleService::MultiBoot(continuation));
        result
    }

    fn hle_multi_boot_still_waiting(
        &mut self,
        bus: &mut Bus,
        continuation: &HleMultiBootContinuation,
    ) -> Option<StepResult> {
        let control = read(bus, 0x0400_0128, AccessWidth::Half) as u16;
        if control & 0x0080 == 0 {
            return None;
        }
        if bus.now() - continuation.wait_started_at >= TIMEOUT_CYCLES {
            return Some(self.finish_hle_multi_boot(false));
        }
        bus.elapse(POLL_CYCLES);
        self.hle_service = Some(HleService::MultiBoot(continuation.clone()));
        Some(step_result(continuation))
    }

    fn finish_hle_multi_boot(&mut self, success: bool) -> StepResult {
        let Some(HleService::MultiBoot(continuation)) = self.hle_service.take() else {
            unreachable!("MultiBoot continuation")
        };
        self.registers[0] = u32::from(!success);
        step_result(&continuation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ZiGBA `HleMultiBootContinuation.random_data` defaults to FF in all three
    /// slots; clients absent from `client_bit` still contribute FF to the CRC.
    #[test]
    fn inactive_multiboot_clients_contribute_ff_to_crc() {
        for (mode, client_bit, expected_random) in
            [(2, 0x02, [0x42, 0xFF, 0xFF]), (1, 0x06, [0x42, 0x42, 0xFF])]
        {
            let mut bus = Bus::init(&[]);
            let mut cpu = Cpu::init(0x0800_0000, 0x1F, true);
            cpu.registers[0] = 0x0200_0000;
            cpu.registers[1] = mode;
            bus.ewram[0x14] = 0x11;
            bus.ewram[0x19..0x1C].copy_from_slice(&[0x22, 0x33, 0x44]);
            bus.ewram[0x1C] = 0x55;
            bus.ewram[0x1E] = client_bit;
            bus.ewram[0x20..0x24].copy_from_slice(&0x0200_0100_u32.to_le_bytes());
            bus.ewram[0x24..0x28].copy_from_slice(&0x0200_0200_u32.to_le_bytes());

            cpu.execute_hle_multi_boot(&mut bus);
            let Some(HleService::MultiBoot(continuation)) = cpu.hle_service.as_mut() else {
                panic!("valid MultiBoot parameters should start the service")
            };
            assert_eq!(continuation.random_data, [0xFF; 3]);
            assert!(collect_random(continuation, [0x7342, 0x7342, 0xFFFF]));
            assert_eq!(continuation.random_data, expected_random);
        }
    }
}
