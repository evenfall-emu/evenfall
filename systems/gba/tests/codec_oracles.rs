//! Codec values, schemas and validation through the public API.

use evenfall_state::{Codec, Error, Sink, Source};

fn encode<T: Codec>(value: &T) -> Vec<u8> {
    let mut bytes = Vec::new();
    value.encode(&mut Sink::new(Some(&mut bytes), None)).unwrap();
    bytes
}

fn schema<T: Codec>() -> Vec<u8> {
    let mut bytes = Vec::new();
    T::schema(&mut Sink::new(Some(&mut bytes), None)).unwrap();
    bytes
}

#[derive(Clone, Copy, Codec, Debug, Default, PartialEq, Eq)]
#[codec(tag_bits = 2)]
enum Kind {
    #[default]
    Idle,
    Active,
    Done,
}

#[derive(Clone, Copy, Codec, Debug, Default, PartialEq, Eq)]
enum Payload {
    #[default]
    None,
    Sample(#[codec(bits = 13)] i16),
    Bytes([u8; 3]),
}

#[derive(Clone, Copy, Codec, Debug, Default, PartialEq, Eq)]
struct PackedFlags {
    first: bool,
    second: bool,
    third: bool,
}

#[derive(Codec, Debug, Default, PartialEq, Eq)]
struct Example {
    enabled: bool,
    kind: Kind,
    #[codec(bits = 3)]
    narrow: u8,
    #[codec(bits = 13)]
    signed: i16,
    #[codec(bits = 12)]
    maybe: Option<u16>,
    payload: Payload,
    flags: PackedFlags,
    host_count: usize,
}

/// Values round-trip in their encoded lengths, not Rust's memory layout.
#[test]
fn canonical_values_round_trip_without_struct_padding() {
    let expected = Example {
        enabled: true,
        kind: Kind::Active,
        narrow: 5,
        signed: -1234,
        maybe: Some(0xabc),
        payload: Payload::Bytes([3, 2, 1]),
        flags: PackedFlags { first: true, second: false, third: true },
        host_count: 0x1234,
    };
    let encoded = encode(&expected);
    assert_eq!(encoded.len(), 23);
    assert!(encoded.len() < std::mem::size_of::<Example>());
    let mut source = Source::new(&encoded);
    let mut actual = Example::default();
    actual.decode(&mut source).unwrap();
    assert_eq!(actual, expected);
    assert_eq!(source.count, encoded.len() as u64);
}

#[derive(Codec, Default)]
enum FirstPayload {
    #[default]
    None,
    Bytes([u8; 4]),
}

#[derive(Codec, Default)]
struct First {
    maybe: Option<u16>,
    payload: FirstPayload,
}

#[derive(Codec, Default)]
enum ChangedPayload {
    #[default]
    None,
    Bytes([u8; 8]),
}

#[derive(Codec, Default)]
struct ChangedInactivePayload {
    maybe: Option<u32>,
    payload: ChangedPayload,
}

/// An inactive variant's or absent option's type still changes the schema.
#[test]
fn schema_encoding_covers_inactive_union_and_optional_payload_types() {
    let first = schema::<First>();
    assert_eq!(first, schema::<First>());
    assert_ne!(first, schema::<ChangedInactivePayload>());
}

#[derive(Codec, Default)]
struct Flagged {
    flag: bool,
    #[codec(bits = 3)]
    narrow: u8,
}

#[derive(Codec, Default)]
enum Choice {
    #[default]
    Nothing,
    Number(u16),
}

#[test]
fn decoder_rejects_illegal_tagged_values_and_narrow_high_bits() {
    for invalid in [[2, 0], [1, 8]] {
        assert_eq!(
            Flagged::default().decode(&mut Source::new(&invalid)),
            Err(Error::InvalidSaveState)
        );
    }
    assert_eq!(Choice::default().decode(&mut Source::new(&[3])), Err(Error::InvalidSaveState));
}
