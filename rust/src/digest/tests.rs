use crate::{Digest, DigestAlgorithm};

#[test]
fn every_algorithm_round_trips_its_canonical_token() {
    for algorithm in DigestAlgorithm::ALL {
        assert_eq!(
            DigestAlgorithm::from_str(algorithm.as_str()).unwrap(),
            algorithm
        );
        assert_eq!(algorithm.to_string(), algorithm.as_str());
    }
    assert_eq!(
        DigestAlgorithm::ALL.map(DigestAlgorithm::as_str),
        ["xxh32", "xxh64", "xxh3-64", "xxh3-128"],
    );
}

#[test]
fn reference_entry_point_spellings_parse_to_canonical_algorithms() {
    assert_eq!(
        DigestAlgorithm::from_str("xxh3").unwrap(),
        DigestAlgorithm::Xxh3
    );
    assert_eq!(
        DigestAlgorithm::from_str("XXH128").unwrap(),
        DigestAlgorithm::Xxh128
    );
    assert_eq!(
        DigestAlgorithm::from_str("xxh3").unwrap().as_str(),
        "xxh3-64"
    );
    assert_eq!(
        DigestAlgorithm::from_str("xxh128").unwrap().as_str(),
        "xxh3-128"
    );
}

#[test]
fn an_unknown_token_names_the_accepted_vocabulary() {
    let error = DigestAlgorithm::from_str("xxh256").unwrap_err();
    let rendered = error.to_string();
    assert!(rendered.contains("xxh3-64"), "{rendered}");
    assert!(rendered.contains("xxh3-128"), "{rendered}");
}

#[test]
fn widths_and_capabilities_follow_the_algorithm() {
    assert_eq!(DigestAlgorithm::Xxh32.width(), 4);
    assert_eq!(DigestAlgorithm::Xxh64.width(), 8);
    assert_eq!(DigestAlgorithm::Xxh3.width(), 8);
    assert_eq!(DigestAlgorithm::Xxh128.width(), 16);
    for algorithm in DigestAlgorithm::ALL {
        assert_eq!(algorithm.bits(), algorithm.width() as u32 * 8);
        assert!(algorithm.is_seedable());
    }
    assert!(!DigestAlgorithm::Xxh32.is_secretable());
    assert!(!DigestAlgorithm::Xxh64.is_secretable());
    assert!(DigestAlgorithm::Xxh3.is_secretable());
    assert!(DigestAlgorithm::Xxh128.is_secretable());
}

#[test]
fn the_default_algorithm_is_the_one_stable_hash_answers() {
    assert_eq!(DigestAlgorithm::default(), DigestAlgorithm::Xxh3);
}

#[test]
fn hex_rendering_round_trips_at_every_width() {
    for algorithm in DigestAlgorithm::ALL {
        let digest = algorithm.digest(b"symbol,price\nAAPL,187.23\n");
        let rendered = digest.to_string();
        assert!(rendered.starts_with(algorithm.as_str()), "{rendered}");
        assert_eq!(
            rendered.split_once(':').unwrap().1.len(),
            algorithm.width() * 2,
            "{rendered}"
        );
        assert_eq!(Digest::from_str(&rendered).unwrap(), digest);
    }
}

#[test]
fn a_leading_zero_stays_in_the_rendering() {
    let digest = Digest::new(DigestAlgorithm::Xxh64, 1);
    assert_eq!(digest.to_string(), "xxh64:0000000000000001");
    assert_eq!(Digest::from_str("xxh64:0000000000000001").unwrap(), digest);
}

#[test]
fn canonical_bytes_are_big_endian_at_the_exact_width() {
    let digest = Digest::new(DigestAlgorithm::Xxh32, 0x0102_0304);
    assert_eq!(&*digest.into_bytes(), &[0x01, 0x02, 0x03, 0x04]);

    let wide = Digest::new(DigestAlgorithm::Xxh128, 0x0102_0304_0506_0708);
    assert_eq!(wide.into_bytes().len(), 16);
    assert_eq!(&wide.into_bytes()[8..], &[1, 2, 3, 4, 5, 6, 7, 8]);

    for algorithm in DigestAlgorithm::ALL {
        let digest = algorithm.digest(b"AAPL");
        assert_eq!(digest.into_bytes().len(), algorithm.width());
        assert_eq!(
            Digest::from_bytes(algorithm, &digest.into_bytes()).unwrap(),
            digest
        );
    }
}

#[test]
fn a_wrong_width_is_rejected_by_name() {
    let error = Digest::from_bytes(DigestAlgorithm::Xxh64, &[0; 4]).unwrap_err();
    let rendered = error.to_string();
    assert!(
        rendered.contains("expected 8 xxh64 bytes, got 4"),
        "{rendered}"
    );

    let error = Digest::from_str("xxh3-128:00").unwrap_err();
    let rendered = error.to_string();
    assert!(
        rendered.contains("expected 32 xxh3-128 hex digits, got 2"),
        "{rendered}"
    );

    let error = Digest::from_str("2d06800538d394c2").unwrap_err();
    assert!(error.to_string().contains("<algorithm>:<hex>"), "{error}");
}

