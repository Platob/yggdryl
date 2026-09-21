//! `rust/src/protocol.rs`: the protocol metadata keys no caller can name.
//!
//! The three metadata keys an integration test cannot reach.
//!
//! `PYTHON_MODULE_KEY`, `PYTHON_QUALNAME_KEY` and `PYTHON_KIND_KEY` are
//! crate-private: they are the keys a declaration is stored under, and a
//! caller reads them back only through the borrowed view. What a caller can
//! observe lives in `tests/types/metadata.rs`.
//!
//! The protocol list an integration test cannot reach.
//!
//! `for_each_well_known_protocol!` is a crate-private macro: it is the one
//! place the well-known protocols are named, and this pin checks each one
//! reaches the key its own scheme spells. The rest of the suite lives in
//! `tests/types/metadata.rs`.

mod python_tests {

    use yggdryl::DataType;
    use yggdryl::internals::protocol::{
        KIND_SHAPE, PYTHON_KIND_KEY, PYTHON_MODULE_KEY, PYTHON_QUALNAME_KEY,
        validate_python_module, validate_python_qualname,
    };
    use yggdryl::{PythonKind, PythonMetadata};

    #[test]
    fn a_declaration_round_trips_through_the_field() {
        let declared =
            PythonMetadata::new("trading.book", "Book.Quote", PythonKind::Field).unwrap();
        let mut field = DataType::Int64.required_field("price");

        field.as_python_mut().set_class(&declared).unwrap();

        assert_eq!(field.get_metadata(PYTHON_KIND_KEY), Some("field"));
        assert_eq!(field.get_metadata(PYTHON_MODULE_KEY), Some("trading.book"));
        assert_eq!(field.get_metadata(PYTHON_QUALNAME_KEY), Some("Book.Quote"));
        assert_eq!(field.as_python().class().unwrap(), Some(declared.clone()));
        assert_eq!(field.as_python().class_name(), Some("Quote"));
        assert_eq!(
            field.as_python().import_path().as_deref(),
            Some("trading.book.Book.Quote")
        );
        assert_eq!(field.as_python_mut().remove_class(), Some(declared));
        assert!(field.as_python().is_empty());
    }

    #[test]
    fn a_partial_declaration_names_no_class() {
        let mut field = DataType::Int64.required_field("price");
        field.as_python_mut().set_module("trading.book").unwrap();

        assert_eq!(field.as_python().module(), Some("trading.book"));
        assert_eq!(field.as_python().class().unwrap(), None);
        assert_eq!(field.as_python().import_path(), None);
    }

    #[test]
    fn a_class_declared_in_a_function_is_not_importable() {
        let declared =
            PythonMetadata::new("app", "build.<locals>.Row", PythonKind::Dataclass).unwrap();

        assert_eq!(declared.class_name(), "Row");
        assert!(!declared.is_importable());
        assert!(
            PythonMetadata::new("app", "Row", PythonKind::Dataclass)
                .unwrap()
                .is_importable()
        );
    }

    #[test]
    fn every_stored_form_round_trips_its_spelling() {
        for kind in PythonKind::ALL {
            assert_eq!(PythonKind::from_str(kind.as_str()).unwrap(), kind);
            // The refusal sentence is written out, so it is the one thing that
            // can fall behind a form added to the list.
            assert!(KIND_SHAPE.contains(kind.as_str()), "{kind}");
        }
        assert!(PythonKind::from_str("record").is_err());
    }

    #[test]
    fn a_name_python_could_not_have_written_is_refused() {
        for refused in ["", "trading.", ".book", "trading book", "1book", "class"] {
            assert!(validate_python_module(refused).is_err(), "{refused:?}");
        }
        for accepted in ["trading", "trading.book", "_private", "match", "données"] {
            assert!(validate_python_module(accepted).is_ok(), "{accepted:?}");
        }
        assert!(validate_python_qualname("build.<locals>.Row").is_ok());
        assert!(validate_python_module("build.<locals>.Row").is_err());
    }
}

mod tests {
    use yggdryl::internals::protocol::well_known_protocol_probes;

    /// Every named pair reaches the key its own scheme spells.
    ///
    /// The list is crate-private and a macro, which cannot cross the crate
    /// boundary, so the walk over it is behind one forwarder: it stores `x`
    /// as `1` through each mutable view and answers what each pair of views
    /// then spells. The literal key is what proves the accessor, the newtype
    /// and the scheme constant of one list entry agree; both `prefix` values
    /// would still agree if all three drifted together.
    #[test]
    fn every_named_protocol_view_spells_its_own_scheme_prefix() {
        let probes = well_known_protocol_probes();
        assert!(!probes.is_empty(), "the protocol list is empty");
        for probe in probes {
            let key = format!("{}:x", probe.scheme_prefix);

            assert_eq!(probe.stored.as_deref(), Some("1"), "{key}");
            assert_eq!(probe.view_prefix, probe.scheme_prefix);
            assert_eq!(probe.view_mut_prefix, probe.scheme_prefix);
            assert_eq!(probe.key, key);
            assert_eq!(
                probe.entries,
                [(String::from("x"), String::from("1"))],
                "{key}"
            );
        }
    }
}
