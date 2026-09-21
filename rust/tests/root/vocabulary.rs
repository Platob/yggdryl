//! `rust/src/vocabulary.rs`: a row declared in FIX's datatype names is an
//! ordinary row everywhere else.

mod rows {
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::DateTimeType;
    use yggdryl::{DataType, DataTypeId, Field, Scalar, StringEnum, TimeUnit, Timezone};

    /// The declaration a FIX-fed writer would hand the schema, in FIX spellings.
    const FIX_ROW: &str = "struct<ccy: Currency, venue: Exchange, px: Price, qty: Qty, \
                       at: UTCTimestamp, day: LocalMktDate, seq: SeqNum>";

    #[test]
    fn a_fix_declared_row_projects_to_the_arrow_types_the_names_resolved() {
        let row = Field::new("row", DataType::from_str(FIX_ROW).unwrap(), false);
        let schema = row.clone().into_arrow_schema().unwrap();
        let arrow: Vec<(&str, &ArrowDataType)> = schema
            .fields()
            .iter()
            .map(|field| (field.name().as_str(), field.data_type()))
            .collect();
        assert_eq!(
            arrow,
            [
                // `Currency` and `Exchange` resolve to datatypes of their own,
                // each storing as the text it is under its own extension name.
                ("ccy", &ArrowDataType::Utf8),
                ("venue", &ArrowDataType::Utf8),
                // The float family is FIX `float`, which states no scale.
                ("px", &ArrowDataType::Float64),
                ("qty", &ArrowDataType::Float64),
                (
                    "at",
                    &ArrowDataType::Timestamp(
                        arrow_schema::TimeUnit::Nanosecond,
                        Some("UTC".into())
                    )
                ),
                // A FIX date is that day's midnight, and a local market date
                // states no zone - so it crosses as a zone-less timestamp rather
                // than as a day a consumer would have to cast to compare.
                (
                    "day",
                    &ArrowDataType::Timestamp(arrow_schema::TimeUnit::Nanosecond, None)
                ),
                ("seq", &ArrowDataType::Int64),
            ]
        );

        // The names are inert: what came back is the datatype, not a registration,
        // so the Arrow round trip is the ordinary one.
        assert_eq!(Field::from_arrow_schema("row", &schema).unwrap(), row);
        assert_eq!(
            row.dtype().get_field_by_path("at").map(Field::dtype),
            Some(&DataType::DateTime(DateTimeType::DateTime64 {
                unit: TimeUnit::Nanosecond,
                timezone: Timezone::UTC
            }))
        );
        // A row declared in the resolved spellings is the same row.
        let resolved = Field::new(
            "row",
            DataType::from_str(&row.dtype().to_string()).unwrap(),
            false,
        );
        assert_eq!(resolved, row);
    }

    #[test]
    fn a_fix_declared_row_types_the_text_a_message_carried() {
        let row = Field::new("row", DataType::from_str(FIX_ROW).unwrap(), false);
        // A FIX value is text on the wire, so the field-directed parser is where
        // the registration earns its keep: the name typed the column, and the
        // column typed the string.
        let message = r#"{"ccy":"USD","venue":"XCME","px":"101.25","qty":"7",
        "at":"2026-09-04T10:00:00.000000001Z","day":"2026-09-04T00:00:00","seq":9}"#;
        let value = yggdryl::json::from_utf8_with_field(message, &row).unwrap();
        let columns = value.as_sequence().expect("a canonical row");

        assert_eq!(columns[0].id(), DataTypeId::Currency);
        assert_eq!(columns[0].as_str(), Some("USD"));
        assert_eq!(columns[1].id(), DataTypeId::MicCode);
        assert_eq!(columns[1].as_str(), Some("XCME"));
        // The specification declares the float family as `float` and states no
        // scale, so a price reads as a double and the exact characters stay in
        // the message the row was typed from.
        assert_eq!(columns[2], Scalar::from(101.25_f64));
        assert_eq!(columns[3], Scalar::from(7.0_f64));
        assert_eq!(columns[6], Scalar::from(9_i64));

        // The instant reads at nanoseconds, so the fraction survives.
        let at = &columns[4];
        assert_eq!(at.temporal_count(), Some(1_788_516_000_000_000_001));
        assert_eq!(at.temporal_unit(), Some(TimeUnit::Nanosecond));
        let day = &columns[5];
        // The same day, now counted from the epoch in nanoseconds because a date
        // is an instant at midnight.
        assert_eq!(day.temporal_count(), Some(20_700 * 86_400 * 1_000_000_000));
        assert_eq!(day.temporal_unit(), Some(TimeUnit::Nanosecond));

