//! Cartridge serial peripherals exercised through Bus transactions.
use super::*;

fn rtc_begin(bus: &mut Bus) {
    write_half(bus, 0x0800_00c6, 7);
    write_half(bus, 0x0800_00c4, 1);
    write_half(bus, 0x0800_00c4, 5);
}
fn rtc_end(bus: &mut Bus) {
    write_half(bus, 0x0800_00c4, 1);
}
fn rtc_send_byte(bus: &mut Bus, value: u8) {
    for bit in 0..8 {
        let sio = if value & (1 << bit) != 0 { 2 } else { 0 };
        write_half(bus, 0x0800_00c4, 4 | sio);
        write_half(bus, 0x0800_00c4, 5 | sio);
    }
}
fn rtc_read_bit(bus: &mut Bus) -> u8 {
    write_half(bus, 0x0800_00c4, 4);
    write_half(bus, 0x0800_00c4, 5);
    (read_half(bus, 0x0800_00c4) >> 1) as u8 & 1
}
fn rtc_read(bus: &mut Bus, command: u8, output: &mut [u8]) {
    rtc_begin(bus);
    rtc_send_byte(bus, command);
    write_half(bus, 0x0800_00c6, 5);
    for byte in output {
        *byte = 0;
        for bit in 0..8 {
            *byte |= rtc_read_bit(bus) << bit;
        }
    }
    rtc_end(bus);
}
fn rtc_write(bus: &mut Bus, command: u8, input: &[u8]) {
    rtc_begin(bus);
    rtc_send_byte(bus, command);
    for &byte in input {
        rtc_send_byte(bus, byte);
    }
    rtc_end(bus);
}

#[test]
fn rtc_control_abort_recovery_reset_and_force_irq_use_serial_commands() {
    let mut bus = Bus::init(&[0; 0xca]);
    bus.enable_rtc(981_205_445);
    write_half(&mut bus, 0x0800_00c8, 1);
    rtc_begin(&mut bus);
    rtc_send_byte(&mut bus, 0xc6);
    write_half(&mut bus, 0x0800_00c6, 5);
    rtc_read_bit(&mut bus);
    rtc_end(&mut bus);
    rtc_write(&mut bus, 0x46, &[0x2a]);
    let mut control = [0];
    rtc_read(&mut bus, 0xc6, &mut control);
    assert_eq!([0x2a], control);
    rtc_write(&mut bus, 0x06, &[]);
    rtc_read(&mut bus, 0xc6, &mut control);
    assert_eq!([0], control);
    write_half(&mut bus, 0x0400_0200, 1 << 13);
    bus.enter_halt();
    rtc_write(&mut bus, 0x36, &[]);
    assert_ne!(0, read_half(&mut bus, 0x0400_0202) & (1 << 13));
    assert!(bus.halt_wake_pending());
    rtc_write(&mut bus, 0x46, &[0x40]);
    rtc_read(&mut bus, 0x86, &mut []);
    rtc_read(&mut bus, 0xc6, &mut control);
    assert_eq!([0], control);
    write_half(&mut bus, 0x0400_0202, 1 << 13);
    bus.enter_halt();
    rtc_read(&mut bus, 0xb6, &mut []);
    assert_ne!(0, read_half(&mut bus, 0x0400_0202) & (1 << 13));
    assert!(bus.halt_wake_pending());
}

#[test]
fn rtc_date_time_writes_rebase_deterministic_time_and_preserve_the_written_weekday() {
    let mut bus = Bus::init(&[]);
    bus.enable_rtc(0);
    write_half(&mut bus, 0x0800_00c8, 1);
    rtc_write(&mut bus, 0x46, &[0x40]);
    rtc_write(&mut bus, 0x26, &[0x04, 0x02, 0x29, 0x00, 0x23, 0x59, 0x58]);
    let mut date_time = [0; 7];
    rtc_read(&mut bus, 0xa6, &mut date_time);
    assert_eq!([0x04, 0x02, 0x29, 0x00, 0x23, 0x59, 0x58], date_time);
    bus.elapse(3 * 16_777_216);
    rtc_read(&mut bus, 0xa6, &mut date_time);
    assert_eq!([0x04, 0x03, 0x01, 0x01, 0x00, 0x00, 0x01], date_time);
}

