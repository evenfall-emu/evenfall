use evenfall_state::envelope::{self, Binding};
use evenfall_state::{Codec, Error, Sink, Source};

#[derive(Codec, Default, Debug, PartialEq)]
struct Core {
    x: u16,
    #[codec(bits = 3)]
    small: [u8; 2],
}
#[derive(Codec, Default, Debug, PartialEq)]
#[codec(module = "zigba.bus.state.v6")]
struct Device {
    #[codec(flatten)]
    core: Core,
    #[codec(skip)]
    transient: u32,
    #[codec(rename = "raw")]
    r#type: Option<u8>,
}
#[derive(Codec, Default, Debug, PartialEq)]
#[codec(tag_bits = 3)]
enum Kind {
    #[default]
    Nonsequential,
    CpuCode = 3,
    Flash128,
}
#[derive(Codec, Default, Debug, PartialEq)]
enum Choice {
    #[default]
    Empty,
    Value(i16),
    Record {
        a: u8,
        b: u16,
    },
}

fn encoded<T: Codec>(v: &T) -> Vec<u8> {
    let mut out = Vec::new();
    v.encode(&mut Sink::new(Some(&mut out), None)).unwrap();
    out
}
fn schema<T: Codec>() -> Vec<u8> {
    let mut out = Vec::new();
    T::schema(&mut Sink::new(Some(&mut out), None)).unwrap();
    out
}
fn name(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(&(s.len() as u32).to_le_bytes());
    out.extend_from_slice(s.as_bytes());
}

