//! `rust/src/metadata.rs`: the shared metadata map no caller can reach.
//!
//! A metadata map is one shared pointer, and that it is shared - by an empty
//! map, by a clone - is the whole reason a field clone costs nothing. The
//! pointer is private, so the pin is here, beside what a caller can observe;
//! the metadata a field carries is in `rust/tests/metadata/`.

#[cfg(feature = "internals")]
mod internal {
    use yggdryl::Metadata;
    use yggdryl::internals::metadata::{remove, shares_storage_with};

    #[test]
    fn empty_and_cloned_metadata_share_their_backing_map() {
        let empty = Metadata::new();
        let other_empty = Metadata::new();
        assert!(shares_storage_with(&empty, &other_empty));

        let metadata = Metadata::from_entries([("source", "orders")]).unwrap();
        let clone = metadata.clone();
        assert!(shares_storage_with(&metadata, &clone));
    }

    /// Removal takes a folded protocol key, which is what a field's own
    /// `without_*` reaches through. The method is crate-private, so the pin is
    /// here.
    #[test]
    fn removing_by_any_spelling_takes_the_one_folded_entry() {
        let mut metadata =
            Metadata::from_entries([("HTTPS:Content-Type", "text/plain; charset=utf-8")]).unwrap();
        assert_eq!(
            remove(&mut metadata, "HTTPS:CONTENT-TYPE").as_deref(),
            Some("text/plain; charset=utf-8")
        );
        assert!(!metadata.contains_key("HTTP:content-type"));
        assert_eq!(remove(&mut metadata, "HTTP:content-type"), None);
    }
}

mod generic {

    use std::sync::Arc;

    use yggdryl::{DataType, Field, Metadata, Scheme, Url};

    #[test]
    fn metadata_behaves_as_a_sorted_transactional_mapping() {
        let mut field = Field::new("id", DataType::Int64, false);
        field
            .update_metadata([("z", "last"), ("a", "first")])
            .unwrap();
        assert_eq!(field.metadata_len(), 2);
        assert!(!field.is_metadata_empty());
        assert!(field.has_metadata("a"));
        // Metadata is reached through its view, never by subscripting the node:
        // `field["..."]` descends into children.
        assert_eq!(field.get_metadata("z"), Some("last"));
        assert_eq!(field.as_metadata().get("z"), Some("last"));
        assert_eq!(
            field.metadata_iter().collect::<Vec<_>>(),
            [("a", "first"), ("z", "last")]
        );

        let snapshot = field.clone();
        assert!(
            field
                .set_metadata([("dup", "one"), ("dup", "two")])
                .is_err()
        );
        assert_eq!(field, snapshot);
        assert!(field.insert_metadata("", "bad").is_err());
        assert_eq!(field.remove_metadata("a").as_deref(), Some("first"));
        field.clear_metadata();
        assert!(field.is_metadata_empty());
    }

