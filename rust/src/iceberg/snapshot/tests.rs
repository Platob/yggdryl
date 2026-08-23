//! Branches, tags, and the retention their references declare.

use smol_str::SmolStr;

use super::{MAIN_BRANCH, Snapshot, SnapshotRef};
use crate::iceberg::metadata::now_ms;
use crate::iceberg::{FormatVersion, PartitionSpec, TableMetadata, assign_field_ids};
use crate::{DataType, Field};

/// The one-column schema every branching test starts from.
fn schema() -> Field {
    let mut schema = DataType::from_fields([DataType::Int64.required_field("id")])
        .unwrap()
        .required_field("row");
    assign_field_ids(&mut schema, 1).unwrap();
    schema.insert_metadata("iceberg:schema-id", "0").unwrap();
    schema
}

/// A v2 table over [`schema`], unpartitioned and never written to.
fn table() -> TableMetadata {
    TableMetadata::new(
        FormatVersion::V2,
        "file:///tmp/branches",
        schema(),
        PartitionSpec::unpartitioned(),
    )
    .unwrap()
}

/// One snapshot in a parent chain, committed at `timestamp_ms`.
fn snapshot(snapshot_id: i64, parent_snapshot_id: Option<i64>, timestamp_ms: i64) -> Snapshot {
    Snapshot {
        snapshot_id,
        parent_snapshot_id,
        sequence_number: Some(snapshot_id),
        timestamp_ms,
        manifest_list: SmolStr::new_static("file:///tmp/branches/metadata/snap.avro"),
        summary: vec![(
            SmolStr::new_static("operation"),
            SmolStr::new_static("append"),
        )],
        schema_id: Some(0),
        first_row_id: None,
        added_rows: None,
    }
}

/// A table whose `main` holds a chain of `count` snapshots, ids `1..=count`,
/// committed `step_ms` apart with the head `step_ms` before now.
fn chained(count: i64, step_ms: i64) -> TableMetadata {
    let mut metadata = table();
    let now = now_ms();
    for id in 1..=count {
        let parent = (id > 1).then_some(id - 1);
        metadata.set_current_snapshot(snapshot(id, parent, now - (count - id + 1) * step_ms));
    }
    metadata
}

mod references {
    use super::{SnapshotRef, TableMetadata, chained};
    use crate::Value;

    #[test]
    fn a_branch_and_a_tag_report_their_kind() {
        let branch = SnapshotRef::branch(7);
        assert_eq!(branch.snapshot_id, 7);
        assert_eq!(branch.kind, "branch");
        assert!(branch.is_branch());
        assert!(!branch.is_tag());

        let tag = SnapshotRef::tag(7);
        assert_eq!(tag.snapshot_id, 7);
        assert_eq!(tag.kind, "tag");
        assert!(tag.is_tag());
        assert!(!tag.is_branch());

        // A fresh reference declares no retention at all.
        assert_eq!(branch.min_snapshots_to_keep, None);
        assert_eq!(branch.max_snapshot_age_ms, None);
        assert_eq!(branch.max_ref_age_ms, None);
    }

    #[test]
    fn retention_setters_keep_their_values_and_refuse_non_positive_ones() {
        let branch = SnapshotRef::branch(7)
            .with_min_snapshots_to_keep(3)
            .unwrap()
            .with_max_snapshot_age_ms(86_400_000)
            .unwrap()
            .with_max_ref_age_ms(604_800_000)
            .unwrap();
        assert_eq!(branch.min_snapshots_to_keep, Some(3));
        assert_eq!(branch.max_snapshot_age_ms, Some(86_400_000));
        assert_eq!(branch.max_ref_age_ms, Some(604_800_000));

        let message = SnapshotRef::branch(7)
            .with_min_snapshots_to_keep(0)
            .unwrap_err()
            .to_string();
        assert!(
            message.contains("positive min-snapshots-to-keep"),
            "{message}"
        );
        assert!(message.contains("got 0"), "{message}");
        let message = SnapshotRef::branch(7)
            .with_max_snapshot_age_ms(-5)
            .unwrap_err()
            .to_string();
        assert!(
            message.contains("positive max-snapshot-age-ms"),
            "{message}"
        );
        assert!(message.contains("got -5"), "{message}");
        let message = SnapshotRef::branch(7)
            .with_max_ref_age_ms(0)
            .unwrap_err()
            .to_string();
        assert!(message.contains("positive max-ref-age-ms"), "{message}");
        assert!(message.contains("got 0"), "{message}");
    }

