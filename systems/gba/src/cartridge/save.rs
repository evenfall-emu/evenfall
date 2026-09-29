//! Cartridge save-media state machines.

use crate::types::Cycle;
use evenfall_state::{Codec, NarrowCodec, Sink, Source};

const FLASH_SECTOR_ERASE_CYCLES: Cycle = 30_000;
const FLASH_ATMEL_FLAG: Cycle = 1 << 63;
const FLASH_ATMEL_PAGE_ACTIVE_FLAG: Cycle = 1 << 62;
const FLASH_ATMEL_EXPECTED_OFFSET_MASK: Cycle = 0xffff;
const EEPROM_WRITE_CYCLES: Cycle = 108_368;
const SRAM_SIZE: usize = 32 * 1024;
const FLASH_BANK_SIZE: usize = 64 * 1024;
const FLASH_MAX_SIZE: usize = 128 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Type {
    Auto,
    None,
    Sram,
    Flash64k,
    Flash64kAtmel,
    Flash128k,
    Eeprom512,
    Eeprom8k,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Detection {
    Unknown,
    Sram,
    Flash64k,
    Flash128k,
    EepromCapacityRequired,
    ConflictingMarkers,
}

pub fn detect_type(rom: &[u8]) -> Detection {
    let mut detection = Detection::Unknown;
    for offset in (0..rom.len()).step_by(4) {
        let Some(head) = rom.get(offset..offset + 4) else {
            break;
        };
        let candidate = match u32::from_le_bytes(head.try_into().unwrap()) {
            0x5341_4c46 => {
                if marker_at(rom, offset, b"FLASH1M_V") {
                    Detection::Flash128k
                } else if marker_at(rom, offset, b"FLASH512_V")
                    || marker_at(rom, offset, b"FLASH_V")
                {
                    Detection::Flash64k
                } else {
                    Detection::Unknown
                }
            }
            0x4d41_5253 => {
                if marker_at(rom, offset, b"SRAM_V") {
                    Detection::Sram
                } else {
                    Detection::Unknown
                }
            }
            0x5250_4545 => {
                if marker_at(rom, offset, b"EEPROM_V") {
                    Detection::EepromCapacityRequired
                } else {
                    Detection::Unknown
                }
            }
            _ => Detection::Unknown,
        };
        if candidate == Detection::Unknown {
            continue;
        }
        if detection == Detection::Unknown {
            detection = candidate;
        } else if detection != candidate {
            return Detection::ConflictingMarkers;
        }
    }
    detection
}

fn marker_at(rom: &[u8], offset: usize, prefix: &[u8]) -> bool {
    let Some(candidate) = rom.get(offset..offset + prefix.len() + 3) else {
        return false;
    };
    candidate.starts_with(prefix) && candidate[prefix.len()..].iter().all(u8::is_ascii_digit)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tag {
    Auto,
    None,
    Sram,
    Flash,
    Eeprom,
}

pub struct Device {
    pub tag: Tag,
    pub state: State,
}
impl_clone_fields!(Device { tag, state });

pub enum State {
    Auto(Auto),
    None,
    Sram(Box<Sram>),
    Flash(Box<Flash>),
    Eeprom(Box<Eeprom>),
}

/// Copying into the same kind of memory reuses its box.
impl Clone for State {
    fn clone(&self) -> Self {
        match self {
            Self::Auto(auto) => Self::Auto(*auto),
            Self::None => Self::None,
            Self::Sram(sram) => Self::Sram(sram.clone()),
            Self::Flash(flash) => Self::Flash(flash.clone()),
            Self::Eeprom(eeprom) => Self::Eeprom(eeprom.clone()),
        }
    }

    fn clone_from(&mut self, source: &Self) {
        match (self, source) {
            (Self::Sram(target), Self::Sram(source)) => target.clone_from(source),
            (Self::Flash(target), Self::Flash(source)) => target.clone_from(source),
            (Self::Eeprom(target), Self::Eeprom(source)) => target.clone_from(source),
            (target, source) => *target = source.clone(),
        }
    }
}

// The schema is independent of allocation: ZiGBA's union payloads are the
// in-place structs. The runtime state boxes the large variants to avoid
// stack temporaries during construction and reads.
#[allow(dead_code)]
#[derive(Codec)]
enum StateSchema {
    Auto(Auto),
    None([u8; 0]),
    Sram(Sram),
    Flash(Flash),
    Eeprom(Eeprom),
}

impl Codec for State {
    fn encode(&self, sink: &mut Sink<'_>) -> evenfall_state::Result<()> {
        match self {
            Self::Auto(v) => {
                0u8.encode_bits(sink, 3)?;
                v.encode(sink)
            }
            Self::None => 1u8.encode_bits(sink, 3),
            Self::Sram(v) => {
                2u8.encode_bits(sink, 3)?;
                v.encode(sink)
            }
            Self::Flash(v) => {
                3u8.encode_bits(sink, 3)?;
                v.encode(sink)
            }
            Self::Eeprom(v) => {
                4u8.encode_bits(sink, 3)?;
                v.encode(sink)
            }
        }
    }
    fn decode(&mut self, source: &mut Source<'_>) -> evenfall_state::Result<()> {
        let mut tag = 0u8;
        tag.decode_bits(source, 3)?;
        *self = match tag {
            0 => {
                let mut value = Auto::default();
                value.decode(source)?;
                Self::Auto(value)
            }
            1 => Self::None,
            2 => {
                let mut value = Box::new(Sram::init());
                value.data.decode(source)?;
                Self::Sram(value)
            }
            3 => {
                let mut value = Box::new(Flash::init(false));
                value.decode(source)?;
                Self::Flash(value)
            }
            4 => {
                let mut value = Box::new(Eeprom::init(false));
                value.decode(source)?;
                Self::Eeprom(value)
            }
            _ => return Err(evenfall_state::Error::InvalidSaveState),
        };
        Ok(())
    }
    fn schema(sink: &mut Sink<'_>) -> evenfall_state::Result<()> {
        StateSchema::schema(sink)
    }
}

impl Device {
    pub fn init(save_type: Type) -> Self {
        let state = match save_type {
            Type::Auto => State::Auto(Auto::default()),
            Type::None => State::None,
            Type::Sram => State::Sram(Box::default()),
            Type::Flash64k => State::Flash(Box::new(Flash::init(false))),
            Type::Flash64kAtmel => State::Flash(Box::new(Flash::init_atmel())),
            Type::Flash128k => State::Flash(Box::new(Flash::init(true))),
            Type::Eeprom512 => State::Eeprom(Box::new(Eeprom::init(false))),
            Type::Eeprom8k => State::Eeprom(Box::new(Eeprom::init(true))),
        };
        let tag = state.tag();
        Self { tag, state }
    }
    pub fn init_with_data(save_type: Type, bytes: &[u8]) -> Self {
        assert_ne!(save_type, Type::Auto, "unresolved save media cannot be initialized from bytes");
        let mut device = Self::init(save_type);
        let destination = device.mutable_data();
        assert_eq!(
            bytes.len(),
            destination.len(),
            "save data size does not match its configured media type"
        );
        destination.copy_from_slice(bytes);
        device
    }
    pub fn write_state(&self, sink: &mut Sink<'_>) -> evenfall_state::Result<()> {
        self.state.encode(sink)
    }
    pub fn write_state_schema(sink: &mut Sink<'_>) -> evenfall_state::Result<()> {
        State::schema(sink)
    }
    pub fn read_state(&mut self, source: &mut Source<'_>) -> evenfall_state::Result<()> {
        self.state.decode(source)?;
        self.validate_state().map_err(|_| evenfall_state::Error::InvalidSaveState)
    }
}

/// The save medium is a `Bus` field with a module-owned encoding: the bus
/// writes it, describes it and reads it (validation included) through these
/// functions, as ZiGBA's `Bus.writeState` and `Bus.readState` do.
impl Codec for Device {
    fn encode(&self, sink: &mut Sink<'_>) -> evenfall_state::Result<()> {
        self.write_state(sink)
    }
    fn decode(&mut self, source: &mut Source<'_>) -> evenfall_state::Result<()> {
        self.read_state(source)
    }
    fn schema(sink: &mut Sink<'_>) -> evenfall_state::Result<()> {
        Self::write_state_schema(sink)
    }
}

impl Device {
    pub fn validate_state(&mut self) -> Result<(), InvalidSaveState> {
        self.tag = match &self.state {
            State::Auto(auto) if auto.pending_count > 2 => return Err(InvalidSaveState),
            State::Flash(flash) => {
                if !flash.large && flash.bank != 0 {
                    return Err(InvalidSaveState);
                }
                if flash.is_atmel() {
                    let page_active = flash.atmel_page_active();
                    let expected_offset = flash.atmel_expected_offset();
                    if flash.large
                        || flash.autodetect_capacity
                        || flash.busy_until
                            & !(FLASH_ATMEL_FLAG
                                | FLASH_ATMEL_PAGE_ACTIVE_FLAG
                                | FLASH_ATMEL_EXPECTED_OFFSET_MASK)
                            != 0
                        || (page_active
                            && (flash.phase != Phase::Program || expected_offset & 0x7f == 0))
                        || (!page_active && expected_offset != 0)
                    {
                        return Err(InvalidSaveState);
                    }
                } else if flash.busy_until & FLASH_ATMEL_PAGE_ACTIVE_FLAG != 0 {
                    return Err(InvalidSaveState);
                }
                Tag::Flash
            }
            State::Eeprom(eeprom) => {
                let input_limit = if eeprom.large { 80 } else { 72 };
                if eeprom.input_count > input_limit
                    || eeprom.output_index.is_some_and(|index| index > 67)
                    || (!eeprom.large && eeprom.read_block > 63)
                {
                    return Err(InvalidSaveState);
                }
                Tag::Eeprom
            }
            state => state.tag(),
        };
        Ok(())
    }
    pub fn data(&self) -> &[u8] {
        match &self.state {
            State::Auto(_) | State::None => &[],
            State::Sram(sram) => &sram.data[..],
            State::Flash(flash) => {
                &flash.data[..if flash.large { FLASH_MAX_SIZE } else { FLASH_BANK_SIZE }]
            }
            State::Eeprom(eeprom) => {
                &eeprom.data[..if eeprom.large { Eeprom::MAX_SIZE } else { 512 }]
            }
        }
    }
    fn mutable_data(&mut self) -> &mut [u8] {
        match &mut self.state {
            State::Auto(_) => panic!("unresolved save media has no mutable payload"),
            State::None => &mut [],
            State::Sram(sram) => &mut sram.data[..],
            State::Flash(flash) => {
                &mut flash.data[..if flash.large { FLASH_MAX_SIZE } else { FLASH_BANK_SIZE }]
            }
            State::Eeprom(eeprom) => {
                &mut eeprom.data[..if eeprom.large { Eeprom::MAX_SIZE } else { 512 }]
            }
        }
    }
    pub fn is_eeprom(&self) -> bool {
        self.tag == Tag::Eeprom
    }
    pub fn is_auto(&self) -> bool {
        self.tag == Tag::Auto
    }
    pub fn resolved_type(&self) -> Type {
        match &self.state {
            State::Auto(_) => Type::Auto,
            State::None => Type::None,
            State::Sram(_) => Type::Sram,
            State::Flash(flash) => {
                if flash.large {
                    Type::Flash128k
                } else if flash.is_atmel() {
                    Type::Flash64kAtmel
                } else {
                    Type::Flash64k
                }
            }
            State::Eeprom(eeprom) => {
                if eeprom.large {
                    Type::Eeprom8k
                } else {
                    Type::Eeprom512
                }
            }
        }
    }
    pub fn resolve_eeprom_transfer(&mut self, bit_count: u32) -> bool {
        if self.tag != Tag::Auto {
            return self.is_eeprom();
        }
        let large = match bit_count {
            9 | 73 => false,
            17 | 81 => true,
            _ => return false,
        };
        self.tag = Tag::Eeprom;
        self.state = State::Eeprom(Box::new(Eeprom::init(large)));
        true
    }
    pub fn read_byte(&mut self, offset: u16, now: Cycle) -> u8 {
        if let State::Auto(auto) = &self.state
            && auto.pending_count != 0
        {
            self.promote_auto_to_sram(None);
        }
        match &self.state {
            State::Auto(_) | State::None | State::Eeprom(_) => 0xff,
            State::Sram(sram) => sram.data[usize::from(offset) & (SRAM_SIZE - 1)],
            State::Flash(flash) => flash.read(offset, now),
        }
    }
    pub fn peek_byte(&self, offset: u16, now: Cycle) -> u8 {
        match &self.state {
            State::Auto(auto) => auto.pending[..usize::from(auto.pending_count)].iter().fold(
                0xff,
                |value, pending| if pending.offset == offset { pending.value } else { value },
            ),
            State::None | State::Eeprom(_) => 0xff,
            State::Sram(sram) => sram.data[usize::from(offset) & (SRAM_SIZE - 1)],
            State::Flash(flash) => flash.read(offset, now),
        }
    }
    pub fn write_byte(&mut self, offset: u16, value: u8, now: Cycle) {
        if self.tag == Tag::Auto {
            self.write_auto_byte(offset, value);
            return;
        }
        match &mut self.state {
            State::Auto(_) => unreachable!(),
            State::None | State::Eeprom(_) => {}
            State::Sram(sram) => sram.data[usize::from(offset) & (SRAM_SIZE - 1)] = value,
            State::Flash(flash) => flash.write(offset, value, now),
        }
    }
    pub fn write_eeprom_bit(&mut self, bit: u8, now: Cycle) {
        if let State::Eeprom(eeprom) = &mut self.state {
            eeprom.write_bit(bit & 1, now);
        } else {
            unreachable!();
        }
    }
    pub fn read_eeprom_bit(&mut self, now: Cycle) -> u8 {
        if let State::Eeprom(eeprom) = &mut self.state {
            eeprom.read_bit(now)
        } else {
            unreachable!()
        }
    }
    fn write_auto_byte(&mut self, offset: u16, value: u8) {
        let State::Auto(auto) = &mut self.state else { unreachable!() };
        match auto.pending_count {
            0 if offset == 0x5555 && value == 0xaa => {
                auto.pending[0] = PendingWrite { offset, value };
                auto.pending_count = 1;
                return;
            }
            1 if offset == 0x2aaa && value == 0x55 => {
                auto.pending[1] = PendingWrite { offset, value };
                auto.pending_count = 2;
                return;
            }
            0 | 1 => {
                self.promote_auto_to_sram(Some(PendingWrite { offset, value }));
                return;
            }
            _ => {}
        }
        if offset != 0x5555 || !matches!(value, 0x80 | 0x90 | 0xa0 | 0xb0 | 0xf0) {
            self.promote_auto_to_sram(Some(PendingWrite { offset, value }));
            return;
        }
        let pending = auto.pending;
        let mut flash = Flash::init(value == 0xb0);
        flash.autodetect_capacity = true;
        self.tag = Tag::Flash;
        self.state = State::Flash(Box::new(flash));
        let State::Flash(flash) = &mut self.state else { unreachable!() };
        flash.write(pending[0].offset, pending[0].value, 0);
        flash.write(pending[1].offset, pending[1].value, 0);
        flash.write(offset, value, 0);
    }
    fn promote_auto_to_sram(&mut self, final_write: Option<PendingWrite>) {
        let State::Auto(auto) = &self.state else { unreachable!() };
        let pending = auto.pending;
        let pending_count = auto.pending_count;
        self.tag = Tag::Sram;
        self.state = State::Sram(Box::default());
        let State::Sram(sram) = &mut self.state else { unreachable!() };
        for write in pending[..usize::from(pending_count)].iter().chain(final_write.iter()) {
            sram.data[usize::from(write.offset) & (SRAM_SIZE - 1)] = write.value;
        }
    }
}

impl State {
    fn tag(&self) -> Tag {
        match self {
            Self::Auto(_) => Tag::Auto,
            Self::None => Tag::None,
            Self::Sram(_) => Tag::Sram,
            Self::Flash(_) => Tag::Flash,
            Self::Eeprom(_) => Tag::Eeprom,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct InvalidSaveState;

#[derive(Clone, Copy, Default, Codec)]
pub struct PendingWrite {
    pub offset: u16,
    pub value: u8,
}
#[derive(Clone, Copy, Default, Codec)]
pub struct Auto {
    pub pending: [PendingWrite; 2],
    #[codec(bits = 2)]
    pub pending_count: u8,
}
#[derive(Codec)]
pub struct Sram {
    pub data: Box<[u8; SRAM_SIZE]>,
}
impl_clone_fields!(Sram { data });
impl Sram {
    fn init() -> Self {
        Self { data: vec![0xff; SRAM_SIZE].into_boxed_slice().try_into().unwrap() }
    }
}
impl Default for Sram {
    fn default() -> Self {
        Self::init()
    }
}

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, Codec)]
#[codec(tag_bits = 3)]
pub enum Phase {
    #[default]
    Ready,
    Unlock,
    Command,
    Program,
    Bank,
    EraseUnlock,
    EraseUnlockSecond,
    EraseCommand,
}

#[derive(Codec)]
pub struct Flash {
    pub data: Box<[u8; FLASH_MAX_SIZE]>,
    pub large: bool,
    #[codec(bits = 1)]
    pub bank: u8,
    pub identification: bool,
    pub phase: Phase,
    pub autodetect_capacity: bool,
    pub busy_until: Cycle,
    #[codec(bits = 4)]
    pub busy_sector: u8,
}
impl_clone_fields!(Flash {
    data,
    large,
    bank,
    identification,
    phase,
    autodetect_capacity,
    busy_until,
    busy_sector,
});
impl Default for Flash {
    fn default() -> Self {
        Self::init(false)
    }
}
impl Flash {
    fn init(large: bool) -> Self {
        Self {
            data: vec![0xff; FLASH_MAX_SIZE].into_boxed_slice().try_into().unwrap(),
            large,
            bank: 0,
            identification: false,
            phase: Phase::Ready,
            autodetect_capacity: false,
            busy_until: 0,
            busy_sector: 0,
        }
    }
    fn init_atmel() -> Self {
        let mut flash = Self::init(false);
        flash.busy_until = FLASH_ATMEL_FLAG;
        flash
    }
    fn read(&self, offset: u16, now: Cycle) -> u8 {
        if self.identification {
            return match offset {
                0 => {
                    if self.is_atmel() {
                        0x1f
                    } else {
                        0xc2
                    }
                }
                1 => {
                    if self.is_atmel() {
                        0x3d
                    } else if self.large {
                        0x09
                    } else {
                        0x1c
                    }
                }
                _ => 0xff,
            };
        }
        if !self.is_atmel() && now < self.busy_until && (offset >> 12) as u8 == self.busy_sector {
            return 0x5f;
        }
        self.data[self.index(offset)]
    }
    fn write(&mut self, offset: u16, value: u8, now: Cycle) {
        match self.phase {
            Phase::Ready => {
                if offset == 0x5555 && value == 0xaa {
                    self.phase = Phase::Unlock;
                }
            }
            Phase::Unlock => {
                self.phase =
                    if offset == 0x2aaa && value == 0x55 { Phase::Command } else { Phase::Ready }
            }
            Phase::Command => {
                self.phase = if offset != 0x5555 {
                    Phase::Ready
                } else {
                    match value {
                        0xa0 => Phase::Program,
                        0xb0 => {
                            if self.autodetect_capacity {
                                self.large = true;
                            }
                            if self.large { Phase::Bank } else { Phase::Ready }
                        }
                        0x80 => Phase::EraseUnlock,
                        0x90 => {
                            self.identification = true;
                            Phase::Ready
                        }
                        0xf0 => {
                            self.identification = false;
                            Phase::Ready
                        }
                        _ => Phase::Ready,
                    }
                };
            }
            Phase::Program => {
                if self.is_atmel() {
                    self.write_atmel_page_byte(offset, value);
                } else {
                    let index = self.index(offset);
                    self.data[index] &= value;
                    self.phase = Phase::Ready;
                }
            }
            Phase::Bank => {
                if offset == 0 {
                    self.bank = value & 1;
                }
                self.phase = Phase::Ready;
            }
            Phase::EraseUnlock => {
                self.phase = if offset == 0x5555 && value == 0xaa {
                    Phase::EraseUnlockSecond
                } else {
                    Phase::Ready
                }
            }
            Phase::EraseUnlockSecond => {
                self.phase = if offset == 0x2aaa && value == 0x55 {
                    Phase::EraseCommand
                } else {
                    Phase::Ready
                }
            }
            Phase::EraseCommand => {
                if !self.is_atmel() && value == 0x30 && offset & 0x0fff == 0 {
                    let first = self.index(offset) & !0x0fff;
                    self.data[first..first + 0x1000].fill(0xff);
                    self.busy_sector = (offset >> 12) as u8;
                    self.busy_until = now.saturating_add(FLASH_SECTOR_ERASE_CYCLES);
                } else if offset == 0x5555 && value == 0x10 {
                    let size = if self.large { FLASH_MAX_SIZE } else { FLASH_BANK_SIZE };
                    self.data[..size].fill(0xff);
                }
                self.phase = Phase::Ready;
            }
        }
    }
    fn index(&self, offset: u16) -> usize {
        (if self.large { usize::from(self.bank) * FLASH_BANK_SIZE } else { 0 })
            + usize::from(offset)
    }
    fn is_atmel(&self) -> bool {
        self.busy_until & FLASH_ATMEL_FLAG != 0
    }
    fn atmel_page_active(&self) -> bool {
        self.busy_until & FLASH_ATMEL_PAGE_ACTIVE_FLAG != 0
    }
    fn atmel_expected_offset(&self) -> u16 {
        (self.busy_until & FLASH_ATMEL_EXPECTED_OFFSET_MASK) as u16
    }
    fn set_atmel_page_progress(&mut self, expected_offset: Option<u16>) {
        self.busy_until = FLASH_ATMEL_FLAG;
        if let Some(offset) = expected_offset {
            self.busy_until |= FLASH_ATMEL_PAGE_ACTIVE_FLAG | u64::from(offset);
        }
    }
    fn write_atmel_page_byte(&mut self, offset: u16, value: u8) {
        if !self.atmel_page_active() {
            if offset & 0x7f != 0 {
                self.phase = Phase::Ready;
                return;
            }
            self.data[usize::from(offset)..usize::from(offset) + 128].fill(0xff);
        } else if offset != self.atmel_expected_offset() {
            self.set_atmel_page_progress(None);
            self.phase = Phase::Ready;
            return;
        }
        self.data[usize::from(offset)] = value;
        if offset & 0x7f == 0x7f {
            self.set_atmel_page_progress(None);
            self.phase = Phase::Ready;
        } else {
            self.set_atmel_page_progress(Some(offset + 1));
        }
    }
}

#[derive(Codec)]
pub struct Eeprom {
    pub data: Box<[u8; Self::MAX_SIZE]>,
    pub large: bool,
    pub input_count: usize,
    #[codec(bits = 2)]
    pub command: u8,
    #[codec(bits = 14)]
    pub address: u16,
    pub write_value: u64,
    #[codec(bits = 7)]
    pub output_index: Option<u8>,
    #[codec(bits = 10)]
    pub read_block: u16,
    pub busy_until: Cycle,
}
impl_clone_fields!(Eeprom {
    data,
    large,
    input_count,
    command,
    address,
    write_value,
    output_index,
    read_block,
    busy_until,
});
impl Default for Eeprom {
    fn default() -> Self {
        Self::init(false)
    }
}
impl Eeprom {
    pub const MAX_SIZE: usize = 8 * 1024;
    fn init(large: bool) -> Self {
        Self {
            data: vec![0xff; Self::MAX_SIZE].into_boxed_slice().try_into().unwrap(),
            large,
            input_count: 0,
            command: 0,
            address: 0,
            write_value: 0,
            output_index: None,
            read_block: 0,
            busy_until: 0,
        }
    }
    fn write_bit(&mut self, bit: u8, now: Cycle) {
        let address_bits = if self.large { 14 } else { 6 };
        let address_end = 2 + address_bits;
        let data_end = address_end + 64;
        if self.input_count < 2 {
            self.command = (self.command << 1 | bit) & 3;
        } else if self.input_count < address_end {
            self.address = (self.address << 1 | u16::from(bit)) & 0x3fff;
        } else if self.command == 0b10 && self.input_count < data_end {
            self.write_value = self.write_value << 1 | u64::from(bit);
        } else {
            if bit == 0 {
                self.finish_command(now);
            }
            self.reset_input();
            return;
        }
        self.input_count += 1;
    }
    fn read_bit(&mut self, now: Cycle) -> u8 {
        let Some(index) = self.output_index else {
            return u8::from(now >= self.busy_until);
        };
        let bit = if index < 4 {
            0
        } else {
            let serial_index = usize::from(index - 4);
            let byte = self.data[usize::from(self.read_block) * 8 + serial_index / 8];
            (byte >> (7 - serial_index % 8)) & 1
        };
        self.output_index = if index == 67 { None } else { Some(index + 1) };
        bit
    }
    fn finish_command(&mut self, now: Cycle) {
        if self.large && self.address & 0x3c00 != 0 {
            return;
        }
        let block = self.address & if self.large { 0x03ff } else { 0x003f };
        match self.command {
            0b10 => {
                let first = usize::from(block) * 8;
                for index in 0..8 {
                    self.data[first + index] = (self.write_value >> ((7 - index) * 8)) as u8;
                }
                self.busy_until = now.saturating_add(EEPROM_WRITE_CYCLES);
            }
            0b11 => {
                self.read_block = block;
                self.output_index = Some(0);
            }
            _ => {}
        }
    }
    fn reset_input(&mut self) {
        self.input_count = 0;
        self.command = 0;
        self.address = 0;
        self.write_value = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Zig: save-type detection preserves aligned signature validation and conflicts
    #[test]
    fn save_type_detection_preserves_aligned_signature_validation_and_conflicts() {
        for (rom, expected) in [
            (b"....FLASH1M_V103".as_slice(), Detection::Flash128k),
            (b"....FLASH512_V130", Detection::Flash64k),
            (b"....FLASH_V123", Detection::Flash64k),
            (b"....SRAM_V110", Detection::Sram),
            (b"....EEPROM_V124", Detection::EepromCapacityRequired),
            (b".FLASH1M_V103", Detection::Unknown),
            (b"....FLASH1M_V10x", Detection::Unknown),
            (b"FLASH1M_V103....SRAM_V110", Detection::ConflictingMarkers),
        ] {
            assert_eq!(detect_type(rom), expected);
        }
    }
    /// Zig: EEPROM byte-window traffic is open and leaves the serial protocol unchanged
    #[test]
    fn eeprom_byte_window_traffic_is_open_and_leaves_the_serial_protocol_unchanged() {
        for (configured, detection_bits, address_bits) in [
            (Type::Eeprom512, None, 6),
            (Type::Eeprom8k, None, 14),
            (Type::Auto, Some(73), 6),
            (Type::Auto, Some(81), 14),
        ] {
            let mut device = Device::init(configured);
            if let Some(bit_count) = detection_bits {
                assert!(device.resolve_eeprom_transfer(bit_count));
            }
            device.write_eeprom_bit(1, 0);
            assert_eq!(device.read_byte(0x1234, 0), 0xff);
            device.write_byte(0x1234, 0, 0);
            device.write_eeprom_bit(1, 0);
            for _ in 0..address_bits {
                device.write_eeprom_bit(0, 0);
            }
            device.write_eeprom_bit(0, 0);
            for _ in 0..4 {
                assert_eq!(device.read_eeprom_bit(0), 0);
            }
            for _ in 0..64 {
                assert_eq!(device.read_eeprom_bit(0), 1);
            }
            assert_eq!(device.data()[0], 0xff);
        }
    }
    /// Zig: Atmel Flash identifies itself and erase-writes one aligned 128-byte page
    #[test]
    fn atmel_flash_identifies_itself_and_erase_writes_one_aligned_128_byte_page() {
        let mut initial = vec![0xff; FLASH_BANK_SIZE];
        initial[0x180..0x200].fill(0);
        let mut device = Device::init_with_data(Type::Flash64kAtmel, &initial);
        assert_eq!(device.resolved_type(), Type::Flash64kAtmel);
        for (offset, value) in [(0x5555, 0xaa), (0x2aaa, 0x55), (0x5555, 0x90)] {
            device.write_byte(offset, value, 0);
        }
        assert_eq!(device.read_byte(0, 0), 0x1f);
        assert_eq!(device.read_byte(1, 0), 0x3d);
        for (offset, value) in [
            (0x5555, 0xaa),
            (0x2aaa, 0x55),
            (0x5555, 0xf0),
            (0x5555, 0xaa),
            (0x2aaa, 0x55),
            (0x5555, 0xa0),
        ] {
            device.write_byte(offset, value, 0);
        }
        for index in 0..128 {
            device.write_byte(0x180 + index, ((index * 37) ^ 0xa5) as u8, 0);
        }
        for index in 0..128 {
            assert_eq!(device.read_byte(0x180 + index, 0), ((index * 37) ^ 0xa5) as u8);
        }
        assert_eq!(device.read_byte(0x17f, 0), 0xff);
        assert_eq!(device.read_byte(0x200, 0), 0xff);
        for (offset, value) in [
            (0x5555, 0xaa),
            (0x2aaa, 0x55),
            (0x5555, 0x80),
            (0x5555, 0xaa),
            (0x2aaa, 0x55),
            (0, 0x30),
        ] {
            device.write_byte(offset, value, 0);
        }
        assert_eq!(device.resolved_type(), Type::Flash64kAtmel);
        assert_eq!(device.read_byte(0x180, 0), 0xa5);
    }

    /// Zig: Atmel Flash page progress survives a save-state round trip
    #[test]
    fn atmel_flash_page_progress_survives_a_save_state_round_trip() {
        let mut original = Device::init(Type::Flash64kAtmel);
        for (offset, value) in [(0x5555, 0xaa), (0x2aaa, 0x55), (0x5555, 0xa0)] {
            original.write_byte(offset, value, 0);
        }
        for index in 0..17 {
            original.write_byte(0x280 + index, (index ^ 0x5a) as u8, 0);
        }
        let mut encoded = Vec::new();
        original.write_state(&mut Sink::new(Some(&mut encoded), None)).unwrap();
        let mut restored = Device::init(Type::None);
        restored.read_state(&mut Source::new(&encoded)).unwrap();
        assert_eq!(restored.resolved_type(), Type::Flash64kAtmel);
        for index in 17..128 {
            restored.write_byte(0x280 + index, (index ^ 0x5a) as u8, 0);
        }
        for index in 0..128 {
            assert_eq!(restored.read_byte(0x280 + index, 0), (index ^ 0x5a) as u8);
        }
    }

    #[test]
    fn state_schema_matches_zig_save_device() {
        let mut schema = Vec::new();
        Device::write_state_schema(&mut Sink::new(Some(&mut schema), None)).unwrap();
        assert_eq!(schema.len(), 621);
        assert_eq!(
            blake3::hash(&schema).to_hex().as_str(),
            "4558f680874aaddcaeb6946adaa33f9cade4f15f2e900d46bee22688216ef111"
        );
    }

    /// Zig bus_test: runtime SRAM detection replays a broken Flash unlock prefix
    #[test]
    fn runtime_sram_detection_replays_a_broken_flash_unlock_prefix() {
        let mut device = Device::init(Type::Auto);
        device.write_byte(0x5555, 0xaa, 0);
        assert_eq!(device.resolved_type(), Type::Auto);
        device.write_byte(0x1234, 0xa5, 0);
        assert_eq!(device.resolved_type(), Type::Sram);
        assert_eq!(device.read_byte(0x5555, 0), 0xaa);
        assert_eq!(device.read_byte(0x1234, 0), 0xa5);
        let mut read_after_prefix = Device::init(Type::Auto);
        read_after_prefix.write_byte(0x5555, 0xaa, 0);
        assert_eq!(read_after_prefix.read_byte(0x5555, 0), 0xaa);
        assert_eq!(read_after_prefix.resolved_type(), Type::Sram);
        let mut invalid_command = Device::init(Type::Auto);
        for (offset, value) in [(0x5555, 0xaa), (0x2aaa, 0x55), (0x5555, 0x10)] {
            invalid_command.write_byte(offset, value, 0);
        }
        assert_eq!(invalid_command.resolved_type(), Type::Sram);
        assert_eq!(invalid_command.read_byte(0x5555, 0), 0x10);
        assert_eq!(invalid_command.read_byte(0x2aaa, 0), 0x55);
    }

    /// Zig bus_test: runtime save detection recognizes Flash commands and later bank promotion
    #[test]
    fn runtime_save_detection_recognizes_flash_commands_and_later_bank_promotion() {
        let mut device = Device::init(Type::Auto);
        device.write_byte(0x5555, 0xaa, 0);
        device.write_byte(0x2aaa, 0x55, 0);
        assert_eq!(device.resolved_type(), Type::Auto);
        device.write_byte(0x5555, 0xa0, 0);
        assert_eq!(device.resolved_type(), Type::Flash64k);
        device.write_byte(0x123, 0x5a, 0);
        assert_eq!(device.read_byte(0x123, 0), 0x5a);
        assert_eq!(device.data().len(), 65536);
        for (offset, value) in [(0x5555, 0xaa), (0x2aaa, 0x55), (0x5555, 0xb0), (0, 1)] {
            device.write_byte(offset, value, 0);
        }
        assert_eq!(device.resolved_type(), Type::Flash128k);
        assert_eq!(device.data().len(), 131072);
        for (offset, value) in [(0x5555, 0xaa), (0x2aaa, 0x55), (0x5555, 0xa0), (0x123, 0xa5)] {
            device.write_byte(offset, value, 0);
        }
        assert_eq!(device.read_byte(0x123, 0), 0xa5);
        for (offset, value) in [(0x5555, 0xaa), (0x2aaa, 0x55), (0x5555, 0xb0), (0, 0)] {
            device.write_byte(offset, value, 0);
        }
        assert_eq!(device.read_byte(0x123, 0), 0x5a);
        let mut fixed = Device::init(Type::Flash64k);
        for (offset, value) in [(0x5555, 0xaa), (0x2aaa, 0x55), (0x5555, 0xb0), (0, 1)] {
            fixed.write_byte(offset, value, 0);
        }
        assert_eq!(fixed.resolved_type(), Type::Flash64k);
    }
}
