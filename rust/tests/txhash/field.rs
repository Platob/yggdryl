//! `rust/src/txhash/field.rs`: the two properties that make a digest holder
//! couple an instant.

mod txhash {
    use yggdryl::txhash::DEFAULT_UNIT;

    use yggdryl::{DataType, DigestAlgorithm, Error, Field, TimeUnit};

    fn coupled_holder(dtype: DataType) -> Field {
        let mut field = Field::new("key", dtype, false);
        field.as_digest_mut().set_holder().unwrap();
        field
    }

    #[test]
    fn the_digest_protocol_couples_a_holder_with_an_instant() {
        let mut holder = coupled_holder(DataType::fixed_binary(16).unwrap());
        assert!(!holder.as_digest().is_coupled());
        assert_eq!(holder.as_digest().time(), None);
        assert_eq!(holder.as_digest().unit().unwrap(), None);
        assert_eq!(holder.as_digest().coupled_unit().unwrap(), DEFAULT_UNIT);

        holder.as_digest_mut().set_time("event").unwrap();
        assert!(holder.as_digest().is_coupled());
        assert_eq!(holder.as_digest().time(), Some("event"));
        assert_eq!(holder.get_metadata("DIGEST:time"), Some("event"));
        assert_eq!(holder.as_digest().coupled_unit().unwrap(), DEFAULT_UNIT);

        holder.as_digest_mut().set_unit(TimeUnit::Second).unwrap();
        assert_eq!(holder.as_digest().unit().unwrap(), Some(TimeUnit::Second));
        assert_eq!(holder.as_digest().coupled_unit().unwrap(), TimeUnit::Second);
        assert_eq!(holder.get_metadata("DIGEST:unit"), Some("s"));

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
            matches!(&refused, Error::InvalidMetadataValue { key, .. } if key == "DIGEST:time"),
            "{refused}"
        );
        assert_eq!(
            narrow.as_digest().time(),
            None,
            "refusal leaves the field unchanged"
        );

        // A declared algorithm pins the coupled width.
        let mut pinned = coupled_holder(DataType::fixed_binary(16).unwrap());
        pinned
            .as_digest_mut()
            .set_algorithm(DigestAlgorithm::Xxh128)
            .unwrap();
        assert!(pinned.as_digest_mut().set_time("event").is_err());
        let mut widened = coupled_holder(DataType::fixed_binary(24).unwrap());
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
        let mut odd = coupled_holder(DataType::fixed_binary(20).unwrap());
        assert!(odd.as_digest_mut().set_time("event").is_err());

        // Not a holder at all.
        let mut plain = Field::new("event", DataType::fixed_binary(16).unwrap(), false);
        assert!(plain.as_digest_mut().set_time("event").is_err());
        assert!(plain.as_digest_mut().set_unit(TimeUnit::Second).is_err());

        // The unit needs the instant, and only a clock resolution is one.
        let mut holder = coupled_holder(DataType::fixed_binary(12).unwrap());
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
        let mut holder = coupled_holder(DataType::fixed_binary(16).unwrap());
        holder.as_digest_mut().set_time("event").unwrap();
        // The raw property route canonicalizes a unit spelling the way every
        // typed key is, and refuses what is no clock resolution.
        holder.as_digest_mut().insert("unit", "micros").unwrap();
        assert_eq!(holder.get_metadata("DIGEST:unit"), Some("us"));
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
            matches!(&refused, Error::InvalidMetadataValue { key, .. } if key == "DIGEST:unit"),
            "{refused}"
        );
        assert!(holder.as_digest_mut().insert("unit", "fortnight").is_err());
        assert_eq!(
            holder.as_digest().unit().unwrap(),
            Some(TimeUnit::Millisecond)
        );
        let refused = holder.as_digest_mut().insert("time", "").unwrap_err();
        assert!(
            matches!(&refused, Error::InvalidMetadataValue { key, .. } if key == "DIGEST:time"),
            "{refused}"
        );
        assert!(holder.as_digest_mut().insert("time", "*").is_err());
        assert_eq!(holder.as_digest().time(), Some("event"));
    }
}
