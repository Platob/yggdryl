//! `rust/src/timezone.rs`: zone canonicalization, offsets, and
//! daylight-saving transitions.
//!
//! Time zone canonicalization, offsets, and daylight-saving transitions.
//!
//! The offset expectations here are real answers checked against the IANA
//! database, not against this implementation - a test that only agrees with
//! the code it tests would pass on a wrong table.

#[cfg(feature = "internals")]
mod internal {
    use yggdryl::Timezone;

    /// Parse a zone or fail the test with the reason.
    fn zone(value: &str) -> Timezone {
        Timezone::from_str(value).expect("a valid time zone")
    }

    #[test]
    fn a_timezone_is_one_copyable_four_byte_handle() {
        fn assert_copy<T: Copy>() {}

        assert_copy::<Timezone>();
        assert_eq!(std::mem::size_of::<Timezone>(), 4);
    }

    #[test]
    fn naive_is_a_canonical_non_zoned_marker() {
        assert_eq!(zone("naive"), Timezone::NAIVE);
        assert!(Timezone::NAIVE.is_naive());
        assert_eq!(Timezone::NAIVE.as_str(), "NAIVE");
        assert_eq!(Timezone::NAIVE.offset_at(0), None);
    }

    /// Seconds since the Unix epoch for a UTC civil date and time.
    fn utc(year: i32, month: u32, day: u32, hour: i64, minute: i64) -> i64 {
        yggdryl::internals::timezone::days_from_civil(year, month, day) * 86_400
            + hour * 3_600
            + minute * 60
    }

    mod the_registry {

        #[test]
        fn zones_are_sorted_so_the_binary_search_is_valid() {
            let names: Vec<&str> = yggdryl::internals::timezone::zones()
                .iter()
                .map(|zone| zone.name)
                .collect();
            let mut sorted = names.clone();
            sorted.sort_unstable();

            assert_eq!(names, sorted, "the zone table must be sorted by name");
        }

        #[test]
        fn aliases_are_sorted_and_never_shadow_a_real_zone() {
            let names: Vec<&str> = yggdryl::internals::timezone::aliases()
                .iter()
                .map(|(from, _)| *from)
                .collect();
            let mut sorted = names.clone();
            sorted.sort_unstable();
            assert_eq!(names, sorted, "the alias table must be sorted by alias");

            for (from, _) in yggdryl::internals::timezone::aliases() {
                assert!(
                    !yggdryl::internals::timezone::is_zone(from),
                    "{from} is both a zone and an alias"
                );
            }
        }

        #[test]
        fn every_offset_is_a_whole_number_of_minutes_within_a_day() {
            for zone in yggdryl::internals::timezone::zones() {
                assert_eq!(
                    zone.standard % 60,
                    0,
                    "{} has a sub-minute offset",
                    zone.name
                );
                assert!(
                    zone.standard.abs() < 24 * 3_600,
                    "{} is more than a day from UTC",
                    zone.name
                );
                if let Some(saving) = zone.saving {
                    assert!(
                        saving > 0 && saving <= 2 * 3_600,
                        "{} saves an implausible amount",
                        zone.name
                    );
                    assert!(
                        zone.daylight_abbreviation.is_some(),
                        "{} observes saving but has no daylight abbreviation",
                        zone.name
                    );
                }
            }
        }

        #[test]
        fn a_zone_that_observes_no_saving_declares_no_daylight_abbreviation() {
            for zone in yggdryl::internals::timezone::zones() {
                if zone.saving.is_none() {
                    assert!(
                        zone.daylight_abbreviation.is_none(),
                        "{} has a daylight abbreviation but no rule",
                        zone.name
                    );
                }
            }
        }
    }

    mod canonicalization {
        use std::hash::{DefaultHasher, Hash, Hasher};

        use super::{Timezone, zone};

