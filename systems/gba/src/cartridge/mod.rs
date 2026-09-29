//! Cartridge metadata and peripherals.

pub mod gpio;
pub mod save;
pub mod tilt;

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct Features {
    pub rtc: bool,
    pub solar_sensor: bool,
    pub gyro: bool,
    pub rumble: bool,
    pub tilt: bool,
}

/// Detects hardware only from known retail game codes in the GBA header.
pub fn detect_features(rom: &[u8]) -> Features {
    if rom.len() < 0xb0 {
        return Features::default();
    }
    let code = &rom[0xac..0xb0];
    if has_rtc_regional_prefix(code, b"AXV")
        || has_rtc_regional_prefix(code, b"AXP")
        || has_rtc_regional_prefix(code, b"BPE")
    {
        return Features { rtc: true, ..Features::default() };
    }
    for known in [b"U3IJ", b"U3IE", b"U3IP", b"U32J", b"U32E", b"U32P", b"U33J"] {
        if code == known {
            return Features { rtc: true, solar_sensor: true, ..Features::default() };
        }
    }
    if has_any_region(code, b"RZW") {
        return Features { gyro: true, rumble: true, ..Features::default() };
    }
    if has_any_region(code, b"V49") {
        return Features { rumble: true, ..Features::default() };
    }
    if has_any_region(code, b"KYG") || code == b"KHPJ" {
        return Features { tilt: true, ..Features::default() };
    }
    for known in [b"BLJJ", b"BLJK", b"BLVJ", b"BR4J", b"BKAJ"] {
        if code == known {
            return Features { rtc: true, ..Features::default() };
        }
    }
    if rom.windows(5).any(|window| window == b"RTC_V") {
        return Features { rtc: true, ..Features::default() };
    }
    Features::default()
}

fn has_rtc_regional_prefix(code: &[u8], prefix: &[u8; 3]) -> bool {
    &code[..3] == prefix && b"JEPISDF".contains(&code[3])
}
fn has_any_region(code: &[u8], prefix: &[u8; 3]) -> bool {
    &code[..3] == prefix && b"JEP".contains(&code[3])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Zig: RTC feature detection uses known game codes without broad Pokemon guesses
    #[test]
    fn rtc_feature_detection_uses_known_game_codes_without_broad_pokemon_guesses() {
        let mut rom = [0; 0xb0];
        for code in
            [b"AXVE", b"AXPJ", b"BPEF", b"U3IP", b"U32J", b"U33J", b"BLJK", b"BR4J", b"BKAJ"]
        {
            rom[0xac..0xb0].copy_from_slice(code);
            assert!(detect_features(&rom).rtc);
        }
        for code in [b"BPRE", b"BPGE", b"B24E", b"AXVX"] {
            rom[0xac..0xb0].copy_from_slice(code);
            assert!(!detect_features(&rom).rtc);
        }
        assert!(!detect_features(&rom[..0xaf]).rtc);
        rom[0xac..0xb0].copy_from_slice(b"U3IE");
        assert!(detect_features(&rom).solar_sensor);
        rom[0xac..0xb0].copy_from_slice(b"AXVE");
        assert!(!detect_features(&rom).solar_sensor);
        rom[0xac..0xb0].copy_from_slice(b"RZWE");
        assert!(detect_features(&rom).gyro);
        assert!(detect_features(&rom).rumble);
        rom[0xac..0xb0].copy_from_slice(b"V49P");
        assert!(!detect_features(&rom).gyro);
        assert!(detect_features(&rom).rumble);
        rom[0xac..0xb0].copy_from_slice(b"KYGE");
        assert!(detect_features(&rom).tilt);
        rom[0xac..0xb0].copy_from_slice(b"KHPJ");
        assert!(detect_features(&rom).tilt);
    }

    /// Zig: RTC feature detection falls back to the RTC_V hardware string
    #[test]
    fn rtc_feature_detection_falls_back_to_the_rtc_v_hardware_string() {
        let mut rom = [0; 0x100];
        rom[0xac..0xb0].copy_from_slice(b"XXXX");
        assert!(!detect_features(&rom).rtc);
        rom[0xf0..0xf5].copy_from_slice(b"RTC_V");
        assert!(detect_features(&rom).rtc);
    }
}
