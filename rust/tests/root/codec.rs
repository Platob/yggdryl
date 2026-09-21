//! `rust/src/codec.rs`: behavior shared by every content coding, checked
//! against each one.

mod dispatch {
    use std::io::{Read, Write};

    use yggdryl::{Codec, Level};
    use yggdryl::{MediaType, MimeType, Url};
    use yggdryl::{gzip, zlib, zstd};

    /// A payload with enough structure that compression actually shrinks it.
    fn sample() -> Vec<u8> {
        let mut bytes = Vec::new();
        for index in 0..2_000 {
            bytes
                .extend_from_slice(format!("{{\"symbol\":\"AAPL\",\"seq\":{index}}}\n").as_bytes());
        }
        bytes
    }

    #[test]
    fn every_codec_round_trips_a_buffer() {
        let payload = sample();
        for codec in Codec::ALL {
            let encoded = codec.dump(&payload).unwrap_or_else(|error| {
                panic!("{codec} could not encode: {error}");
            });
            let decoded = codec.load(&encoded).unwrap_or_else(|error| {
                panic!("{codec} could not decode: {error}");
            });
            assert_eq!(decoded, payload, "{codec} round trip");

            if !codec.is_identity() {
                assert!(
                    encoded.len() < payload.len(),
                    "{codec} grew the payload: {} -> {}",
                    payload.len(),
                    encoded.len()
                );
            }
        }
    }

    #[test]
    fn every_codec_round_trips_a_stream_without_buffering_whole() {
        let payload = sample();
        for codec in Codec::ALL {
            let mut encoded = Vec::new();
            let mut writer = codec.writer(&mut encoded);
            // Write in chunks so the streaming path is genuinely exercised.
            for chunk in payload.chunks(97) {
                writer.write_all(chunk).unwrap();
            }
            writer.finish().unwrap_or_else(|error| {
                panic!("{codec} could not finish: {error}");
            });

            let mut decoded = Vec::new();
            codec
                .reader(encoded.as_slice())
                .read_to_end(&mut decoded)
                .unwrap_or_else(|error| {
                    panic!("{codec} could not stream-decode: {error}");
                });
            assert_eq!(decoded, payload, "{codec} stream round trip");
        }
    }

    #[test]
    fn empty_input_round_trips() {
        for codec in Codec::ALL {
            let encoded = codec.dump(b"").unwrap();
            assert_eq!(codec.load(&encoded).unwrap(), b"");
        }
    }

    #[test]
    fn every_level_encodes_correctly_and_compresses() {
        // Output size is deliberately not asserted to be monotonic in the level.
        // A higher DEFLATE level searches harder but may select a match sequence
        // that encodes marginally worse, so level 9 can exceed level 1 on a highly
        // repetitive payload. Only correctness and net compression are guaranteed.
        let payload = sample();
        for codec in [Codec::Gzip, Codec::Zlib, Codec::Deflate, Codec::Zstd] {
            for level in [Level::NONE, Level::FAST, Level::DEFAULT, Level::BEST] {
                let encoded = codec.dump_with_level(&payload, level).unwrap();
                assert_eq!(
                    codec.load(&encoded).unwrap(),
                    payload,
                    "{codec} at level {level}"
                );
                if level != Level::NONE {
                    assert!(
                        encoded.len() < payload.len(),
                        "{codec} at level {level} grew the payload"
                    );
                }
            }
        }
    }

    #[test]
    fn level_clamps_to_the_shared_scale() {
        assert_eq!(Level::new(200).get(), 9);
        assert_eq!(Level::new(3).get(), 3);
        assert_eq!(Level::default(), Level::DEFAULT);
    }