        fn hash(value: Timezone) -> u64 {
            let mut state = DefaultHasher::new();
            value.hash(&mut state);
            state.finish()
        }

        #[test]
        fn an_alias_resolves_to_what_it_stands_for() {
            assert_eq!(zone("Asia/Calcutta"), zone("Asia/Kolkata"));
            assert_eq!(zone("US/Eastern"), zone("America/New_York"));
            assert_eq!(zone("Europe/Kiev"), zone("Europe/Kyiv"));
            assert_eq!(zone("Japan").as_str(), "Asia/Tokyo");
        }

        #[test]
        fn every_alias_interns_to_its_canonical_handle() {
            for &(alias, canonical) in yggdryl::internals::timezone::aliases() {
                let alias = zone(alias);
                let canonical = zone(canonical);

                assert_eq!(alias, canonical);
                assert_eq!(hash(alias), hash(canonical));
                assert!(std::ptr::eq(alias.as_smol_str(), canonical.as_smol_str()));
            }
        }

        #[test]
        fn every_spelling_of_utc_is_utc() {
            for value in [
                "UTC",
                "utc",
                "Z",
                "Zulu",
                "GMT",
                "Etc/UTC",
                "Universal",
                "+00:00",
                "-00:00",
            ] {
                assert_eq!(zone(value), Timezone::UTC, "{value} should be UTC");
                assert!(zone(value).is_utc());
            }
        }

        #[test]
        fn case_is_normalized_for_a_registered_name() {
            assert_eq!(zone("america/new_york").as_str(), "America/New_York");
            assert_eq!(zone("EUROPE/PARIS").as_str(), "Europe/Paris");
        }

        #[test]
        fn a_fixed_offset_normalizes_to_one_spelling() {
            for value in ["+05:30", "+0530", "UTC+05:30"] {
                assert_eq!(zone(value).as_str(), "+05:30", "{value} should normalize");
            }
            assert_eq!(zone("-08").as_str(), "-08:00");
            assert_eq!(zone("-0800").as_str(), "-08:00");
        }

        #[test]
        fn every_fixed_offset_round_trips_through_its_reserved_handle() {
            for minutes in -(24 * 60 - 1)..=(24 * 60 - 1) {
                let from_count = Timezone::from_offset(minutes * 60).unwrap();
                let from_name = zone(from_count.as_str());

                assert_eq!(from_count, from_name, "{minutes}");
            }
        }

        #[test]
        fn an_unregistered_name_is_kept_exactly_as_written() {
            // A schema naming a zone this build has no rules for must still round
            // trip unchanged, or importing a foreign schema would corrupt it.
            let custom = zone("Custom/Accepted");

            assert_eq!(custom.as_str(), "Custom/Accepted");
            assert!(!custom.is_known());
            assert_eq!(custom.offset_at(0), None);
        }

        #[test]
        fn an_unregistered_name_is_retained_once_for_the_process() {
            let first = zone("Custom/Interned");
            let second = zone("Custom/Interned");

            assert_eq!(first, second);
            assert!(std::ptr::eq(first.as_smol_str(), second.as_smol_str()));
        }

        #[test]
        fn an_impossible_name_is_refused() {
            assert!(Timezone::from_str("").is_err());
            assert!(Timezone::from_str("Europe/\u{7}Paris").is_err());
            // A spelling that looks like an offset but is not one is a typo.
            assert!(Timezone::from_str("+25:00").is_err());
            assert!(Timezone::from_str("+05:75").is_err());
        }

        #[test]
        fn a_fixed_offset_can_be_built_from_seconds() {
            assert_eq!(Timezone::from_offset(0).unwrap(), Timezone::UTC);
            assert_eq!(Timezone::from_offset(19_800).unwrap().as_str(), "+05:30");
            assert_eq!(Timezone::from_offset(-28_800).unwrap().as_str(), "-08:00");

            assert!(Timezone::from_offset(24 * 3_600).is_err());
            assert!(Timezone::from_offset(90).is_err());
        }

