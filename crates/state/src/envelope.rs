//! The save-state envelope around a machine-owned payload and media identity.
use crate::{Codec, Digest, Error, Result, Sink, Source};
use alloc::vec::Vec;

pub const MAGIC: &[u8; 8] = b"EVENFALL";
pub const VERSION: u32 = 7;
/// Names the encoding rules at the start of every schema description.
pub const CODEC_NAME: &str = "evenfall.state-codec.fixed-slots.v1";
pub const MAX_PAYLOAD: u64 = 1024 * 1024;
pub const HEADER_SIZE: usize = 133;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Binding {
    pub boot_mode: u8,
    pub rom_len: u64,
    pub rom_digest: Digest,
    pub bios_len: u64,
    pub bios_digest: Digest,
}
impl Binding {
    pub fn from_media(boot_mode: u8, rom: &[u8], bios: &[u8]) -> Self {
        Self {
            boot_mode,
            rom_len: rom.len() as u64,
            rom_digest: *blake3::hash(rom).as_bytes(),
            bios_len: bios.len() as u64,
            bios_digest: *blake3::hash(bios).as_bytes(),
        }
    }
}

/// The digest of the codec rules, the payload's name and the payload schema
/// that `schema` writes.
pub fn schema_digest(
    payload: &str,
    schema: impl FnOnce(&mut Sink<'_>) -> Result<()>,
) -> Result<Digest> {
    let mut hash = blake3::Hasher::new();
    let mut sink = Sink::new(None, Some(&mut hash));
    crate::schema_name(&mut sink, CODEC_NAME)?;
    crate::schema_name(&mut sink, payload)?;
    schema(&mut sink)?;
    Ok(*hash.finalize().as_bytes())
}

fn header(output: &mut Vec<u8>, length: u64, digest: Digest, media: Binding) -> Result<()> {
    if length > MAX_PAYLOAD {
        return Err(Error::InvalidSaveState);
    }
    let mut sink = Sink::new(Some(output), None);
    sink.write_all(MAGIC)?;
    VERSION.encode(&mut sink)?;
    digest.encode(&mut sink)?;
    length.encode(&mut sink)?;
    media.boot_mode.encode(&mut sink)?;
    media.rom_len.encode(&mut sink)?;
    media.rom_digest.encode(&mut sink)?;
    media.bios_len.encode(&mut sink)?;
    media.bios_digest.encode(&mut sink)?;
    Ok(())
}

/// Counting pass followed by streaming the payload directly into the output.
pub fn write_streaming(
    media: Binding,
    schema: Digest,
    payload: impl Fn(&mut Sink<'_>) -> Result<()>,
) -> Result<Vec<u8>> {
    let mut counter = Sink::new(None, None);
    payload(&mut counter)?;
    let mut output = Vec::new();
    header(&mut output, counter.count, schema, media)?;
    let mut hasher = blake3::Hasher::new();
    let mut sink = Sink::new(Some(&mut output), Some(&mut hasher));
    payload(&mut sink)?;
    if sink.count != counter.count {
        return Err(Error::InvalidSaveState);
    }
    output.extend_from_slice(hasher.finalize().as_bytes());
    Ok(output)
}

/// One traversal into a buffer, filling in the header after the payload is known.
pub fn write_buffered(
    media: Binding,
    schema: Digest,
    payload: impl FnOnce(&mut Sink<'_>) -> Result<()>,
) -> Result<Vec<u8>> {
    let mut output = alloc::vec![0; HEADER_SIZE];
    let mut sink = Sink::new(Some(&mut output), None);
    payload(&mut sink)?;
    if sink.count > MAX_PAYLOAD {
        return Err(Error::InvalidSaveState);
    }
    let length = sink.count;
    let hash = *blake3::hash(&output[HEADER_SIZE..]).as_bytes();
    output.extend_from_slice(&hash);
    let mut prefix = Vec::new();
    header(&mut prefix, length, schema, media)?;
    output[..HEADER_SIZE].copy_from_slice(&prefix);
    Ok(output)
}

/// Decode into caller-supplied scratch storage; owners validate semantic state
/// and replace their live machine only after this succeeds.
pub fn read(
    input: &[u8],
    media: Binding,
    schema: Digest,
    decode: impl FnOnce(&mut Source<'_>) -> Result<()>,
) -> Result<()> {
    let mut source = Source::new(input);
    let mut magic = [0; 8];
    source.read_all(&mut magic)?;
    if &magic != MAGIC {
        return Err(Error::InvalidSaveState);
    }
    let mut version = 0u32;
    version.decode(&mut source)?;
    if version != VERSION {
        return Err(Error::UnsupportedSaveStateVersion);
    }
    let mut actual_schema = [0; 32];
    actual_schema.decode(&mut source)?;
    if actual_schema != schema {
        return Err(Error::SaveStateSchemaMismatch);
    }
    let mut length = 0u64;
    length.decode(&mut source)?;
    if length > MAX_PAYLOAD {
        return Err(Error::InvalidSaveState);
    }
    let mut boot_mode = 0u8;
    boot_mode.decode(&mut source)?;
    if boot_mode != media.boot_mode {
        return Err(Error::SaveStateBootModeMismatch);
    }
    let mut rom_len = 0u64;
    let mut rom_digest = [0; 32];
    rom_len.decode(&mut source)?;
    rom_digest.decode(&mut source)?;
    if rom_len != media.rom_len || rom_digest != media.rom_digest {
        return Err(Error::SaveStateRomMismatch);
    }
    let mut bios_len = 0u64;
    let mut bios_digest = [0; 32];
    bios_len.decode(&mut source)?;
    bios_digest.decode(&mut source)?;
    if bios_len != media.bios_len || bios_digest != media.bios_digest {
        return Err(Error::SaveStateBiosMismatch);
    }
    let remaining = source.input;
    let mut hash = blake3::Hasher::new();
    let consumed = {
        let mut payload = Source::new(remaining);
        payload.limit = Some(length);
        payload.hasher = Some(&mut hash);
        decode(&mut payload)?;
        if payload.count != length {
            return Err(Error::InvalidSaveState);
        }
        length as usize
    };
    let rest = &remaining[consumed..];
    if rest.len() < 32 || rest[..32] != *hash.finalize().as_bytes() || rest.len() != 32 {
        return Err(Error::InvalidSaveState);
    }
    Ok(())
}
