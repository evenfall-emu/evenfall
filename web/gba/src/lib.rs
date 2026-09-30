//! One-session, import-free WebAssembly ABI consumed by the private JS adapter.
//! Output pointers are borrowed until the next command; JS copies them at once.

// The exports need `#[unsafe(no_mangle)]`; the crate has no `unsafe` code.
#![allow(unsafe_code)]

use evenfall_audio::StereoSample;
use evenfall_gba::{
    bus::{KeyState, TimedInput, TimedInputChange},
    cartridge::save::Type as SaveType,
    machine::{StateError, StepOutcome, StopReason},
    session::{self, Config, RewindCompression, RewindConfig, Session},
    video_recovery,
};
use std::cell::RefCell;

const MAX_INPUT: usize = 68 * 1024 * 1024;
const FRAME_PIXELS: usize = 240 * 160;

struct State {
    session: Option<Session>,
    input: Vec<u8>,
    output: Vec<u8>,
    output_ptr: usize,
    output_len: usize,
    error: [u8; 1024],
    error_len: usize,
    cpu: [u32; 18],
    audio: [StereoSample; 8192],
    inputs_consumed: u32,
}

impl State {
    const fn new() -> Self {
        Self {
            session: None,
            input: Vec::new(),
            output: Vec::new(),
            output_ptr: 0,
            output_len: 0,
            error: [0; 1024],
            error_len: 0,
            cpu: [0; 18],
            audio: [StereoSample::new(0, 0); 8192],
            inputs_consumed: 0,
        }
    }

    fn clear_output(&mut self) {
        self.output.clear();
        self.output = Vec::new();
        self.output_ptr = 0;
        self.output_len = 0;
    }

    fn replace_output(&mut self, bytes: Vec<u8>) {
        self.clear_output();
        self.output_ptr = bytes.as_ptr() as usize;
        self.output_len = bytes.len();
        self.output = bytes;
    }

    fn borrow_output(&mut self, ptr: *const u8, len: usize) {
        self.clear_output();
        self.output_ptr = ptr as usize;
        self.output_len = len;
    }

    fn fail(&mut self, name: &str) -> i32 {
        self.error_len = name.len().min(self.error.len());
        self.error[..self.error_len].copy_from_slice(&name.as_bytes()[..self.error_len]);
        -1
    }
}

thread_local! {
    static STATE: RefCell<State> = const { RefCell::new(State::new()) };
}

fn call(f: impl FnOnce(&mut State) -> Result<i32, String>) -> i32 {
    STATE.with_borrow_mut(|state| match f(state) {
        Ok(result) => result,
        Err(name) => state.fail(&name),
    })
}

fn active(state: &mut State) -> Result<&mut Session, String> {
    state.session.as_mut().ok_or_else(|| "NoSession".into())
}

fn state_error(error: StateError) -> String {
    match error {
        StateError::Codec(error) => format!("{error:?}"),
        StateError::MachineFaulted => "MachineFaulted".into(),
        StateError::FramebufferNotCanonical => "FramebufferNotCanonical".into(),
        StateError::Io(_) => "OutOfMemory".into(),
    }
}

fn recovery_error(error: video_recovery::Error) -> String {
    match error {
        video_recovery::Error::Run(error) => format!("{error:?}"),
        other => format!("{other:?}"),
    }
}

fn session_error(error: session::Error) -> String {
    match error {
        session::Error::State(error) => state_error(error),
        session::Error::Run(error) => format!("{error:?}"),
        session::Error::Image(error) => format!("{error:?}"),
        session::Error::Rewind(error) => format!("{error:?}"),
        session::Error::Recovery(error) => recovery_error(error),
        other => format!("{other:?}"),
    }
}