#[test]
fn a_digest_reads_only_its_own_width() {
    let narrow = DigestAlgorithm::Xxh32.digest(b"AAPL");
    assert!(narrow.as_u32().is_some());
    assert!(narrow.as_u64().is_none());
    assert!(narrow.as_u128().is_none());

    let wide = DigestAlgorithm::Xxh128.digest(b"AAPL");
    assert!(wide.as_u32().is_none());
    assert!(wide.as_u64().is_none());
    assert!(wide.as_u128().is_some());
}

#[test]
fn a_payload_wider_than_the_algorithm_cannot_be_stored() {
    let digest = Digest::new(DigestAlgorithm::Xxh32, u128::MAX);
    assert_eq!(digest.as_u32(), Some(u32::MAX));
    assert_eq!(digest.to_string(), "xxh32:ffffffff");
}

#[test]
fn two_algorithms_never_compare_equal() {
    let left = Digest::new(DigestAlgorithm::Xxh64, 7);
    let right = Digest::new(DigestAlgorithm::Xxh3, 7);
    assert_ne!(left, right);
    assert!(left < right, "the canonical order sorts by algorithm first");

    let mut sorted = [
        Digest::new(DigestAlgorithm::Xxh128, 0),
        Digest::new(DigestAlgorithm::Xxh64, 9),
        Digest::new(DigestAlgorithm::Xxh32, 1),
        Digest::new(DigestAlgorithm::Xxh64, 2),
    ];
    sorted.sort_unstable();
    assert_eq!(
        sorted.map(|digest| digest.algorithm()),
        [
            DigestAlgorithm::Xxh32,
            DigestAlgorithm::Xxh64,
            DigestAlgorithm::Xxh64,
            DigestAlgorithm::Xxh128,
        ]
    );
    assert_eq!(sorted[1].as_u64(), Some(2));
}

#[test]
fn serde_carries_the_algorithm_with_the_value() {
    for algorithm in DigestAlgorithm::ALL {
        let digest = algorithm.digest(b"AAPL");
        let document = serde_json::to_string(&digest).unwrap();
        assert_eq!(document, format!("\"{digest}\""));
        assert_eq!(serde_json::from_str::<Digest>(&document).unwrap(), digest);

        let document = serde_json::to_string(&algorithm).unwrap();
        assert_eq!(document, format!("\"{algorithm}\""));
        assert_eq!(
            serde_json::from_str::<DigestAlgorithm>(&document).unwrap(),
            algorithm
        );
    }
}

#[test]
fn equal_digests_hash_equally() {
    let left = DigestAlgorithm::Xxh3.digest(b"AAPL");
    let right = DigestAlgorithm::Xxh3.digest(b"AAPL");
    assert_eq!(left.stable_hash(), right.stable_hash());
    assert_ne!(
        left.stable_hash(),
        DigestAlgorithm::Xxh64.digest(b"AAPL").stable_hash()
    );
}

#[test]
fn the_digester_agrees_with_the_one_shot_at_every_algorithm() {
    let payload = b"symbol,price\nAAPL,187.23\nMSFT,410.10\n";
    for algorithm in DigestAlgorithm::ALL {
        let mut digester = algorithm.digester();
        assert_eq!(digester.algorithm(), algorithm);
        for chunk in payload.chunks(7) {
            digester.write_bytes(chunk);
        }
        assert_eq!(digester.as_digest(), algorithm.digest(payload));
        // Answering does not consume the state.
        assert_eq!(digester.as_digest(), algorithm.digest(payload));

        digester.clear();
        assert_eq!(digester.as_digest(), algorithm.digest(b""));
    }
}

#[test]
fn the_digester_reads_a_reader_in_bounded_chunks() {
    let payload = vec![0xa5_u8; 300_000];
    for algorithm in DigestAlgorithm::ALL {
        let mut digester = algorithm.digester();
        let consumed = digester.write_reader(&mut payload.as_slice()).unwrap();
        assert_eq!(consumed, payload.len() as u64);
        assert_eq!(digester.as_digest(), algorithm.digest(&payload));
    }
}

#[test]
fn the_digester_is_a_hasher() {
    use std::hash::Hasher as _;

    let mut digester = DigestAlgorithm::Xxh3.digester();
    digester.write(b"abc");
    assert_eq!(digester.finish(), crate::xxhash::xxh3(b"abc"));

    let mut wide = DigestAlgorithm::Xxh128.digester();
    wide.write(b"abc");
    assert_eq!(wide.finish(), crate::xxhash::xxh3(b"abc"));
}