        #[test]
        fn the_canonical_name_survives_serde() {
            let value = zone("US/Pacific");
            let text = serde_json::to_string(&value).unwrap();

            assert_eq!(text, "\"America/Los_Angeles\"");
            assert_eq!(
                serde_json::from_str::<Timezone>("\"Asia/Calcutta\"").unwrap(),
                zone("Asia/Kolkata")
            );
        }
    }

    mod offsets {
        use super::{utc, zone};

        #[test]
        fn a_northern_zone_switches_on_its_own_rule() {
            let new_york = zone("America/New_York");

            // The United States switches on the second Sunday in March at 02:00
            // local standard, which in 2024 is the 10th, 07:00 UTC.
            assert_eq!(
                new_york.offset_at(utc(2024, 3, 10, 6, 59)),
                Some(-5 * 3_600)
            );
            assert_eq!(new_york.offset_at(utc(2024, 3, 10, 7, 0)), Some(-4 * 3_600));

            // ... and back on the first Sunday in November, the 3rd, 06:00 UTC.
            assert_eq!(
                new_york.offset_at(utc(2024, 11, 3, 5, 59)),
                Some(-4 * 3_600)
            );
            assert_eq!(new_york.offset_at(utc(2024, 11, 3, 6, 0)), Some(-5 * 3_600));
        }

        #[test]
        fn the_european_rule_switches_the_whole_union_at_one_instant() {
            let paris = zone("Europe/Paris");
            let helsinki = zone("Europe/Helsinki");
            let london = zone("Europe/London");

            // Last Sunday in March 2024 is the 31st, at 01:00 UTC exactly.
            let before = utc(2024, 3, 31, 0, 59);
            let after = utc(2024, 3, 31, 1, 0);

            assert_eq!(paris.offset_at(before), Some(3_600));
            assert_eq!(paris.offset_at(after), Some(2 * 3_600));
            assert_eq!(helsinki.offset_at(before), Some(2 * 3_600));
            assert_eq!(helsinki.offset_at(after), Some(3 * 3_600));
            assert_eq!(london.offset_at(before), Some(0));
            assert_eq!(london.offset_at(after), Some(3_600));
        }

        #[test]
        fn a_southern_zone_is_saving_across_the_new_year() {
            let sydney = zone("Australia/Sydney");

            // January is inside the saving period that began the previous October.
            assert_eq!(sydney.offset_at(utc(2024, 1, 15, 0, 0)), Some(11 * 3_600));
            // July is outside it.
            assert_eq!(sydney.offset_at(utc(2024, 7, 15, 0, 0)), Some(10 * 3_600));

            // First Sunday in April 2024 is the 7th, 02:00 local standard = 16:00
            // UTC on the 6th.
            assert_eq!(sydney.offset_at(utc(2024, 4, 6, 15, 59)), Some(11 * 3_600));
            assert_eq!(sydney.offset_at(utc(2024, 4, 6, 16, 0)), Some(10 * 3_600));
        }

        #[test]
        fn new_zealand_starts_on_the_last_sunday_in_september() {
            let auckland = zone("Pacific/Auckland");

            // 2024's last Sunday in September is the 29th, 02:00 local standard
            // (+12) = 14:00 UTC on the 28th.
            assert_eq!(
                auckland.offset_at(utc(2024, 9, 28, 13, 59)),
                Some(12 * 3_600)
            );
            assert_eq!(
                auckland.offset_at(utc(2024, 9, 28, 14, 0)),
                Some(13 * 3_600)
            );
        }