#[test]
fn structs_flatten_skip_narrow_and_module_schema() {
    let original =
        Device { core: Core { x: 0x1234, small: [5, 7] }, transient: 99, r#type: Some(9) };
    assert_eq!(encoded(&original), [0x34, 0x12, 5, 7, 1, 9]);
    let mut expected = Vec::new();
    name(&mut expected, "zigba.bus.state.v6");
    name(&mut expected, "x");
    expected.extend_from_slice(b"u\x10\0");
    name(&mut expected, "small");
    expected.extend_from_slice(b"a\x02\0\0\0\0\0\0\0u\x03\0");
    name(&mut expected, "raw");
    expected.extend_from_slice(b"?u\x08\0");
    assert_eq!(schema::<Device>(), expected);
    let mut decoded = Device { transient: 42, ..Device::default() };
    decoded.decode(&mut Source::new(&encoded(&original))).unwrap();
    assert_eq!(decoded.transient, 42);
    assert_eq!(decoded.core, original.core);
    assert_eq!(decoded.r#type, original.r#type);
    assert_eq!(decoded.decode(&mut Source::new(&[0, 0, 8, 0, 0])), Err(Error::InvalidSaveState));
}

#[test]
fn enum_and_union_tag_schema_and_values() {
    assert_eq!(encoded(&Kind::Flash128), [4]);
    assert_eq!(encoded(&Choice::Value(-1234)), [1, 0x2e, 0xfb]);
    assert_eq!(encoded(&Choice::Record { a: 7, b: 0x1234 }), [2, 7, 0x34, 0x12]);
    let mut kind = b"eu\x03\0\x03\0\0\0".to_vec();
    for (n, value) in [("nonsequential", 0), ("cpu_code", 3), ("flash128", 4)] {
        name(&mut kind, n);
        kind.push(value);
    }
    assert_eq!(schema::<Kind>(), kind);
    let mut union = b"veu\x02\0\x03\0\0\0".to_vec();
    for (n, value) in [("empty", 0), ("value", 1), ("record", 2)] {
        name(&mut union, n);
        union.push(value);
    }
    union.extend_from_slice(&3u32.to_le_bytes());
    name(&mut union, "empty");
    union.push(b'0');
    name(&mut union, "value");
    union.extend_from_slice(b"i\x10\0");
    name(&mut union, "record");
    union.extend_from_slice(b"s\x02\0\0\0");
    name(&mut union, "a");
    union.extend_from_slice(b"u\x08\0");
    name(&mut union, "b");
    union.extend_from_slice(b"u\x10\0");
    assert_eq!(schema::<Choice>(), union);
    assert_eq!(Kind::default().decode(&mut Source::new(&[7])), Err(Error::InvalidSaveState));
    assert_eq!(Choice::default().decode(&mut Source::new(&[3])), Err(Error::InvalidSaveState));
}

#[test]
fn envelope_framing_and_validation_order() {
    let binding = Binding::from_media(2, b"rom", b"bios");
    let digest = envelope::schema_digest(|s| {
        u64::schema(s)?;
        Core::schema(s)
    })
    .unwrap();
    let value = Core { x: 0x1234, small: [5, 7] };
    let write = |s: &mut Sink<'_>| {
        0x0123_4567_89ab_cdefu64.encode(s)?;
        value.encode(s)
    };
    let state = envelope::write_buffered(binding, digest, write).unwrap();
    assert_eq!(state, envelope::write_streaming(binding, digest, write).unwrap());
    assert_eq!(&state[..8], b"ZIGBAST1");
    assert_eq!(&state[8..12], &6u32.to_le_bytes());
    assert_eq!(&state[44..52], &12u64.to_le_bytes());
    assert_eq!(&state[52..61], &[2, 3, 0, 0, 0, 0, 0, 0, 0]);
    let load = |bytes: &[u8], media, schema| {
        envelope::read(bytes, media, schema, |s| {
            let mut cycle = 0u64;
            cycle.decode(s)?;
            let mut decoded = Core::default();
            decoded.decode(s)?;
            Ok(())
        })
    };
    assert_eq!(load(&state, binding, digest), Ok(()));
    let mut changed = state.clone();
    changed[8] = 7;
    assert_eq!(load(&changed, binding, digest), Err(Error::UnsupportedSaveStateVersion));
    assert_eq!(load(&state, binding, [0; 32]), Err(Error::SaveStateSchemaMismatch));
    changed = state.clone();
    changed[44] = 0xff;
    assert_eq!(load(&changed, binding, digest), Err(Error::InvalidSaveState));
    changed = state.clone();
    changed[52] = 0;
    assert_eq!(load(&changed, binding, digest), Err(Error::SaveStateBootModeMismatch));
    changed = state.clone();
    changed[61] ^= 1;
    assert_eq!(load(&changed, binding, digest), Err(Error::SaveStateRomMismatch));
    changed = state.clone();
    changed[101] ^= 1;
    assert_eq!(load(&changed, binding, digest), Err(Error::SaveStateBiosMismatch));
    changed = state.clone();
    changed[133] ^= 1;
    assert_eq!(load(&changed, binding, digest), Err(Error::InvalidSaveState));
    changed = state.clone();
    changed.push(0);
    assert_eq!(load(&changed, binding, digest), Err(Error::InvalidSaveState));
    assert_eq!(load(&state[..state.len() - 1], binding, digest), Err(Error::InvalidSaveState));
}

#[derive(Codec, Default)]
struct Narrow {
    #[codec(bits = 13)]
    signed: i16,
    #[codec(bits = 12)]
    maybe: Option<[u16; 2]>,
    #[codec(bits = 0)]
    zero: u8,
    #[codec(bits = 3)]
    boxed: Box<[u8; 2]>,
}

#[test]
fn narrow_widths_sign_extend_and_reject_high_bits() {
    let value =
        Narrow { signed: -1234, maybe: Some([0xabc, 0x123]), zero: 0, boxed: Box::new([5, 2]) };
    assert_eq!(encoded(&value), [0x2e, 0x1b, 1, 0xbc, 0x0a, 0x23, 0x01, 5, 2]);
    let mut actual = Narrow::default();
    actual.decode(&mut Source::new(&encoded(&value))).unwrap();
    assert_eq!(actual.signed, -1234);
    assert_eq!(actual.maybe, value.maybe);
    let mut corrupt = encoded(&value);
    corrupt[1] |= 0xe0;
    assert_eq!(actual.decode(&mut Source::new(&corrupt)), Err(Error::InvalidSaveState));
    corrupt = encoded(&value);
    corrupt[4] |= 0xf0;
    assert_eq!(actual.decode(&mut Source::new(&corrupt)), Err(Error::InvalidSaveState));
}

#[derive(Codec, Default)]
struct Canonical {
    count: u8,
    #[codec(encode_with = encode_used)]
    slots: [u8; 3],
}
fn encode_used(value: &Canonical, sink: &mut Sink<'_>) -> evenfall_state::Result<()> {
    let mut slots = value.slots;
    slots[usize::from(value.count)..].fill(0);
    slots.encode(sink)
}

#[test]
fn custom_encoder_and_tracer_do_not_change_payload() {
    let value = Canonical { count: 1, slots: [7, 8, 9] };
    let mut bytes = Vec::new();
    let mut trace = Vec::new();
    let mut sink = Sink::new(Some(&mut bytes), None);
    sink.trace = Some(&mut trace);
    value.encode(&mut sink).unwrap();
    assert_eq!(bytes, [1, 7, 0, 0]);
    assert_eq!(
        trace.iter().map(|(path, range)| (path.as_str(), range.clone())).collect::<Vec<_>>(),
        [("count", 0..1), ("slots", 1..4)]
    );
}

#[derive(Codec)]
#[codec(schema_prefix = "zigba.cpu.state.v6")]
struct Prefixed {
    value: u8,
}

#[derive(Codec)]
#[codec(transparent)]
struct Transparent {
    state: Prefixed,
    #[codec(skip)]
    tag: u8,
}

#[test]
fn cpu_prefix_and_transparent_save_device_schema() {
    let mut expected = Vec::new();
    name(&mut expected, "zigba.cpu.state.v6");
    expected.extend_from_slice(b"s\x01\0\0\0");
    name(&mut expected, "value");
    expected.extend_from_slice(b"u\x08\0");
    assert_eq!(schema::<Prefixed>(), expected);
    assert_eq!(schema::<Transparent>(), expected);
    let value = Transparent { state: Prefixed { value: 7 }, tag: 99 };
    assert_eq!(value.tag, 99);
    assert_eq!(encoded(&value), [7]);
}
