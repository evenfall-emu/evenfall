use super::*;

fn state_bytes(apu: &Apu) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    apu.write_state(&mut Sink::new(Some(&mut bytes), None))?;
    Ok(bytes)
}

/// ZiGBA `Apu.writeStateSchema` and `Apu.writeState`, for default and active
/// square/noise/wave/FIFO state, including 13 cycles of deferred oscillator debt.
#[test]
fn apu_state_schema_and_payload_match_zigba() {
    let mut schema = Vec::new();
    Apu::write_state_schema(&mut Sink::new(Some(&mut schema), None)).unwrap();
    assert_eq!(schema.len(), 1726);
    assert_eq!(
        blake3::hash(&schema).to_hex().as_str(),
        "bf1a9650daa72882643f2fa37846b0a34488c95490ad5af8f5f9e2987432d349"
    );

    let mut apu = Apu::default();
    let default = state_bytes(&apu).unwrap();
    assert_eq!(default.len(), 247);
    assert_eq!(
        blake3::hash(&default).to_hex().as_str(),
        "d9bc79ca8d75d4c8f6c97584ddd0f0f06886776e5d08118e79e6a23df6594a8b"
    );
    for (address, value) in [
        (SOUND_CONTROL_X, 0x80),
        (SOUND_CONTROL_L, 0xff77),
        (SOUND_CONTROL_H, 0x730e),
        (SOUND1_CONTROL_H, 0xf040),
        (SOUND1_CONTROL_X, 0x87ff),
        (SOUND4_CONTROL_L, 0xd000),
        (SOUND4_CONTROL_H, 0x8000),
        (SOUND3_CONTROL_L, 0xa0),
        (SOUND3_CONTROL_H, 0x2000),
        (SOUND3_CONTROL_X, 0x87fd),
        (SOUND_BIAS, 0x8200),
    ] {
        assert!(apu.write_half(address, value));
    }
    for i in 0..16 {
        assert!(apu.write_byte(WAVE_RAM_BASE + i, (i * 17) as u8));
    }
    apu.write_fifo(FIFO_A_LOW, 0xa1b2_c3d4, 4);
    apu.timer_overflow(0, 1);
    apu.elapse(1037);
    let active = state_bytes(&apu).unwrap();
    assert_eq!(active.len(), 247);
    assert_eq!(
        blake3::hash(&active).to_hex().as_str(),
        "002eb627c63ddd56c2e891dadbfe2bdd774f2c337982d9408a8f5cc0eabb3dee"
    );
}

fn configure_active_square_noise(apu: &mut Apu) {
    assert!(apu.write_half(SOUND_CONTROL_X, 0x80));
    assert!(apu.write_half(SOUND_CONTROL_L, 0xff77));
    assert!(apu.write_half(SOUND_CONTROL_H, 2));
    assert!(apu.write_half(SOUND1_CONTROL_H, 0xf040));
    assert!(apu.write_half(SOUND1_CONTROL_X, 0x87ff));
    assert!(apu.write_half(SOUND2_CONTROL_L, 0xf07f));
    assert!(apu.write_half(SOUND2_CONTROL_H, 0xc7e1));
    assert!(apu.write_half(SOUND4_CONTROL_L, 0xd000));
    assert!(apu.write_half(SOUND4_CONTROL_H, 0x8000));
}

// "elapseEagerForTest": independent per-sample and per-frame reference.
fn elapse_eager(apu: &mut Apu, cycles: u64) {
    assert_eq!(apu.pending_square_noise_cycles, 0);
    let interval = apu.sample_interval();
    let mut remaining = cycles;
    while remaining > 0 {
        let until_frame = if apu.master_enabled { 32_768 - apu.frame_phase } else { remaining };
        let step = remaining.min(interval - apu.sample_phase).min(until_frame);
        if apu.square1.enabled {
            apu.square1.elapse_enabled(step);
        }
        if apu.square2.enabled {
            apu.square2.elapse_enabled(step);
        }
        if apu.wave.enabled {
            apu.wave.elapse_enabled(step);
        }
        if apu.noise.enabled {
            apu.noise.elapse_enabled(step);
        }
        apu.sample_phase += step;
        if apu.master_enabled {
            apu.frame_phase += step;
        }
        remaining -= step;
        if apu.sample_phase == interval {
            apu.sample_phase = 0;
            apu.push_sample(apu.current_sample());
        }
        if apu.master_enabled && apu.frame_phase == 32_768 {
            apu.frame_phase = 0;
            apu.clock_frame_sequencer();
        }
    }
}

fn assert_logical_equal(eager: &Apu, deferred: &Apu) {
    let mut materialized = deferred.clone();
    materialized.flush_square_noise();
    assert_eq!(eager, &materialized);
}