        #[test]
        fn a_zone_without_saving_answers_the_same_all_year() {
            for name in ["Asia/Tokyo", "Asia/Kolkata", "America/Phoenix", "UTC"] {
                let value = zone(name);
                let winter = value.offset_at(utc(2024, 1, 15, 12, 0));
                let summer = value.offset_at(utc(2024, 7, 15, 12, 0));

                assert_eq!(winter, summer, "{name} should not move");
                assert_eq!(value.is_saving_at(utc(2024, 7, 15, 12, 0)), Some(false));
                assert!(!value.observes_saving());
            }
            assert_eq!(zone("Asia/Kolkata").offset_at(0), Some(5 * 3_600 + 1_800));
            assert_eq!(zone("Asia/Kathmandu").offset_at(0), Some(5 * 3_600 + 2_700));
        }

        #[test]
        fn abbreviations_follow_the_saving_state() {
            let new_york = zone("America/New_York");
            assert_eq!(
                new_york.abbreviation_at(utc(2024, 1, 15, 12, 0)),
                Some("EST")
            );
            assert_eq!(
                new_york.abbreviation_at(utc(2024, 7, 15, 12, 0)),
                Some("EDT")
            );

            let berlin = zone("Europe/Berlin");
            assert_eq!(berlin.abbreviation_at(utc(2024, 1, 15, 12, 0)), Some("CET"));
            assert_eq!(
                berlin.abbreviation_at(utc(2024, 7, 15, 12, 0)),
                Some("CEST")
            );

            assert_eq!(zone("Custom/Unknown").abbreviation_at(0), None);
        }

        #[test]
        fn a_fixed_offset_never_observes_saving() {
            let value = zone("+05:30");

            assert_eq!(value.offset_at(utc(2024, 1, 1, 0, 0)), Some(19_800));
            assert_eq!(value.offset_at(utc(2024, 7, 1, 0, 0)), Some(19_800));
            assert_eq!(value.is_saving_at(0), Some(false));
            assert!(value.is_fixed());
            assert!(value.is_known());
        }

        #[test]
        fn the_standard_offset_ignores_saving_entirely() {
            assert_eq!(zone("America/New_York").standard_offset(), Some(-5 * 3_600));
            assert_eq!(zone("Europe/Paris").standard_offset(), Some(3_600));
            assert_eq!(zone("Custom/Unknown").standard_offset(), None);
        }

        #[test]
        fn transitions_land_on_a_sunday_for_every_rule_and_year() {
            // The nth-weekday arithmetic is the part most likely to be subtly
            // wrong, so check it holds across a span of years including leap ones.
            for year in 2020..2036 {
                for name in ["America/New_York", "Europe/Paris", "Australia/Sydney"] {
                    let value = zone(name);
                    let january = value.offset_at(utc(year, 1, 15, 12, 0)).unwrap();
                    let july = value.offset_at(utc(year, 7, 15, 12, 0)).unwrap();

                    assert_ne!(january, july, "{name} in {year} should move once a year");
                }
            }
        }
    }

    mod conversions {
        use super::{utc, zone};

        #[test]
        fn a_utc_instant_becomes_a_local_reading() {
            let new_york = zone("America/New_York");
            let instant = utc(2024, 7, 4, 16, 0);

            // 16:00 UTC in July is 12:00 in New York, which is EDT.
            assert_eq!(
                new_york.into_local(instant).unwrap(),
                utc(2024, 7, 4, 12, 0)
            );
            // In January the same reading is one hour further back.
            let winter = utc(2024, 1, 4, 16, 0);
            assert_eq!(new_york.into_local(winter).unwrap(), utc(2024, 1, 4, 11, 0));
        }

        #[test]
        fn a_local_reading_becomes_the_instant_it_names() {
            let paris = zone("Europe/Paris");
            let local = utc(2024, 7, 4, 14, 0);

            // 14:00 in Paris in July is 12:00 UTC.
            assert_eq!(paris.into_utc(local).unwrap(), utc(2024, 7, 4, 12, 0));
            // A round trip through both directions is the identity.
            let instant = utc(2024, 7, 4, 12, 0);
            assert_eq!(
                paris.into_utc(paris.into_local(instant).unwrap()).unwrap(),
                instant
            );
        }

