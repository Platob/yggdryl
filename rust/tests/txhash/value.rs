//! `rust/src/txhash/value.rs`: one time-coupled hash - the instant first, the
//! digest after it - and every spelling it round-trips through.

mod coupled {
    use yggdryl::txhash::{
        DEFAULT_UNIT, TxHash, UNIX_WIDTH, digest, dtype, restate_unix, txh3, txh64, txh128,
    };

    use yggdryl::{Digest, DigestAlgorithm, Error, TimeUnit};

    const INSTANT: i64 = 1_700_000_000_000_000;

    /// One instant, one payload, every algorithm.
    fn every_algorithm() -> impl Iterator<Item = (DigestAlgorithm, TxHash)> {
        DigestAlgorithm::ALL
            .into_iter()
            .map(|algorithm| (algorithm, digest(b"AAPL", INSTANT, algorithm)))
    }

    #[test]
    fn canonical_bytes_are_the_instant_then_the_digest() {
        for (algorithm, value) in every_algorithm() {
            let bytes = value.into_bytes();
            assert_eq!(bytes.len(), value.width(), "{algorithm}");
            assert_eq!(&bytes[..UNIX_WIDTH], &INSTANT.to_be_bytes(), "{algorithm}");
            assert_eq!(
                &bytes[UNIX_WIDTH..],
                &*value.digest().into_bytes(),
                "{algorithm}"
            );
            assert_eq!(
                TxHash::from_bytes(DEFAULT_UNIT, algorithm, &bytes).unwrap(),
                value,
                "{algorithm}"
            );
            assert_eq!(bytes.as_ref(), &*bytes);
            assert_eq!(format!("{bytes:?}"), format!("{:?}", &*bytes));
        }
        let negative = txh3(b"", -1);
        assert_eq!(&negative.into_bytes()[..UNIX_WIDTH], &[0xff; 8]);
        assert_eq!(
            TxHash::from_bytes(DEFAULT_UNIT, DigestAlgorithm::Xxh3, &negative.into_bytes())
                .unwrap(),
            negative
        );
    }

    #[test]
    fn uuid_projection_packs_the_microsecond_instant_as_a_uuidv7() {
        let digest = Digest::new(DigestAlgorithm::Xxh64, 0x0123_4567_89ab_cdef);
        for (nanoseconds, expected) in [
            (0, "00000000-0000-7000-8123-456789abcdef"),
            // The sub-microsecond nanoseconds are floored away.
            (999, "00000000-0000-7000-8123-456789abcdef"),
            (1_000, "00000000-0000-7004-8123-456789abcdef"),
            (999_999, "00000000-0000-7ffb-8123-456789abcdef"),
            (1_000_000, "00000000-0001-7000-8123-456789abcdef"),
            (1_000_000_000, "00000000-03e8-7000-8123-456789abcdef"),
            (i64::MAX, "08637bd0-5af6-7c66-8123-456789abcdef"),
        ] {
            let value = TxHash::new_in(nanoseconds, TimeUnit::Nanosecond, digest).unwrap();
            let raw = value.into_bytes();
            let projected = value.into_uuid().unwrap();
            assert_eq!(projected.to_string(), expected, "{nanoseconds}");
            let bytes = projected.into_bytes();
            assert_eq!(bytes[6] >> 4, 7);
            assert_eq!(bytes[8] >> 6, 2);
            assert_eq!(
                value.into_bytes(),
                raw,
                "projection does not mutate the value"
            );
            assert_eq!(&raw[..UNIX_WIDTH], &nanoseconds.to_be_bytes());
            assert_eq!(&raw[UNIX_WIDTH..], &*digest.into_bytes());
        }
    }