    #[test]
    fn a_tag_refuses_the_branch_only_retention_fields_by_name() {
        let message = SnapshotRef::tag(7)
            .with_min_snapshots_to_keep(2)
            .unwrap_err()
            .to_string();
        assert!(message.contains("min-snapshots-to-keep"), "{message}");
        assert!(message.contains("tag"), "{message}");
        let message = SnapshotRef::tag(7)
            .with_max_snapshot_age_ms(1_000)
            .unwrap_err()
            .to_string();
        assert!(message.contains("max-snapshot-age-ms"), "{message}");
        assert!(message.contains("tag"), "{message}");

        // The ref's own age is not about ancestors, so a tag may bound it.
        let tag = SnapshotRef::tag(7).with_max_ref_age_ms(1_000).unwrap();
        assert_eq!(tag.max_ref_age_ms, Some(1_000));
    }

    #[test]
    fn a_reference_round_trips_through_json_with_and_without_retention() {
        let bare = SnapshotRef::tag(7);
        let document = bare.clone().into_json().unwrap();
        assert!(document.get_key_str("min-snapshots-to-keep").is_none());
        assert!(document.get_key_str("max-snapshot-age-ms").is_none());
        assert!(document.get_key_str("max-ref-age-ms").is_none());
        assert_eq!(SnapshotRef::from_json(&document).unwrap(), bare);

        let retained = SnapshotRef::branch(9)
            .with_min_snapshots_to_keep(2)
            .unwrap()
            .with_max_snapshot_age_ms(1_000)
            .unwrap()
            .with_max_ref_age_ms(2_000)
            .unwrap();
        let document = retained.clone().into_json().unwrap();
        assert_eq!(
            document
                .get_key_str("min-snapshots-to-keep")
                .and_then(Value::as_i64),
            Some(2)
        );
        assert_eq!(
            document
                .get_key_str("max-snapshot-age-ms")
                .and_then(Value::as_i64),
            Some(1_000)
        );
        assert_eq!(
            document
                .get_key_str("max-ref-age-ms")
                .and_then(Value::as_i64),
            Some(2_000)
        );
        assert_eq!(SnapshotRef::from_json(&document).unwrap(), retained);
    }

    #[test]
    fn refs_round_trip_through_the_table_document_with_and_without_retention() {
        let mut metadata = chained(3, 1_000);
        metadata.create_tag("v1", 1).unwrap();
        metadata
            .set_snapshot_ref(
                "dev",
                SnapshotRef::branch(2)
                    .with_min_snapshots_to_keep(2)
                    .unwrap()
                    .with_max_snapshot_age_ms(86_400_000)
                    .unwrap()
                    .with_max_ref_age_ms(604_800_000)
                    .unwrap(),
            )
            .unwrap();

        let document = metadata.clone().into_json().unwrap();
        let read = TableMetadata::from_json(&document).unwrap();
        assert_eq!(read.refs, metadata.refs);
        assert_eq!(read.into_json().unwrap(), document);
    }
}

mod branches {
    use super::{MAIN_BRANCH, SnapshotRef, chained};

    #[test]
    fn create_branch_and_create_tag_point_at_a_retained_snapshot() {
        let mut metadata = chained(3, 1_000);
        metadata.create_branch("dev", 2).unwrap();
        metadata.create_tag("v1", 1).unwrap();
        assert!(metadata.ref_by_name("dev").unwrap().is_branch());
        assert_eq!(metadata.ref_by_name("dev").unwrap().snapshot_id, 2);
        assert!(metadata.ref_by_name("v1").unwrap().is_tag());
        assert_eq!(metadata.ref_by_name("v1").unwrap().snapshot_id, 1);
        assert_eq!(
            metadata.current_snapshot_id,
            Some(3),
            "neither creation moves the current snapshot"
        );
        assert!(metadata.ref_by_name("missing").is_none());
        metadata.validate().unwrap();
    }

    #[test]
    fn an_existing_name_is_refused_naming_its_kind() {
        let mut metadata = chained(3, 1_000);
        metadata.create_branch("dev", 2).unwrap();
        metadata.create_tag("v1", 1).unwrap();

        let message = metadata.create_branch("dev", 3).unwrap_err().to_string();
        assert!(message.contains("no ref named \"dev\""), "{message}");
        assert!(message.contains("branch"), "{message}");
        let message = metadata.create_tag("dev", 3).unwrap_err().to_string();
        assert!(message.contains("branch"), "{message}");
        let message = metadata.create_branch("v1", 3).unwrap_err().to_string();
        assert!(message.contains("no ref named \"v1\""), "{message}");
        assert!(message.contains("tag"), "{message}");
        assert_eq!(
            metadata.ref_by_name("dev").unwrap().snapshot_id,
            2,
            "a refusal moves nothing"
        );
    }

    #[test]
    fn an_unretained_snapshot_is_refused_for_both_kinds() {
        let mut metadata = chained(3, 1_000);
        let message = metadata.create_branch("dev", 99).unwrap_err().to_string();
        assert!(message.contains("dev"), "{message}");
        assert!(message.contains("99"), "{message}");
        let message = metadata.create_tag("v1", 99).unwrap_err().to_string();
        assert!(message.contains("99"), "{message}");
        assert_eq!(
            metadata.refs.len(),
            1,
            "only main exists after the refusals"
        );
    }