        #[test]
        fn a_conversion_refuses_a_zone_it_has_no_rules_for() {
            let unknown = zone("Custom/Unknown");

            let message = unknown.into_local(0).unwrap_err().to_string();
            assert!(message.contains("Custom/Unknown"), "{message}");
            assert!(unknown.into_utc(0).is_err());
        }

        #[test]
        fn the_registry_is_installed_without_any_environment() {
            let registered: Vec<String> = super::Timezone::registered()
                .map(|zone| zone.as_str().to_owned())
                .collect();

            assert!(registered.len() > 60, "{}", registered.len());
            assert!(registered.iter().any(|name| name == "UTC"));
            assert!(registered.iter().any(|name| name == "Europe/Paris"));
            // Every registered name must itself parse back to the same zone.
            for name in &registered {
                assert_eq!(zone(name).as_str(), name);
            }
            assert!(super::Timezone::aliases().len() > 20);
        }
    }
}

mod families {

    use arrow_schema::DataType as ArrowDataType;
    use std::sync::Arc;
    use yggdryl::DataType;

    #[test]
    fn long_timezones_reuse_process_interned_storage_across_arrow_conversions() {
        let timezone: Arc<str> = Arc::from("America/Argentina/Buenos_Aires");
        let arrow = ArrowDataType::Timestamp(
            arrow_schema::TimeUnit::Nanosecond,
            Some(Arc::clone(&timezone)),
        );

        let borrowed = DataType::from_arrow_datatype(&arrow).unwrap();
        let DataType::DateTime64 {
            unit: _,
            timezone: borrowed_timezone,
        } = &borrowed
        else {
            panic!("timestamp import changed variant");
        };
        let borrowed_timezone = *borrowed_timezone;

        let borrowed_arrow = borrowed.into_arrow_datatype().unwrap();
        let ArrowDataType::Timestamp(_, Some(borrowed_arrow_timezone)) = borrowed_arrow else {
            panic!("timestamp projection changed variant");
        };
        assert_eq!(borrowed_arrow_timezone.as_ref(), timezone.as_ref());

        let owned = DataType::try_from(arrow).unwrap();
        let DataType::DateTime64 {
            unit: _,
            timezone: owned_timezone,
        } = &owned
        else {
            panic!("timestamp import changed variant");
        };
        assert!(std::ptr::eq(
            borrowed_timezone.as_smol_str(),
            owned_timezone.as_smol_str()
        ));

        let owned_arrow = owned.into_arrow_datatype().unwrap();
        let ArrowDataType::Timestamp(_, Some(owned_arrow_timezone)) = owned_arrow else {
            panic!("timestamp projection changed variant");
        };
        assert_eq!(owned_arrow_timezone.as_ref(), timezone.as_ref());
    }
}

mod zones {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    use std::sync::Arc;

    use arrow_array::{Array, RecordBatch, StringArray};
    use arrow_schema::DataType as ArrowDataType;

    use yggdryl::DataType;
    use yggdryl::FieldValue as _;
    use yggdryl::arrow::{scalar_array, scalar_value};
    use yggdryl::{
        ArrowCastOptions, DataTypeId, DataTypeKind, Field, FieldScalar, Scalar, StructType,
        Timezone, TimezoneField,
    };

    fn zone(text: &str) -> Scalar {
        DataType::Timezone.scalar(text).unwrap()
    }

    fn root(field: Field) -> Field {
        StructType::from_fields([field])
            .map(DataType::from)
            .unwrap()
            .required_field("row")
    }

    fn text_of(value: &Scalar) -> String {
        match value {
            Scalar::Timezone(zone) => zone.as_str().to_owned(),
            other => panic!("expected a timezone scalar, got {other:?}"),
        }
    }