    #[test]
    fn uuid_projection_orders_microsecond_instants_before_every_digest_bit() {
        let instants = [
            0,
            1_000,
            15_000,
            16_000,
            65_535_000,
            65_536_000,
            1_000_000_000,
            i64::MAX,
        ];
        for pair in instants.windows(2) {
            let earlier = TxHash::new_in(
                pair[0],
                TimeUnit::Nanosecond,
                Digest::new(DigestAlgorithm::Xxh64, u128::MAX),
            )
            .unwrap();
            let later = TxHash::new_in(
                pair[1],
                TimeUnit::Nanosecond,
                Digest::new(DigestAlgorithm::Xxh64, 0),
            )
            .unwrap();
            assert!(
                earlier < later,
                "native ordering still compares the signed count"
            );
            assert!(
                earlier.into_uuid().unwrap() < later.into_uuid().unwrap(),
                "{pair:?}"
            );
        }
        // Within one microsecond the instant ties, and the digest orders.
        let low = TxHash::new_in(
            1_000,
            TimeUnit::Nanosecond,
            Digest::new(DigestAlgorithm::Xxh64, 1),
        )
        .unwrap();
        let high = TxHash::new_in(
            1_999,
            TimeUnit::Nanosecond,
            Digest::new(DigestAlgorithm::Xxh64, 2),
        )
        .unwrap();
        assert!(low.into_uuid().unwrap() < high.into_uuid().unwrap());
        // Before the epoch there is no UUIDv7: the projection is refused at `$`,
        // while the raw bytes still hold the two's-complement count.
        let before = TxHash::new_in(
            -1,
            TimeUnit::Nanosecond,
            Digest::new(DigestAlgorithm::Xxh64, 0),
        )
        .unwrap();
        let epoch = TxHash::new_in(0, TimeUnit::Nanosecond, before.digest()).unwrap();
        assert!(matches!(
            before.into_uuid().unwrap_err(),
            Error::InvalidRecord { ref path, .. } if path == "$"
        ));
        assert!(
            before.into_bytes() > epoch.into_bytes(),
            "raw bytes retain two's-complement ordering"
        );
    }

    #[test]
    fn uuid_projection_normalizes_units_and_preserves_restatement_overflow() {
        let digest = Digest::new(DigestAlgorithm::Xxh3, 7);
        for seconds in [0, 2] {
            let expected = TxHash::new_in(seconds, TimeUnit::Second, digest)
                .unwrap()
                .into_uuid()
                .unwrap();
            for (unit, scale) in [
                (TimeUnit::Second, 1),
                (TimeUnit::Millisecond, 1_000),
                (TimeUnit::Microsecond, 1_000_000),
                (TimeUnit::Nanosecond, 1_000_000_000),
            ] {
                let value = TxHash::new_in(seconds * scale, unit, digest).unwrap();
                assert_eq!(value.into_uuid().unwrap(), expected, "{unit}");
            }
        }
        for (unit, scale) in [
            (TimeUnit::Second, 1_000_000_000),
            (TimeUnit::Millisecond, 1_000_000),
            (TimeUnit::Microsecond, 1_000),
        ] {
            assert!(
                TxHash::new_in(i64::MAX / scale, unit, digest)
                    .unwrap()
                    .into_uuid()
                    .is_ok()
            );
            // A count below the epoch restates, and is then refused as a UUIDv7.
            assert!(matches!(
                TxHash::new_in(i64::MIN / scale, unit, digest)
                    .unwrap()
                    .into_uuid()
                    .unwrap_err(),
                Error::InvalidRecord { ref path, .. } if path == "$"
            ));
            for count in [i64::MIN / scale - 1, i64::MAX / scale + 1] {
                let expected = restate_unix(count, unit, TimeUnit::Nanosecond).unwrap_err();
                let error = TxHash::new_in(count, unit, digest)
                    .unwrap()
                    .into_uuid()
                    .unwrap_err();
                assert_eq!(error.to_string(), expected.to_string());
                assert!(matches!(
                    error,
                    Error::ArithmeticOverflow {
                        operation: "unix restatement",
                        kind: "int64"
                    }
                ));
            }
        }
    }

    #[test]
    fn uuid_projection_discards_only_the_high_two_digest_bits_and_algorithm() {
        let project = |algorithm, payload| {
            TxHash::new_in(1, TimeUnit::Nanosecond, Digest::new(algorithm, payload))
                .unwrap()
                .into_uuid()
                .unwrap()
        };
        let payload = 0x0123_4567_89ab_cdef;
        let expected = project(DigestAlgorithm::Xxh64, payload);
        assert_eq!(project(DigestAlgorithm::Xxh3, payload), expected);
        for bit in 62..64 {
            assert_eq!(
                project(DigestAlgorithm::Xxh64, payload ^ (1_u128 << bit)),
                expected
            );
        }
        for bit in 0..62 {
            assert_ne!(
                project(DigestAlgorithm::Xxh64, payload ^ (1_u128 << bit)),
                expected
            );
        }
    }

