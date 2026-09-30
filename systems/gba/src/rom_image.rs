//! Byte-oriented cartridge import and save selection. No host I/O.

use crate::cartridge::save::{self, Detection, Type};

pub const MAX_ROM_SIZE: usize = 32 * 1024 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    OutOfMemory,
    ZipNoEndRecord,
    ZipMultiDiskUnsupported,
    NoGbaInZip,
    ZipTruncated,
    ZipBadCentralHeader,
    MultipleGbaInZip,
    ZipBadLocalHeader,
    ZipEncryptedUnsupported,
    Zip64Unsupported,
    InvalidRomSize,
    ZipBadStoredSizes,
    ZipUnsupportedMethod,
    ZipCrcMismatch,
    ZipDecompressFailed,
    InvalidSaveFileSize,
    SaveTypeConflict,
    SaveTypeOverrideRequired,
}

fn read_u16(bytes: &[u8], index: usize) -> u16 {
    u16::from_le_bytes(bytes[index..index + 2].try_into().unwrap())
}
fn read_u32(bytes: &[u8], index: usize) -> u32 {
    u32::from_le_bytes(bytes[index..index + 4].try_into().unwrap())
}

/// Extract exactly one `.gba` entry, validating central metadata, size and CRC.
pub fn extract_single_gba(zip: &[u8]) -> Result<Vec<u8>, Error> {
    let end = find_end_record(zip).ok_or(Error::ZipNoEndRecord)?;
    let this_disk = read_u16(zip, end + 4);
    let cd_disk = read_u16(zip, end + 6);
    let cd_count = read_u16(zip, end + 10);
    let cd_offset = read_u32(zip, end + 16);
    if this_disk != 0 || cd_disk != 0 {
        return Err(Error::ZipMultiDiskUnsupported);
    }
    if cd_count == 0 {
        return Err(Error::NoGbaInZip);
    }
    let mut selected = None;
    let mut matches = 0;
    let mut cursor = cd_offset as usize;
    for _ in 0..cd_count {
        if zip.len().saturating_sub(cursor) < 46 {
            return Err(Error::ZipTruncated);
        }
        if &zip[cursor..cursor + 4] != b"PK\x01\x02" {
            return Err(Error::ZipBadCentralHeader);
        }
        let name_len = read_u16(zip, cursor + 28) as usize;
        let extra_len = read_u16(zip, cursor + 30) as usize;
        let comment_len = read_u16(zip, cursor + 32) as usize;
        let method = read_u16(zip, cursor + 10);
        let crc = read_u32(zip, cursor + 16);
        let comp_size = read_u32(zip, cursor + 20);
        let uncomp_size = read_u32(zip, cursor + 24);
        let header_offset = read_u32(zip, cursor + 42);
        if name_len + extra_len + comment_len > zip.len() - cursor - 46 {
            return Err(Error::ZipTruncated);
        }
        let name = &zip[cursor + 46..cursor + 46 + name_len];
        if !is_apple_double_sidecar(name)
            && name.len() >= 4
            && name[name.len() - 4..].eq_ignore_ascii_case(b".gba")
            && !name.ends_with(b"/")
        {
            matches += 1;
            selected = Some((header_offset, method, crc, comp_size, uncomp_size));
        }
        cursor += 46 + name_len + extra_len + comment_len;
    }
    if matches == 0 {
        return Err(Error::NoGbaInZip);
    }
    if matches > 1 {
        return Err(Error::MultipleGbaInZip);
    }
    let (local, method, crc, comp_size, uncomp_size) = selected.unwrap();
    let local = local as usize;
    if zip.len().saturating_sub(local) < 30 {
        return Err(Error::ZipTruncated);
    }
    if &zip[local..local + 4] != b"PK\x03\x04" {
        return Err(Error::ZipBadLocalHeader);
    }
    let flags = read_u16(zip, local + 6);
    let local_name_len = read_u16(zip, local + 26) as usize;
    let local_extra_len = read_u16(zip, local + 28) as usize;
    if flags & 1 != 0 {
        return Err(Error::ZipEncryptedUnsupported);
    }
    if comp_size == u32::MAX || uncomp_size == u32::MAX {
        return Err(Error::Zip64Unsupported);
    }
    if uncomp_size == 0 || uncomp_size as usize > MAX_ROM_SIZE {
        return Err(Error::InvalidRomSize);
    }
    if local_name_len + local_extra_len > zip.len() - local - 30 {
        return Err(Error::ZipTruncated);
    }
    let data_start = local + 30 + local_name_len + local_extra_len;
    if comp_size as usize > zip.len() - data_start {
        return Err(Error::ZipTruncated);
    }
    let comp = &zip[data_start..data_start + comp_size as usize];
    let out = match method {
        0 => {
            if comp_size != uncomp_size {
                return Err(Error::ZipBadStoredSizes);
            }
            let mut out = Vec::new();
            out.try_reserve_exact(uncomp_size as usize).map_err(|_| Error::OutOfMemory)?;
            out.extend_from_slice(comp);
            out
        }
        8 => {
            // Rewind's inflater handles its own format, not raw ZIP DEFLATE.
            let mut out = Vec::new();
            out.try_reserve_exact(uncomp_size as usize).map_err(|_| Error::OutOfMemory)?;
            out.resize(uncomp_size as usize, 0);
            let written = miniz_oxide::inflate::decompress_slice_iter_to_slice(
                &mut out,
                core::iter::once(comp),
                false,
                false,
            )
            .map_err(|_| Error::ZipDecompressFailed)?;
            if written != uncomp_size as usize {
                return Err(Error::ZipDecompressFailed);
            }
            out
        }
        _ => return Err(Error::ZipUnsupportedMethod),
    };
    if crc32(&out) != crc {
        return Err(Error::ZipCrcMismatch);
    }
    Ok(out)
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0_u32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ if crc & 1 != 0 { 0xedb8_8320 } else { 0 };
        }
    }
    !crc
}