    #[test]
    fn datatype_identity_naming_and_serde_are_total() {
        let dtype = DataType::Timezone;
        assert_eq!(dtype.id(), DataTypeId::Timezone);
        assert_eq!(dtype.kind(), DataTypeKind::Text);
        assert_eq!(dtype.name(), "timezone");
        assert_eq!(dtype.to_string(), "timezone");
        assert_eq!("TIMEZONE".parse::<DataType>().unwrap(), dtype);
        // Both other spellings a schema is written in reach the one datatype.
        assert_eq!("tz".parse::<DataType>().unwrap(), dtype);
        assert_eq!("timezone_name".parse::<DataType>().unwrap(), dtype);
        assert_eq!(
            "TIMEZONE".parse::<DataTypeId>().unwrap(),
            DataTypeId::Timezone
        );
        assert_eq!(DataTypeId::Timezone.as_str(), "timezone");
        // In the text family's range, because `as_u8` is a wire contract laid
        // out by family.
        assert_eq!(DataTypeId::Timezone.as_u8(), 0x66);
        assert_eq!(DataTypeId::Timezone.fixed_byte_width(), None);
        assert!(!DataTypeId::Timezone.is_parameterized());
        assert!(!dtype.is_nested());
        dtype.validate().unwrap();

        assert_eq!(dtype.clone().into_json().unwrap(), r#"{"type":"timezone"}"#);
        assert_eq!(
            DataType::from_json(r#"{"type":"timezone"}"#).unwrap(),
            dtype
        );
    }

    #[test]
    fn a_value_is_canonicalized_and_refuses_what_names_no_zone() {
        // An alias resolves to what it stands for and case folds, so two
        // spellings of one zone are one value.
        assert_eq!(text_of(&zone("Asia/Calcutta")), "Asia/Kolkata");
        assert_eq!(text_of(&zone("utc")), "UTC");
        assert_eq!(zone("Z"), zone("UTC"));
        // A fixed offset normalizes to `+HH:MM`.
        assert_eq!(text_of(&zone("-0800")), "-08:00");
        // The zone-free marker is a value like any other.
        assert_eq!(zone("NAIVE"), Scalar::Timezone(Timezone::NAIVE));
        // Re-reading the canonical text answers the same value.
        let once = zone("Asia/Calcutta");
        assert_eq!(zone(&text_of(&once)), once);

        // An empty text cell entering a non-text column is no value.
        assert_eq!(DataType::Timezone.scalar("").unwrap(), Scalar::Null);
        assert!(DataType::Timezone.scalar("+99:00").is_err());
        assert_eq!(
            DataType::Timezone.scalar(Scalar::Null).unwrap(),
            Scalar::Null
        );
    }

    #[test]
    fn the_scalar_carries_the_datatype_and_orders_by_canonical_name() {
        let value = zone("America/New_York");
        assert_eq!(value.id(), DataTypeId::Timezone);
        assert_eq!(value.kind(), "timezone");

        // Equal values hash equally, which is what a key column needs.
        let hash = |value: &Scalar| {
            let mut hasher = DefaultHasher::new();
            value.hash(&mut hasher);
            hasher.finish()
        };
        assert_eq!(hash(&zone("Asia/Calcutta")), hash(&zone("Asia/Kolkata")));
        assert_ne!(hash(&zone("UTC")), hash(&zone("America/New_York")));
    }

    #[test]
    fn structured_text_round_trips_the_canonical_spelling() {
        let value = zone("Asia/Calcutta");
        let tagged = serde_json::to_string(&value).unwrap();
        assert_eq!(tagged, r#"{"type":"timezone","value":"Asia/Kolkata"}"#);
        assert_eq!(serde_json::from_str::<Scalar>(&tagged).unwrap(), value);
    }

    #[test]
    fn arrow_stores_canonical_utf8_under_an_extension_name_that_survives_a_round_trip() {
        let field = Field::new("zone", DataType::Timezone, true);
        let arrow = field.clone().into_arrow_field().unwrap();
        assert_eq!(arrow.data_type(), &ArrowDataType::Utf8);
        assert_eq!(
            arrow
                .metadata()
                .get("ARROW:extension:name")
                .map(String::as_str),
            Some("yggdryl.timezone")
        );
        assert_eq!(
            Field::from_arrow_field(&arrow).unwrap().dtype(),
            &DataType::Timezone
        );

        let value = zone("America/New_York");
        let stored = scalar_array(&field, &value).unwrap();
        assert_eq!(stored.data_type(), &ArrowDataType::Utf8);
        assert_eq!(
            stored
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap()
                .value(0),
            "America/New_York"
        );
        assert_eq!(scalar_value(&field, stored.as_ref()).unwrap(), value);
    }

    #[test]
    fn a_text_column_is_ingested_and_canonicalized_and_a_bad_row_names_itself() {
        let target = root(Field::new("zone", DataType::Timezone, true));
        let source_schema = root(Field::new("zone", DataType::utf8(), true))
            .into_arrow_schema()
            .unwrap();
        let batch = RecordBatch::try_new(
            Arc::clone(&source_schema),
            vec![Arc::new(StringArray::from(vec![
                Some("Asia/Calcutta"),
                None,
            ]))],
        )
        .unwrap();
        let cast = target
            .cast_arrow_batch(batch, ArrowCastOptions::new().with_safe(false))
            .unwrap();
        let column = cast
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        assert_eq!(column.value(0), "Asia/Kolkata");
        assert!(column.is_null(1));

        let bad = RecordBatch::try_new(
            source_schema,
            vec![Arc::new(StringArray::from(vec![Some("+99:00")]))],
        )
        .unwrap();
        let error = target
            .cast_arrow_batch(bad, ArrowCastOptions::new().with_safe(false))
            .unwrap_err()
            .to_string();
        assert!(error.contains("row 0"), "{error}");
        assert!(error.contains("does not read as time zone"), "{error}");
    }

    #[test]
    fn defaults_merges_and_typed_fields_do_not_fall_through() {
        // The zone-free marker is the zero every temporal already defaults to.
        assert_eq!(
            text_of(&DataType::Timezone.default_value().unwrap()),
            "NAIVE"
        );
        assert!(DataType::Timezone.is_default_value(&zone("NAIVE")).unwrap());

        assert_eq!(
            DataType::Timezone
                .merge_with(&DataType::Timezone, true)
                .unwrap(),
            DataType::Timezone
        );
        let refused = DataType::Timezone
            .merge_with(&DataType::utf8(), true)
            .unwrap_err()
            .to_string();
        assert!(refused.contains("timezone"), "{refused}");

        let typed = TimezoneField::unit("zone", true);
        assert_eq!(typed.dtype(), &DataType::Timezone);
        let typed_field = typed.to_field();
        let scalar = FieldScalar::new(&typed_field, zone("UTC")).unwrap();
        assert_eq!(scalar.dtype(), &DataType::Timezone);
        assert!(FieldScalar::new(&typed.to_field(), 7_i64).is_err());
    }

    #[test]
    fn the_value_type_is_the_one_every_temporal_already_carries() {
        // Not a second zone type: the scalar carries `yggdryl::Timezone`, so a zone
        // read out of a column is the zone a datetime column declares.
        let value = zone("America/New_York");
        let Scalar::Timezone(held) = &value else {
            panic!("expected a timezone scalar")
        };
        let held: Timezone = *held;
        assert_eq!(held.offset_at(1_700_000_000), Some(-5 * 3600));
        assert_eq!(held.abbreviation_at(1_688_000_000), Some("EDT"));
        assert_eq!(
            Scalar::datetime64(0, yggdryl::TimeUnit::Second, held)
                .unwrap()
                .temporal_timezone(),
            Some(held)
        );
    }
}