/// "silent output preserves exact APU states and resumed PCM"
#[test]
fn silent_output_preserves_exact_apu_states_and_resumed_pcm() {
    let mut audible = Apu::default();
    configure_active_square_noise(&mut audible);
    assert!(audible.write_half(SOUND3_CONTROL_L, 0x80));
    assert!(audible.write_half(SOUND3_CONTROL_H, 0x2000));
    assert!(audible.write_half(SOUND3_CONTROL_X, 0x87fd));
    assert!(audible.write_half(SOUND_CONTROL_H, 0x730e));
    for i in 0..16 {
        assert!(audible.write_byte(WAVE_RAM_BASE + i, (i * 17) as u8));
    }
    audible.elapse(37);
    let mut silent = audible.clone();
    silent.set_sample_output(false);
    let mut samples = [StereoSample::default(); SAMPLE_QUEUE_CAPACITY];
    let chunks = [1, 511, 3, 32767, 65537, 77, 1021];
    for i in 0u32..70 {
        let cycles = chunks[i as usize % chunks.len()];
        audible.flush_square_noise();
        elapse_eager(&mut audible, cycles);
        silent.elapse(cycles);
        let _ = audible.drain_samples(&mut samples);
        assert_eq!(silent.drain_samples(&mut samples), 0);
        let value = i.wrapping_mul(0x1234567);
        audible.write_fifo(FIFO_A_LOW, value, 4);
        silent.write_fifo(FIFO_A_LOW, value, 4);
        assert_eq!(
            audible.timer_overflow(i as usize % 2, 3),
            silent.timer_overflow(i as usize % 2, 3)
        );
        if i % 5 == 0 {
            let bias = ((i % 4) as u16) << 14 | 0x200;
            assert!(audible.write_half(SOUND_BIAS, bias));
            assert!(silent.write_half(SOUND_BIAS, bias));
            assert!(audible.write_half(SOUND1_CONTROL_X, 0x87f0));
            assert!(silent.write_half(SOUND1_CONTROL_X, 0x87f0));
        }
        // The queues and host policy differ; the circuit and canonical bytes must not.
        assert_eq!(state_bytes(&audible).unwrap(), state_bytes(&silent).unwrap());
    }
    audible.set_sample_output(false);
    audible.set_sample_output(true);
    silent.set_sample_output(true);
    audible.elapse(65537);
    silent.elapse(65537);
    let count = audible.drain_samples(&mut samples);
    assert!(count > 0);
    let mut resumed = [StereoSample::default(); SAMPLE_QUEUE_CAPACITY];
    assert_eq!(count, silent.drain_samples(&mut resumed));
    assert_eq!(samples[..count], resumed[..count]);
}

/// "refill prediction counts overflows through the first request in every queue phase"
#[test]
fn refill_prediction_counts_overflows_through_the_first_request_in_every_queue_phase() {
    for words in 0..9 {
        for bytes in 0..5 {
            for routing in [0x0300, 0x3000, 0x7300, 0x4300, 0x0700] {
                let mut apu = Apu::default();
                assert!(apu.write_half(SOUND_CONTROL_X, 0x80));
                assert!(apu.write_half(SOUND_CONTROL_H, routing));
                for i in 0..words {
                    apu.write_fifo(FIFO_A_LOW, i + 1, 4);
                    apu.write_fifo(FIFO_B_LOW, i + 1, 4);
                }
                apu.fifo_a.playback_bytes_remaining = bytes as u8;
                apu.fifo_b.playback_bytes_remaining = ((bytes + 3) % 5) as u8;
                for timer in 0..2 {
                    let predicted = apu.overflows_until_refill(timer);
                    let mut consumed = apu.clone();
                    let actual = (1..=64).find(|_| consumed.timer_overflow(timer, 1) != 0);
                    assert_eq!(predicted, actual);
                }
            }
        }
    }
    let mut disabled = Apu::default();
    assert!(disabled.write_half(SOUND_CONTROL_H, 0x3300));
    assert_eq!(disabled.overflows_until_refill(0), None);
}

