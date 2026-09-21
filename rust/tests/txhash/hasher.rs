//! `rust/src/txhash/hasher.rs`: one configuration - resolution, algorithm,
//! seed, secret - answering many values.

mod txhash {
    use yggdryl::txhash::{DEFAULT_UNIT, TxHasher, txh3};
    use yggdryl::xxhash::{self, Xxh3};
    use yggdryl::{DataType, DigestAlgorithm, Error, Scalar, TimeUnit, Timezone};

    const INSTANT: i64 = 1_700_000_000_000_000;

    #[test]
    fn a_hasher_carries_unit_seed_and_secret() {
        let plain = TxHasher::new(DigestAlgorithm::Xxh3);
        assert_eq!(plain.unit(), DEFAULT_UNIT);
        assert_eq!(plain.algorithm(), DigestAlgorithm::Xxh3);
        assert_eq!(plain.width(), 16);
        assert_eq!(plain.dtype(), DataType::fixed_binary(16).unwrap());
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
                .unix_of(
                    &Scalar::from_datetime(INSTANT, TimeUnit::Microsecond, Timezone::UTC).unwrap()
                )
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
        let mut expected = yggdryl::xxhash::Xxh32::with_seed(9);
        expected.write_bytes(b"abc");
        let dispatcher: yggdryl::Digester = expected.clone().into();
        assert_eq!(dispatcher.as_digest(), expected.as_digest());
        assert_eq!(
            yggdryl::Digester::from(yggdryl::xxhash::Xxh64::new()).algorithm(),
            DigestAlgorithm::Xxh64
        );
        assert_eq!(
            yggdryl::Digester::from(yggdryl::xxhash::Xxh128::new()).algorithm(),
            DigestAlgorithm::Xxh128
        );
    }
}