fn keys(mask: u32) -> KeyState {
    KeyState {
        a: mask & 1 != 0,
        b: mask & 2 != 0,
        select: mask & 4 != 0,
        start: mask & 8 != 0,
        right: mask & 16 != 0,
        left: mask & 32 != 0,
        up: mask & 64 != 0,
        down: mask & 128 != 0,
        r: mask & 256 != 0,
        l: mask & 512 != 0,
    }
}

fn copy_upload(bytes: &[u8]) -> Result<Vec<u8>, &'static str> {
    let mut copy = Vec::new();
    copy.try_reserve_exact(bytes.len()).map_err(|_| "OutOfMemory")?;
    copy.extend_from_slice(bytes);
    Ok(copy)
}

#[unsafe(no_mangle)]
pub extern "C" fn ef_version() -> u32 {
    1
}

#[unsafe(no_mangle)]
pub extern "C" fn ef_error_ptr() -> u32 {
    STATE.with_borrow(|state| state.error.as_ptr() as u32)
}

#[unsafe(no_mangle)]
pub extern "C" fn ef_error_len() -> u32 {
    STATE.with_borrow(|state| state.error_len as u32)
}

#[unsafe(no_mangle)]
pub extern "C" fn ef_output_ptr() -> u32 {
    STATE.with_borrow(|state| state.output_ptr as u32)
}

#[unsafe(no_mangle)]
pub extern "C" fn ef_output_len() -> u32 {
    STATE.with_borrow(|state| state.output_len as u32)
}

#[unsafe(no_mangle)]
pub extern "C" fn ef_input_ptr() -> u32 {
    STATE.with_borrow(|state| state.input.as_ptr() as u32)
}