    #[test]
    fn uuid_projection_refuses_non_64_bit_digests_without_narrowing() {
        for algorithm in [DigestAlgorithm::Xxh32, DigestAlgorithm::Xxh128] {
            for payload in [0, 7, u128::MAX] {
                let value =
                    TxHash::new_in(0, TimeUnit::Nanosecond, Digest::new(algorithm, payload))
                        .unwrap();
                let Error::InvalidRecord { path, reason } = value.into_uuid().unwrap_err() else {
                    panic!("a wrong-width digest must raise a located value refusal");
                };
                assert_eq!(path, "$.digest");
                assert_eq!(
                    reason,
                    format!(
                        "expected a 64-bit digest for UUIDv7, got {algorithm} ({} bits)",
                        algorithm.width() * 8
                    )
                );
            }
        }
    }

    #[test]
    fn bytes_of_the_wrong_width_or_unit_are_refused() {
        let value = txh3(b"AAPL", INSTANT);
        let bytes = value.into_bytes();
        let short = TxHash::from_bytes(DEFAULT_UNIT, DigestAlgorithm::Xxh3, &bytes[..15]);
        assert!(
            matches!(
                short,
                Err(Error::Parse {
                    target: "txhash",
                    ..
                })
            ),
            "{short:?}"
        );
        let wide = TxHash::from_bytes(DEFAULT_UNIT, DigestAlgorithm::Xxh128, &bytes);
        assert!(
            matches!(
                wide,
                Err(Error::Parse {
                    target: "txhash",
                    ..
                })
            ),
            "{wide:?}"
        );
        let day = TxHash::from_bytes(TimeUnit::Day, DigestAlgorithm::Xxh3, &bytes);
        assert!(
            matches!(day, Err(Error::InvalidDataType { kind: "TxHash", .. })),
            "{day:?}"
        );
        // The same sixteen bytes read under XXH64 are a different value, because
        // the algorithm is part of the value rather than of the bytes.
        let other = TxHash::from_bytes(DEFAULT_UNIT, DigestAlgorithm::Xxh64, &bytes).unwrap();
        assert_ne!(other, value);
        assert_eq!(other.into_bytes(), bytes);
    }

    #[test]
    fn bytes_sort_as_instants_from_the_epoch_on() {
        let mut values: Vec<TxHash> = [3_i64, 1, 2, 0, i64::MAX]
            .into_iter()
            .flat_map(|instant| [txh3(b"AAPL", instant), txh3(b"MSFT", instant)])
            .collect();
        values.sort_unstable();
        let mut bytes: Vec<_> = values.iter().map(|value| value.into_bytes()).collect();
        bytes.sort_unstable();
        assert_eq!(
            bytes,
            values
                .iter()
                .map(|value| value.into_bytes())
                .collect::<Vec<_>>(),
            "value order and byte order agree from the epoch on"
        );
        for pair in values.windows(2) {
            assert!(pair[0].unix() <= pair[1].unix());
        }
        // Before the epoch the value still orders first, and the bytes last: the
        // edge the contract states rather than hides.
        let before = txh3(b"AAPL", -1);
        let epoch = txh3(b"AAPL", 0);
        assert!(before < epoch);
        assert!(before.into_bytes() > epoch.into_bytes());
    }

