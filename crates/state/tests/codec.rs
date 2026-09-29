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