        // A value that does not fit the resolved datatype is refused by that
        // datatype, never by the name that spelled it.
        let refused = yggdryl::json::from_utf8_with_field(
            r#"{"ccy":"EURO!","venue":"XCME","px":"1","qty":"1","at":"2026-09-04T10:00:00Z","day":"2026-09-04T00:00:00","seq":1}"#,
            &row,
        )
        .unwrap_err()
        .to_string();
        assert!(refused.contains("at most 3 bytes"), "{refused}");
    }

    #[test]
    fn a_prebuilt_vocabulary_declares_the_codes_a_venue_column_carries() {
        let venues = StringEnum::from_logical_name("Exchange").unwrap();
        assert_eq!(venues.len(), StringEnum::MICS.len());
        assert_eq!(venues.get("XCME"), Some("XCME"));

        // The listing is what a field declares, so a venue column crosses Arrow
        // carrying the vocabulary its values come from.
        let venue = Field::new("venue", DataType::MicCode, false)
            .try_with_string_enum(&venues)
            .unwrap();
        let recovered =
            Field::from_arrow_field(&venue.clone().into_arrow_field().unwrap()).unwrap();
        assert_eq!(recovered, venue);
        assert_eq!(recovered.string_enum().unwrap().as_ref(), Some(&venues));

        // A member's code is the value's own bytes under the resolved width, so
        // two processes reading this schema answer the same integers.
        let members = venues.into_members(&DataType::MicCode).unwrap();
        for (member, code) in &members {
            assert_eq!(
                *code,
                DataType::MicCode.ascii_packed(member.as_bytes()).unwrap()
            );
        }
        assert_eq!(
            StringEnum::from_logical_name("mic")
                .unwrap()
                .into_members(&DataType::MicCode)
                .unwrap(),
            members
        );
    }
}

mod logical {
    use yggdryl::Timezone;
    use yggdryl::{DataType, DateTimeType, TimeType};
    use yggdryl::{TimeUnit, UnionMode};

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

    /// The whole registry, as the module documents it. A change to a mapping
    /// changes what a stored schema string means, so it changes here first.
    fn registered() -> Vec<(&'static str, DataType)> {
        vec![
            ("currency", DataType::Currency),
            ("country", DataType::Country),
            ("mic", DataType::MicCode),
            ("exchange", DataType::MicCode),
            ("cfi", DataType::CfiCode),
            ("isin", DataType::IsinCode),
            ("cusip", DataType::CusipCode),
            ("sedol", DataType::SedolCode),
            ("bloomberg", DataType::BloombergCode),
            ("figi", DataType::FIGICode),
            ("side", DataType::Side),
            ("state", DataType::State),
            ("timeinforce", DataType::TimeInForce),
            ("language", DataType::fixed_ascii(2).unwrap()),
            ("monthyear", DataType::fixed_ascii(8).unwrap()),
            ("tenor", DataType::fixed_ascii(8).unwrap()),
            ("pattern", DataType::utf8()),
            ("length", DataType::Int32),
            ("tagnum", DataType::Int32),
            ("seqnum", DataType::Int64),
            ("numingroup", DataType::Int32),
            ("dayofmonth", DataType::Int8),
            ("reserved100plus", DataType::Int32),
            ("reserved1000plus", DataType::Int32),
            ("reserved4000plus", DataType::Int32),
            ("qty", DataType::Float64),
            ("price", DataType::Float64),
            ("priceoffset", DataType::Float64),
            ("percentage", DataType::Float64),
            ("amt", DataType::Float64),
            (
                "utctimestamp",
                DataType::DateTime(DateTimeType::DateTime64 {
                    unit: TimeUnit::Nanosecond,
                    timezone: Timezone::UTC,
                }),
            ),
            (
                "tztimestamp",
                DataType::DateTime(DateTimeType::DateTime64 {
                    unit: TimeUnit::Nanosecond,
                    timezone: Timezone::UTC,
                }),
            ),
            (
                "utctimeonly",
                DataType::Time(TimeType::Time64(TimeUnit::Nanosecond)),
            ),
            (
                "localmkttime",
                DataType::Time(TimeType::Time64(TimeUnit::Nanosecond)),
            ),
            (
                "utcdate",
                DataType::DateTime(DateTimeType::DateTime64 {
                    unit: TimeUnit::Nanosecond,
                    timezone: Timezone::UTC,
                }),
            ),
            (
                "utcdateonly",
                DataType::DateTime(DateTimeType::DateTime64 {
                    unit: TimeUnit::Nanosecond,
                    timezone: Timezone::UTC,
                }),
            ),
            (
                "localmktdate",
                DataType::DateTime(DateTimeType::DateTime64 {
                    unit: TimeUnit::Nanosecond,
                    timezone: Timezone::NAIVE,
                }),
            ),
            (
                "localmktdatetime",
                DataType::DateTime(DateTimeType::DateTime64 {
                    unit: TimeUnit::Nanosecond,
                    timezone: Timezone::NAIVE,
                }),
            ),
            (
                "tztimeonly",
                DataType::DateTime(DateTimeType::DateTime64 {
                    unit: TimeUnit::Nanosecond,
                    timezone: Timezone::UTC,
                }),
            ),
            ("multiplecharvalue", DataType::utf8()),
            ("multiplestringvalue", DataType::utf8()),
            ("xid", DataType::utf8()),
            ("xidref", DataType::utf8()),
            ("data", DataType::binary()),
            ("xmldata", DataType::binary()),
        ]
    }