#[test]
fn rtc_time_writes_convert_hour_modes_while_invalid_bcd_remains_atomic() {
    let mut bus = Bus::init(&[]);
    bus.enable_rtc(0);
    write_half(&mut bus, 0x0800_00c8, 1);
    rtc_write(&mut bus, 0x46, &[0x40]);
    rtc_write(&mut bus, 0x26, &[0x24, 0x06, 0x15, 0x06, 0x08, 0x09, 0x10]);
    rtc_write(&mut bus, 0x46, &[0]);
    rtc_write(&mut bus, 0x66, &[0x81, 0x02, 0x03]);
    let mut time = [0; 3];
    rtc_read(&mut bus, 0xe6, &mut time);
    assert_eq!([0x81, 0x02, 0x03], time);
    rtc_write(&mut bus, 0x46, &[0x40]);
    rtc_read(&mut bus, 0xe6, &mut time);
    assert_eq!([0x13, 0x02, 0x03], time);
    rtc_write(&mut bus, 0x26, &[0x25, 0x1a, 0x01, 0x02, 0x01, 0x02, 0x03]);
    let mut date_time = [0; 7];
    rtc_read(&mut bus, 0xa6, &mut date_time);
    assert_eq!([0x24, 0x06, 0x15, 0x06, 0x13, 0x02, 0x03], date_time);
    rtc_write(&mut bus, 0x06, &[]);
    rtc_read(&mut bus, 0xa6, &mut date_time);
    assert_eq!([0, 1, 1, 0, 0, 0, 0], date_time);
}

#[test]
fn boktai_solar_sensor_counts_clock_pulses_and_shares_gpio_with_rtc() {
    let mut bus = Bus::init(&[0; 0xca]);
    bus.enable_rtc(981_205_445);
    bus.enable_solar_sensor(252);
    write_half(&mut bus, 0x0800_00c8, 1);
    let mut control = [0];
    rtc_read(&mut bus, 0xc6, &mut control);
    assert_eq!([0x40], control);
    write_half(&mut bus, 0x0800_00c6, 7);
    write_half(&mut bus, 0x0800_00c4, 2);
    write_half(&mut bus, 0x0800_00c4, 0);
    for _ in 0..3 {
        write_half(&mut bus, 0x0800_00c4, 1);
        write_half(&mut bus, 0x0800_00c4, 0);
        assert_eq!(0, read_half(&mut bus, 0x0800_00c4) & 8);
    }
    write_half(&mut bus, 0x0800_00c4, 1);
    write_half(&mut bus, 0x0800_00c4, 0);
    assert_ne!(0, read_half(&mut bus, 0x0800_00c4) & 8);
    bus.set_solar_level(0);
    assert_eq!(0, read_half(&mut bus, 0x0800_00c4) & 8);
    bus.set_solar_level(252);
    assert_ne!(0, read_half(&mut bus, 0x0800_00c4) & 8);
    write_half(&mut bus, 0x0800_00c4, 2);
    write_half(&mut bus, 0x0800_00c4, 4);
    for _ in 0..8 {
        write_half(&mut bus, 0x0800_00c4, 5);
        write_half(&mut bus, 0x0800_00c4, 4);
    }
    write_half(&mut bus, 0x0800_00c4, 0);
    assert_eq!(0, read_half(&mut bus, 0x0800_00c4) & 8);
}

fn read_gyro_sample(bus: &mut Bus) -> u16 {
    write_half(bus, 0x0800_00c4, 0xb);
    write_half(bus, 0x0800_00c4, 0xa);
    let mut value = 0;
    for _ in 0..16 {
        value = (value << 1) | ((read_half(bus, 0x0800_00c4) >> 2) & 1);
        write_half(bus, 0x0800_00c4, 8);
        write_half(bus, 0x0800_00c4, 0xa);
    }
    value
}

#[test]
fn wario_gyro_shifts_four_dummy_bits_and_a_12_bit_sample_while_rumble_stays_active() {
    let mut bus = Bus::init(&[0; 0xca]);
    bus.enable_gyro(0x6c0);
    bus.enable_rumble();
    write_half(&mut bus, 0x0800_00c8, 1);
    write_half(&mut bus, 0x0800_00c6, 0xb);
    assert_eq!(0x06c0, read_gyro_sample(&mut bus));
    assert!(bus.rumble_active());
    bus.set_gyro_sample(0x73a);
    assert_eq!(0x073a, read_gyro_sample(&mut bus));
    write_half(&mut bus, 0x0800_00c6, 3);
    assert!(!bus.rumble_active());
    assert_eq!(0, read_half(&mut bus, 0x0800_00c4) & 8);
}

#[test]
fn tilt_cartridge_latches_two_axis_samples_through_byte_wide_sram_registers() {
    use evenfall_gba::cartridge::tilt::Sample;
    let mut bus = Bus::init_with_save(&[], SaveType::None);
    bus.enable_tilt(Sample { x: 0x392, y: 0x3a0 });
    assert_eq!(0xff, read_byte(&mut bus, 0x0e00_8200));
    assert_eq!(0x8f, read_byte(&mut bus, 0x0e00_8300));
    write_byte(&mut bus, 0x0e00_8000, 0x55);
    write_byte(&mut bus, 0x0e00_8100, 0xaa);
    let x_low = bus.read(access(
        0x0e00_8200,
        AccessWidth::Byte,
        AccessKind::Nonsequential,
        AccessSource::CpuData,
    ));
    assert_eq!(0x92, x_low.value);
    assert_eq!(5, x_low.cycles);
    assert_eq!(0x8383, read_half(&mut bus, 0x0e00_8300));
    assert_eq!(0xa0, read_byte(&mut bus, 0x0e00_8400));
    assert_eq!(3, read_byte(&mut bus, 0x0e00_8500));
    bus.set_tilt(Sample { x: 0x477, y: 0x2c3 });
    assert_eq!(0x92, read_byte(&mut bus, 0x0e00_8200));
    write_byte(&mut bus, 0x0e00_8100, 0xaa);
    assert_eq!(0x92, read_byte(&mut bus, 0x0e00_8200));
    write_byte(&mut bus, 0x0e00_8000, 0x55);
    write_byte(&mut bus, 0x0e00_8100, 0xaa);
    assert_eq!(0x77, read_byte(&mut bus, 0x0e00_8200));
    assert_eq!(0x84, read_byte(&mut bus, 0x0e00_8300));
    assert_eq!(0xc3, read_byte(&mut bus, 0x0e00_8400));
    assert_eq!(2, read_byte(&mut bus, 0x0e00_8500));
    assert_eq!(0xff, read_byte(&mut bus, 0x0e00_8600));
}