    #[test]
    fn values_order_by_unit_then_instant_then_digest() {
        let seconds =
            TxHash::new_in(5, TimeUnit::Second, DigestAlgorithm::Xxh3.digest(b"z")).unwrap();
        let micros =
            TxHash::new_in(1, TimeUnit::Microsecond, DigestAlgorithm::Xxh3.digest(b"a")).unwrap();
        assert!(seconds < micros, "the unit ranks first");
        assert!(txh3(b"AAPL", 1) < txh3(b"AAPL", 2));
        let (small, large) = {
            let a = txh3(b"AAPL", 1);
            let b = txh3(b"MSFT", 1);
            if a.digest() < b.digest() {
                (a, b)
            } else {
                (b, a)
            }
        };
        assert!(small < large);
        // The instant ranks before the digest: the earlier value wins even when
        // its digest is the larger of the two.
        let (earlier_payload, later_payload) =
            if txh3(b"AAPL", 1).digest() > txh3(b"MSFT", 1).digest() {
                (b"AAPL", b"MSFT")
            } else {
                (b"MSFT", b"AAPL")
            };
        let earlier = txh3(earlier_payload, 1);
        let later = txh3(later_payload, 2);
        assert!(earlier.digest() > later.digest());
        assert!(earlier < later, "the instant ranks before the digest");
        assert_ne!(
            txh3(b"AAPL", 1),
            txh64(b"AAPL", 1),
            "two algorithms are never equal"
        );
        assert_ne!(
            seconds,
            seconds.with_unit(TimeUnit::Microsecond).unwrap(),
            "two units are never equal"
        );
    }

    #[test]
    fn the_spelling_round_trips_and_names_unit_and_algorithm() {
        let value = txh3(b"AAPL", INSTANT);
        let spelled = value.to_string();
        assert_eq!(
            spelled,
            format!("{INSTANT}@us:{}", DigestAlgorithm::Xxh3.digest(b"AAPL"))
        );
        assert_eq!(TxHash::from_str(&spelled).unwrap(), value);
        assert_eq!(TxHash::from_str(&format!("  {spelled}\n")).unwrap(), value);
        let seconds = value.with_unit(TimeUnit::Second).unwrap();
        assert!(seconds.to_string().contains("@s:"));
        assert_eq!(TxHash::from_str(&seconds.to_string()).unwrap(), seconds);
        let negative = txh128(b"", -42);
        assert!(negative.to_string().starts_with("-42@us:xxh3-128:"));
        assert_eq!(TxHash::from_str(&negative.to_string()).unwrap(), negative);
        // The reference-library spelling of an algorithm is accepted on input.
        assert_eq!(
            TxHash::from_str(&spelled.replace("xxh3-64", "xxh3")).unwrap(),
            value
        );
    }

    #[test]
    fn a_malformed_spelling_is_refused_by_target() {
        let digest = DigestAlgorithm::Xxh3.digest(b"AAPL");
        // Every refusal is a parse error under this value's own target, at the
        // offset of the part that failed in the whole spelling.
        for (spelling, why, at) in [
            ("1700000000000000", "no instant separator", 0),
            (&format!("x@us:{digest}"), "a non-numeric instant", 0),
            (&format!("1@{digest}"), "no unit separator", 2),
            (
                &format!("1@d:{digest}"),
                "a day count is not a clock resolution",
                2,
            ),
            (&format!("1@year_month:{digest}"), "an interval layout", 2),
            ("1@us:xxh3-64:zz", "hex that is not hex", 5),
            ("1@us:md5:00", "an unknown algorithm", 5),
            ("1@bogus:xxh3-64:78af5f94892f3950", "an unknown unit", 2),
        ] {
            let error = TxHash::from_str(spelling).unwrap_err();
            match error {
                Error::Parse {
                    target, position, ..
                } => {
                    assert_eq!(target, "txhash", "{why}");
                    assert!(position >= at, "{why}: position {position} is before {at}");
                }
                other => panic!("{why}: expected a parse error, got {other}"),
            }
        }
    }

    #[test]
    fn serde_uses_the_canonical_spelling() {
        let value = txh64(b"AAPL", INSTANT);
        let json = serde_json::to_string(&value).unwrap();
        assert_eq!(json, format!("{:?}", value.to_string()));
        let back: TxHash = serde_json::from_str(&json).unwrap();
        assert_eq!(back, value);
        assert!(serde_json::from_str::<TxHash>("\"1@d:xxh64:0000000000000000\"").is_err());
    }

