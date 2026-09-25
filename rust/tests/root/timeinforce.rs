//! `rust/src/timeinforce.rs`: the coded datatypes FIX's constant vocabulary
//! earns.
//!
//! `DataType` is `#[non_exhaustive]` and the datatype layer carries some sixty
//! wildcard arms, so a new variant compiles clean while behaving wrongly. A
//! green build proves nothing; these are the invariants a wildcard cannot
//! satisfy by accident.

mod coded {

    use std::path::PathBuf;

    use yggdryl::{
        DataType, DataTypeKind, Field, Scalar, StringEnum, TIMEINFORCE_CODES, TimeInForce,
    };

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

    #[test]
    fn the_code_table_is_the_shipped_code_set_and_a_spelling_stores_the_code() {
        // The table agrees with the registry the crate ships, entry for entry
        // and in its order, so a name reads to the code the dictionary gives.
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../config/fix/codesets/timeinforcecodeset.json");
        let shipped: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let shipped: Vec<(&str, &str)> = shipped["codes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|code| {
                (
                    code["value"].as_str().unwrap(),
                    code["name"].as_str().unwrap(),
                )
            })
            .collect();
        assert_eq!(shipped, TIMEINFORCE_CODES.to_vec());
        // The listing is the same set of wire values, sorted.
        let mut listed: Vec<&str> = TIMEINFORCE_CODES.iter().map(|(code, _)| *code).collect();
        listed.sort_unstable();
        assert_eq!(listed.as_slice(), StringEnum::TIMESINFORCE);

        // A code, a name folded, or a stored value kept as stated: the value
        // stored is the code.
        for (spelling, stored) in [
            ("day", "0"),
            ("GoodTillCancel", "1"),
            ("1", "1"),
            ("GTX", "GTX"),
            ("good_till_cancel", "1"),
            ("GOOD TILL CANCEL", "1"),
            ("C", "C"),
            ("GoodForMonth", "C"),
            // A wire letter does not fold; held as the venue value it is.
            ("c", "c"),
            ("D", "D"),
        ] {
            assert_eq!(
                TimeInForce::from_spelling(spelling).unwrap().as_str(),
                stored,
                "{spelling}"
            );
            assert_eq!(TimeInForce::read(spelling).unwrap().as_str(), stored);
        }
        for (code, name) in TIMEINFORCE_CODES {
            assert_eq!(TimeInForce::from_spelling(code).unwrap().as_str(), code);
            assert_eq!(TimeInForce::from_spelling(name).unwrap().as_str(), code);
        }

        // A ninth byte names the field it cannot fit.
        assert!(TimeInForce::from_spelling("TOOLONGTIF").is_none());
        let refused = TimeInForce::read("TOOLONGTIF").unwrap_err().to_string();
        assert!(refused.contains("TimeInForce(59)"), "{refused}");
        assert!(refused.contains("8"), "{refused}");
        assert!(refused.contains("TOOLONGTIF"), "{refused}");
        // Eight bytes of a venue's own is the most a value may be, and is held.
        assert_eq!(TimeInForce::read("GTXGTXGT").unwrap().as_str(), "GTXGTXGT");
    }
}
