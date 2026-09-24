//! `rust/src/unit.rs`: the unit a quantity is stated in, FIX's
//! `UnitOfMeasure(996)`, as one registered code.
//!
//! `DataType` is `#[non_exhaustive]` and the datatype layer carries some sixty
//! wildcard arms, so a new variant compiles clean while behaving wrongly. A
//! green build proves nothing; these are the invariants a wildcard cannot
//! satisfy by accident, once, for this code.

mod coded {
    use std::sync::Arc;

    use arrow_array::{Array, ArrayRef, StringArray};
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::CodeValue as _;
    use yggdryl::xxhash::Xxh3;
    use yggdryl::{
        ArrowCastOptions, DataType, DataTypeId, DataTypeKind, Field, Scalar, Serie, Unit, UnitField,
    };

    fn text(values: &[&str]) -> ArrayRef {
        Arc::new(StringArray::from(values.to_vec()))
    }

    fn shares() -> Scalar {
        Scalar::Unit(Unit::new("Shares").unwrap())
    }

    #[test]
    fn unit_is_a_registered_code_the_grammar_spells_one_way() {
        // Parsed, displayed and folded like any other name.
        assert_eq!(DataType::from_str("unit").unwrap(), DataType::Unit);
        assert_eq!(DataType::from_str("UNIT").unwrap(), DataType::Unit);
        assert_eq!(DataType::from_logical_name("unit").unwrap(), DataType::Unit);
        assert_eq!(DataType::Unit.to_string(), "unit");
        assert_eq!(DataType::Unit.name(), "unit");
        assert_eq!(DataType::unit(), DataType::Unit);

        // Identity: the discriminant, the family, the width, and the listing.
        assert_eq!(DataType::Unit.id(), DataTypeId::Unit);
        assert_eq!(DataTypeId::Unit.as_u8(), 0x7d);
        assert_eq!(DataTypeId::Unit.as_str(), "unit");
        assert_eq!(DataType::Unit.kind(), DataTypeKind::Code);
        assert!(DataType::Unit.is_code());
        assert!(!DataType::Unit.is_string());
        assert_eq!(DataType::Unit.code_name(), Some("unit"));
        assert_eq!(DataType::Unit.code_width(), Some(32));
        assert_eq!(DataTypeId::Unit.code_width(), Some(32));
        assert_eq!(DataType::Unit.fixed_byte_width(), None);
        assert!(DataTypeId::ALL.contains(&DataTypeId::Unit));
        assert!(DataType::CODES.iter().any(|(name, dtype, width)| {
            *name == "unit" && *dtype == DataType::Unit && *width == 32
        }));

        // The datatype's own wire is the one spelling, and it round-trips.
        let json = DataType::Unit.into_json().unwrap();
        assert_eq!(json, r#"{"type":"unit"}"#);
        assert_eq!(DataType::from_json(&json).unwrap(), DataType::Unit);
        let rendered = serde_json::to_string(&DataType::Unit).unwrap();
        assert_eq!(
            serde_json::from_str::<DataType>(&rendered).unwrap(),
            DataType::Unit
        );
    }

    #[test]
    fn a_unit_field_is_the_typed_marker_over_the_variant() {
        let field = Field::new("unit", DataType::Unit, true);
        assert_eq!(field.dtype(), &DataType::Unit);
        let typed = UnitField::unit("unit", true);
        assert_eq!(typed.dtype(), &DataType::Unit);
        assert_eq!(typed.to_field(), field);
    }

    #[test]
    fn a_unit_value_is_the_code_under_its_own_identity_and_its_wire_names_it() {
        let value = DataType::Unit.scalar(Scalar::from("Shares")).unwrap();
        assert_eq!(value, shares());
        assert!(value.is_code());
        assert_eq!(value.id(), DataTypeId::Unit);
        assert_eq!(value.kind(), "unit");
        assert_eq!(value.as_str(), Some("Shares"));
        assert_eq!(value.dtype().unwrap(), DataType::Unit);
        assert_eq!(
            value.code_storage().map(|held| held.as_str()),
            Some("Shares")
        );

        let wire = serde_json::to_string(&value).unwrap();
        assert_eq!(wire, r#"{"type":"unit","value":"Shares"}"#);
        assert_eq!(serde_json::from_str::<Scalar>(&wire).unwrap(), value);

        // The text a unit is made of is not the unit, and a unit of the same
        // bytes as another code is not that code: the identity leads.
        assert_ne!(value, Scalar::from("Shares"));
        assert_ne!(
            DataType::Unit.scalar(Scalar::from("BUY")).unwrap(),
            DataType::Side.scalar(Scalar::from("BUY")).unwrap()
        );
    }

    #[test]
    fn a_unit_travels_the_value_stream_as_its_own_leaf() {
        let value = shares();
        let bytes = value.into_value_bytes();
        assert_eq!(bytes[1], DataTypeId::Unit.as_u8());
        let read = Scalar::decode_value_bytes(&bytes).unwrap();
        assert_eq!(read, value);
        assert_eq!(read.id(), DataTypeId::Unit, "the leaf travels");
        assert_eq!(read.dtype().unwrap(), DataType::Unit);

        // The stream is the same bytes, cut one leaf per chunk.
        let chunks: Vec<Vec<u8>> = value.encode_value_stream_bytes().collect();
        assert_eq!(chunks.concat(), bytes);
        assert_eq!(Scalar::decode_value_stream_bytes(&chunks).unwrap(), value);
    }

    #[test]
    fn the_canonical_landing_is_the_trimmed_text_under_the_unit_identity() {
        let field = Field::new("unit", DataType::Unit, false);
        // Text lands as the unit through both doors; a unit already stored
        // lands as itself.
        assert_eq!(field.scalar(Scalar::from("Shares")).unwrap(), shares());
        assert_eq!(
            DataType::Unit.scalar(Scalar::from("Shares")).unwrap(),
            shares()
        );
        assert_eq!(DataType::Unit.scalar(shares()).unwrap(), shares());
        // Padding a fixed slot wrote is the slot's, not the value's.
        assert_eq!(
            DataType::Unit.scalar(Scalar::from("MWh\0\0")).unwrap(),
            Scalar::Unit(Unit::new("MWh").unwrap())
        );
        // The neutral member is the empty unit, and it is this code's default.
        assert_eq!(
            DataType::Unit.default_value().unwrap(),
            Scalar::Unit(Unit::none())
        );
        // A value of another code lands as the text it holds, under this
        // identity: the datatype decides what the bytes are, as it does for
        // every other code.
        assert_eq!(
            DataType::Unit
                .scalar(DataType::Side.scalar(Scalar::from("BUY")).unwrap())
                .unwrap(),
            Scalar::Unit(Unit::new("BUY").unwrap())
        );
        // The width holds through the datatype door, naming itself.
        let refused = DataType::Unit
            .scalar(Scalar::from("X".repeat(33).as_str()))
            .unwrap_err()
            .to_string();
        assert!(refused.contains("32 bytes"), "{refused}");
    }

    #[test]
    fn a_unit_column_crosses_arrow_as_text_under_its_own_extension() {
        let field = Field::new("unit", DataType::Unit, false);
        let arrow = field.clone().into_arrow_field().unwrap();
        assert_eq!(arrow.data_type(), &ArrowDataType::Utf8);
        assert_eq!(arrow.metadata()["ARROW:extension:name"], "yggdryl.unit");
        assert_eq!(arrow.metadata()["ARROW:extension:metadata"], "");
        // The identity round-trips: the same bytes come back the same code.
        assert_eq!(Field::from_arrow_field(&arrow).unwrap(), field);

        // A value is stored as exactly its own bytes and read back as itself.
        let stored = Serie::from_scalars(field.clone(), [shares()])
            .unwrap()
            .require_arrow_array()
            .unwrap();
        let cells = stored.as_any().downcast_ref::<StringArray>().unwrap();
        assert_eq!(cells.value(0), "Shares");
        assert_eq!(cells.value_length(0), 6);
        let back = Serie::from_arrow_array(
            Some(&field),
            Arc::clone(&stored),
            ArrowCastOptions::default(),
        )
        .unwrap();
        assert_eq!(back.scalar(0).unwrap(), shares());
    }

    #[test]
    fn text_casts_into_a_unit_at_its_width_and_the_refusal_names_it() {
        let field = Field::new("unit", DataType::Unit, false);
        let strict = || ArrowCastOptions::new().with_safe(false);

        let cast =
            Serie::from_arrow_array(Some(&field), text(&["Shares", "MWh"]), strict()).unwrap();
        assert_eq!(cast.scalar(0).unwrap(), shares());
        assert_eq!(
            cast.scalar(1).unwrap(),
            Scalar::Unit(Unit::new("MWh").unwrap())
        );
        let cells = cast.require_arrow_array().unwrap();
        let cells = cells.as_any().downcast_ref::<StringArray>().unwrap();
        assert_eq!(cells.value(0), "Shares");
        assert_eq!(cells.value(1), "MWh");

        // Exactly the width fits; one byte more is refused at the code's own
        // width, whatever ASCII width would come next.
        let full = "X".repeat(32);
        assert!(Serie::from_arrow_array(Some(&field), text(&[full.as_str()]), strict()).is_ok());
        let over = "X".repeat(33);
        let refused = Serie::from_arrow_array(Some(&field), text(&[over.as_str()]), strict())
            .unwrap_err()
            .to_string();
        assert!(refused.contains("at most 32 bytes"), "{refused}");
    }

    #[test]
    fn the_digest_of_a_unit_value_is_pinned() {
        // First pinned here, when the leaf landed: the feed is the unit's own
        // identifier, the byte count and the bytes, and it must never move.
        let mut state = Xxh3::new();
        state.write_scalar(&shares());
        assert_eq!(state.as_u64(), 0x87f1_9de5_ecc7_0e57);
    }

    #[test]
    fn none_is_the_empty_unit_and_a_merge_takes_the_other_over_it() {
        assert_eq!(Unit::none().as_str(), "");
        assert!(Unit::none().is_none());
        assert_eq!(Unit::default(), Unit::none());
        assert_eq!(Unit::new("").unwrap(), Unit::none());
        assert!(!Unit::new("Shares").unwrap().is_none());

        let mwh = Unit::new("MWh").unwrap();
        let bbl = Unit::new("Bbl").unwrap();
        assert_eq!(Unit::none().merge_with(&mwh), mwh);
        assert_eq!(bbl.clone().merge_with(&mwh), bbl);
        assert_eq!(bbl.clone().merge_with(&Unit::none()), bbl);
    }

    #[test]
    fn a_thirty_third_byte_and_a_byte_past_ascii_are_refused_naming_why() {
        assert!(Unit::new("X".repeat(32)).is_ok());
        let refused = Unit::new("X".repeat(33)).unwrap_err().to_string();
        assert!(refused.contains("32 bytes"), "{refused}");

        let refused = Unit::new("m\u{b2}").unwrap_err().to_string();
        assert!(refused.contains("non-ASCII"), "{refused}");
        assert!(DataType::Unit.scalar(Scalar::from("m\u{b2}")).is_err());
    }
}
