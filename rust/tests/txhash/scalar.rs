//! `rust/src/txhash/scalar.rs`: a value's own time-coupled digest.

mod txhash {
    use yggdryl::txhash::{DEFAULT_UNIT, TxHash, txh3};

    use yggdryl::{DataType, DigestAlgorithm, Field, Scalar, StructType, TimeUnit, Timezone};

    const INSTANT: i64 = 1_700_000_000_000_000;

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
        assert_eq!(scalar.dtype().unwrap(), DataType::fixed_binary(16).unwrap());
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
        assert!(
            TxHash::from_scalar(DEFAULT_UNIT, DigestAlgorithm::Xxh3, &Scalar::from(1)).is_err()
        );
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
        let typed = yggdryl::FieldScalar::new(&field, 1_i64).unwrap();
        assert_eq!(
            typed.txhash(1, DigestAlgorithm::Xxh3),
            Scalar::from(1_i64).txhash(1, DigestAlgorithm::Xxh3)
        );
        let row = StructType::from_fields([field.clone()])
            .map(DataType::from)
            .unwrap()
            .required_field("row");
        let record =
            yggdryl::FieldRecord::new(&row, Scalar::from_sequence([Scalar::from(1_i64)])).unwrap();
        assert_eq!(
            record.txhash(1, DigestAlgorithm::Xxh3),
            Scalar::from_sequence([Scalar::from(1_i64)]).txhash(1, DigestAlgorithm::Xxh3)
        );
    }
}
