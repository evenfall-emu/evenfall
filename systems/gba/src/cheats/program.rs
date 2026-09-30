//! The operations every code format decodes to, and their interpreter.

/// Access width of a cheat read or write.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Width {
    Byte,
    Half,
    Word,
}

impl Width {
    pub(crate) const fn bytes(self) -> u32 {
        match self {
            Self::Byte => 1,
            Self::Half => 2,
            Self::Word => 4,
        }
    }

    const fn mask(self) -> u32 {
        match self {
            Self::Byte => 0xff,
            Self::Half => 0xffff,
            Self::Word => u32::MAX,
        }
    }

    /// Sign-extends a value of this width.
    const fn signed(self, value: u32) -> i32 {
        match self {
            Self::Byte => value as u8 as i8 as i32,
            Self::Half => value as u16 as i16 as i32,
            Self::Word => value as i32,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Cond {
    Equal,
    NotEqual,
    SignedLess,
    SignedGreater,
    UnsignedLess,
    UnsignedGreater,
    /// `memory AND value` is nonzero.
    AnySet,
    /// `memory AND value` is zero: every listed key of active-low KEYINPUT
    /// is held.
    NoneSet,
    Never,
}

/// What a false condition skips.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Skip {
    /// The next `n` codes.
    Next(u8),
    /// Every code up to the next else or end-if.
    Block,
    /// Every remaining code of the cheat.
    Rest,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Update {
    Add,
    Or,
    And,
}

/// One device code. Multi-line codes decode to one operation, so the codes
/// a conditional skips count operations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Op {
    /// Master-code hook address; the device's own scheduling is not modeled.
    Hook(u32),
    /// Master-code game identifier.
    GameId(u32),
    /// A code without effect: encryption changes, CodeBreaker game IDs.
    Nop,
    /// A device-button write or slowdown: no effect, as the device's button
    /// is never pressed.
    DeviceOnly,
    /// End of the code list.
    Stop,
    Else,
    EndIf,
    /// `count` writes of `value + k * value_step` to `address + k * address_step`.
    Slide {
        address: u32,
        width: Width,
        value: u32,
        count: u32,
        address_step: u32,
        value_step: u32,
    },
    /// One word written to each address in order.
    WriteList {
        addresses: Box<[u32]>,
        value: u32,
    },
    WriteBytes {
        address: u32,
        bytes: Box<[u8]>,
    },
    /// `[[pointer] + offset] = value`.
    Pointer {
        pointer: u32,
        offset: u32,
        width: Width,
        value: u32,
    },
    Update {
        address: u32,
        width: Width,
        update: Update,
        value: u32,
    },
    If {
        address: u32,
        width: Width,
        cond: Cond,
        value: u32,
        skip: Skip,
    },
}

impl Op {
    pub(crate) const fn write(address: u32, width: Width, value: u32) -> Self {
        Self::Slide {
            address,
            width,
            value: value & width.mask(),
            count: 1,
            address_step: 0,
            value_step: 0,
        }
    }

    pub(crate) const fn fill(address: u32, width: Width, value: u32, count: u32) -> Self {
        Self::Slide {
            address,
            width,
            value: value & width.mask(),
            count,
            address_step: width.bytes(),
            value_step: 0,
        }
    }

    /// Bytes this operation may write in one pass.
    pub(crate) fn written_bytes(&self) -> u64 {
        match self {
            Self::Slide { width, count, .. } => u64::from(*count) * u64::from(width.bytes()),
            Self::WriteList { addresses, .. } => addresses.len() as u64 * 4,
            Self::WriteBytes { bytes, .. } => bytes.len() as u64,
            Self::Pointer { width, .. } | Self::Update { width, .. } => u64::from(width.bytes()),
            _ => 0,
        }
    }
}

/// Zero-time memory a program reads and writes. Accesses of a width are
/// aligned to it, as the device's ARM stores are.
pub(crate) trait Memory {
    fn read(&self, address: u32, width: Width) -> u32;
    fn write(&mut self, address: u32, width: Width, value: u32);
}

/// Runs one cheat's operations once, in order.
pub(crate) fn run(ops: &[Op], memory: &mut impl Memory) {
    let mut skipping_block = false;
    let mut index = 0;
    while let Some(op) = ops.get(index) {
        index += 1;
        if skipping_block && !matches!(op, Op::Else | Op::EndIf) {
            continue;
        }
        match *op {
            Op::Hook(_) | Op::GameId(_) | Op::Nop | Op::DeviceOnly => {}
            Op::Stop => return,
            Op::Else => skipping_block = !skipping_block,
            Op::EndIf => skipping_block = false,
            Op::Slide { address, width, value, count, address_step, value_step } => {
                let (mut address, mut value) = (address, value);
                for _ in 0..count {
                    memory.write(address, width, value & width.mask());
                    address = address.wrapping_add(address_step);
                    value = value.wrapping_add(value_step);
                }
            }
            Op::WriteList { ref addresses, value } => {
                for &address in addresses {
                    memory.write(address, Width::Word, value);
                }
            }
            Op::WriteBytes { address, ref bytes } => {
                for (offset, &byte) in bytes.iter().enumerate() {
                    memory.write(address.wrapping_add(offset as u32), Width::Byte, byte.into());
                }
            }
            Op::Pointer { pointer, offset, width, value } => {
                let base = memory.read(pointer, Width::Word);
                memory.write(base.wrapping_add(offset), width, value);
            }
            Op::Update { address, width, update, value } => {
                let current = memory.read(address, width);
                let next = match update {
                    Update::Add => current.wrapping_add(value),
                    Update::Or => current | value,
                    Update::And => current & value,
                };
                memory.write(address, width, next & width.mask());
            }
            Op::If { address, width, cond, value, skip } => {
                let current = if cond == Cond::Never { 0 } else { memory.read(address, width) };
                let value = value & width.mask();
                let holds = match cond {
                    Cond::Equal => current == value,
                    Cond::NotEqual => current != value,
                    Cond::SignedLess => width.signed(current) < width.signed(value),
                    Cond::SignedGreater => width.signed(current) > width.signed(value),
                    Cond::UnsignedLess => current < value,
                    Cond::UnsignedGreater => current > value,
                    Cond::AnySet => current & value != 0,
                    Cond::NoneSet => current & value == 0,
                    Cond::Never => false,
                };
                if !holds {
                    match skip {
                        Skip::Next(count) => index += usize::from(count),
                        Skip::Block => skipping_block = true,
                        Skip::Rest => return,
                    }
                }
            }
        }
    }
}

/// Where a plausible code reads or writes: RAM, I/O and video memory.
pub(crate) fn plausible_data_address(address: u32, bytes: u32) -> bool {
    let size = match address >> 24 {
        2 => 0x4_0000,
        3 => 0x8000,
        4 | 5 | 7 => 0x400,
        6 => 0x1_8000,
        _ => return false,
    };
    let offset = address & 0x00ff_ffff;
    offset < size && offset + bytes <= size
}

/// Whether decoded operations look like a real code: every address in RAM,
/// I/O or video memory, hooks in the Game Pak, game identifiers printable.
/// Autodetection uses it to reject a wrong format's decryption.
pub(crate) fn plausible(ops: &[Op]) -> bool {
    ops.iter().all(|op| match *op {
        Op::Hook(address) => (0x0800_0000..0x0a00_0000).contains(&address),
        Op::GameId(code) => {
            code.to_le_bytes().iter().all(|&c| c.is_ascii_uppercase() || c.is_ascii_digit())
        }
        Op::Nop | Op::Stop | Op::Else | Op::EndIf => true,
        // Rare in real lists, and what other formats' readings often decode to.
        Op::DeviceOnly => false,
        Op::Slide { address, width, count, address_step, .. } => {
            let last = address.wrapping_add(address_step.wrapping_mul(count.saturating_sub(1)));
            plausible_data_address(address, width.bytes())
                && plausible_data_address(last, width.bytes())
        }
        Op::WriteList { ref addresses, .. } => {
            // The GameShark's group write also targets its own value.
            addresses.iter().skip(1).all(|&address| plausible_data_address(address, 4))
        }
        Op::WriteBytes { address, ref bytes } => {
            plausible_data_address(address, bytes.len() as u32)
        }
        Op::Pointer { pointer, .. } => plausible_data_address(pointer, 4),
        Op::Update { address, width, .. } | Op::If { address, width, .. } => {
            plausible_data_address(address, width.bytes())
        }
    })
}