/// "clocked catch-up consumes each overflow after the samples and frame step at its cycle"
#[test]
fn clocked_catch_up_consumes_each_overflow_after_the_samples_and_frame_step_at_its_cycle() {
    let mut explicit = Apu::default();
    configure_active_square_noise(&mut explicit);
    assert!(explicit.write_half(SOUND_CONTROL_H, 0x730e));
    for i in 0u32..7 {
        explicit.write_fifo(FIFO_A_LOW, i.wrapping_mul(0x1357_9bdf), 4);
        explicit.write_fifo(FIFO_B_LOW, i.wrapping_mul(0x2468_ace1), 4);
    }
    let mut clocked = explicit.clone();
    let mut clock = FifoClock { next: [256, 32_768], period: [256, 1000] };
    let mut next = clock.next;
    let mut now = 0;
    for (i, cycles) in [255, 1, 256, 32_000, 257, 3000, 0, 511, 70_001].into_iter().enumerate() {
        let target = now + cycles;
        let mut expected_requests = 0;
        while next[0].min(next[1]) <= target {
            let at = next[0].min(next[1]);
            elapse_eager(&mut explicit, at - now);
            now = at;
            for (index, deadline) in next.iter_mut().enumerate() {
                if *deadline != at {
                    continue;
                }
                expected_requests |= explicit.timer_overflow(index, 1);
                *deadline += clock.period[index];
            }
        }
        elapse_eager(&mut explicit, target - now);
        let requests = clocked.elapse_clocked(target - cycles, target, &mut clock);
        now = target;
        assert_eq!(expected_requests, requests);
        assert_eq!(next, clock.next);
        assert_logical_equal(&explicit, &clocked);
        let value = (i as u32).wrapping_mul(0x9e37_79b9);
        for apu in [&mut explicit, &mut clocked] {
            apu.write_fifo(FIFO_A_LOW, value, 4);
            apu.write_fifo(FIFO_B_LOW, !value, 4);
        }
    }
}

/// "bulk sample pushes match single pushes through queue wraparound"
#[test]
fn bulk_sample_pushes_match_single_pushes_through_queue_wraparound() {
    let counts = [0, 1, 5, 2047, 2048, 2049, 6151];
    let starts = [(0, 0), (100, 37), (2045, 2047), (2048, 5)];
    for (start, drained_count) in starts {
        for count in counts {
            let mut single = Apu::default();
            for i in 0..start {
                single.push_sample(StereoSample::new((i % 300) as i16, -3));
            }
            let mut drained = [StereoSample::default(); 64];
            let n = (drained_count % 64).min(single.sample_count);
            single.drain_samples(&mut drained[..n]);
            let mut bulk = single.clone();
            for _ in 0..count {
                single.push_sample(StereoSample::new(7, -9));
            }
            bulk.push_samples(StereoSample::new(7, -9), count);
            assert_eq!(single, bulk);
        }
    }
}

/// "constant sample runs match the eager scheduler across silent and audible PSG"
#[test]
fn constant_sample_runs_match_the_eager_scheduler_across_silent_and_audible_psg() {
    let mut eager = Apu::default();
    for (address, value) in [
        (SOUND_CONTROL_X, 0x80),
        (SOUND_CONTROL_L, 0xff77),
        (SOUND_CONTROL_H, 0x330e),
        (SOUND1_CONTROL_H, 0x0980),
        (SOUND1_CONTROL_X, 0x87c0),
        (SOUND2_CONTROL_L, 0x307f),
        (SOUND2_CONTROL_H, 0xc700),
        (SOUND4_CONTROL_L, 0x0100),
        (SOUND4_CONTROL_H, 0x8012),
    ] {
        assert!(eager.write_half(address, value));
    }
    let mut deferred = eager.clone();
    let mut expected = [StereoSample::default(); SAMPLE_QUEUE_CAPACITY];
    let mut actual = expected;
    let chunks = [1, 511, 3, 32767, 65537, 77, 1021, 262145, 924, 0];
    for i in 0u32..90 {
        let cycles = chunks[i as usize % chunks.len()];
        elapse_eager(&mut eager, cycles);
        deferred.elapse(cycles);
        let value = i.wrapping_mul(0x9e37_79b9);
        for apu in [&mut eager, &mut deferred] {
            apu.write_fifo(FIFO_A_LOW, value, 4);
            apu.write_fifo(FIFO_B_LOW, !value, 4);
        }
        assert_eq!(
            eager.timer_overflow(i as usize % 2, 1 + u64::from(i % 3)),
            deferred.timer_overflow(i as usize % 2, 1 + u64::from(i % 3))
        );
        if i % 7 == 3 {
            let bias = ((i % 4) as u16) << 14 | 0x200;
            assert!(eager.write_half(SOUND_BIAS, bias));
            assert!(deferred.write_half(SOUND_BIAS, bias));
        }
        if i % 11 == 5 {
            assert!(eager.write_half(SOUND1_CONTROL_X, 0x87c0));
            assert!(deferred.write_half(SOUND1_CONTROL_X, 0x87c0));
        }
        assert_logical_equal(&eager, &deferred);
        if i % 4 == 0 {
            let count = eager.drain_samples(&mut expected);
            assert_eq!(count, deferred.drain_samples(&mut actual));
            assert_eq!(expected[..count], actual[..count]);
        }
    }
}