    #[test]
    fn rename_ref_keeps_the_reference_under_the_new_name() {
        let mut metadata = chained(3, 1_000);
        metadata
            .set_snapshot_ref(
                "dev",
                SnapshotRef::branch(2)
                    .with_min_snapshots_to_keep(4)
                    .unwrap(),
            )
            .unwrap();
        let before = metadata.ref_by_name("dev").unwrap().clone();
        metadata.rename_ref("dev", "feature").unwrap();
        assert!(metadata.ref_by_name("dev").is_none());
        assert_eq!(
            metadata.ref_by_name("feature"),
            Some(&before),
            "retention travels with the name"
        );
    }

    #[test]
    fn renaming_main_a_missing_ref_or_onto_an_existing_name_is_refused() {
        let mut metadata = chained(3, 1_000);
        metadata.create_branch("dev", 2).unwrap();

        let message = metadata
            .rename_ref(MAIN_BRANCH, "trunk")
            .unwrap_err()
            .to_string();
        assert!(message.contains("\"main\""), "{message}");
        let message = metadata
            .rename_ref("missing", "trunk")
            .unwrap_err()
            .to_string();
        assert!(message.contains("\"missing\""), "{message}");
        assert!(message.contains("2 refs"), "{message}");
        let message = metadata.rename_ref("dev", "main").unwrap_err().to_string();
        assert!(message.contains("no ref named \"main\""), "{message}");
        assert!(
            metadata.ref_by_name("dev").is_some(),
            "a refusal renames nothing"
        );
    }

    #[test]
    fn fast_forward_branch_moves_a_branch_to_a_descendant() {
        let mut metadata = chained(3, 1_000);
        metadata.create_branch("dev", 1).unwrap();
        metadata.fast_forward_branch("dev", 3).unwrap();
        assert_eq!(metadata.ref_by_name("dev").unwrap().snapshot_id, 3);
        assert_eq!(
            metadata.current_snapshot_id,
            Some(3),
            "moving another branch leaves main alone"
        );
    }

    #[test]
    fn fast_forwarding_main_moves_the_current_snapshot_and_the_log_with_it() {
        let mut metadata = chained(3, 1_000);
        // Point main back at the root, then fast-forward it to the head again.
        metadata
            .set_snapshot_ref(MAIN_BRANCH, SnapshotRef::branch(1))
            .unwrap();
        assert_eq!(metadata.current_snapshot_id, Some(1));
        metadata.fast_forward_branch(MAIN_BRANCH, 3).unwrap();
        assert_eq!(metadata.current_snapshot_id, Some(3));
        assert_eq!(metadata.snapshot_log.last().map(|(_, id)| *id), Some(3));
    }

    #[test]
    fn a_non_descendant_fast_forward_is_refused_naming_both_ids() {
        let mut metadata = chained(3, 1_000);
        // Backwards is exactly the move a fast-forward must not make.
        let message = metadata
            .fast_forward_branch(MAIN_BRANCH, 1)
            .unwrap_err()
            .to_string();
        assert!(
            message.contains("expected 1 to descend from 3"),
            "{message}"
        );
        assert_eq!(
            metadata.ref_by_name(MAIN_BRANCH).unwrap().snapshot_id,
            3,
            "a refusal moves nothing"
        );
    }

    #[test]
    fn fast_forwarding_a_tag_a_missing_ref_or_to_an_unknown_snapshot_is_refused() {
        let mut metadata = chained(3, 1_000);
        metadata.create_tag("v1", 1).unwrap();
        let message = metadata
            .fast_forward_branch("v1", 3)
            .unwrap_err()
            .to_string();
        assert!(message.contains("\"v1\""), "{message}");
        assert!(message.contains("tag"), "{message}");
        let message = metadata
            .fast_forward_branch("missing", 3)
            .unwrap_err()
            .to_string();
        assert!(message.contains("\"missing\""), "{message}");
        let message = metadata
            .fast_forward_branch(MAIN_BRANCH, 99)
            .unwrap_err()
            .to_string();
        assert!(message.contains("99"), "{message}");
    }
}

mod expiration {
    use super::{MAIN_BRANCH, SnapshotRef, chained, now_ms, table};