fn is_apple_double_sidecar(name: &[u8]) -> bool {
    if name.starts_with(b"__MACOSX/") {
        return true;
    }
    let base = name.rsplit(|&c| c == b'/').next().unwrap_or(name);
    base.starts_with(b"._")
}

fn find_end_record(zip: &[u8]) -> Option<usize> {
    if zip.len() < 22 {
        return None;
    }
    let mut offset = zip.len() - 22;
    loop {
        if &zip[offset..offset + 4] == b"PK\x05\x06"
            && read_u16(zip, offset + 20) as usize == zip.len() - offset - 22
        {
            return Some(offset);
        }
        if offset == 0 || zip.len() - offset > 22 + 65536 {
            return None;
        }
        offset -= 1;
    }
}

#[derive(Clone, Copy)]
pub struct SaveOverride {
    pub game_code: [u8; 4],
    pub crc32: Option<u32>,
    pub save_type: Type,
}

pub const SAVE_OVERRIDES: [SaveOverride; 6] = [
    SaveOverride { game_code: *b"AXVE", crc32: Some(0x6164_1576), save_type: Type::Flash128k },
    SaveOverride { game_code: *b"AXVE", crc32: Some(0xaeac_73e6), save_type: Type::Flash128k },
    SaveOverride { game_code: *b"AXVE", crc32: Some(0xf081_5ee7), save_type: Type::Flash128k },
    SaveOverride { game_code: *b"A2YE", crc32: None, save_type: Type::None },
    SaveOverride { game_code: *b"AGFE", crc32: None, save_type: Type::Flash64k },
    SaveOverride { game_code: *b"RZWE", crc32: None, save_type: Type::Sram },
];

pub fn find_save_override(rom: &[u8], overrides: &[SaveOverride]) -> Option<Type> {
    let code = rom.get(0xac..0xb0)?;
    let mut fallback = None;
    let mut crc = None;
    for entry in overrides {
        if code != entry.game_code {
            continue;
        }
        if let Some(expected) = entry.crc32 {
            let actual = *crc.get_or_insert_with(|| crc32(rom));
            if actual == expected {
                return Some(entry.save_type);
            }
        } else {
            fallback = Some(entry.save_type);
        }
    }
    fallback
}