/// "high-speed audio preserves all APU state except square and noise oscillators"
#[test]
fn high_speed_audio_preserves_all_apu_state_except_square_and_noise_oscillators() {
    let mut exact = Apu::default();
    configure_active_square_noise(&mut exact);
    for (address, value) in [
        (SOUND1_CONTROL_L, 0x21),
        (SOUND1_CONTROL_X, 0x81ff),
        (SOUND4_CONTROL_L, 0xd900),
        (SOUND3_CONTROL_L, 0xa0),
        (SOUND3_CONTROL_H, 0x2000),
        (SOUND3_CONTROL_X, 0x87fd),
        (SOUND_CONTROL_H, 0x730e),
    ] {
        assert!(exact.write_half(address, value));
    }
    for i in 0..16 {
        assert!(exact.write_byte(WAVE_RAM_BASE + i, (i * 17) as u8));
    }
    exact.set_sample_output(false);
    exact.elapse(37);
    exact.flush_square_noise();
    let mut fast = exact.clone();
    fast.set_high_speed_audio(true);
    let chunks = [1, 511, 3, 32767, 65537, 77, 262145];
    for i in 0u32..28 {
        let before = fast.clone();
        exact.elapse(chunks[i as usize % chunks.len()]);
        fast.elapse(chunks[i as usize % chunks.len()]);
        assert_eq!(
            (
                before.square1.phase_index,
                before.square1.phase_cycles,
                before.square2.phase_index,
                before.square2.phase_cycles,
                before.noise.phase_cycles,
                before.noise.lfsr,
                before.noise.output_high
            ),
            (
                fast.square1.phase_index,
                fast.square1.phase_cycles,
                fast.square2.phase_index,
                fast.square2.phase_cycles,
                fast.noise.phase_cycles,
                fast.noise.lfsr,
                fast.noise.output_high
            )
        );
        let value = i.wrapping_mul(0x1234567);
        for apu in [&mut exact, &mut fast] {
            apu.write_fifo(FIFO_A_LOW, value, 4);
            apu.write_fifo(FIFO_B_LOW, !value, 4);
        }
        assert_eq!(exact.timer_overflow(i as usize % 2, 5), fast.timer_overflow(i as usize % 2, 5));
        let bias = ((i % 4) as u16) << 14 | 0x200;
        for apu in [&mut exact, &mut fast] {
            assert!(apu.write_half(SOUND_BIAS, bias));
        }
        if i % 5 == 0 {
            for apu in [&mut exact, &mut fast] {
                assert!(apu.write_half(SOUND4_CONTROL_H, 0x8008));
            }
        }
        exact.flush_square_noise();
        let mut normalized = exact.clone();
        normalized.high_speed_audio_enabled = true;
        normalized.square1.phase_index = fast.square1.phase_index;
        normalized.square1.phase_cycles = fast.square1.phase_cycles;
        normalized.square2.phase_index = fast.square2.phase_index;
        normalized.square2.phase_cycles = fast.square2.phase_cycles;
        normalized.noise.phase_cycles = fast.noise.phase_cycles;
        normalized.noise.lfsr = fast.noise.lfsr;
        normalized.noise.output_high = fast.noise.output_high;
        assert_eq!(normalized, fast);
    }
}

/// "high-speed audio transitions settle prior debt and resume without catch-up"
#[test]
fn high_speed_audio_transitions_settle_prior_debt_and_resume_without_catch_up() {
    let mut apu = Apu::default();
    configure_active_square_noise(&mut apu);
    apu.elapse(513);
    assert!(apu.sample_count > 0);
    assert!(apu.pending_square_noise_cycles > 0);
    let at_entry = apu.materialized_square_noise();
    apu.set_high_speed_audio(true);
    assert_eq!(at_entry, apu.materialized_square_noise());
    assert_eq!(apu.pending_square_noise_cycles, 0);
    assert_eq!(apu.sample_count, 0);
    apu.elapse(123_457);
    assert_eq!(apu.current_sample(), StereoSample::default());
    assert_eq!(apu.sample_count, 0);
    apu.set_sample_output(false);
    apu.set_sample_output(true);
    apu.elapse(512);
    assert_eq!(apu.sample_count, 0);
    apu.set_sample_output(false);
    let at_exit = apu.materialized_square_noise();
    let sample_phase = apu.sample_phase;
    let frame_phase = apu.frame_phase;
    apu.set_high_speed_audio(false);
    assert!(!apu.sample_output_enabled);
    assert_eq!(at_exit, apu.materialized_square_noise());
    assert_eq!((sample_phase, frame_phase), (apu.sample_phase, apu.frame_phase));
    apu.elapse(512);
    assert_eq!(apu.sample_count, 0);
    apu.set_sample_output(true);
    let mut eager = apu.clone();
    elapse_eager(&mut eager, 65_537);
    apu.elapse(65_537);
    assert_logical_equal(&eager, &apu);
}

