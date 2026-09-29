use evenfall_state::{
    Codec, Error,
    envelope::{self, Binding},
    rewind::{self, Compression, RewindBuffer, RewindConfig},
};

/// "word-run diff round-trips length changes and rejects malformed deltas"
#[test]
fn word_run_diff_round_trips_length_changes_and_rejects_malformed_deltas() {
    let base = b"0123456789abcdefghij";
    for target in [
        &b"0123456789abcdefghij"[..],
        &b"0123X56789abcdeZZhij"[..],
        b"0123",
        b"0123456789abcdefghij-longer!",
        b"",
    ] {
        let delta = rewind::diff(base, target);
        assert_eq!(rewind::patch(base, &delta, target.len()).unwrap(), target);
    }
    let delta = rewind::diff(base, b"0123X56789abcdeZZhij");
    // ZiGBA rewind.zig's independent output: one skipped and four copied words.
    assert_eq!(
        delta,
        [2, 1, 4, 88, 53, 54, 55, 56, 57, 97, 98, 99, 100, 101, 90, 90, 104, 105, 106]
    );
    assert_eq!(rewind::patch(base, &delta[..delta.len() - 1], 20), Err(Error::InvalidSaveState));
    assert_eq!(rewind::patch(base, &delta, 8), Err(Error::InvalidSaveState));
    assert_eq!(rewind::patch(base, &[2, 0x7f, 0x7f], 20), Err(Error::InvalidSaveState));
    assert_eq!(rewind::patch(base, &[0xff], 20), Err(Error::InvalidSaveState));
}

/// "persisted inflater accepts independent stored fixed and dynamic vectors and rejects bad framing"
#[test]
fn persisted_inflater_accepts_independent_stored_fixed_and_dynamic_vectors() {
    let cases: &[(&str, Vec<u8>)] = &[
        ("010300fcff414243", b"ABC".to_vec()),
        (
            "73740a7274767471740a72543430343236313533b7b0741c151d151d151d151d151d151d151d15a5bb2800",
            b"ABRACADABRA!0123456789".repeat(100),
        ),
        ("edc43101000008c0a0acd3fe1d0ce10b07d5cc66dbb66ddbb66ddbb61f1f", b"aaabbc".repeat(1000)),
    ];
    for (hex, expected) in cases {
        let mut stored = (expected.len() as u64).to_le_bytes().to_vec();
        for chunk in hex.as_bytes().as_chunks::<2>().0 {
            stored.push(u8::from_str_radix(core::str::from_utf8(chunk).unwrap(), 16).unwrap());
        }
        assert_eq!(
            &rewind::inflate(&stored).unwrap_or_else(|err| panic!("{hex}: {err:?}")),
            expected
        );
        for end in 8..stored.len() {
            assert_eq!(rewind::inflate(&stored[..end]), Err(Error::InvalidSaveState));
        }
        stored.push(0);
        assert_eq!(rewind::inflate(&stored), Err(Error::InvalidSaveState));
        stored.pop();
        stored[..8].copy_from_slice(&((expected.len() + 1) as u64).to_le_bytes());
        assert_eq!(rewind::inflate(&stored), Err(Error::InvalidSaveState));
    }
    for payload in [&[7][..], &[5, 0, 0, 0], &[3, 2, 0], &[1, 1, 0, 0, 0, 65]] {
        let mut stored = 16u64.to_le_bytes().to_vec();
        stored.extend_from_slice(payload);
        assert_eq!(rewind::inflate(&stored), Err(Error::InvalidSaveState));
    }
}

/// "deflate framing rejects over-long and truncated payloads without large allocs"
#[test]
fn deflate_framing_rejects_overlong_and_truncated_payloads() {
    assert_eq!(
        rewind::inflate(&(rewind::MAX_DECODED_BYTES as u64 + 1).to_le_bytes()),
        Err(Error::InvalidSaveState)
    );
    assert_eq!(rewind::inflate(&[1, 2, 3]), Err(Error::InvalidSaveState));
    let raw = [0u8; 4096];
    let mut stored = rewind::deflate(&raw, 1);
    assert!(stored.len() < raw.len() / 20);
    assert_eq!(rewind::inflate(&stored).unwrap(), raw);
    stored[..8].copy_from_slice(&(raw.len() as u64 - 1).to_le_bytes());
    assert_eq!(rewind::inflate(&stored), Err(Error::InvalidSaveState));
    stored[..8].copy_from_slice(&(raw.len() as u64).to_le_bytes());
    for end in 8..stored.len() {
        assert_eq!(rewind::inflate(&stored[..end]), Err(Error::InvalidSaveState));
    }
    let delta = rewind::encode_delta(b"hello", b"world!");
    assert_eq!(rewind::decode_delta(b"hello", &delta).unwrap(), b"world!");
}