    #[test]
    fn the_registry_is_the_documented_mapping_and_holds_no_repeat() {
        assert_eq!(DataType::LOGICAL_NAMES, registered().as_slice());
        let mut names: Vec<&str> = DataType::LOGICAL_NAMES
            .iter()
            .map(|(name, _)| *name)
            .collect();
        let registered = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), registered, "a name is registered twice");
        // Every stored name is already folded, so a lookup finds it verbatim.
        for name in names {
            assert!(
                name.bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit()),
                "{name} is not stored folded"
            );
            assert!(DataType::from_logical_name(name).is_ok(), "{name}");
        }
    }

    #[test]
    fn a_name_folds_case_separators_and_surrounding_space() {
        for spelling in [
            "UTCTimestamp",
            "utctimestamp",
            " UTC_Timestamp ",
            "utc-timestamp",
            "UTC Timestamp",
        ] {
            assert_eq!(
                DataType::from_logical_name(spelling).unwrap(),
                DataType::DateTime(DateTimeType::DateTime64 {
                    unit: TimeUnit::Nanosecond,
                    timezone: Timezone::UTC
                }),
                "{spelling}"
            );
        }
    }

    #[test]
    fn the_grammar_resolves_a_name_and_displays_the_datatype_it_named() {
        for (spelling, dtype) in [
            ("Price", DataType::Float64),
            ("Amt", DataType::Float64),
            ("SeqNum", DataType::Int64),
            ("DayOfMonth", DataType::Int8),
            (
                "LocalMktDate",
                DataType::DateTime(DateTimeType::DateTime64 {
                    unit: TimeUnit::Nanosecond,
                    timezone: Timezone::NAIVE,
                }),
            ),
            (
                "LocalMktTime",
                DataType::Time(TimeType::Time64(TimeUnit::Nanosecond)),
            ),
            (
                "UTCTimeOnly",
                DataType::Time(TimeType::Time64(TimeUnit::Nanosecond)),
            ),
            ("XMLData", DataType::binary()),
            ("data", DataType::binary()),
            ("Tenor", DataType::fixed_ascii(8).unwrap()),
        ] {
            let parsed: DataType = spelling.parse().unwrap();
            assert_eq!(parsed, dtype, "{spelling}");
            // One canonical spelling: a name displays as what it resolved to.
            assert_eq!(parsed.to_string(), dtype.to_string(), "{spelling}");
            assert_eq!(parsed.to_string().parse::<DataType>().unwrap(), parsed);
        }

        // A name types a column wherever a datatype is accepted, and a
        // postfix list still applies to it.
        let row: DataType = "struct<ccy: Currency, px: Price, legs: Qty[]>"
            .parse()
            .unwrap();
        assert_eq!(
            row.get_field_by_path("px")
                .map(|field| field.dtype().clone()),
            Some(DataType::Float64)
        );
        assert_eq!(
            row.get_field_by_path("legs.item")
                .map(|field| field.dtype().clone()),
            Some(DataType::Float64)
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
                !DataType::LOGICAL_NAMES
                    .iter()
                    .any(|(name, _)| *name == spelling.to_ascii_lowercase()),
                "{spelling} must not be registered"
            );
        }
    }

    #[test]
    fn an_unregistered_name_is_refused_by_both_entry_points() {
        let error = DataType::from_logical_name("figx").unwrap_err().to_string();
        assert!(error.contains("currency"), "{error}");
        assert!(error.contains("\"figx\""), "{error}");
        // The grammar reports an unregistered word as unknown.
        let error = "figx".parse::<DataType>().unwrap_err().to_string();
        assert!(error.contains("unknown datatype \"figx\""), "{error}");
    }

    /// A registered name is inert everywhere but the grammar: it adds no
    /// variant, so identity, family, and union type ids are untouched.
    #[test]
    fn a_name_adds_no_datatype_of_its_own() {
        let price = DataType::from_logical_name("price").unwrap();
        assert_eq!(price.id(), DataType::Float64.id());
        assert_eq!(price.kind(), DataType::Float64.kind());
        let union: DataType = "union(dense,0=px: Price,1=ccy: Currency)".parse().unwrap();
        let DataType::Union(_, mode) = &union else {
            panic!("a union, got {union}");
        };
        assert_eq!(*mode, UnionMode::Dense);

        // The prebuilt vocabularies are keyed by the same names.
        for (name, _) in yggdryl::StringEnum::PREBUILT {
            assert!(
                DataType::LOGICAL_NAMES
                    .iter()
                    .any(|(other, _)| other == name),
                "{name} prebuilds nothing registered"
            );
        }
    }
}