#[derive(Clone, Copy)]
enum InvalidStateCase {
    SampleDeadline,
    FrameDeadline,
    SweepCounterZero,
    SweepCounterAbovePeriod,
    FifoATooManyPlaybackBytes,
    FifoBTooManyPlaybackBytes,
}

fn invalid_state(case: InvalidStateCase) -> Apu {
    let mut apu = Apu::default();
    match case {
        InvalidStateCase::SampleDeadline => apu.sample_phase = 512,
        InvalidStateCase::FrameDeadline => apu.frame_phase = 32_768,
        InvalidStateCase::SweepCounterZero => {
            apu.sweep1.enabled = true;
            apu.sweep1.counter = 0;
        }
        InvalidStateCase::SweepCounterAbovePeriod => {
            apu.sweep1.enabled = true;
            apu.sweep1.counter = 9;
        }
        InvalidStateCase::FifoATooManyPlaybackBytes => apu.fifo_a.playback_bytes_remaining = 5,
        InvalidStateCase::FifoBTooManyPlaybackBytes => apu.fifo_b.playback_bytes_remaining = 5,
    }
    apu
}

const INVALID_CASES: [InvalidStateCase; 6] = [
    InvalidStateCase::SampleDeadline,
    InvalidStateCase::FrameDeadline,
    InvalidStateCase::SweepCounterZero,
    InvalidStateCase::SweepCounterAbovePeriod,
    InvalidStateCase::FifoATooManyPlaybackBytes,
    InvalidStateCase::FifoBTooManyPlaybackBytes,
];

/// "APU state writer rejects unreachable counters and deferred debt past the frame phase"
#[test]
fn apu_state_writer_rejects_unreachable_counters_and_deferred_debt_past_the_frame_phase() {
    for case in INVALID_CASES {
        let malformed = invalid_state(case);
        assert_eq!(malformed.write_state(&mut Sink::new(None, None)), Err(Error::InvalidSaveState));
    }
    let indebted = Apu { pending_square_noise_cycles: 1, ..Apu::default() };
    assert_eq!(indebted.write_state(&mut Sink::new(None, None)), Err(Error::InvalidSaveState));
}

/// "APU state reader rejects counters before timing or playback can advance"
#[test]
fn apu_state_reader_rejects_counters_before_timing_or_playback_can_advance() {
    for case in INVALID_CASES {
        let malformed = invalid_state(case);
        // Bypass the module writer's semantic validation, as ZiGBA does.
        let mut bytes = Vec::new();
        malformed.encode(&mut Sink::new(Some(&mut bytes), None)).unwrap();
        let mut restored = Apu::default();
        assert_eq!(restored.read_state(&mut Source::new(&bytes)), Err(Error::InvalidSaveState));
    }
}

/// "deferred square and noise oscillators match eager endpoint ordering"
#[test]
fn deferred_square_and_noise_oscillators_match_eager_endpoint_ordering() {
    let mut eager = Apu::default();
    configure_active_square_noise(&mut eager);
    let mut deferred = eager.clone();
    eager.sample_phase = 504;
    eager.frame_phase = 32_760;
    deferred.sample_phase = eager.sample_phase;
    deferred.frame_phase = eager.frame_phase;
    for cycles in [1, 2, 4] {
        elapse_eager(&mut eager, cycles);
        deferred.elapse(cycles);
    }
    assert_eq!(deferred.pending_square_noise_cycles, 7);
    let pending = deferred.pending_square_noise_cycles;
    assert_eq!(eager.current_sample(), deferred.current_sample());
    assert_eq!(pending, deferred.pending_square_noise_cycles);
    assert_logical_equal(&eager, &deferred);
    elapse_eager(&mut eager, 1);
    deferred.elapse(1);
    assert_eq!(deferred.pending_square_noise_cycles, 0);
    assert!(!deferred.square2.enabled);
    assert_ne!(deferred.samples[deferred.sample_read_index].left, 0);
    assert_logical_equal(&eager, &deferred);
    elapse_eager(&mut eager, 37);
    deferred.elapse(37);
    assert_eq!(deferred.pending_square_noise_cycles, 37);
    assert!(eager.write_byte(SOUND1_CONTROL_X, 0xe0));
    assert!(deferred.write_byte(SOUND1_CONTROL_X, 0xe0));
    assert_eq!(deferred.pending_square_noise_cycles, 0);
    assert_logical_equal(&eager, &deferred);
    elapse_eager(&mut eager, 31);
    deferred.elapse(31);
    for (address, value) in [(SOUND4_CONTROL_H, 8), (SOUND4_CONTROL_H + 1, 0x80)] {
        assert!(eager.write_byte(address, value));
        assert!(deferred.write_byte(address, value));
    }
    assert_logical_equal(&eager, &deferred);
    let mut remaining = 2 * 32_768 + 777;
    let chunks = [1, 511, 3, 257, 64, 9, 1021];
    let mut i = 0;
    while remaining > 0 {
        let cycles = remaining.min(chunks[i % chunks.len()]);
        elapse_eager(&mut eager, cycles);
        deferred.elapse(cycles);
        remaining -= cycles;
        i += 1;
    }
    assert_eq!(eager.current_sample(), deferred.current_sample());
    assert_logical_equal(&eager, &deferred);
}