    #[test]
    fn codings_are_recovered_from_filenames_and_media_types() {
        let cases = [
            ("file:///trades.json.gz", Codec::Gzip),
            ("file:///trades.json.zst", Codec::Zstd),
            ("file:///trades.json.zz", Codec::Zlib),
            ("file:///trades.json", Codec::Identity),
            ("file:///trades.parquet", Codec::Identity),
        ];
        for (text, expected) in cases {
            let url = Url::from_str(text).unwrap();
            assert_eq!(Codec::from_url(&url), expected, "{text}");
        }

        assert_eq!(Codec::from_mime_type(&MimeType::GZIP), Codec::Gzip);
        assert_eq!(Codec::from_mime_type(&MimeType::ZSTD), Codec::Zstd);
        assert_eq!(Codec::from_mime_type(&MimeType::JSON), Codec::Identity);
        assert_eq!(
            Codec::from_media_type(&MediaType::from(MimeType::JSON)),
            Codec::Identity
        );
    }

    #[test]
    fn only_canonical_codec_names_parse() {
        assert_eq!(Codec::from_str("gzip").unwrap(), Codec::Gzip);
        assert_eq!(Codec::from_str("GZIP").unwrap(), Codec::Gzip);
        assert_eq!(Codec::from_str(" deflate ").unwrap(), Codec::Deflate);
        assert_eq!(Codec::from_str("zstd").unwrap(), Codec::Zstd);

        let invalid = "lzma";
        let message = Codec::from_str(invalid).unwrap_err().to_string();
        assert!(message.contains(&format!("{invalid:?}")), "{message}");
        assert!(message.contains("gzip"), "{message}");
    }

    #[test]
    fn zlib_and_raw_deflate_are_distinct_framings() {
        let payload = b"the same bytes, two framings";
        let framed = zlib::dump(payload).unwrap();
        let raw = zlib::dump_raw(payload).unwrap();

        assert_ne!(framed, raw);
        assert_eq!(zlib::load(&framed).unwrap(), payload);
        assert_eq!(zlib::load_raw(&raw).unwrap(), payload);

        // Each framing rejects the other.
        assert!(zlib::load(&raw).is_err());
        assert!(zlib::load_raw(&framed).is_err());
    }

    #[test]
    fn a_corrupt_payload_is_rejected_rather_than_truncated() {
        let encoded = gzip::dump(&sample()).unwrap();
        let mut corrupt = encoded.clone();
        let midpoint = corrupt.len() / 2;
        corrupt[midpoint] ^= 0xFF;
        assert!(gzip::load(&corrupt).is_err());

        // A truncated stream is also an error, not a short read.
        let truncated = &encoded[..encoded.len() - 4];
        assert!(gzip::load(truncated).is_err());
    }

    #[test]
    fn format_modules_agree_with_the_codec_dispatch() {
        let payload = sample();
        assert_eq!(
            Codec::Gzip.dump(&payload).unwrap(),
            gzip::dump(&payload).unwrap()
        );
        assert_eq!(
            Codec::Zlib.dump(&payload).unwrap(),
            zlib::dump(&payload).unwrap()
        );
        assert_eq!(
            Codec::Zstd.dump(&payload).unwrap(),
            zstd::dump(&payload).unwrap()
        );
        assert_eq!(
            Codec::Deflate.dump(&payload).unwrap(),
            zlib::dump_raw(&payload).unwrap()
        );
    }
}

mod handles {

    /// Restart points: where a decoder may begin an encoded stream partway in.
    mod restarts {
        use std::io::Write as _;

        use yggdryl::{Codec, Level};

        /// Three segments of a payload, each begun at a restart point.
        fn segmented(codec: Codec) -> (Vec<u8>, Vec<Vec<u8>>) {
            let segments: Vec<Vec<u8>> = (0..3)
                .map(|part| format!("part {part}: symbol,price\nAAPL,187.23\n").repeat(64))
                .map(String::into_bytes)
                .collect();
            let mut encoded = Vec::new();
            let mut encoder = codec.writer_with_level(&mut encoded, Level::DEFAULT);
            for (index, segment) in segments.iter().enumerate() {
                if index > 0 {
                    encoder.restart().expect("the coding restarts");
                }
                encoder.write_all(segment).expect("the segment encodes");
            }
            encoder.finish().expect("the stream finishes");
            (encoded, segments)
        }

