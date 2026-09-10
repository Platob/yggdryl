use super::value::algorithm_of_width;
use super::{
    DEFAULT_UNIT, TxHash, TxHasher, UNIX_WIDTH, digest, dtype, restate_unix, txh3, txh32, txh64,
    txh128, unix_from_scalar, unix_now, width,
};
use crate::xxhash::{self, Xxh3};
use crate::{DataType, Digest, DigestAlgorithm, Error, Field, Scalar, TimeUnit, Timezone};

const INSTANT: i64 = 1_700_000_000_000_000;

/// One instant, one payload, every algorithm.
fn every_algorithm() -> impl Iterator<Item = (DigestAlgorithm, TxHash)> {
    DigestAlgorithm::ALL
        .into_iter()
        .map(|algorithm| (algorithm, digest(b"AAPL", INSTANT, algorithm)))
}

#[test]
fn one_shots_couple_the_instant_with_the_plain_digest() {
    for (algorithm, value) in every_algorithm() {
        assert_eq!(value.unix(), INSTANT, "{algorithm}");
        assert_eq!(value.unit(), DEFAULT_UNIT, "{algorithm}");
        assert_eq!(value.digest(), algorithm.digest(b"AAPL"), "{algorithm}");
        assert_eq!(value.algorithm(), algorithm);
        assert_eq!(value.width(), UNIX_WIDTH + algorithm.width());
        assert_eq!(value.dtype(), dtype(algorithm));
    }
    assert_eq!(
        txh32(b"AAPL", 1),
        digest(b"AAPL", 1, DigestAlgorithm::Xxh32)
    );
    assert_eq!(
        txh64(b"AAPL", 1),
        digest(b"AAPL", 1, DigestAlgorithm::Xxh64)
    );
    assert_eq!(txh3(b"AAPL", 1), digest(b"AAPL", 1, DigestAlgorithm::Xxh3));
    assert_eq!(
        txh128(b"AAPL", 1),
        digest(b"AAPL", 1, DigestAlgorithm::Xxh128)
    );
    assert_eq!(DEFAULT_UNIT, TimeUnit::Microsecond);
}