/// "APU state round trip canonicalizes nonzero deferred oscillator debt"
#[test]
fn apu_state_round_trip_canonicalizes_nonzero_deferred_oscillator_debt() {
    let mut original = Apu::default();
    configure_active_square_noise(&mut original);
    original.elapse(37);
    assert_eq!(original.pending_square_noise_cycles, 37);
    let mut materialized = original.clone();
    materialized.flush_square_noise();
    let bytes = state_bytes(&original).unwrap();
    assert_eq!(bytes, state_bytes(&materialized).unwrap());
    let mut restored = Apu::default();
    restored.read_state(&mut Source::new(&bytes)).unwrap();
    assert_eq!(restored.pending_square_noise_cycles, 0);
    assert_eq!(materialized, restored);
    original.elapse(2 * 512 + 29);
    restored.elapse(2 * 512 + 29);
    assert_eq!(original, restored);
}

/// "PSG length enable and trigger apply the frame-phase extra clock"
#[test]
fn psg_length_enable_and_trigger_apply_the_frame_phase_extra_clock() {
    let mut apu = Apu::default();
    assert!(apu.write_half(SOUND_CONTROL_X, 0x80));
    apu.elapse(32_768);
    assert_eq!(apu.frame_step, 0);
    assert!(apu.write_half(SOUND1_CONTROL_H, 0xf03f));
    assert!(apu.write_half(SOUND1_CONTROL_X, 0xc000));
    assert_eq!(apu.square1.length, 63);
    assert!(apu.square1.enabled);
    assert!(apu.write_half(SOUND2_CONTROL_L, 0xf03f));
    assert!(apu.write_byte(SOUND2_CONTROL_H + 1, 0xc0));
    assert_eq!(apu.square2.length, 63);
    assert!(apu.square2.enabled);
    assert!(apu.write_half(SOUND3_CONTROL_L, 0x80));
    assert!(apu.write_half(SOUND3_CONTROL_H, 0x20ff));
    assert!(apu.write_half(SOUND3_CONTROL_X, 0xc000));
    assert_eq!(apu.wave.length, 255);
    assert!(apu.wave.enabled);
    assert!(apu.write_half(SOUND4_CONTROL_L, 0xf03f));
    assert!(apu.write_half(SOUND4_CONTROL_H, 0xc000));
    assert_eq!(apu.noise.length, 63);
    assert!(apu.noise.enabled);
}

/// "PSG length unit keeps clocking after channel output stops"
#[test]
fn psg_length_unit_keeps_clocking_after_channel_output_stops() {
    let mut apu = Apu::default();
    assert!(apu.write_half(SOUND_CONTROL_X, 0x80));
    assert!(apu.write_half(SOUND2_CONTROL_L, 0xf03f));
    assert!(apu.write_half(SOUND2_CONTROL_H, 0xc000));
    assert_eq!(apu.square2.length, 1);
    assert!(apu.write_byte(SOUND2_CONTROL_L + 1, 0));
    assert!(!apu.square2.enabled);
    apu.elapse(32_768);
    assert_eq!(apu.square2.length, 0);
}

fn naive_noise_elapse(channel: &mut NoiseChannel, cycles: u64) {
    let divisor_factor = if channel.divisor == 0 { 1 } else { 2 * u64::from(channel.divisor) };
    let period = (divisor_factor << channel.clock_shift) * 32;
    let accumulated = channel.phase_cycles + cycles;
    if accumulated < period {
        channel.phase_cycles = accumulated;
        return;
    }
    let mut steps = if accumulated < 2 * period {
        channel.phase_cycles = accumulated - period;
        1
    } else {
        channel.phase_cycles = accumulated % period;
        accumulated / period
    };
    let feedback = if channel.narrow { 0x60 } else { 0x6000 };
    while steps > 0 {
        let low = channel.lfsr & 1 != 0;
        channel.lfsr >>= 1;
        if low {
            channel.lfsr ^= feedback;
        }
        channel.output_high = low;
        steps -= 1;
    }
}

// Fixed deterministic PRNG for the same randomized domains as ZiGBA's test.
fn random(seed: &mut u64) -> u64 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 7;
    *seed ^= *seed << 17;
    *seed
}

