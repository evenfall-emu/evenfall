#[test]
fn zigba_version_six_schema_fixture_has_the_state_envelope_digest() {
    // From ZiGBA Machine.stateSchemaDigest, which also writes the 32 bytes
    // after ZIGBAST1 + version=6 in every ZiGBA save-state envelope.
    let schema = include_bytes!("../../../tools/parity/fixtures/schema-v6.bin");
    let digest = include_str!("../../../tools/parity/fixtures/schema-v6.blake3").trim();
    assert_eq!(blake3::hash(schema).to_hex().as_str(), digest);
}
