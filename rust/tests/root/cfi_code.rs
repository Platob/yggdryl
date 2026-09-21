//! `rust/src/cfi_code.rs`: the coded datatypes FIX's constant vocabulary earns.
//!
//! `DataType` is `#[non_exhaustive]` and the datatype layer carries some sixty
//! wildcard arms, so a new variant compiles clean while behaving wrongly. A
//! green build proves nothing; these are the invariants a wildcard cannot
//! satisfy by accident.

mod coded {

    use arrow_array::{Array, StringArray};

    use yggdryl::arrow::{scalar_array, scalar_value};
    use yggdryl::{DataType, Field, Scalar};

    #[test]
    fn a_cfi_stores_the_six_characters_it_is_and_nothing_beside_them() {
        let cfi = Field::new("classification", DataType::CfiCode, false);
        let stored = scalar_array(&cfi, &Scalar::from("ESVUFR")).unwrap();
        let cells = stored.as_any().downcast_ref::<StringArray>().unwrap();

        assert_eq!(cells.value(0), "ESVUFR");
        assert_eq!(cells.value_length(0), 6);
        assert_eq!(
            scalar_value(&cfi, stored.as_ref()).unwrap(),
            DataType::CfiCode.scalar(Scalar::from("ESVUFR")).unwrap()
        );
        // A width of six bytes is spellable and is still not a CFI code.
        assert_eq!(
            DataType::from_str("fixed_ascii(6)").unwrap(),
            DataType::fixed_ascii(6).unwrap()
        );
        assert_ne!(DataType::CfiCode, DataType::fixed_ascii(6).unwrap());
    }
}