/// "age exponent selects exact historical states with oldest-first ties"
#[test]
fn age_exponent_selects_exact_historical_states_with_oldest_first_ties() {
    for (exponent, expected) in [
        (0.5, [1, 5, 9, 11]),
        (0.0, [1, 5, 8, 11]),
        (0.25, [1, 5, 8, 11]),
        (1.0, [1, 8, 10, 11]),
        (f64::MAX, [1, 9, 10, 11]),
    ] {
        let mut buffer = RewindBuffer::new(RewindConfig {
            budget_bytes: 4 * (50 + 64),
            compression: Compression::Off,
            age_exponent: exponent,
            ..RewindConfig::default()
        })
        .unwrap();
        for frame in 1u64..12 {
            buffer.push(frame, frame * 1000, &[frame as u8; 50]).unwrap();
        }
        assert_eq!(buffer.depth(), 4);
        for (i, frame) in expected.into_iter().enumerate() {
            assert_eq!(buffer.info(i).unwrap().frame, frame);
            assert_eq!(buffer.decode(i).unwrap(), [frame as u8; 50]);
        }
    }
}

/// "square-root age weighting still prefers freeing expensive entries"
#[test]
fn square_root_age_weighting_prefers_freeing_expensive_entries() {
    let mut buffer = RewindBuffer::new(RewindConfig {
        budget_bytes: 400,
        compression: Compression::Off,
        ..RewindConfig::default()
    })
    .unwrap();
    for (i, len) in [10, 10, 100, 10, 10].into_iter().enumerate() {
        buffer.push((i + 1) as u64, (i + 1) as u64, &vec![0; len]).unwrap();
    }
    assert_eq!(buffer.memory_bytes(), 296);
    assert_eq!(
        (0..buffer.depth()).map(|i| buffer.info(i).unwrap().frame).collect::<Vec<_>>(),
        [1, 2, 4, 5]
    );
}

/// "capture cadence ticks once per interval, scales with speed, and resets on clear"
#[test]
fn capture_cadence_scales_with_speed_and_resets() {
    let mut buffer = RewindBuffer::new(RewindConfig {
        compression: Compression::Off,
        ..RewindConfig::default()
    })
    .unwrap();
    assert!(!buffer.due_for_capture(59));
    assert!(buffer.due_for_capture(60));
    buffer.push(60, 60, b"a").unwrap();
    assert!(!buffer.due_for_capture(119));
    buffer.note_skipped(120);
    buffer.set_speed_scale(1024).unwrap();
    assert!(!buffer.due_for_capture(359));
    assert!(buffer.due_for_capture(360));
    assert_eq!(buffer.set_speed_scale(256 * 17), Err(Error::InvalidRewindSpeed));
    buffer.clear();
    assert!(buffer.due_for_capture(240));
}