/// "noise jump table matches per-step LFSR advance"
#[test]
fn noise_jump_table_matches_per_step_lfsr_advance() {
    let mut seed = 0x5eed;
    for step in 0..500 {
        let mut reference = NoiseChannel {
            divisor: (random(&mut seed) as u8) & 7,
            clock_shift: (random(&mut seed) as u8) & 15,
            narrow: random(&mut seed) & 1 != 0,
            lfsr: (random(&mut seed) as u16) & 0x7fff,
            output_high: random(&mut seed) & 1 != 0,
            phase_cycles: (random(&mut seed) as u32 % 64) as u64,
            ..NoiseChannel::default()
        };
        let mut actual = reference;
        let cycles = match step % 4 {
            0 => 1 + random(&mut seed) as u8 as u64 % 31,
            1 => 32,
            2 => 8 * 32,
            _ => 1 + random(&mut seed) as u16 as u64 % 5000,
        };
        naive_noise_elapse(&mut reference, cycles);
        actual.elapse_enabled(cycles);
        assert_eq!(reference, actual);
    }
}

/// "idle PSG oscillator fast path preserves device clocks and samples"
#[test]
fn idle_psg_oscillator_fast_path_preserves_device_clocks_and_samples() {
    let mut apu = Apu::default();
    assert!(apu.write_half(SOUND_CONTROL_X, 0x80));
    apu.square1.phase_cycles = 11;
    apu.square1.length = 1;
    apu.square1.length_enabled = true;
    apu.square2.phase_cycles = 22;
    apu.square2.length = 1;
    apu.square2.length_enabled = true;
    apu.wave.phase_cycles = 33;
    apu.wave.length = 1;
    apu.wave.length_enabled = true;
    apu.noise.phase_cycles = 44;
    apu.noise.length = 1;
    apu.noise.length_enabled = true;
    apu.elapse(32_768);
    assert_eq!(
        (
            apu.square1.phase_cycles,
            apu.square2.phase_cycles,
            apu.wave.phase_cycles,
            apu.noise.phase_cycles
        ),
        (11, 22, 33, 44)
    );
    assert_eq!(
        (apu.square1.length, apu.square2.length, apu.wave.length, apu.noise.length),
        (0, 0, 0, 0)
    );
    assert_eq!(apu.frame_step, 0);
    assert_eq!(apu.sample_count, 64);
    let mut samples = [StereoSample::default(); 64];
    assert_eq!(apu.drain_samples(&mut samples), samples.len());
    assert!(samples.iter().all(|&s| s == StereoSample::default()));
}

/// "silent square and noise channels stay deferred across samples"
#[test]
fn silent_square_and_noise_channels_stay_deferred_across_samples() {
    let mut eager = Apu::default();
    for (address, value) in [
        (SOUND_CONTROL_X, 0x80),
        (SOUND_CONTROL_L, 0xff77),
        (SOUND_CONTROL_H, 2),
        (SOUND1_CONTROL_H, 0x0980),
        (SOUND1_CONTROL_X, 0x8700),
        (SOUND4_CONTROL_L, 0x0800),
        (SOUND4_CONTROL_H, 0x8000),
    ] {
        assert!(eager.write_half(address, value));
    }
    assert!(eager.square1.enabled && eager.noise.enabled);
    let mut deferred = eager.clone();
    let mut expected = [StereoSample::default(); SAMPLE_QUEUE_CAPACITY];
    let mut actual = expected;
    let mut saw_deferred_samples = false;
    let mut saw_audible_square = false;
    let chunks = [1, 511, 3, 1024, 257, 4096, 9, 1021];
    let mut total = 0;
    let mut index = 0;
    while total < 12 * 32_768 {
        let cycles = chunks[index % chunks.len()];
        elapse_eager(&mut eager, cycles);
        deferred.elapse(cycles);
        total += cycles;
        if deferred.pending_square_noise_cycles > deferred.sample_interval() {
            saw_deferred_samples = true;
        }
        if eager.square1.current_volume != 0 {
            saw_audible_square = true;
        }
        let count = eager.drain_samples(&mut expected);
        assert_eq!(count, deferred.drain_samples(&mut actual));
        assert_eq!(expected[..count], actual[..count]);
        assert_logical_equal(&eager, &deferred);
        if index == 20 {
            assert!(eager.write_byte(SOUND4_CONTROL_H, 0x11));
            assert!(deferred.write_byte(SOUND4_CONTROL_H, 0x11));
        }
        index += 1;
    }
    assert!(saw_deferred_samples && saw_audible_square);
}

/// "LFSR jump tables match per-step advances for every state"
#[test]
fn lfsr_jump_tables_match_per_step_advances_for_every_state() {
    for (feedback, tables) in [(0x6000, &NOISE_JUMP_WIDE), (0x0060, &NOISE_JUMP_NARROW)] {
        for state in 0..=0xffffu32 {
            let mut expected = state as u16;
            for steps in 1..=8 {
                expected = lfsr_advance_combined(expected, feedback);
                assert_eq!(expected, lfsr_jump(state as u16, tables, steps));
            }
        }
    }
}