#[test]
fn widths_and_datatypes_follow_the_algorithm() {
    assert_eq!(width(DigestAlgorithm::Xxh32), 12);
    assert_eq!(width(DigestAlgorithm::Xxh64), 16);
    assert_eq!(width(DigestAlgorithm::Xxh3), 16);
    assert_eq!(width(DigestAlgorithm::Xxh128), 24);
    assert_eq!(dtype(DigestAlgorithm::Xxh32), DataType::FixedSizeBinary(12));
    assert_eq!(
        dtype(DigestAlgorithm::Xxh128),
        DataType::FixedSizeBinary(24)
    );
    assert_eq!(algorithm_of_width(12), Some(DigestAlgorithm::Xxh32));
    // Sixteen bytes answer the project default, not XXH64.
    assert_eq!(algorithm_of_width(16), Some(DigestAlgorithm::Xxh3));
    assert_eq!(algorithm_of_width(24), Some(DigestAlgorithm::Xxh128));
    assert_eq!(algorithm_of_width(8), None);
    assert_eq!(algorithm_of_width(20), None);
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
        TxHash::from_bytes(DEFAULT_UNIT, DigestAlgorithm::Xxh3, &negative.into_bytes()).unwrap(),
        negative
    );
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
    let seconds = TxHash::new_in(5, TimeUnit::Second, DigestAlgorithm::Xxh3.digest(b"z")).unwrap();
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
    let (earlier_payload, later_payload) = if txh3(b"AAPL", 1).digest() > txh3(b"MSFT", 1).digest()
    {
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
fn restate_unix_floors_to_coarser_and_scales_to_finer() {
    use TimeUnit::{Day, Microsecond, Millisecond, Nanosecond, Second};
    let cases: [(i64, TimeUnit, TimeUnit, i64); 10] = [
        (1_999, Nanosecond, Microsecond, 1),
        (-1, Nanosecond, Microsecond, -1),
        (-1_000, Nanosecond, Microsecond, -1),
        (-1_001, Nanosecond, Microsecond, -2),
        (1_999, Millisecond, Second, 1),
        (7, Second, Second, 7),
        (1, Second, Nanosecond, 1_000_000_000),
        (1, Day, Second, 86_400),
        (-1, Day, Millisecond, -86_400_000),
        (
            1_700_000_000_123,
            Millisecond,
            Microsecond,
            1_700_000_000_123_000,
        ),
    ];
    for (count, from, into, expected) in cases {
        assert_eq!(
            restate_unix(count, from, into).unwrap(),
            expected,
            "{count} {from} -> {into}"
        );
    }
    assert!(matches!(
        restate_unix(i64::MAX, Second, Millisecond),
        Err(Error::ArithmeticOverflow { .. })
    ));
    assert!(matches!(
        restate_unix(1, TimeUnit::YearMonth, Second),
        Err(Error::InvalidDataType { kind: "TxHash", .. })
    ));
    assert!(matches!(
        restate_unix(1, Second, Day),
        Err(Error::InvalidDataType { kind: "TxHash", .. })
    ));
}

#[test]
fn unix_from_scalar_reads_every_instant_spelling() {
    let unit = TimeUnit::Microsecond;
    let read = |value: Scalar| unix_from_scalar(&value, unit).unwrap();
    assert_eq!(read(Scalar::from(7_i64)), 7);
    assert_eq!(read(Scalar::from(7_u8)), 7);
    assert_eq!(read(Scalar::from(-7_i32)), -7);
    // A zoned datetime already counts from the epoch; the zone moves nothing.
    let kolkata = Timezone::from_str("Asia/Kolkata").unwrap();
    assert_eq!(
        read(Scalar::from_datetime(1_700_000_000, TimeUnit::Second, kolkata).unwrap()),
        1_700_000_000_000_000
    );
    // A naive datetime is read as if it were UTC.
    assert_eq!(
        read(
            Scalar::from_datetime(1_700_000_000_000, TimeUnit::Millisecond, Timezone::NAIVE)
                .unwrap()
        ),
        1_700_000_000_000_000
    );
    assert_eq!(
        read(Scalar::from_datetime(1_999, TimeUnit::Nanosecond, Timezone::UTC).unwrap()),
        1
    );
    assert_eq!(read(Scalar::date32(1)), 86_400_000_000);
    assert_eq!(read(Scalar::date64(1_500)), 1_500_000);
    assert_eq!(read(Scalar::from("1970-01-01T00:00:01Z")), 1_000_000);
    assert_eq!(
        read(Scalar::from("1970-01-01T00:00:01+01:00")),
        -3_599_000_000
    );
    assert_eq!(read(Scalar::from("1970-01-01T00:00:01")), 1_000_000);
    assert_eq!(read(Scalar::from("1970-01-01T00:00:01.5")), 1_500_000);
    assert_eq!(read(Scalar::from("1970-01-01T00:00:00.0000019")), 1);
    // Date-only text is that day's midnight, as a date scalar is.
    assert_eq!(read(Scalar::from("1970-01-02")), 86_400_000_000);
    assert_eq!(
        unix_from_scalar(&Scalar::from(1_700_000_000_i64), TimeUnit::Second).unwrap(),
        1_700_000_000
    );
    // Text is read at the resolution its digits spell, so an instant past
    // what nanoseconds count is still a count of seconds ...
    assert_eq!(
        unix_from_scalar(&Scalar::from("2400-01-01T00:00:00Z"), TimeUnit::Second).unwrap(),
        13_569_465_600
    );
    // ... until its digits name nanoseconds.
    assert!(matches!(
        unix_from_scalar(
            &Scalar::from("2400-01-01T00:00:00.1234567Z"),
            TimeUnit::Second
        ),
        Err(Error::Parse { .. })
    ));

    for (value, why) in [
        (Scalar::Null, "a null"),
        (Scalar::from(true), "a boolean"),
        (Scalar::from(1.5_f64), "a float"),
        (
            Scalar::from_time(1, TimeUnit::Second, Timezone::NAIVE).unwrap(),
            "a time of day",
        ),
        (
            Scalar::from_duration(1, TimeUnit::Second, Timezone::NAIVE).unwrap(),
            "a duration",
        ),
        (Scalar::from_sequence([Scalar::from(1)]), "a sequence"),
    ] {
        assert!(
            matches!(
                unix_from_scalar(&value, unit),
                Err(Error::InvalidRecord { .. })
            ),
            "{why}"
        );
    }
    assert!(
        matches!(
            unix_from_scalar(&Scalar::from("yesterday"), unit),
            Err(Error::Parse { .. })
        ),
        "text that is no timestamp"
    );
    assert!(matches!(
        unix_from_scalar(&Scalar::from(i128::MAX), unit),
        Err(Error::ArithmeticOverflow { .. })
    ));
    assert!(matches!(
        unix_from_scalar(&Scalar::from(1), TimeUnit::Day),
        Err(Error::InvalidDataType { kind: "TxHash", .. })
    ));
}

#[test]
fn unix_now_counts_forward_at_every_resolution() {
    let seconds = unix_now(TimeUnit::Second).unwrap();
    let millis = unix_now(TimeUnit::Millisecond).unwrap();
    let micros = unix_now(TimeUnit::Microsecond).unwrap();
    let nanos = unix_now(TimeUnit::Nanosecond).unwrap();
    assert!(seconds > 1_700_000_000, "the clock reads after 2023");
    assert!(millis / 1_000 >= seconds);
    assert!(micros / 1_000_000 >= seconds);
    assert!(nanos / 1_000_000_000 >= seconds);
    assert!(matches!(
        unix_now(TimeUnit::Day),
        Err(Error::InvalidDataType { kind: "TxHash", .. })
    ));
}

#[test]
fn a_value_projects_to_a_datetime_and_a_fixed_byte_scalar() {
    let value = txh3(b"AAPL", INSTANT);
    assert_eq!(
        value.into_datetime(),
        Scalar::from_datetime(INSTANT, TimeUnit::Microsecond, Timezone::UTC).unwrap()
    );
    let seconds = value.with_unit(TimeUnit::Second).unwrap();
    assert_eq!(
        seconds.into_datetime(),
        Scalar::from_datetime(1_700_000_000, TimeUnit::Second, Timezone::UTC).unwrap()
    );
    let scalar = value.into_scalar();
    assert_eq!(scalar.as_bytes(), Some(&*value.into_bytes()));
    assert_eq!(scalar.dtype().unwrap(), DataType::FixedSizeBinary(16));
    assert_eq!(
        TxHash::from_scalar(DEFAULT_UNIT, DigestAlgorithm::Xxh3, &scalar).unwrap(),
        value
    );
    assert_eq!(
        TxHash::from_scalar(
            DEFAULT_UNIT,
            DigestAlgorithm::Xxh3,
            &Scalar::from(value.to_string())
        )
        .unwrap(),
        value
    );
    // A spelling naming another unit or algorithm is refused rather than
    // silently restated.
    assert!(
        TxHash::from_scalar(
            TimeUnit::Second,
            DigestAlgorithm::Xxh3,
            &Scalar::from(value.to_string())
        )
        .is_err()
    );
    assert!(
        TxHash::from_scalar(
            DEFAULT_UNIT,
            DigestAlgorithm::Xxh64,
            &Scalar::from(value.to_string())
        )
        .is_err()
    );
    assert!(TxHash::from_scalar(DEFAULT_UNIT, DigestAlgorithm::Xxh3, &Scalar::from(1)).is_err());
    assert!(
        TxHash::from_scalar(
            DEFAULT_UNIT,
            DigestAlgorithm::Xxh3,
            &Scalar::from(&[0_u8; 8])
        )
        .is_err()
    );
}

#[test]
fn a_scalar_couples_its_own_digest() {
    let value = Scalar::from_sequence([Scalar::from("AAPL"), Scalar::from(100)]);
    for algorithm in DigestAlgorithm::ALL {
        let coupled = value.txhash(INSTANT, algorithm);
        assert_eq!(coupled.unix(), INSTANT);
        assert_eq!(coupled.unit(), DEFAULT_UNIT);
        assert_eq!(coupled.digest(), value.digest(algorithm));
    }
    // Equal values answer one coupled value across widths.
    assert_eq!(
        Scalar::from(1_i8).txhash(1, DigestAlgorithm::Xxh3),
        Scalar::from(1_i64).txhash(1, DigestAlgorithm::Xxh3)
    );
    let field = Field::new("id", DataType::Int64, false);
    let typed = crate::TypedScalar::new(&field, 1_i64).unwrap();
    assert_eq!(
        typed.txhash(1, DigestAlgorithm::Xxh3),
        Scalar::from(1_i64).txhash(1, DigestAlgorithm::Xxh3)
    );
    let row = DataType::from_fields([field.clone()])
        .unwrap()
        .required_field("row");
    let record =
        crate::TypedRecord::new(&row, Scalar::from_sequence([Scalar::from(1_i64)])).unwrap();
    assert_eq!(
        record.txhash(1, DigestAlgorithm::Xxh3),
        Scalar::from_sequence([Scalar::from(1_i64)]).txhash(1, DigestAlgorithm::Xxh3)
    );
}

#[test]
fn a_hasher_carries_unit_seed_and_secret() {
    let plain = TxHasher::new(DigestAlgorithm::Xxh3);
    assert_eq!(plain.unit(), DEFAULT_UNIT);
    assert_eq!(plain.algorithm(), DigestAlgorithm::Xxh3);
    assert_eq!(plain.width(), 16);
    assert_eq!(plain.dtype(), DataType::FixedSizeBinary(16));
    assert_eq!(plain.digest(b"AAPL", INSTANT), txh3(b"AAPL", INSTANT));
    assert_eq!(
        plain.digest_scalar(&Scalar::from("AAPL"), INSTANT),
        Scalar::from("AAPL").txhash(INSTANT, DigestAlgorithm::Xxh3)
    );
    assert_eq!(plain.digester().algorithm(), DigestAlgorithm::Xxh3);

    let seconds = TxHasher::new_in(TimeUnit::Second, DigestAlgorithm::Xxh64)
        .unwrap()
        .with_seed(7);
    let value = seconds.digest(b"AAPL", 1_700_000_000);
    assert_eq!(value.unit(), TimeUnit::Second);
    assert_eq!(value.unix(), 1_700_000_000);
    assert_eq!(
        value.digest().as_u64(),
        Some(xxhash::xxh64_with_seed(b"AAPL", 7))
    );
    assert_eq!(
        seconds
            .unix_of(&Scalar::from_datetime(INSTANT, TimeUnit::Microsecond, Timezone::UTC).unwrap())
            .unwrap(),
        1_700_000_000
    );
    assert!(matches!(
        TxHasher::new_in(TimeUnit::Day, DigestAlgorithm::Xxh3),
        Err(Error::InvalidDataType { kind: "TxHash", .. })
    ));

    // A secret travels through a configured state, and bytes fed to that
    // state before it became a prototype are forgotten.
    let secret = vec![0x5a_u8; xxhash::SECRET_MINIMUM_LENGTH];
    let mut state = Xxh3::from_seed_and_secret(3, &secret).unwrap();
    state.write_bytes(b"already fed");
    let secretive = TxHasher::from_digester(TimeUnit::Millisecond, state.into()).unwrap();
    let long = vec![0x11_u8; 241];
    assert_eq!(
        secretive.digest(&long, 5).digest().as_u64(),
        Some(xxhash::xxh3_with_seed_and_secret(&long, 3, &secret).unwrap())
    );
    assert_eq!(secretive.unit(), TimeUnit::Millisecond);
    // Answering never changes the hasher.
    assert_eq!(secretive.digest(&long, 5), secretive.digest(&long, 5));
    // A seed is a whole configuration: it does not keep the secret.
    let reseeded = secretive.with_seed(9);
    assert_eq!(reseeded.unit(), TimeUnit::Millisecond);
    assert_eq!(
        reseeded.digest(&long, 5).digest().as_u64(),
        Some(xxhash::xxh3_with_seed(&long, 9))
    );
    assert!(TxHasher::from_digester(TimeUnit::Day, DigestAlgorithm::Xxh3.digester()).is_err());
}

#[test]
fn every_state_becomes_the_dispatcher_it_names() {
    let mut expected = crate::xxhash::Xxh32::with_seed(9);
    expected.write_bytes(b"abc");
    let dispatcher: crate::Digester = expected.clone().into();
    assert_eq!(dispatcher.as_digest(), expected.as_digest());
    assert_eq!(
        crate::Digester::from(crate::xxhash::Xxh64::new()).algorithm(),
        DigestAlgorithm::Xxh64
    );
    assert_eq!(
        crate::Digester::from(crate::xxhash::Xxh128::new()).algorithm(),
        DigestAlgorithm::Xxh128
    );
}

#[test]
fn stable_hash_is_deterministic_and_value_wide() {
    let value = txh3(b"AAPL", INSTANT);
    assert_eq!(value.stable_hash(), value.stable_hash());
    assert_ne!(
        value.stable_hash(),
        txh3(b"AAPL", INSTANT + 1).stable_hash()
    );
    assert_ne!(
        value.stable_hash(),
        value.with_unit(TimeUnit::Second).unwrap().stable_hash()
    );
}

fn coupled_holder(dtype: DataType) -> Field {
    let mut field = Field::new("key", dtype, false);
    field.as_digest_mut().set_holder().unwrap();
    field
}

#[test]
fn the_digest_protocol_couples_a_holder_with_an_instant() {
    let mut holder = coupled_holder(DataType::FixedSizeBinary(16));
    assert!(!holder.as_digest().is_coupled());
    assert_eq!(holder.as_digest().time(), None);
    assert_eq!(holder.as_digest().unit().unwrap(), None);
    assert_eq!(holder.as_digest().coupled_unit().unwrap(), DEFAULT_UNIT);

    holder.as_digest_mut().set_time("event").unwrap();
    assert!(holder.as_digest().is_coupled());
    assert_eq!(holder.as_digest().time(), Some("event"));
    assert_eq!(holder.get_metadata("digest:time"), Some("event"));
    assert_eq!(holder.as_digest().coupled_unit().unwrap(), DEFAULT_UNIT);

    holder.as_digest_mut().set_unit(TimeUnit::Second).unwrap();
    assert_eq!(holder.as_digest().unit().unwrap(), Some(TimeUnit::Second));
    assert_eq!(holder.as_digest().coupled_unit().unwrap(), TimeUnit::Second);
    assert_eq!(holder.get_metadata("digest:unit"), Some("s"));

    // Sixteen coupled bytes hold a 64-bit digest of either family, never the
    // 128-bit one.
    holder
        .as_digest_mut()
        .set_algorithm(DigestAlgorithm::Xxh64)
        .unwrap();
    assert!(
        holder
            .as_digest_mut()
            .set_algorithm(DigestAlgorithm::Xxh128)
            .is_err()
    );
    assert!(
        holder
            .as_digest_mut()
            .set_algorithm(DigestAlgorithm::Xxh32)
            .is_err()
    );
    holder.as_digest_mut().remove_algorithm();

    // Removal order: the unit before the instant, the instant before the role.
    assert!(holder.as_digest_mut().remove_time().is_err());
    assert!(holder.as_digest_mut().remove_role().is_err());
    assert_eq!(holder.as_digest_mut().remove_unit(), Some("s".to_owned()));
    assert!(holder.as_digest_mut().remove_role().is_err());
    assert_eq!(
        holder.as_digest_mut().remove_time().unwrap(),
        Some("event".to_owned())
    );
    assert!(!holder.as_digest().is_coupled());
    holder.as_digest_mut().remove_role().unwrap();
    assert!(!holder.as_digest().is_holder());
}

#[test]
fn coupling_refuses_the_wrong_storage_role_and_spelling() {
    // A plain integer holder cannot store an instant in front of its digest.
    let mut narrow = coupled_holder(DataType::UInt64);
    let refused = narrow.as_digest_mut().set_time("event").unwrap_err();
    assert!(
        matches!(&refused, Error::InvalidMetadataValue { key, .. } if key == "digest:time"),
        "{refused}"
    );
    assert_eq!(
        narrow.as_digest().time(),
        None,
        "refusal leaves the field unchanged"
    );

    // A declared algorithm pins the coupled width.
    let mut pinned = coupled_holder(DataType::FixedSizeBinary(16));
    pinned
        .as_digest_mut()
        .set_algorithm(DigestAlgorithm::Xxh128)
        .unwrap();
    assert!(pinned.as_digest_mut().set_time("event").is_err());
    let mut widened = coupled_holder(DataType::FixedSizeBinary(24));
    assert!(
        widened
            .as_digest_mut()
            .set_algorithm(DigestAlgorithm::Xxh128)
            .is_err()
    );
    widened.as_digest_mut().set_time("event").unwrap();
    widened
        .as_digest_mut()
        .set_algorithm(DigestAlgorithm::Xxh128)
        .unwrap();

    // Twenty bytes are no coupled width.
    let mut odd = coupled_holder(DataType::FixedSizeBinary(20));
    assert!(odd.as_digest_mut().set_time("event").is_err());

    // Not a holder at all.
    let mut plain = Field::new("event", DataType::FixedSizeBinary(16), false);
    assert!(plain.as_digest_mut().set_time("event").is_err());
    assert!(plain.as_digest_mut().set_unit(TimeUnit::Second).is_err());

    // The unit needs the instant, and only a clock resolution is one.
    let mut holder = coupled_holder(DataType::FixedSizeBinary(12));
    assert!(holder.as_digest_mut().set_unit(TimeUnit::Second).is_err());
    holder.as_digest_mut().set_time("event").unwrap();
    assert!(holder.as_digest_mut().set_unit(TimeUnit::Day).is_err());
    assert!(
        holder
            .as_digest_mut()
            .set_unit(TimeUnit::MonthDayNano)
            .is_err()
    );
    assert_eq!(holder.as_digest().unit().unwrap(), None);

    // The path is one non-empty name, never the select-everything spelling.
    assert!(holder.as_digest_mut().set_time("").is_err());
    assert!(holder.as_digest_mut().set_time("*").is_err());
    assert_eq!(holder.as_digest().time(), Some("event"));
}

#[test]
fn stored_coupling_metadata_is_validated_and_canonicalized_on_write() {
    let mut holder = coupled_holder(DataType::FixedSizeBinary(16));
    holder.as_digest_mut().set_time("event").unwrap();
    // The raw property route canonicalizes a unit spelling the way every
    // typed key is, and refuses what is no clock resolution.
    holder.as_digest_mut().insert("unit", "micros").unwrap();
    assert_eq!(holder.get_metadata("digest:unit"), Some("us"));
    holder
        .as_digest_mut()
        .insert("unit", "Milliseconds")
        .unwrap();
    assert_eq!(
        holder.as_digest().unit().unwrap(),
        Some(TimeUnit::Millisecond)
    );
    let refused = holder.as_digest_mut().insert("unit", "day").unwrap_err();
    assert!(
        matches!(&refused, Error::InvalidMetadataValue { key, .. } if key == "digest:unit"),
        "{refused}"
    );
    assert!(holder.as_digest_mut().insert("unit", "fortnight").is_err());
    assert_eq!(
        holder.as_digest().unit().unwrap(),
        Some(TimeUnit::Millisecond)
    );
    let refused = holder.as_digest_mut().insert("time", "").unwrap_err();
    assert!(
        matches!(&refused, Error::InvalidMetadataValue { key, .. } if key == "digest:time"),
        "{refused}"
    );
    assert!(holder.as_digest_mut().insert("time", "*").is_err());
    assert_eq!(holder.as_digest().time(), Some("event"));
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