pub fn choose_save_type(
    rom: &[u8],
    save_type_override: Option<Type>,
    save_data: Option<&[u8]>,
) -> Result<Type, Error> {
    if let Some(save_type) = save_type_override
        && save_type != Type::Auto
    {
        return Ok(save_type);
    }
    let overridden = find_save_override(rom, &SAVE_OVERRIDES);
    let detected = save::detect_type(rom);
    if let Some(data) = save_data {
        let from_size = match save_type_from_size(data.len()) {
            Ok(value) => value,
            Err(_) => {
                let known = if let Some(value) = overridden {
                    value
                } else {
                    match detected {
                        Detection::Sram => Type::Sram,
                        Detection::Flash64k => Type::Flash64k,
                        Detection::Flash128k => Type::Flash128k,
                        Detection::ConflictingMarkers => {
                            return Err(Error::SaveTypeOverrideRequired);
                        }
                        _ => return Err(Error::InvalidSaveFileSize),
                    }
                };
                if save_size(known) == 0 || data.len() < save_size(known) {
                    return Err(Error::InvalidSaveFileSize);
                }
                return Ok(known);
            }
        };
        if let Some(save_type) = overridden {
            if from_size != save_type {
                return Err(Error::SaveTypeConflict);
            }
            return Ok(save_type);
        }
        match detected {
            Detection::Unknown => {}
            Detection::Sram if from_size != Type::Sram => return Err(Error::SaveTypeConflict),
            Detection::Flash64k if from_size != Type::Flash64k => {
                return Err(Error::SaveTypeConflict);
            }
            Detection::Flash128k if from_size != Type::Flash128k => {
                return Err(Error::SaveTypeConflict);
            }
            Detection::EepromCapacityRequired
                if !matches!(from_size, Type::Eeprom512 | Type::Eeprom8k) =>
            {
                return Err(Error::SaveTypeConflict);
            }
            Detection::ConflictingMarkers => return Err(Error::SaveTypeOverrideRequired),
            _ => {}
        }
        return Ok(from_size);
    }
    if let Some(save_type) = overridden {
        return Ok(save_type);
    }
    match detected {
        Detection::Unknown | Detection::EepromCapacityRequired => Ok(Type::Auto),
        Detection::Sram => Ok(Type::Sram),
        Detection::Flash64k => Ok(Type::Flash64k),
        Detection::Flash128k => Ok(Type::Flash128k),
        Detection::ConflictingMarkers => Err(Error::SaveTypeOverrideRequired),
    }
}

fn save_type_from_size(size: usize) -> Result<Type, Error> {
    match size {
        512 => Ok(Type::Eeprom512),
        8192 => Ok(Type::Eeprom8k),
        32768 => Ok(Type::Sram),
        65536 => Ok(Type::Flash64k),
        131072 => Ok(Type::Flash128k),
        _ => Err(Error::InvalidSaveFileSize),
    }
}
pub fn save_size(save_type: Type) -> usize {
    match save_type {
        Type::Auto | Type::None => 0,
        Type::Sram => 32768,
        Type::Flash64k | Type::Flash64kAtmel => 65536,
        Type::Flash128k => 131072,
        Type::Eeprom512 => 512,
        Type::Eeprom8k => 8192,
    }
}
pub fn save_prefix(save_type: Type, data: &[u8]) -> Result<&[u8], Error> {
    let size = save_size(save_type);
    if size == 0 || data.len() < size {
        return Err(Error::InvalidSaveFileSize);
    }
    Ok(&data[..size])
}

#[cfg(test)]
mod tests {
    use super::*;