    #[test]
    fn with_unit_restates_the_instant_and_keeps_the_digest() {
        let micros = txh3(b"AAPL", 1_700_000_000_999_999);
        let seconds = micros.with_unit(TimeUnit::Second).unwrap();
        assert_eq!(seconds.unix(), 1_700_000_000);
        assert_eq!(seconds.unit(), TimeUnit::Second);
        assert_eq!(seconds.digest(), micros.digest());
        let nanos = seconds.with_unit(TimeUnit::Nanosecond).unwrap();
        assert_eq!(nanos.unix(), 1_700_000_000_000_000_000);
        assert_eq!(
            nanos.with_unit(TimeUnit::Microsecond).unwrap().unix(),
            1_700_000_000_000_000
        );
        let far = txh3(b"", i64::MAX);
        assert!(matches!(
            far.with_unit(TimeUnit::Nanosecond),
            Err(Error::ArithmeticOverflow { .. })
        ));
        assert!(matches!(
            far.with_unit(TimeUnit::DayTime),
            Err(Error::InvalidDataType { kind: "TxHash", .. })
        ));
        assert!(matches!(
            TxHash::new_in(0, TimeUnit::Day, micros.digest()),
            Err(Error::InvalidDataType { kind: "TxHash", .. })
        ));
    }

    #[test]
    fn a_coupled_value_of_every_width_reads_back_through_its_datatype() {
        for (algorithm, value) in every_algorithm() {
            let scalar = value.into_scalar();
            let checked = dtype(algorithm).scalar(scalar.clone()).unwrap();
            assert_eq!(checked, scalar, "{algorithm}");
            assert_eq!(
                Digest::from_bytes(algorithm, &value.into_bytes()[UNIX_WIDTH..]).unwrap(),
                value.digest()
            );
        }
    }

    /// The sixteen ordered bytes carry the whole instant and the whole digest,
    /// and they sort as the instants do - including across the epoch, where the
    /// two's-complement bytes of [`TxHash::into_bytes`] sort the other way.
    ///
    /// This is what a FIX identity column holds, so the ordering is
    /// the column's ordering and the digest is not the lossy 58 bits
    /// [`TxHash::into_uuid`] keeps.
    #[test]
    fn ordered_bytes_lead_with_the_instant_and_keep_every_digest_bit() {
        let value = TxHash::new_in(
            1,
            TimeUnit::Nanosecond,
            Digest::new(DigestAlgorithm::Xxh64, u64::MAX.into()),
        )
        .unwrap();
        let held = value.into_ordered_bytes().unwrap();
        // The instant leads, its sign bit flipped so that negatives sort first.
        assert_eq!(&held[..8], &[0x80, 0, 0, 0, 0, 0, 0, 1]);
        // Every one of the digest's 64 bits survives, where a UUID keeps 58.
        assert_eq!(&held[8..], &[0xff; 8]);
        assert_ne!(
            held,
            value.into_uuid().unwrap().into_bytes(),
            "the uuid spends six of those bits on its version and variant"
        );

        // The bytes order as the instants do, across the epoch.
        let mut sorted: Vec<[u8; 16]> = [1_i64, -1, 0, i64::MIN, i64::MAX]
            .into_iter()
            .map(|unix| {
                TxHash::new_in(unix, TimeUnit::Nanosecond, value.digest())
                    .unwrap()
                    .into_ordered_bytes()
                    .unwrap()
            })
            .collect();
        sorted.sort_unstable();
        let instants: Vec<i64> = sorted
            .iter()
            .map(|held| {
                let mut bytes = [0_u8; 8];
                bytes.copy_from_slice(&held[..8]);
                i64::from_be_bytes((u64::from_be_bytes(bytes) ^ (1 << 63)).to_be_bytes())
            })
            .collect();
        assert_eq!(instants, [i64::MIN, -1, 0, 1, i64::MAX]);

        // A digest of another width has nothing to lay down, and says so where
        // `into_uuid` says it too.
        let wide = TxHash::new_in(
            0,
            TimeUnit::Nanosecond,
            Digest::new(DigestAlgorithm::Xxh128, 1),
        )
        .unwrap();
        let refused = wide.into_ordered_bytes().expect_err("a 128-bit digest");
        assert!(
            matches!(&refused, Error::InvalidRecord { path, .. } if path == "$.digest"),
            "{refused}"
        );

        // An instant no signed nanosecond count can hold refuses the same way
        // `into_uuid` refuses it.
        let far = TxHash::new_in(i64::MAX, TimeUnit::Second, value.digest()).unwrap();
        assert!(far.into_ordered_bytes().is_err());
    }
}