#[unsafe(no_mangle)]
pub extern "C" fn ef_prepare(size: u32) -> i32 {
    call(|state| {
        if size as usize > MAX_INPUT {
            return Err("InputTooLarge".into());
        }
        state.clear_output();
        if state.input.len() != size as usize {
            let mut next = Vec::new();
            next.try_reserve_exact(size as usize).map_err(|_| "OutOfMemory")?;
            next.resize(size as usize, 0);
            state.input = next;
        }
        Ok(0)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn ef_load(
    image_len: u32,
    bios_len: u32,
    save_len: u32,
    zipped: u32,
    save_type: i32,
    rtc_mode: i32,
    epoch: u64,
    sample_rate: u32,
) -> i32 {
    call(|state| {
        let image_len = image_len as usize;
        let bios_len = bios_len as usize;
        let save_len = save_len as usize;
        if image_len.checked_add(bios_len).and_then(|len| len.checked_add(save_len))
            != Some(state.input.len())
        {
            return Err("InvalidInputLength".into());
        }
        let types = [
            SaveType::Auto,
            SaveType::None,
            SaveType::Sram,
            SaveType::Flash64k,
            SaveType::Flash64kAtmel,
            SaveType::Flash128k,
            SaveType::Eeprom512,
            SaveType::Eeprom8k,
        ];
        if !(-1..=7).contains(&save_type) || !(-1..=1).contains(&rtc_mode) || zipped > 1 {
            return Err("InvalidOption".into());
        }
        let bios = copy_upload(&state.input[image_len..image_len + bios_len])?;
        let save = copy_upload(&state.input[image_len + bios_len..])?;
        let upload = std::mem::take(&mut state.input);
        let config = Config {
            zipped: zipped == 1,
            bios: &bios,
            save: (save_len != 0).then_some(save.as_slice()),
            save_type: (save_type != -1).then_some(types[save_type.max(0) as usize]),
            rtc_enabled: (rtc_mode != -1).then_some(rtc_mode == 1),
            rtc_epoch: epoch,
            output_rate: sample_rate,
            rewind: RewindConfig { budget_bytes: 0, ..RewindConfig::default() },
            ..Config::default()
        };
        let next = match Session::create_from_owned_buffer(config, upload, image_len) {
            Ok(session) => session,
            Err((error, upload)) => {
                state.input = upload;
                return Err(session_error(error));
            }
        };
        state.clear_output();
        state.session = Some(next);
        Ok(0)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn ef_dispose() {
    STATE.with_borrow_mut(|state| {
        state.session = None;
        state.input = Vec::new();
        state.clear_output();
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn ef_cycle() -> u64 {
    STATE.with_borrow(|state| state.session.as_ref().map_or(0, |s| s.machine.now()))
}

#[unsafe(no_mangle)]
pub extern "C" fn ef_frames() -> u64 {
    STATE.with_borrow(|state| state.session.as_ref().map_or(0, |s| s.machine.completed_frames()))
}

#[unsafe(no_mangle)]
pub extern "C" fn ef_rom_hash_ptr() -> u32 {
    STATE.with_borrow(|state| {
        state.session.as_ref().map_or(state.error.as_ptr() as u32, |s| s.rom_sha1.as_ptr() as u32)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn ef_rtc_enabled() -> u32 {
    STATE.with_borrow(|state| state.session.as_ref().is_some_and(|s| s.rtc_enabled) as u32)
}

#[unsafe(no_mangle)]
pub extern "C" fn ef_save_type() -> i32 {
    STATE.with_borrow(|state| state.session.as_ref().map_or(-1, |s| s.machine.save_type() as i32))
}

#[unsafe(no_mangle)]
pub extern "C" fn ef_inputs_consumed() -> u32 {
    STATE.with_borrow(|state| state.inputs_consumed)
}

#[unsafe(no_mangle)]
pub extern "C" fn ef_keys(mask: u32) -> i32 {
    call(|state| {
        let session = active(state)?;
        if mask > 1023 {
            return Err("InvalidKeys".into());
        }
        session.set_keys(keys(mask)).map_err(session_error)?;
        Ok(0)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn ef_run_until(deadline: u64, event_count: u32, input_deadline: u64) -> i32 {
    call(|state| {
        let session = state.session.as_mut().ok_or("NoSession")?;
        if deadline > session.machine.now() && deadline - session.machine.now() > 280_896 {
            return Err("ExecutionBudgetExceeded".into());
        }
        if event_count > 1024 || (event_count as usize) * 16 > state.input.len() {
            return Err("InvalidInputLength".into());
        }
        let mut events = Vec::new();
        events.try_reserve_exact(event_count as usize).map_err(|_| "OutOfMemory")?;
        for record in state.input.as_chunks::<16>().0.iter().take(event_count as usize) {
            let mask = u32::from_le_bytes(record[8..12].try_into().unwrap());
            let reserved = u32::from_le_bytes(record[12..16].try_into().unwrap());
            if mask > 1023 || reserved != 0 {
                return Err("InvalidKeys".into());
            }
            events.push(TimedInput {
                at: u64::from_le_bytes(record[..8].try_into().unwrap()),
                change: TimedInputChange::Keys(keys(mask)),
            });
        }
        let result = session
            .run_slice_with_input(deadline, input_deadline, &events)
            .map_err(session_error)?;
        state.inputs_consumed = result.inputs_consumed as u32;
        Ok(match result.reason {
            StopReason::Deadline => 0,
            StopReason::InstructionBoundaryAfterDeadline => 1,
            StopReason::PowerStopped => 2,
            StopReason::Faulted(_) => 3,
        })
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn ef_step() -> i32 {
    call(|state| {
        Ok(match active(state)?.step().map_err(session_error)? {
            StepOutcome::Executed(_) => 0,
            StepOutcome::Waiting(evenfall_gba::cpu::WaitReason::Halted) => 1,
            StepOutcome::Waiting(evenfall_gba::cpu::WaitReason::Stopped) => 2,
            StepOutcome::Faulted(_) => 3,
            StepOutcome::Waiting(evenfall_gba::cpu::WaitReason::DmaStall) => 4,
        })
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn ef_cpu_ptr() -> u32 {
    STATE.with_borrow_mut(|state| {
        state.cpu = [0; 18];
        if let Some(session) = &state.session {
            let snapshot = session.machine.inspect_cpu();
            state.cpu[..16].copy_from_slice(&snapshot.registers);
            state.cpu[16] = snapshot.cpsr;
            state.cpu[17] = snapshot.next_instruction_address;
        }
        state.cpu.as_ptr() as u32
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn ef_frame_ptr() -> u32 {
    STATE.with_borrow(|state| {
        state
            .session
            .as_ref()
            .map_or(state.cpu.as_ptr() as u32, |s| s.machine.framebuffer().as_ptr() as u32)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn ef_canonical_frame() -> i32 {
    call(|state| {
        active(state)?.canonical_framebuffer().map_err(session_error)?;
        Ok(0)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn ef_frame_canonical() -> u32 {
    STATE.with_borrow(|state| {
        state.session.as_ref().is_some_and(|s| s.machine.bus.is_framebuffer_canonical()) as u32
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn ef_present_frame_ptr() -> u32 {
    STATE.with_borrow(|state| {
        state.session.as_ref().map_or(state.cpu.as_ptr() as u32, |s| {
            s.machine.bus.published_framebuffer().as_ptr() as u32
        })
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn ef_video_frame_id() -> u64 {
    STATE.with_borrow(|state| state.session.as_ref().map_or(0, |s| s.machine.bus.video_frame_id))
}

#[unsafe(no_mangle)]
pub extern "C" fn ef_video_frame_cycle() -> u64 {
    STATE.with_borrow(|state| state.session.as_ref().map_or(0, |s| s.machine.bus.video_frame_cycle))
}

#[unsafe(no_mangle)]
pub extern "C" fn ef_video_interval(interval: u32) -> i32 {
    call(|state| {
        if !(1..=16).contains(&interval) {
            return Err("InvalidVideoInterval".into());
        }
        active(state)?.set_video_interval(interval as u16).map_err(session_error)?;
        Ok(0)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn ef_audio_ptr() -> u32 {
    STATE.with_borrow(|state| state.audio.as_ptr() as u32)
}

#[unsafe(no_mangle)]
pub extern "C" fn ef_drain_audio() -> i32 {
    call(|state| {
        let State { session, audio, .. } = state;
        Ok(session.as_mut().ok_or("NoSession")?.drain_audio(audio).map_err(session_error)? as i32)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn ef_audio_output(enabled: u32) -> i32 {
    call(|state| {
        let session = active(state)?;
        if enabled > 1 {
            return Err("InvalidAudioOutput".into());
        }
        session.set_audio_output(enabled == 1);
        Ok(0)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn ef_read_memory(address: u32, length: u32) -> i32 {
    call(|state| {
        let session = active(state)?;
        if length > 1024 * 1024 {
            return Err("ReadTooLarge".into());
        }
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(length as usize).map_err(|_| "OutOfMemory")?;
        bytes.resize(length as usize, 0);
        session.machine.inspect_memory(address, &mut bytes).map_err(|_| "AddressOverflow")?;
        state.replace_output(bytes);
        Ok(0)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn ef_export_rom() -> i32 {
    call(|state| {
        let (ptr, len) = {
            let rom = active(state)?.rom();
            (rom.as_ptr(), rom.len())
        };
        state.borrow_output(ptr, len);
        Ok(0)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn ef_export_save() -> i32 {
    call(|state| {
        let (ptr, len) = {
            let save = active(state)?.machine.save_data();
            (save.as_ptr(), save.len())
        };
        state.borrow_output(ptr, len);
        Ok(0)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn ef_write_state() -> i32 {
    call(|state| {
        state.clear_output();
        let bytes = active(state)?.write_state_fallible().map_err(session_error)?;
        state.replace_output(bytes);
        Ok(0)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn ef_load_state() -> i32 {
    call(|state| {
        let State { session, input, .. } = state;
        session.as_mut().ok_or("NoSession")?.load_state(input).map_err(session_error)?;
        *input = Vec::new();
        state.clear_output();
        Ok(0)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn ef_write_rewind_bundle() -> i32 {
    call(|state| {
        state.clear_output();
        let bytes = active(state)?.write_rewind_bundle_fallible().map_err(session_error)?;
        state.replace_output(bytes);
        Ok(0)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn ef_load_rewind_bundle() -> i32 {
    call(|state| {
        state.clear_output();
        let upload = std::mem::take(&mut state.input);
        let result = active(state)?.load_rewind_bundle_owned(upload).map_err(session_error);
        result?;
        Ok(0)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn ef_reset_save() -> i32 {
    call(|state| {
        let State { session, input, .. } = state;
        session.as_mut().ok_or("NoSession")?.reset_with_save(input).map_err(session_error)?;
        *input = Vec::new();
        state.clear_output();
        Ok(0)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn ef_fault() -> i32 {
    call(|state| {
        let fault = active(state)?.machine.fault();
        if let Some(fault) = fault {
            let mut bytes = Vec::new();
            fault.write_diagnostic(&mut bytes).map_err(|_| "OutOfMemory")?;
            state.replace_output(bytes);
            Ok(1)
        } else {
            Ok(0)
        }
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn ef_rewind_config(
    budget_bytes: u32,
    interval_frames: u32,
    compression: u32,
) -> i32 {
    call(|state| {
        if interval_frames == 0 || interval_frames > 65535 {
            return Err("InvalidRewindConfig".into());
        }
        let mode = match compression {
            0 => RewindCompression::Off,
            1 => RewindCompression::Fast,
            2 => RewindCompression::Delta,
            _ => return Err("InvalidRewindConfig".into()),
        };
        let config = RewindConfig {
            budget_bytes,
            interval_frames: interval_frames as u16,
            compression: mode,
            ..RewindConfig::default()
        };
        active(state)?.set_rewind_config(config).map_err(session_error)?;
        Ok(0)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn ef_rewind_speed(scale_q8: u32) -> i32 {
    call(|state| {
        let session = active(state)?;
        let scale = u16::try_from(scale_q8).map_err(|_| "InvalidRewindSpeed")?;
        session.set_rewind_speed_scale(scale).map_err(session_error)?;
        Ok(0)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn ef_rewind(steps: u32) -> i32 {
    call(|state| {
        let session = active(state)?;
        if steps == 0 || steps > 1024 {
            return Err("InvalidRewindSteps".into());
        }
        session.rewind(steps as usize).map_err(session_error)?;
        Ok(0)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn ef_rewind_depth() -> u32 {
    STATE.with_borrow(|state| {
        state.session.as_ref().map_or(0, |s| s.rewind_depth().min(u32::MAX as usize) as u32)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn ef_rewind_info(index: u32) -> i32 {
    call(|state| {
        let info = active(state)?.rewind_info(index as usize).ok_or("InvalidRewindSteps")?;
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(24).map_err(|_| "OutOfMemory")?;
        bytes.extend_from_slice(&info.frame.to_le_bytes());
        bytes.extend_from_slice(&info.cycle.to_le_bytes());
        bytes.extend_from_slice(&(info.bytes as u64).to_le_bytes());
        state.replace_output(bytes);
        Ok(0)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn ef_rewind_frame(index: u32) -> i32 {
    call(|state| {
        state.clear_output();
        let mut frame = vec![0u16; FRAME_PIXELS];
        active(state)?.rewind_framebuffer(index as usize, &mut frame).map_err(session_error)?;
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(FRAME_PIXELS * 2).map_err(|_| "OutOfMemory")?;
        for pixel in frame {
            bytes.extend_from_slice(&pixel.to_le_bytes());
        }
        state.replace_output(bytes);
        Ok(0)
    })
}