    // A minimal ZIP writer in the test only; CRC is the published CRC-32
    // check value for "123456789", not computed by the production routine.
    fn zip_entry(name: &[u8], compressed: &[u8], method: u16) -> Vec<u8> {
        let mut zip = Vec::new();
        zip.extend_from_slice(b"PK\x03\x04");
        zip.extend_from_slice(&20_u16.to_le_bytes());
        zip.extend_from_slice(&0_u16.to_le_bytes());
        zip.extend_from_slice(&method.to_le_bytes());
        zip.extend_from_slice(&[0; 4]);
        zip.extend_from_slice(&0xcbf4_3926_u32.to_le_bytes());
        zip.extend_from_slice(&(compressed.len() as u32).to_le_bytes());
        zip.extend_from_slice(&9_u32.to_le_bytes());
        zip.extend_from_slice(&(name.len() as u16).to_le_bytes());
        zip.extend_from_slice(&0_u16.to_le_bytes());
        zip.extend_from_slice(name);
        zip.extend_from_slice(compressed);
        let central = zip.len() as u32;
        zip.extend_from_slice(b"PK\x01\x02");
        zip.extend_from_slice(&[0; 2]);
        zip.extend_from_slice(&20_u16.to_le_bytes());
        zip.extend_from_slice(&0_u16.to_le_bytes());
        zip.extend_from_slice(&method.to_le_bytes());
        zip.extend_from_slice(&[0; 4]);
        zip.extend_from_slice(&0xcbf4_3926_u32.to_le_bytes());
        zip.extend_from_slice(&(compressed.len() as u32).to_le_bytes());
        zip.extend_from_slice(&9_u32.to_le_bytes());
        zip.extend_from_slice(&(name.len() as u16).to_le_bytes());
        zip.extend_from_slice(&[0; 12]);
        zip.extend_from_slice(&0_u32.to_le_bytes());
        zip.extend_from_slice(name);
        let central_size = zip.len() as u32 - central;
        zip.extend_from_slice(b"PK\x05\x06");
        zip.extend_from_slice(&[0; 4]);
        zip.extend_from_slice(&1_u16.to_le_bytes());
        zip.extend_from_slice(&1_u16.to_le_bytes());
        zip.extend_from_slice(&central_size.to_le_bytes());
        zip.extend_from_slice(&central.to_le_bytes());
        zip.extend_from_slice(&0_u16.to_le_bytes());
        zip
    }

    #[test]
    fn zip_stored_and_deflate_extract_and_validate_crc() {
        let stored = zip_entry(b"game.GBA", b"123456789", 0);
        assert_eq!(extract_single_gba(&stored).unwrap(), b"123456789");
        let compressed = miniz_oxide::deflate::compress_to_vec(b"123456789", 6);
        let deflated = zip_entry(b"game.gba", &compressed, 8);
        assert_eq!(extract_single_gba(&deflated).unwrap(), b"123456789");
        let mut corrupted = stored;
        corrupted[14] ^= 1;
        // Central-directory CRC, rather than the local-header CRC, owns validation.
        assert_eq!(extract_single_gba(&corrupted).unwrap(), b"123456789");
        let central = corrupted.windows(4).position(|s| s == b"PK\x01\x02").unwrap();
        corrupted[central + 16] ^= 1;
        assert_eq!(extract_single_gba(&corrupted), Err(Error::ZipCrcMismatch));
    }

    #[test]
    fn save_choice_prefers_explicit_and_exact_markers_and_rejects_conflicts() {
        let mut rom = vec![0; 0x100];
        rom[0xac..0xb0].copy_from_slice(b"AGFE");
        rom[0xc0..0xcc].copy_from_slice(b"FLASH1M_V103");
        assert_eq!(choose_save_type(&rom, None, None), Ok(Type::Flash64k));
        assert_eq!(choose_save_type(&rom, Some(Type::Flash128k), None), Ok(Type::Flash128k));
        assert_eq!(choose_save_type(&rom, None, Some(&vec![0; 65536])), Ok(Type::Flash64k));
        assert_eq!(
            choose_save_type(&rom, None, Some(&vec![0; 131072])),
            Err(Error::SaveTypeConflict)
        );
        assert_eq!(save_prefix(Type::Flash64k, &vec![0x5a; 65537]).unwrap().len(), 65536);
    }

    #[test]
    fn pinned_title_save_override_precedes_a_stale_rom_marker() {
        let mut top_gun = [0; 0xb0];
        top_gun[..12].copy_from_slice(b"FLASH1M_V123");
        top_gun[0xac..0xb0].copy_from_slice(b"A2YE");
        assert_eq!(choose_save_type(&top_gun, None, None), Ok(Type::None));
        assert_eq!(choose_save_type(&top_gun, Some(Type::Sram), None), Ok(Type::Sram));
    }

    #[test]
    fn save_override_crc_entries_are_exact_and_fall_back_only_by_game_code() {
        let mut rom = [0; 0xb0];
        rom[0xac..0xb0].copy_from_slice(b"TEST");
        let overrides = [
            SaveOverride {
                game_code: *b"TEST",
                crc32: Some(0xe0fe_12cb),
                save_type: Type::Flash128k,
            },
            SaveOverride { game_code: *b"TEST", crc32: None, save_type: Type::Sram },
        ];
        assert_eq!(find_save_override(&rom, &overrides), Some(Type::Flash128k));
        rom[4] = 1;
        assert_eq!(find_save_override(&rom, &overrides), Some(Type::Sram));
    }
}