    #[test]
    fn metadata_is_a_deterministic_shared_native_value() {
        let metadata = Metadata::from_entries([("z", "last"), ("a", "first")]).unwrap();
        assert_eq!(
            metadata.iter().collect::<Vec<_>>(),
            [("a", "first"), ("z", "last")]
        );
        assert_eq!(metadata.next_entry(None), Some(("a", "first")));
        assert_eq!(metadata.next_entry(Some("a")), Some(("z", "last")));

        let json = metadata.clone().into_json().unwrap();
        assert_eq!(json, r#"{"a":"first","z":"last"}"#);
        assert_eq!(Metadata::from_json(&json).unwrap(), metadata);
        assert_eq!(metadata.to_string().parse::<Metadata>().unwrap(), metadata);
        assert_eq!(metadata.clone().into_json().unwrap(), json);
        assert_eq!(metadata.clone().into_iter().count(), 2);
        assert_eq!(
            metadata.as_ref().get("a").map(String::as_str),
            Some("first")
        );
        assert_eq!(metadata.stable_hash(), metadata.clone().stable_hash());

        let arrow = metadata.clone().into_arrow_metadata();
        assert_eq!(Metadata::from_arrow_metadata(&arrow).unwrap(), metadata);
    }

    #[test]
    fn typed_names_location_and_protocol_properties_share_one_metadata_map() {
        let location = Url::from_str("HTTPS://example.com/warehouse/table").unwrap();
        let mut field = Field::new("trade", DataType::utf8(), false);
        field.set_alias("latest_trade").unwrap();
        field.set_comment("the latest trade").unwrap();
        field.set_display("Latest trade").unwrap();
        // Catalog coordinates belong to whichever protocol names them, not to
        // straight metadata.
        field
            .protocol_mut(&Scheme::ICEBERG)
            .insert("table_name", "trades")
            .unwrap();
        field.set_location(location.clone());
        assert_eq!(field.alias(), Some("latest_trade"));
        assert_eq!(field.comment(), Some("the latest trade"));
        assert_eq!(field.display(), Some("Latest trade"));
        assert_eq!(
            field.get_property(&Scheme::ICEBERG, "table_name"),
            Some("trades")
        );
        assert_eq!(field.get_metadata("table_name"), None);
        assert_eq!(field.location().unwrap(), Some(location.clone()));
        assert_eq!(
            field.get_metadata("location"),
            Some(location.to_string().as_str())
        );

        assert_eq!(
            field
                .set_property(&Scheme::POSTGRES, "ddl", "CREATE TABLE trades")
                .unwrap(),
            None
        );
        field
            .set_property(&Scheme::POSTGRES, "comment", "line one\nline two")
            .unwrap();
        field.set_property(&Scheme::POSTGRES, "empty", "").unwrap();
        field
            .set_property(&Scheme::ICEBERG, "format-version", "2")
            .unwrap();
        assert!(field.has_property(&Scheme::POSTGRES, "ddl"));
        assert_eq!(
            field.property_iter(&Scheme::POSTGRES).collect::<Vec<_>>(),
            [
                ("comment", "line one\nline two"),
                ("ddl", "CREATE TABLE trades"),
                ("empty", "")
            ]
        );
        assert_eq!(
            field.next_property_entry(&Scheme::POSTGRES, Some("comment")),
            Some(("ddl", "CREATE TABLE trades"))
        );

        let cached = Arc::new(field.clone().into_arrow_field().unwrap());
        let mut field = Field::from_arrow_field_ref(Arc::clone(&cached)).unwrap();
        field.set_alias("latest_trade").unwrap();
        field
            .set_property(&Scheme::POSTGRES, "ddl", "CREATE TABLE trades")
            .unwrap();
        field.set_location(location);
        assert!(Arc::ptr_eq(
            &cached,
            &field.clone().into_arrow_field_ref().unwrap()
        ));

        field.clear_properties(&Scheme::POSTGRES);
        assert!(!field.has_property(&Scheme::POSTGRES, "ddl"));
        assert_eq!(
            field.get_property(&Scheme::ICEBERG, "format-version"),
            Some("2")
        );
        assert!(!Arc::ptr_eq(
            &cached,
            &field.clone().into_arrow_field_ref().unwrap()
        ));

        assert_eq!(field.remove_alias().as_deref(), Some("latest_trade"));
        assert_eq!(field.remove_comment().as_deref(), Some("the latest trade"));
        assert_eq!(field.remove_display().as_deref(), Some("Latest trade"));
        assert!(field.remove_location().unwrap().is_some());
    }

    #[test]
    fn metadata_merges_as_a_union_the_receiver_wins() {
        let held = Metadata::from_entries([("owner", "held"), ("only_held", "1")]).unwrap();
        let other = Metadata::from_entries([("owner", "other"), ("only_other", "2")]).unwrap();

        let merged = held.merge_with(&other).unwrap();

        // Every key arrives, and the receiver wins the one they disagree on.
        assert_eq!(merged.get("owner"), Some("held"));
        assert_eq!(merged.get("only_held"), Some("1"));
        assert_eq!(merged.get("only_other"), Some("2"));

        // The direction is the whole rule, so the other way round differs.
        assert_eq!(other.merge_with(&held).unwrap().get("owner"), Some("other"));

        // Merging with itself changes nothing.
        assert_eq!(held.merge_with(&held).unwrap(), held);
    }
}

use yggdryl::Metadata;

#[test]
fn unique_arrow_projection_moves_string_allocations() {
    let key = "protocol-property-key-longer-than-inline-storage";
    let value = "protocol-property-value-longer-than-inline-storage";
    let metadata = Metadata::from_entries([(key, value)]).unwrap();
    let (stored_key, stored_value) = metadata.iter().next().unwrap();
    let key_pointer = stored_key.as_ptr();
    let value_pointer = stored_value.as_ptr();

    let arrow = metadata.into_arrow_metadata();
    let (arrow_key, arrow_scalar) = arrow.iter().next().unwrap();
    assert_eq!(arrow_key.as_ptr(), key_pointer);
    assert_eq!(arrow_scalar.as_ptr(), value_pointer);
}
