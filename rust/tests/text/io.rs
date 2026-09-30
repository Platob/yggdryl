//! `rust/src/text/io.rs`: the plan a handle's name states, and the reads it
//! settles.
//!
//! Format, content coding and charset are one value a caller can build and
//! read, so every read below goes through `yggdryl::text`. The one thing a
//! caller cannot reach is the crate's own stable hash, which is what the
//! plan's value traits are pinned by, in [`internal`].

use yggdryl::IOBase;
use yggdryl::holder::Buffer;
use yggdryl::text::{Format, Plan, from_io, from_io_all, into_io, into_io_all};
use yggdryl::{Codec, Error, Scalar, Url};

#[cfg(feature = "internals")]
mod internal {
    use std::hash::Hash;

    use yggdryl::Codec;
    use yggdryl::internals::hashing_stable::stable_hash_of;
    use yggdryl::text::{Format, Plan};

    #[test]
    fn plans_have_complete_value_traits() {
        fn assert_traits<T: Copy + Eq + Ord + Hash>() {}
        assert_traits::<Plan>();
        let plan = Plan::new(Format::Json, Codec::Gzip);
        assert_eq!(plan, plan.clone());
        assert_eq!(stable_hash_of(&plan), stable_hash_of(&plan));
    }
}

fn sample() -> Scalar {
    Scalar::from_struct([
        ("quantity", Scalar::from(100)),
        ("symbol", Scalar::from("AAPL")),
    ])
    .unwrap()
}

fn handle(name: &str) -> Buffer {
    Buffer::new().with_media_type(
        Url::from_str(&format!("file:///{name}"))
            .unwrap()
            .media_type(),
    )
}

#[test]
fn plans_follow_compound_filenames() {
    let cases = [
        ("a.json", Format::Json, Codec::Identity),
        ("a.json.gz", Format::Json, Codec::Gzip),
        ("a.yaml.zst", Format::Yaml, Codec::Zstd),
        ("a.toml", Format::Toml, Codec::Identity),
        ("a.xml", Format::Xml, Codec::Identity),
        ("a.xml.gz", Format::Xml, Codec::Gzip),
    ];
    for (name, format, codec) in cases {
        let plan = Plan::infer(&handle(name)).unwrap();
        assert_eq!((plan.format(), plan.codec()), (format, codec));
    }
}

#[test]
fn formats_and_codings_round_trip() {
    for name in ["a.json", "a.json.gz", "a.yaml.zst", "a.toml"] {
        let mut target = handle(name);
        into_io(&sample(), &mut target).unwrap();
        assert_eq!(from_io(&target).unwrap(), sample(), "{name}");
    }
}

#[test]
fn an_xml_handle_round_trips_the_one_root_a_document_has() {
    let document = Scalar::from_struct([("sample", sample())]).unwrap();
    for name in ["a.xml", "a.xml.gz"] {
        let mut target = handle(name);
        into_io(&document, &mut target).unwrap();
        assert_eq!(
            from_io(&target).unwrap(),
            yggdryl::xml::from_bytes(&yggdryl::xml::into_bytes(&document).unwrap()).unwrap(),
            "{name}"
        );
    }
}

#[test]
fn multi_document_handles_round_trip() {
    let values = [sample(), sample()];
    for name in ["many.jsonl", "many.yaml"] {
        let mut target = handle(name);
        into_io_all(&values, &mut target).unwrap();
        assert_eq!(from_io_all(&target).unwrap(), values, "{name}");
    }
}

#[test]
fn structured_parsers_cross_stream_batches_under_every_coding() {
    let alphabet = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let mut state = 0xA537_1D09_u32;
    let message = (0..256 * 1024)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            alphabet[state as usize % alphabet.len()] as char
        })
        .collect::<String>();
    let value = Scalar::from_struct([
        ("quantity", Scalar::from(100)),
        ("message", Scalar::from(message)),
    ])
    .unwrap();
    let cases = [
        (Format::Json, "json"),
        (Format::Yaml, "yaml"),
        (Format::Toml, "toml"),
    ];
    let codings = [
        (Codec::Gzip, "gz"),
        (Codec::Zlib, "zz"),
        (Codec::Zstd, "zst"),
    ];

    for (format, extension) in cases {
        let plain = yggdryl::text::into_bytes(&value, format).unwrap();
        assert!(plain.len() > yggdryl::DEFAULT_STREAM_BATCH_SIZE);
        for (codec, suffix) in codings {
            // Deliberately declare only the structured format: the bounded
            // replay prefix must still preserve magic-based coding
            // detection before the parser crosses several decoded chunks.
            let mut source = handle(&format!("large.{extension}"));
            let encoded = codec.dump(&plain).unwrap();
            assert!(
                encoded.len() > yggdryl::DEFAULT_STREAM_BATCH_SIZE,
                "{format}/{codec}"
            );
            source.write_all_bytes(&encoded).unwrap();
            assert_eq!(
                from_io(&source).unwrap(),
                value,
                "{format}/{codec}/{suffix}"
            );
        }
    }
}

#[test]
fn a_container_is_refused_by_name_although_its_stream_holds_documents() {
    // A folder streams its leaves end to end, so its bytes are two JSON
    // documents run together: no one value, and the first says nothing about
    // the second. Every structured read - sniffed or under a named format -
    // refuses it for what it is, never by what the head of its stream happens
    // to parse as.
    use yggdryl::local::{LocalFolder, LocalPath};
    use yggdryl::text::{Json, Jsonl, TextCodec, Yaml};

    let mut root = LocalFolder::temporary().unwrap().path().unwrap();
    root.push(format!("yggdryl-text-io-folder-{}", std::process::id()));
    let mut folder = LocalFolder::new(&root).unwrap();
    folder.remove(true).unwrap();
    for name in ["a.json", "b.json"] {
        folder
            .child_by_path(name)
            .unwrap()
            .write_scalar(&sample())
            .unwrap();
    }
    let spelled = LocalPath::new(format!("{}/", root.display())).unwrap();
    let head = folder.read_all_bytes().unwrap();
    assert!(head.starts_with(b"{"), "{head:?}");
    assert_eq!(spelled.read_all_bytes().unwrap(), head);

    let expected = "expected json, jsonl, yaml, toml, or xml, got inode/directory";
    let handles: [&dyn IOBase; 2] = [&folder, &spelled];
    for handle in handles {
        let field = yggdryl::DataType::from_str("struct<quantity: int64, symbol: utf8>")
            .unwrap()
            .required_field("row");
        for refused in [
            handle.read_scalar(None).unwrap_err(),
            handle.read_scalar(Some(&field)).unwrap_err(),
            from_io(handle).unwrap_err(),
            from_io_all(handle).unwrap_err(),
            Plan::detect(handle, &head).unwrap_err(),
            Json.from_io(handle).unwrap_err(),
            Json.from_io_with_field(handle, &field).unwrap_err(),
            Jsonl.from_io_all(handle).unwrap_err(),
            Yaml.from_io_all(handle).unwrap_err(),
        ] {
            assert!(
                matches!(&refused, Error::Codec { format: "text", reason, .. } if reason.contains(expected)),
                "{refused:?}"
            );
        }
    }

    folder.remove(true).unwrap();
}
