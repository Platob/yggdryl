//! `rust/src/vocabulary.rs`: the core's logical names - its codes and
//! `state` - resolve to the datatype they spell, folded as every datatype
//! word is, and add none of their own; a prebuilt listing declares the codes
//! a column carries; a word no register answers is refused, naming the
//! install only where it is a reserved market kind's. The FIX Latest names
//! `yggdryl-fix` claims are pinned in `rust/fix/tests/root/lib.rs`.

mod rows {
    use yggdryl::{DataType, Field, StringEnum};

    #[test]
    fn a_prebuilt_vocabulary_declares_the_codes_a_venue_column_carries() {
        let venues = StringEnum::from_logical_name("Exchange").unwrap();
        assert_eq!(venues.len(), StringEnum::MICS.len());
        assert_eq!(venues.get("XCME"), Some("XCME"));

        // The listing is what a field declares, so a venue column crosses Arrow
        // carrying the vocabulary its values come from.
        let venue = Field::new("venue", DataType::Mic, false)
            .try_with_string_enum(&venues)
            .unwrap();
        let recovered =
            Field::from_arrow_field(&venue.clone().into_arrow_field().unwrap()).unwrap();
        assert_eq!(recovered, venue);
        assert_eq!(recovered.string_enum().unwrap().as_ref(), Some(&venues));

        // A member's code is the value's own bytes under the resolved width, so
        // two processes reading this schema answer the same integers.
        let members = venues.into_members(&DataType::Mic).unwrap();
        for (member, code) in &members {
            assert_eq!(
                *code,
                DataType::Mic.ascii_packed(member.as_bytes()).unwrap()
            );
        }
        assert_eq!(
            StringEnum::from_logical_name("mic")
                .unwrap()
                .into_members(&DataType::Mic)
                .unwrap(),
            members
        );
    }
}

mod logical {
    use yggdryl::{DataType, UnionMode};

    #[test]
    fn message_codes_are_text_owned_by_the_fix_registry() {
        for spelling in ["msgtype", "MsgType", "MSGTYPE"] {
            assert!(DataType::from_str(spelling).is_err(), "{spelling}");
            assert!(DataType::from_logical_name(spelling).is_err(), "{spelling}");
            assert!(
                yggdryl::DataTypeId::from_str(spelling).is_err(),
                "{spelling}"
            );
            assert!(
                yggdryl::StringEnum::from_logical_name(spelling).is_err(),
                "{spelling}"
            );
        }
        assert!(DataType::from_json(r#"{"type":"msgtype"}"#).is_err());
        let value = "A venue message type longer than eight bytes";
        assert_eq!(
            DataType::utf8().scalar(value).unwrap().as_str(),
            Some(value)
        );
    }

    /// The five FIX base types the Arrow/SQL grammar already owns keep their
    /// meaning: a stored schema string never changes what it means.
    #[test]
    fn the_shared_base_type_spellings_keep_their_grammar_meaning() {
        for (spelling, dtype) in [
            ("int", DataType::Int32),
            ("float", DataType::Float32),
            ("char", DataType::utf8()),
            ("String", DataType::utf8()),
            ("Boolean", DataType::Boolean),
        ] {
            assert_eq!(spelling.parse::<DataType>().unwrap(), dtype, "{spelling}");
            assert!(
                !DataType::logical_names()
                    .iter()
                    .any(|(name, _)| *name == spelling.to_ascii_lowercase()),
                "{spelling} must not be registered"
            );
        }
    }

    #[test]
    fn a_name_folds_case_separators_and_surrounding_space() {
        for spelling in [
            "Exchange",
            "exchange",
            " Ex_Change ",
            "ex-change",
            "Ex Change",
        ] {
            assert_eq!(
                DataType::from_logical_name(spelling).unwrap(),
                DataType::Mic,
                "{spelling}"
            );
        }
    }

    #[test]
    fn the_grammar_resolves_a_name_and_displays_the_datatype_it_named() {
        for spelling in ["Exchange", "EXCHANGE", "exchange"] {
            let parsed: DataType = spelling.parse().unwrap();
            assert_eq!(parsed, DataType::Mic, "{spelling}");
            // One canonical spelling: a name displays as what it resolved to.
            assert_eq!(parsed.to_string(), DataType::Mic.to_string(), "{spelling}");
            assert_eq!(parsed.to_string().parse::<DataType>().unwrap(), parsed);
        }

        // A name types a column wherever a datatype is accepted, and a
        // postfix serie still applies to it.
        let row: DataType = "struct<ccy: Ccy, venue: Exchange, legs: Exchange[]>"
            .parse()
            .unwrap();
        assert_eq!(
            row.get_field_by_path("venue")
                .map(|field| field.dtype().clone()),
            Some(DataType::Mic)
        );
        assert_eq!(
            row.get_field_by_path("legs.item")
                .map(|field| field.dtype().clone()),
            Some(DataType::Mic)
        );
    }

    /// A registered name is inert everywhere but the grammar: it adds no
    /// variant, so identity, family, and union type ids are untouched.
    #[test]
    fn a_name_adds_no_datatype_of_its_own() {
        let venue = DataType::from_logical_name("exchange").unwrap();
        assert_eq!(venue.id(), DataType::Mic.id());
        assert_eq!(venue.kind(), DataType::Mic.kind());
        let union: DataType = "union(dense,0=venue: Exchange,1=ccy: Ccy)".parse().unwrap();
        let DataType::Union(_, mode) = &union else {
            panic!("a union, got {union}");
        };
        assert_eq!(*mode, UnionMode::Dense);

        // The prebuilt vocabularies are keyed by the same names.
        for (name, _) in yggdryl::StringEnum::PREBUILT {
            assert!(
                DataType::logical_names()
                    .iter()
                    .any(|(other, _)| other == name),
                "{name} prebuilds nothing registered"
            );
        }
    }

    /// A reserved market kind's name is read once the crate that owns it
    /// installs: before that, the logical names and the grammar refuse it
    /// naming that install in one sentence, and the listing door refuses it
    /// as the names do.
    #[test]
    fn a_market_kind_before_its_install_is_refused_naming_the_install() {
        let install = "`side` is read only once the crate that claims it is installed \
                       (`yggdryl_market::install()`)";
        let logical = DataType::from_logical_name("side").unwrap_err().to_string();
        assert!(logical.contains("\"side\""), "{logical}");
        assert!(logical.contains("ccy"), "{logical}");
        assert_eq!(
            yggdryl::StringEnum::from_logical_name("side")
                .unwrap_err()
                .to_string(),
            logical
        );
        let grammar = "side".parse::<DataType>().unwrap_err().to_string();
        for refused in [&logical, &grammar] {
            assert!(refused.contains(install), "{refused}");
        }
    }

    /// A word no register answers and no reserved kind owns is refused by
    /// both entry points with no install to name.
    #[test]
    fn an_unregistered_name_is_refused_by_both_entry_points() {
        let error = DataType::from_logical_name("figx").unwrap_err().to_string();
        assert!(error.contains("ccy"), "{error}");
        assert!(error.contains("\"figx\""), "{error}");
        assert!(!error.contains("install"), "{error}");
        // The grammar reports an unregistered word as unknown.
        let error = "figx".parse::<DataType>().unwrap_err().to_string();
        assert!(error.contains("unknown datatype \"figx\""), "{error}");
        assert!(!error.contains("install"), "{error}");
    }
}