        #[test]
        fn a_restarted_stream_decodes_whole_and_from_every_restart() {
            for codec in [Codec::Deflate, Codec::Zstd] {
                let (encoded, segments) = segmented(codec);
                let whole: Vec<u8> = segments.concat();
                assert_eq!(codec.load(&encoded).expect("the whole stream"), whole);

                let mut offsets = Vec::new();
                codec.restart_scan().push(&encoded, &mut offsets);
                assert_eq!(offsets.len(), 2, "{codec}");

                // Each restart begins the segment that follows it, and everything
                // after that segment comes with it.
                let mut behind = whole.len();
                for (index, offset) in offsets.iter().enumerate() {
                    let start = usize::try_from(*offset).expect("an in-memory offset");
                    let decoded = codec.load(&encoded[start..]).expect("the tail decodes");
                    let expected: Vec<u8> = segments[index + 1..].concat();
                    assert_eq!(decoded, expected, "{codec} restart {index}");
                    assert!(decoded.len() < behind);
                    behind = decoded.len();
                }
            }
        }

        #[test]
        fn a_restart_marker_split_across_chunks_is_still_found() {
            for codec in [Codec::Deflate, Codec::Zstd] {
                let (encoded, _) = segmented(codec);
                let mut whole = Vec::new();
                codec.restart_scan().push(&encoded, &mut whole);

                for step in [1_usize, 2, 3, 4, 5, 7] {
                    let mut scan = codec.restart_scan();
                    let mut split = Vec::new();
                    for chunk in encoded.chunks(step) {
                        scan.push(chunk, &mut split);
                    }
                    assert_eq!(split, whole, "{codec} in {step}-byte chunks");
                    assert_eq!(scan.consumed(), encoded.len() as u64);
                }
            }
        }

        #[test]
        fn a_coding_whose_framing_wraps_the_payload_refuses_a_restart() {
            for codec in [Codec::Gzip, Codec::Zlib] {
                assert!(!codec.has_restarts());
                let mut encoded = Vec::new();
                let mut encoder = codec.writer_with_level(&mut encoded, Level::DEFAULT);
                encoder.write_all(b"symbol,price\n").expect("the payload");
                let refused = encoder.restart().expect_err("no restart point");
                assert!(
                    refused.to_string().contains(codec.as_str()),
                    "{refused} names {codec}"
                );
                encoder.finish().expect("the stream still finishes");
                assert_eq!(
                    codec.load(&encoded).expect("the payload"),
                    b"symbol,price\n"
                );

                // A refused restart leaves nothing to find.
                let mut offsets = Vec::new();
                codec.restart_scan().push(&encoded, &mut offsets);
                assert!(offsets.is_empty());
            }
        }

        #[test]
        fn an_identity_stream_restarts_at_every_offset_and_reports_none() {
            assert!(Codec::Identity.has_restarts());
            let mut encoded = Vec::new();
            let mut encoder = Codec::Identity.writer(&mut encoded);
            encoder.write_all(b"symbol").expect("the payload");
            encoder.restart().expect("identity always restarts");
            encoder.write_all(b",price").expect("the payload");
            encoder.finish().expect("the stream finishes");
            assert_eq!(encoded, b"symbol,price");

            let mut offsets = Vec::new();
            Codec::Identity.restart_scan().push(&encoded, &mut offsets);
            assert!(offsets.is_empty());
        }

        #[test]
        fn restarting_costs_size_and_nothing_else() {
            let payload = b"symbol,price\nAAPL,187.23\n".repeat(512);
            let plain = Codec::Deflate.dump(&payload).expect("the plain stream");

            let mut restarted = Vec::new();
            let mut encoder = Codec::Deflate.writer(&mut restarted);
            for chunk in payload.chunks(1024) {
                encoder.restart().expect("the coding restarts");
                encoder.write_all(chunk).expect("the chunk encodes");
            }
            encoder.finish().expect("the stream finishes");

            assert_eq!(
                Codec::Deflate.load(&restarted).expect("the payload"),
                payload
            );
            assert!(restarted.len() > plain.len());
        }
    }
}
