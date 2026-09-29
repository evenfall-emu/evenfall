use evenfall_host::{
    AddressOverflow, PixelFormat, RunError, RunResult, StereoSample, Stop, System, Timed,
    VideoFormat,
};
use evenfall_replay::{
    Assets, BootMode, Config, Cpu, HostAction, HostCommand, Input, SaveType, Scenario, Trace,
    compare, run, run_with_host_actions,
};

struct Fake {
    now: u64,
    key: u16,
    next_sample: u64,
    audio: Vec<StereoSample>,
    pixels: Vec<u16>,
    save: [u8; 3],
}

impl Fake {
    fn new() -> Self {
        Self {
            now: 0,
            key: 0,
            next_sample: 7,
            audio: Vec::new(),
            pixels: vec![0; 240 * 160],
            save: [0, 0, 0],
        }
    }
}

impl System for Fake {
    type Input = Timed<u16>;
    type Fault = ();
    type StateError = ();
    type Pixel = u16;
    const MASTER_CLOCK_HZ: u64 = 16_777_216;
    const CYCLES_PER_FRAME: u64 = 14;
    const VIDEO: VideoFormat = VideoFormat { width: 240, height: 160, pixel: PixelFormat::Bgr555 };
    fn now(&self) -> u64 {
        self.now
    }
    fn run(
        &mut self,
        yield_at: u64,
        input_deadline: u64,
        inputs: &[Self::Input],
    ) -> Result<RunResult<()>, RunError> {
        let before = self.now;
        let end = yield_at.saturating_add(1);
        let mut consumed = 0;
        while self.next_sample <= end {
            while let Some(input) = inputs.get(consumed) {
                if input.at > self.next_sample.min(input_deadline) {
                    break;
                }
                if input.at < before {
                    return Err(RunError::InputBeforeCurrentCycle);
                }
                self.key = input.input;
                consumed += 1;
            }
            self.audio.push(StereoSample::new(self.key as i16, -(self.key as i16)));
            self.next_sample += 7;
        }
        while let Some(input) = inputs.get(consumed) {
            if input.at > end.min(input_deadline) {
                break;
            }
            if input.at < before {
                return Err(RunError::InputBeforeCurrentCycle);
            }
            self.key = input.input;
            consumed += 1;
        }
        self.save[0] = self.key as u8;
        self.now = end;
        self.pixels[7 * 240 + 19] = self.key;
        Ok(RunResult {
            started_at: before,
            stopped_at: end,
            reason: Stop::InstructionBoundaryAfterDeadline,
            inputs_consumed: consumed,
        })
    }
    fn fault(&self) -> Option<&()> {
        None
    }
    fn completed_frames(&self) -> u64 {
        self.now / 14
    }
    fn framebuffer(&self) -> &[u16] {
        &self.pixels
    }
    fn drain_audio(&mut self, output: &mut [StereoSample]) -> usize {
        let n = output.len().min(self.audio.len());
        output[..n].copy_from_slice(&self.audio[..n]);
        self.audio.drain(..n);
        n
    }
    fn audio_sample_rate(&self) -> u32 {
        32_768
    }
    fn save_media(&self) -> &[u8] {
        &self.save
    }
    fn write_state(&self) -> Result<Vec<u8>, ()> {
        Ok([self.now.to_le_bytes().as_slice(), &self.save].concat())
    }
    fn load_state(&mut self, _: &[u8]) -> Result<(), ()> {
        unimplemented!()
    }
    fn inspect(&self, _: u32, _: &mut [u8]) -> Result<(), AddressOverflow> {
        unimplemented!()
    }
}