    #[test]
    fn a_chain_of_five_expires_exactly_what_retention_leaves_unprotected() {
        // Ids 1..=5, committed 100 seconds apart, the head 100 seconds ago.
        let mut metadata = chained(5, 100_000);
        metadata
            .set_snapshot_ref(
                MAIN_BRANCH,
                SnapshotRef::branch(5)
                    .with_min_snapshots_to_keep(2)
                    .unwrap(),
            )
            .unwrap();
        // A week-lived tag anchors snapshot 2, which is 400 seconds old.
        metadata
            .set_snapshot_ref(
                "v1",
                SnapshotRef::tag(2)
                    .with_max_ref_age_ms(604_800_000)
                    .unwrap(),
            )
            .unwrap();

        // Only the last 150 seconds are young: snapshot 5 survives by age,
        // 4 by count, 2 by the tag; 1 and 3 have nothing keeping them.
        let cutoff = now_ms() - 150_000;
        assert_eq!(
            metadata.expire_snapshots_older_than(cutoff).unwrap(),
            vec![1, 3]
        );
        assert_eq!(
            metadata.current_snapshot_id,
            Some(5),
            "the current snapshot is never removed"
        );
        assert!(
            metadata.snapshot_by_id(2).is_some(),
            "the tag target survives while its ref lives"
        );
        assert!(
            metadata.snapshot_by_id(4).is_some(),
            "min-snapshots-to-keep holds the fourth"
        );
        metadata.validate().unwrap();

        // Once the tag itself is too old, it expires first, freeing its target.
        metadata
            .set_snapshot_ref(
                "v1",
                SnapshotRef::tag(2).with_max_ref_age_ms(50_000).unwrap(),
            )
            .unwrap();
        assert_eq!(
            metadata.expire_snapshots_older_than(cutoff).unwrap(),
            vec![2]
        );
        assert!(
            metadata.ref_by_name("v1").is_none(),
            "an expired ref is removed"
        );
        assert_eq!(
            metadata
                .snapshots
                .iter()
                .map(|snapshot| snapshot.snapshot_id)
                .collect::<Vec<_>>(),
            vec![4, 5]
        );
        assert_eq!(
            metadata
                .snapshot_log
                .iter()
                .map(|(_, id)| *id)
                .collect::<Vec<_>>(),
            vec![4, 5],
            "the log is trimmed with the snapshots"
        );
        metadata.validate().unwrap();
    }

    #[test]
    fn a_branch_with_its_own_age_limit_overrides_the_default_cutoff() {
        let mut metadata = chained(5, 100_000);
        metadata
            .set_snapshot_ref(
                MAIN_BRANCH,
                SnapshotRef::branch(5)
                    .with_max_snapshot_age_ms(250_000)
                    .unwrap(),
            )
            .unwrap();
        // The default cutoff would keep everything; the branch's own 250
        // second limit keeps only the two youngest snapshots.
        let removed = metadata
            .expire_snapshots_older_than(now_ms() - 600_000)
            .unwrap();
        assert_eq!(removed, vec![1, 2, 3]);
        assert_eq!(metadata.snapshots.len(), 2);
    }

    #[test]
    fn expiring_a_table_with_nothing_old_is_ok_and_empty() {
        let mut metadata = chained(3, 1_000);
        // Everything is younger than a cutoff a week in the past.
        assert_eq!(
            metadata
                .expire_snapshots_older_than(now_ms() - 604_800_000)
                .unwrap(),
            Vec::<i64>::new()
        );
        assert_eq!(metadata.snapshots.len(), 3, "nothing was removed");

        // A table with no snapshots at all has nothing to remove either.
        let mut empty = table();
        assert_eq!(
            empty.expire_snapshots_older_than(now_ms()).unwrap(),
            Vec::<i64>::new()
        );
    }
}

mod validation {
    use super::{SmolStr, SnapshotRef, TableMetadata, chained};

    #[test]
    fn validate_names_a_tag_carrying_a_branch_retention_field() {
        let mut metadata = chained(3, 1_000);
        // The bad ref is built by mutating the public fields directly, because
        // every method refuses to produce it.
        metadata.refs.push((
            SmolStr::new_static("v1"),
            SnapshotRef {
                snapshot_id: 1,
                kind: SmolStr::new_static("tag"),
                min_snapshots_to_keep: Some(2),
                max_snapshot_age_ms: None,
                max_ref_age_ms: None,
            },
        ));
        let message = metadata.validate().unwrap_err().to_string();
        assert!(message.contains("min-snapshots-to-keep"), "{message}");
        assert!(message.contains("\"v1\""), "{message}");
        assert!(message.contains("tag"), "{message}");

        metadata.refs.last_mut().unwrap().1 = SnapshotRef {
            snapshot_id: 1,
            kind: SmolStr::new_static("tag"),
            min_snapshots_to_keep: None,
            max_snapshot_age_ms: Some(1_000),
            max_ref_age_ms: None,
        };
        let message = metadata.validate().unwrap_err().to_string();
        assert!(message.contains("max-snapshot-age-ms"), "{message}");

        // A malformed document can be written but never read back quietly.
        let document = metadata.into_json().unwrap();
        assert!(TableMetadata::from_json(&document).is_err());
    }
}