#[test]
fn bundle_round_trip_and_corrupt_history_rejection() {
    for compression in [Compression::Off, Compression::Fast, Compression::Delta] {
        let mut history =
            RewindBuffer::new(RewindConfig { compression, ..RewindConfig::default() }).unwrap();
        for frame in 1u64..10 {
            history.push(frame, frame * 10, &[frame as u8; 2048]).unwrap();
        }
        let live = [10u8; 2048];
        let serialized = history.write_bundle(&live).unwrap();
        let validate = |bytes: &[u8]| Ok((u64::from(bytes[0]), u64::from(bytes[0]) * 10));
        let (loaded_live, mut loaded) = RewindBuffer::read_bundle(&serialized, validate).unwrap();
        assert_eq!(loaded_live, live);
        assert_eq!(loaded.write_bundle(&loaded_live).unwrap(), serialized);
        for i in 0..history.depth() {
            assert_eq!(history.decode(i).unwrap(), loaded.decode(i).unwrap());
        }
        loaded.truncate_newer(4);
        loaded.push(20, 200, &[20; 2048]).unwrap();
        assert_eq!(loaded.newest().unwrap().frame, 20);
        let mut corrupt = serialized.clone();
        corrupt[20] ^= 1;
        assert_eq!(
            RewindBuffer::read_bundle(&corrupt, validate).err(),
            Some(Error::InvalidRewindBundle)
        );
        corrupt = serialized.clone();
        corrupt[8..12].copy_from_slice(&2u32.to_le_bytes());
        let len = corrupt.len() - 32;
        let digest = blake3::hash(&corrupt[..len]);
        corrupt[len..].copy_from_slice(digest.as_bytes());
        assert_eq!(
            RewindBuffer::read_bundle(&corrupt, validate).err(),
            Some(Error::UnsupportedRewindBundleVersion)
        );
    }
}

#[test]
fn zig_frame_inflates_and_rust_frames_preserve_length_and_content() {
    // Produced by Zig 0.16 against rewind.zig `deflate(.level_1)`.
    let zig_frame = [
        220, 0, 0, 0, 0, 0, 0, 0, 115, 116, 10, 114, 116, 118, 116, 1, 81, 138, 6, 134, 70, 198,
        38, 166, 102, 230, 22, 150, 32, 238, 208, 23, 5, 0,
    ];
    let raw = b"ABRACADABRA!0123456789".repeat(10);
    assert_eq!(rewind::inflate(&zig_frame).unwrap(), raw);
    for level in [1, 6] {
        let rust_frame = rewind::deflate(&raw, level);
        assert_eq!(&rust_frame[..8], &(raw.len() as u64).to_le_bytes());
        assert_eq!(rewind::inflate(&rust_frame).unwrap(), raw);
    }
}

fn digest(hex: &str) -> [u8; 32] {
    let mut result = [0; 32];
    for (chunk, byte) in hex.as_bytes().as_chunks::<2>().0.iter().zip(&mut result) {
        *byte = u8::from_str_radix(core::str::from_utf8(chunk).unwrap(), 16).unwrap();
    }
    result
}

#[test]
fn zigba_bundle_imports_and_reexports_original_compressed_entries() {
    // Generated by fixtures/generate-zigba-rewind-bundle.zig with ZiGBA's
    // Session and a synthetic four-byte ROM. It contains three delta entries.
    let fixture = include_bytes!("fixtures/zigba-rewind-bundle.bin");
    let schema = digest("295f36e1576cb4de3009a41143428e8c02a334e046cbb7e13d04c507a8fcb0bd");
    let binding = Binding::from_media(0, &[0xfe, 0xff, 0xff, 0xea], &[]);
    let validate = |bytes: &[u8]| {
        let mut cycle = 0u64;
        envelope::read(bytes, binding, schema, |payload| {
            cycle.decode(payload)?;
            let remaining = (payload.limit.unwrap() - payload.count) as usize;
            payload.read_all(&mut vec![0; remaining])
        })?;
        Ok((cycle / 280_896, cycle))
    };
    let (live, history) = RewindBuffer::read_bundle(fixture, validate).unwrap();
    assert_eq!(history.depth(), 3);
    assert_eq!(
        blake3::hash(&live).as_bytes(),
        &digest("30ecc6b995f24f813c54226b6bf3f6383f663786d8bd138f16faae178b0105f0")
    );
    for (index, expected) in [
        "34815714e5e90439d3a581f96d607c87749e90f362bf2bf313c742adaa8d00d3",
        "5805f0a165c62c578d69b8084b7bcb1a985e77c5b61e529d65e9a09538b2c227",
        "30ecc6b995f24f813c54226b6bf3f6383f663786d8bd138f16faae178b0105f0",
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(history.info(index).unwrap().frame, (index + 1) as u64);
        let decoded = history.decode(index).unwrap();
        assert_eq!(blake3::hash(&decoded).as_bytes(), &digest(expected));
    }
    // The imported payloads are never re-deflated on export.
    assert_eq!(history.write_bundle(&live).unwrap().as_slice(), fixture);
}
