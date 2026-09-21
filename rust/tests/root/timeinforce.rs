//! `rust/src/timeinforce.rs`: the coded datatypes FIX's constant vocabulary
//! earns.
//!
//! `DataType` is `#[non_exhaustive]` and the datatype layer carries some sixty
//! wildcard arms, so a new variant compiles clean while behaving wrongly. A
//! green build proves nothing; these are the invariants a wildcard cannot
//! satisfy by accident.

mod coded {

    use yggdryl::{DataType, DataTypeKind, Field, Scalar};

    #[test]
    fn the_state_and_time_in_force_codes_are_ordinary_datatypes_everywhere_else() {
        for (name, dtype, width) in [
            ("state", DataType::State, 10),
            ("timeinforce", DataType::TimeInForce, 8),
        ] {
            // Parsed, displayed and round-tripped by the grammar like any other.
            assert_eq!(DataType::from_str(name).unwrap(), dtype);
            assert_eq!(dtype.to_string(), name);
            assert_eq!(dtype.kind(), DataTypeKind::Code);
            assert!(dtype.is_code());
            assert_eq!(dtype.code_width(), Some(width));

            // And it crosses Arrow as the text it is, extension name and all, so
            // a column round-trips without becoming anonymous text.
            let field = Field::new(name, dtype.clone(), true);
            let recovered =
                Field::from_arrow_field(&field.clone().into_arrow_field().unwrap()).unwrap();
            assert_eq!(recovered, field);
        }

        // A value wider than the code's own width is refused by the datatype
        // rather than truncated into something that reads.
        assert!(DataType::State.scalar(Scalar::from("20NEW")).is_ok());
        assert!(DataType::State.scalar(Scalar::from("40PARTFILL")).is_ok());
        assert!(
            DataType::State
                .scalar(Scalar::from("80CALCULATED"))
                .is_err()
        );
        assert!(DataType::TimeInForce.scalar(Scalar::from("0")).is_ok());
    }
}