fn run_dma3(bus: &mut Bus, source: u32, destination: u32, count: u16, address_control: u16) {
    write_half(bus, 0x0400_00d4, source as u16);
    write_half(bus, 0x0400_00d6, (source >> 16) as u16);
    write_half(bus, 0x0400_00d8, destination as u16);
    write_half(bus, 0x0400_00da, (destination >> 16) as u16);
    write_half(bus, 0x0400_00dc, count);
    write_half(bus, 0x0400_00de, address_control | (1 << 15));
    bus.elapse(3);
}

fn write_serial_bit(bus: &mut Bus, index: &mut u32, bit: u16) {
    write_half(bus, 0x0200_1000 + *index * 2, bit & 1);
    *index += 1;
}

fn write_eeprom(bus: &mut Bus, block: u16, address_bits: u32, data: [u8; 8]) {
    let mut index = 0;
    write_serial_bit(bus, &mut index, 1);
    write_serial_bit(bus, &mut index, 0);
    for shift in (0..address_bits).rev() {
        write_serial_bit(bus, &mut index, (block >> shift) & 1);
    }
    for byte in data {
        for shift in (0..8).rev() {
            write_serial_bit(bus, &mut index, u16::from((byte >> shift) & 1));
        }
    }
    write_serial_bit(bus, &mut index, 0);
    run_dma3(bus, 0x0200_1000, 0x0d00_0000, index as u16, 2 << 5);
}

fn read_eeprom(bus: &mut Bus, block: u16, address_bits: u32) -> [u8; 8] {
    let mut index = 0;
    write_serial_bit(bus, &mut index, 1);
    write_serial_bit(bus, &mut index, 1);
    for shift in (0..address_bits).rev() {
        write_serial_bit(bus, &mut index, (block >> shift) & 1);
    }
    write_serial_bit(bus, &mut index, 0);
    run_dma3(bus, 0x0200_1000, 0x0d00_0000, index as u16, 2 << 5);
    run_dma3(bus, 0x0d00_0000, 0x0200_1100, 68, 2 << 7);
    for index in 0..4 {
        assert_eq!(0, read_half(bus, 0x0200_1100 + index * 2) & 1);
    }
    let mut data = [0; 8];
    for (byte_index, byte) in data.iter_mut().enumerate() {
        for bit in 0..8 {
            let address = 0x0200_1100 + ((4 + byte_index * 8 + bit) * 2) as u32;
            *byte = (*byte << 1) | read_half(bus, address) as u8;
        }
    }
    data
}

#[test]
fn eeprom_cpu_polling_reports_busy_until_the_write_deadline() {
    let mut bus = Bus::init_with_save(&[], SaveType::Eeprom512);
    write_eeprom(&mut bus, 5, 6, [0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef]);
    let written_at = bus.now();
    assert_eq!(0, read_half(&mut bus, 0x0d00_0000) & 1);
    bus.elapse(108_368 - (bus.now() - written_at));
    assert_eq!(1, read_half(&mut bus, 0x0d00_0000) & 1);
}

#[test]
fn runtime_eeprom_detection_uses_the_complete_dma3_command_length_and_14_bit_addresses() {
    let small_payload = [1, 3, 5, 7, 9, 11, 13, 15];
    let mut small = Bus::init_with_save(&[], SaveType::Auto);
    write_eeprom(&mut small, 0x12, 6, small_payload);
    assert_eq!(SaveType::Eeprom512, small.save_type());
    assert_eq!(512, small.save_data().len());
    assert_eq!(small_payload, read_eeprom(&mut small, 0x12, 6));
    let large_payload = [2, 4, 6, 8, 10, 12, 14, 16];
    let mut large = Bus::init_with_save(&[], SaveType::Auto);
    write_eeprom(&mut large, 0x234, 14, large_payload);
    assert_eq!(SaveType::Eeprom8k, large.save_type());
    assert_eq!(8 * 1024, large.save_data().len());
    assert_eq!(large_payload, read_eeprom(&mut large, 0x234, 14));
    assert_eq!([0xff; 8], read_eeprom(&mut large, 0x034, 14));
}
