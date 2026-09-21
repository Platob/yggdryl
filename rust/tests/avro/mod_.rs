//! `rust/src/avro/mod.rs`: the named-type registry no caller can reach.
//!
//! A record names itself, and a later field may name that record instead of
//! restating it. `Schema::names` is the crate-private table that resolves the
//! second to the first. What a caller can observe is beside it here and in
//! the other files under `rust/tests/avro/`.

#[cfg(feature = "internals")]
mod internal {
    use yggdryl::avro::Schema;
    use yggdryl::internals::avro_schema::names;

    #[test]
    fn a_reference_to_an_earlier_definition_is_not_a_redefinition() {
        let schema = Schema::from_str(
            r#"{"type":"record","name":"row","fields":[
            {"name":"p","type":{"type":"record","name":"kv","fields":[
                {"name":"x","type":"long"}]}},
            {"name":"q","type":"kv"}
        ]}"#,
        )
        .unwrap();
        assert!(names(&schema).contains(&"kv"));
    }
}

use yggdryl::IOBase;
use yggdryl::holder::Buffer;
use yggdryl::{MediaType, MimeType, Scalar};

/// A record schema exercising every branch the manifests use.
fn manifest_shaped_schema() -> Scalar {
    yggdryl::json::from_utf8(
        r#"{"type":"record","name":"row","fields":[
            {"name":"code","type":"int","field-id":1},
            {"name":"name","type":"string","field-id":2},
            {"name":"score","type":["null","double"],"default":null,"field-id":3},
            {"name":"raw","type":["null","bytes"],"default":null,"field-id":4},
            {"name":"tags","type":{"type":"array","element-id":6,"items":"long"},
             "field-id":5},
            {"name":"nested","type":{"type":"record","name":"inner","fields":[
                {"name":"flag","type":"boolean","field-id":8}
            ]},"field-id":7}
        ]}"#,
    )
    .unwrap()
}

fn buffer() -> Buffer {
    let mut buffer = Buffer::new();
    buffer.set_media_type(MediaType::new(MimeType::AVRO));
    buffer
}

mod fuzz_lite {
    //! Seeded mutation sweeps: every outcome must be a value or a typed
    //! error - no panic, no runaway allocation. A longer sweep scales with
    //! `AVRO_FUZZ_ITERATIONS`, matching how the repository gates its slow
    //! legs outside the default test run.

    use yggdryl::IOBase;
    use yggdryl::Limits;
    use yggdryl::avro;

    /// A deterministic pseudo-random byte source.
    struct Lcg(u64);

    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            self.0
        }

        fn byte(&mut self) -> u8 {
            (self.next() >> 33) as u8
        }

        fn below(&mut self, bound: usize) -> usize {
            (self.next() >> 16) as usize % bound.max(1)
        }
    }

    fn iterations() -> usize {
        std::env::var("AVRO_FUZZ_ITERATIONS")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(400)
    }

    #[test]
    fn mutated_containers_never_panic() {
        let schema = super::manifest_shaped_schema();
        let rows = [yggdryl::json::from_utf8(
            r#"{"code":-7,"name":"AAPL","score":1.5,"raw":null,"tags":[1,2,3],
                "nested":{"flag":true}}"#,
        )
        .unwrap()];
        let mut handle = super::buffer();
        avro::write_container(&mut handle, &schema, &[("k", "v")], &rows).unwrap();
        let valid = handle.read_all_bytes().unwrap();

        let limits = Limits::new(32, 1 << 16, 1 << 12, 64);
        let mut random = Lcg(0x5EED);
        for _ in 0..iterations() {
            let mut mutated = valid.clone();
            for _ in 0..1 + random.below(4) {
                let index = random.below(mutated.len());
                mutated[index] = random.byte();
            }
            let mut corrupt = super::buffer();
            corrupt.write_all_bytes(&mutated).unwrap();
            // A value or a typed error; anything else fails the test by panic.
            let _ = avro::read_container_with_limits(&corrupt, limits);
        }
    }

    #[test]
    fn random_bytes_never_panic_any_entry_point() {
        let limits = Limits::new(32, 1 << 14, 1 << 10, 64);
        let mut random = Lcg(0xACE5);
        for _ in 0..iterations() {
            let length = random.below(512);
            let bytes: Vec<u8> = (0..length).map(|_| random.byte()).collect();
            let mut handle = super::buffer();
            handle.write_all_bytes(&bytes).unwrap();
            let _ = avro::read_container_with_limits(&handle, limits);
            if let Ok(mut blocks) = avro::read_blocks_with_limits(&handle, limits) {
                while let Ok(Some(block)) = blocks.next_block() {
                    let _ = block.rows();
                }
            }
            if let Ok(text) = std::str::from_utf8(&bytes) {
                let _ = avro::Schema::from_str(text);
            }
        }
    }

    #[test]
    fn mutated_schema_documents_never_panic() {
        let source = r#"{"type":"record","name":"row","fields":[
            {"name":"a","type":["null","long"],"default":null},
            {"name":"b","type":{"type":"array","items":"string"}},
            {"name":"c","type":{"type":"fixed","name":"f","size":4,"logicalType":"decimal",
             "precision":9,"scale":2}}
        ]}"#;
        let mut random = Lcg(0xF00D);
        let bytes = source.as_bytes();
        for _ in 0..iterations() {
            let mut mutated = bytes.to_vec();
            for _ in 0..1 + random.below(3) {
                let index = random.below(mutated.len());
                mutated[index] = random.byte();
            }
            if let Ok(text) = std::str::from_utf8(&mutated) {
                let _ = avro::Schema::from_str(text);
            }
        }
    }
}