fn scenario() -> Scenario<u16> {
    Scenario {
        name: "fake".into(),
        assets: Assets { rom: "fake.gba".into(), bios: None, save: None },
        config: Config {
            boot: BootMode::Hle,
            save_type: SaveType::None,
            rtc_epoch: None,
            solar_level: None,
            gyro_sample: None,
            tilt: None,
            rumble: false,
        },
        inputs: vec![
            Input { at: 5, change: 3 },
            Input { at: 13, change: 7 },
            Input { at: 25, change: 11 },
        ],
        host_actions: vec![],
        end_cycle: 35,
        checkpoints: vec![14, 28, 35],
        slice_cycles: vec![4, 17],
        terminal_pc: None,
        terminal_r12: None,
    }
}

fn cpu(machine: &Fake) -> Cpu {
    let mut registers = [0; 16];
    registers[3] = machine.key as u32;
    Cpu { registers, pc: machine.now as u32, cpsr: 0x1f }
}

#[test]
fn input_deadline_audio_and_trace_survive_different_slices() {
    // Wrong implementations that defer the input at cycle 13 past an
    // instruction crossing a slice boundary, or drain only at checkpoints,
    // disagree on this asymmetric input and 7-cycle audio cadence.
    let expected = run(&mut Fake::new(), &scenario(), 17, cpu).unwrap();
    let split = run(&mut Fake::new(), &scenario(), 4, cpu).unwrap();
    assert!(compare(&expected, &split).is_empty(), "{:?}", compare(&expected, &split));
    assert_eq!(split.0[0].cpu.registers[3], 7);
    assert_eq!(split.0[1].cpu.registers[3], 11);
    assert_eq!(split.0[0].audio.len(), 8);
    assert_eq!(split.0[1].save, [11, 0, 0]);
    let mut encoded = Vec::new();
    split.write(&mut encoded).unwrap();
    assert_eq!(Trace::read(encoded.as_slice()).unwrap(), split);
    encoded[180] ^= 1;
    assert!(Trace::read(encoded.as_slice()).is_err());
}

#[test]
fn compare_reports_first_location_per_kind() {
    let mut a = run(&mut Fake::new(), &scenario(), 4, cpu).unwrap();
    let mut b = a.clone();
    b.0[0].frame[(7 * 240 + 19) * 2] = 33;
    b.0[0].audio[4] = 19;
    b.0[0].save[2] = 1;
    b.0[0].state[9] = 1;
    b.0[1].save[2] = 2;
    let differences = compare(&a, &b);
    assert_eq!(differences.len(), 4);
    assert!(differences.iter().any(|d| d.kind == "pixel" && d.detail.contains("(19,7)")));
    assert!(
        differences.iter().any(|d| d.kind == "pixel" && d.detail.contains("Some(7) vs Some(33)"))
    );
    assert!(differences.iter().any(|d| d.kind == "audio" && d.detail.contains("sample 1")));
    assert!(differences.iter().any(|d| d.kind == "save" && d.detail.contains("byte 2")));
    assert!(differences.iter().any(|d| d.kind == "state" && d.detail.contains("byte 9")));
    a.0[0].frame[(7 * 240 + 19) * 2] = 33;
    assert_eq!(compare(&a, &b).len(), 3);
}

#[test]
fn host_master_action_is_applied_at_the_crossed_instruction_boundary() {
    let mut scenario = scenario();
    scenario.host_actions.push(HostAction { at: 13, command: HostCommand::JoybusWrite(42) });
    assert!(run(&mut Fake::new(), &scenario, 4, cpu).is_err());
    let mut crossed = Vec::new();
    let short = run_with_host_actions(&mut Fake::new(), &scenario, 4, cpu, |machine, command| {
        assert!(matches!(command, HostCommand::JoybusWrite(42)));
        crossed.push(machine.now);
        machine.save[2] = 42;
        Ok(())
    })
    .unwrap();
    let long = run_with_host_actions(&mut Fake::new(), &scenario, 17, cpu, |machine, _| {
        machine.save[2] = 42;
        Ok(())
    })
    .unwrap();
    assert_eq!(crossed, [14]);
    assert_eq!(short.0[0].save[2], 42);
    assert!(compare(&short, &long).is_empty());
}