// "applyBias": the un-decoded hardware-width quantization reference.
fn apply_bias_reference(apu: &Apu, mixed: i16) -> i16 {
    let bias = i32::from(apu.sound_bias & 0x3ff);
    let clipped = (i32::from(mixed) + bias).clamp(0, 0x3ff);
    let discarded_bits = u32::from(apu.sound_bias >> 14) + 1;
    let mask = !((1i32 << discarded_bits) - 1);
    ((clipped & mask) - (bias & mask)) as i16
}

// "currentSampleReferenceForTest": per-channel reference mixer.
fn current_sample_reference(apu: &Apu) -> StereoSample {
    if !apu.master_enabled || apu.high_speed_audio_enabled {
        return StereoSample::default();
    }
    let (square1, square2, noise) = apu.materialized_square_noise();
    let mut sample = StereoSample::default();
    let scaled_a = i16::from(apu.sample_a) * if apu.sound_control_h & 4 != 0 { 4 } else { 2 };
    let scaled_b = i16::from(apu.sample_b) * if apu.sound_control_h & 8 != 0 { 4 } else { 2 };
    if apu.sound_control_h & 0x200 != 0 {
        sample.left += scaled_a;
    }
    if apu.sound_control_h & 0x100 != 0 {
        sample.right += scaled_a;
    }
    if apu.sound_control_h & 0x2000 != 0 {
        sample.left += scaled_b;
    }
    if apu.sound_control_h & 0x1000 != 0 {
        sample.right += scaled_b;
    }
    let shift = 4 - (apu.sound_control_h & 3).min(2);
    for (channel, level) in [square1.sample(), square2.sample(), apu.wave.sample(), noise.sample()]
        .into_iter()
        .enumerate()
    {
        let square = level << 3;
        if apu.sound_control_l & (1 << (12 + channel)) != 0 {
            let volume = ((apu.sound_control_l >> 4) & 7) as i16 + 1;
            sample.left += (square * volume) >> shift;
        }
        if apu.sound_control_l & (1 << (8 + channel)) != 0 {
            let volume = (apu.sound_control_l & 7) as i16 + 1;
            sample.right += (square * volume) >> shift;
        }
    }
    sample.left = apply_bias_reference(apu, sample.left);
    sample.right = apply_bias_reference(apu, sample.right);
    sample
}

/// "decoded mixer matches the per-channel reference mixer"
#[test]
fn decoded_mixer_matches_the_per_channel_reference_mixer() {
    let mut seed = 0x3a11_ce5d;
    for _ in 0..20_000 {
        let mut apu = Apu {
            sound_control_l: random(&mut seed) as u16,
            sound_control_h: random(&mut seed) as u16,
            sound_bias: random(&mut seed) as u16 & SOUND_BIAS_MASK,
            master_enabled: random(&mut seed) as u8 & 15 != 0,
            high_speed_audio_enabled: random(&mut seed) as u8 & 15 == 0,
            sample_a: random(&mut seed) as i8,
            sample_b: random(&mut seed) as i8,
            ..Apu::default()
        };
        for square in [&mut apu.square1, &mut apu.square2] {
            square.enabled = random(&mut seed) & 1 != 0;
            square.duty = random(&mut seed) as u8 & 3;
            square.phase_index = random(&mut seed) as u8 & 7;
            square.current_volume = random(&mut seed) as u8 & 15;
            square.frequency = random(&mut seed) as u16 & 2047;
            square.phase_cycles =
                random(&mut seed) as u16 as u64 % (16 * (2048 - u64::from(square.frequency)));
        }
        apu.wave.enabled = random(&mut seed) & 1 != 0;
        apu.wave.volume = random(&mut seed) as u8 & 3;
        apu.wave.force_75_percent = random(&mut seed) & 1 != 0;
        apu.wave.two_bank = random(&mut seed) & 1 != 0;
        apu.wave.selected_bank = random(&mut seed) as u8 & 1;
        for byte in &mut apu.wave.ram {
            *byte = random(&mut seed) as u8;
        }
        apu.noise.enabled = random(&mut seed) & 1 != 0;
        apu.noise.output_high = random(&mut seed) & 1 != 0;
        apu.noise.current_volume = random(&mut seed) as u8 & 15;
        apu.noise.lfsr = random(&mut seed) as u16 & 0x7fff;
        apu.noise.divisor = random(&mut seed) as u8 & 7;
        apu.noise.clock_shift = (random(&mut seed) as u8 & 15) % 4;
        if random(&mut seed) & 1 != 0 {
            apu.pending_square_noise_cycles = random(&mut seed) as u16 as u64;
        }
        assert_eq!(current_sample_reference(&apu), apu.current_sample());
    }
}
