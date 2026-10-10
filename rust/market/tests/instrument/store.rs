//! `rust/market/src/instrument/store.rs`: the store a collection is bound to -
//! loaded once at the binding under options resolved once, committed back
//! as one snapshot only where the table moved, a leaf, a plain folder or
//! an Iceberg table alike.

#[cfg(feature = "iceberg")]
use yggdryl::graph::Element;
use yggdryl::media::IORecordOptions;
use yggdryl::{Ccy, IOBase, IOMedia, IOMode, IOResult, Isin, Mic, Url};
use yggdryl_market::{IdType, Instrument, Instruments, Listing};

use crate::counting_filesystem::counted_folder;

const HOLCIM: &str = "CH0012214059";
const NOVARTIS: &str = "CH0012005267";
const APPLE: &str = "US0378331005";
const MICROSOFT: &str = "US5949181045";
/// A real ISIN the seed holds no row of.
const BAE: &str = "GB0002634946";

/// A catalog handle retains its table identity through loading, committing and
/// clearing. Its warehouse never needs to be discovered again.
#[cfg(feature = "iceberg")]
#[test]
fn a_native_table_store_keeps_its_metadata_and_never_rediscovers_its_warehouse() {
    crate::install::installed();
    use yggdryl::holder::Holder;
    use yggdryl::iceberg::{FormatVersion, IcebergTable, PartitionSpec};
    use yggdryl::warehouse::Handle;
    let (filesystem, _) = counted_folder("native-registry");
    let schema = Instrument::field()
        .into_scheme_compat(&yggdryl::Scheme::ICEBERG)
        .unwrap();
    let table = IcebergTable::create(
        Handle::from(folder_on(&filesystem, "instruments")),
        FormatVersion::V3,
        schema,
        PartitionSpec::unpartitioned(),
    )
    .unwrap();
    let holder = Holder::from(yggdryl::Table::from(table));
    let mut registry = None;
    assert_eq!(
        filesystem.costs(|| registry = Some(Instruments::from_holder(holder).unwrap())),
        "none",
        "the table already holds its schema and empty snapshot"
    );
    let mut registry = registry.unwrap();
    registry
        .merge(entry(HOLCIM, &[(IdType::Ric, "HOLN.S")]))
        .unwrap();
    registry.merge(entry(APPLE, &[])).unwrap();
    assert_eq!(registry.commit().unwrap().written_rows, 2);
    assert_eq!(
        filesystem.costs(|| assert_eq!(registry.commit().unwrap(), IOResult::default())),
        "none"
    );
    assert_eq!(
        Instruments::from_holder(match registry.holder().unwrap() {
            Holder::Table(table) => Holder::Table(table.clone()),
            _ => unreachable!("the registry retains its native table"),
        })
        .unwrap()
        .len(),
        2
    );
    registry.clear();
    assert_eq!(registry.commit().unwrap().written_rows, 0);
    assert!(
        Instruments::from_holder(match registry.holder().unwrap() {
            Holder::Table(table) => Holder::Table(table.clone()),
            _ => unreachable!("the registry retains its native table"),
        })
        .unwrap()
        .is_empty()
    );
}

/// The row of `text` stating each of `codes`.
fn entry(text: &str, codes: &[(IdType, &str)]) -> Instrument {
    codes.iter().fold(
        Instrument::for_security(Isin::new(text).unwrap()).unwrap(),
        |entry, (kind, value)| entry.try_with_code(kind.clone(), value).unwrap(),
    )
}

/// The folder `name` on `filesystem`, as a holder.
#[cfg(feature = "iceberg")]
fn folder_on(
    filesystem: &std::sync::Arc<crate::counting_filesystem::CountingFileSystem>,
    name: &str,
) -> yggdryl::holder::Holder {
    use std::sync::Arc;
    use yggdryl::fs::{FileSystem, FsFolder};
    yggdryl::holder::Holder::FsFolder(
        FsFolder::from_path(Arc::clone(filesystem) as Arc<dyn FileSystem>, name, None)
            .expect("a location"),
    )
}

#[test]
fn an_unbound_registry_refuses_to_commit() {
    crate::install::installed();
    let mut registry = Instruments::new();
    registry.merge(entry(HOLCIM, &[])).unwrap();
    assert!(registry.holder().is_none());
    let refused = registry.commit().unwrap_err().to_string();
    assert!(refused.contains("holder"), "{refused}");
    assert!(registry.is_dirty(), "a refused commit leaves it dirty");
}

/// A leaf store: an empty first run bound clean, a clean commit costing no
/// call, a dirty one exactly the one overwrite of the snapshot, a reload
/// through another handle the rows as committed, and an emptied registry
/// clearing the leaf.
#[test]
fn a_leaf_store_loads_and_commits_only_where_the_table_moved() {
    crate::install::installed();
    let (filesystem, folder) = counted_folder("isin");
    let leaf = || folder.child_by_path("instruments.arrows").unwrap();
    let mut registry = Instruments::from_holder(leaf()).unwrap();
    assert!(registry.is_empty());
    assert!(!registry.is_dirty());
    assert!(registry.holder().is_some());
    assert_eq!(
        filesystem.costs(|| assert_eq!(registry.commit().unwrap(), IOResult::default())),
        "none",
        "a clean commit"
    );
    registry
        .merge(
            entry(HOLCIM, &[(IdType::Ric, "HOLN.S")])
                .try_with_underlying(Some(APPLE))
                .unwrap(),
        )
        .unwrap();
    registry.merge(entry(APPLE, &[])).unwrap();
    assert!(registry.is_dirty());
    let committed = filesystem.costs(|| {
        let result = registry.commit().unwrap();
        assert_eq!(result.written_rows, 2);
    });
    assert_ne!(committed, "none");
    assert!(!registry.is_dirty());
    assert_eq!(
        filesystem.costs(|| assert_eq!(registry.commit().unwrap(), IOResult::default())),
        "none",
        "a second commit"
    );
    assert!(
        !registry
            .merge(entry(HOLCIM, &[(IdType::Ric, "HOLN.S")]))
            .unwrap()
    );
    assert_eq!(
        filesystem.costs(|| assert_eq!(registry.commit().unwrap(), IOResult::default())),
        "none",
        "a merge moving nothing, then a commit"
    );

    // A dirty commit is exactly the one overwrite of the same rows, under
    // the options the binding resolved once.
    let mut handle = leaf();
    let options = handle
        .record_options()
        .unwrap()
        .with_field(Instrument::field());
    let direct = filesystem.costs(|| {
        handle
            .write_arrow_reader(
                registry.into_arrow_reader().unwrap(),
                IOMode::Overwrite,
                &options,
            )
            .unwrap();
    });
    assert!(
        registry
            .merge(entry(HOLCIM, &[(IdType::Ric, "HOLN.VX")]))
            .unwrap()
    );
    assert_eq!(
        filesystem.costs(|| assert_eq!(registry.commit().unwrap().written_rows, 2)),
        direct,
        "a dirty commit"
    );

    // Reloaded through another handle on the store: the rows as committed,
    // for the holder's kind read once at the binding, its own options and
    // one record read under them.
    let back = Instruments::from_holder(leaf()).unwrap();
    assert!(back.iter().eq(registry.iter()));
    assert!(!back.is_dirty());
    let reading = filesystem.costs(|| {
        let handle = leaf();
        assert!(!handle.is_container());
        for batch in handle
            .read_arrow_reader(&handle.record_options().unwrap())
            .unwrap()
        {
            batch.unwrap();
        }
    });
    assert_eq!(
        filesystem.costs(|| assert_eq!(Instruments::from_holder(leaf()).unwrap().len(), 2)),
        reading,
        "a load"
    );

    // Emptied: the leaf is cleared, and a reload is empty.
    registry.clear();
    assert!(registry.is_dirty());
    registry.commit().unwrap();
    assert!(!registry.is_dirty());
    assert!(Instruments::from_holder(leaf()).unwrap().is_empty());
}

/// Binding a registry already holding rows loads the store's and folds the
/// held ones over them, dirty exactly where a held row moved something; a
/// binding that fails leaves the registry as it was, bound to nothing.
#[test]
fn binding_a_registry_holding_rows_folds_them_over_the_stores() {
    crate::install::installed();
    let (_, folder) = counted_folder("isin");
    let leaf = || folder.child_by_path("instruments.arrows").unwrap();
    let mut stored = Instruments::from_holder(leaf()).unwrap();
    stored
        .merge(entry(
            HOLCIM,
            &[(IdType::Ric, "HOLN.S"), (IdType::Common, "C-1")],
        ))
        .unwrap();
    stored.commit().unwrap();

    let mut same = Instruments::new();
    same.merge(entry(
        HOLCIM,
        &[(IdType::Ric, "HOLN.S"), (IdType::Common, "C-1")],
    ))
    .unwrap();
    assert_eq!(same.set_holder(leaf()).unwrap(), 1);
    assert!(!same.is_dirty(), "the held row moved nothing");
    assert_eq!(same.len(), 1);
    assert!(same.holder().is_some());

    let mut more = Instruments::new();
    more.merge(entry(HOLCIM, &[(IdType::Common, "C-2")]))
        .unwrap();
    more.merge(entry(APPLE, &[])).unwrap();
    let bound = more.try_with_holder(leaf()).unwrap();
    assert!(bound.is_dirty(), "the held rows moved the store's");
    assert_eq!(bound.len(), 2);
    let holcim = bound.get(HOLCIM).unwrap();
    assert_eq!(holcim.get(&IdType::Common), Some("C-2"));
    assert_eq!(holcim.get(&IdType::Ric), Some("HOLN.S"));

    let mut bounded = Instruments::new().with_max_instruments(1);
    bounded.merge(entry(APPLE, &[])).unwrap();
    assert!(
        bounded.set_holder(leaf()).is_err(),
        "two rows past a bound of one"
    );
    assert_eq!(bounded.len(), 1);
    assert!(bounded.get(APPLE).is_some());
    assert!(bounded.holder().is_none());
    assert!(bounded.is_dirty());
}

/// A commit compares content with what the store holds: an instrument whose
/// fact moved and moved back since the load is unchanged - two sources
/// disagreeing on one metadata key within a run leave the last statement
/// and, where the run ends as the store stands, nothing to write - while a
/// fact that stays moved, a later `lastunix`, an instrument added or one
/// removed are changes; `updunix` alone, which follows the flips, is none.
#[test]
fn a_fact_that_moves_and_moves_back_is_no_change_to_the_store() {
    crate::install::installed();
    let (filesystem, folder) = counted_folder("flip");
    let leaf = || folder.child_by_path("instruments.arrows").unwrap();
    let mut registry = Instruments::from_holder(leaf()).unwrap();
    // Every statement within the one window, dated by when its fact moved.
    let typed = |securitytype: &str, updunix: i64| {
        entry(NOVARTIS, &[])
            .try_with_metadata("securitytype", securitytype)
            .unwrap()
            .with_firstunix(Some(1))
            .with_lastunix(Some(1))
            .with_updunix(Some(updunix))
    };
    let securitytype = |registry: &Instruments| {
        registry
            .get(NOVARTIS)
            .unwrap()
            .metadata()
            .get("securitytype")
            .map(|held| held.as_str().to_owned())
    };
    assert!(registry.merge(typed("CS", 1)).unwrap());
    assert_eq!(registry.commit().unwrap().written_rows, 1);
    assert!(!registry.is_dirty());

    // Within one run the FIX lines say `CS` and the bridge lines `equity`:
    // each statement replaces the other, and the run ends where the store
    // stands.
    assert!(registry.merge(typed("equity", 2)).unwrap());
    assert!(registry.is_dirty(), "a fact moved");
    assert_eq!(securitytype(&registry).as_deref(), Some("equity"));
    assert!(registry.merge(typed("CS", 3)).unwrap());
    assert_eq!(
        securitytype(&registry).as_deref(),
        Some("CS"),
        "the last statement stands"
    );
    assert_eq!(
        registry.get(NOVARTIS).unwrap().updunix(),
        Some(3),
        "the instant the fact last moved, which the content code does not read"
    );
    assert!(
        !registry.is_dirty(),
        "moved and moved back: the content is the store's"
    );
    assert_eq!(
        filesystem.costs(|| assert_eq!(registry.commit().unwrap(), IOResult::default())),
        "none",
        "nothing to write"
    );
    assert!(!registry.is_dirty());

    // A fact that stays moved is a change.
    assert!(registry.merge(typed("equity", 3)).unwrap());
    assert!(registry.is_dirty());
    assert_eq!(registry.commit().unwrap().written_rows, 1);
    assert!(!registry.is_dirty());
    // Met later than the store knows: the window moved, and that alone.
    assert!(
        registry
            .merge(typed("equity", 3).with_lastunix(Some(9)))
            .unwrap()
    );
    assert_eq!(registry.get(NOVARTIS).unwrap().updunix(), Some(3));
    assert!(registry.is_dirty(), "a later lastunix");
    assert_eq!(registry.commit().unwrap().written_rows, 1);
    // Removed and stated again as it was: no change.
    let removed = registry.remove(NOVARTIS).unwrap();
    assert!(registry.is_dirty(), "one instrument fewer");
    assert!(registry.merge(removed).unwrap());
    assert!(!registry.is_dirty(), "back as the store holds it");
    assert_eq!(registry.commit().unwrap(), IOResult::default());
    // One more is a change.
    assert!(registry.merge(entry(APPLE, &[])).unwrap());
    assert!(registry.is_dirty(), "one instrument more");
    assert_eq!(registry.commit().unwrap().written_rows, 2);
    // Loaded again, the store is the table as last written, and clean.
    let back = Instruments::from_holder(leaf()).unwrap();
    assert!(back.iter().eq(registry.iter()));
    assert!(!back.is_dirty());
}

/// The default layout: a plain folder that is not there yet is an empty
/// first run, laid out as one Arrow IPC part by the first dirty commit,
/// read back whole, rewritten in place and cleared when emptied.
#[test]
fn a_folder_store_is_laid_out_by_the_first_dirty_commit_and_read_back_whole() {
    crate::install::installed();
    let root = crate::scratch("folder").join("isin");
    // A trailing slash is what makes a location that is not there yet a
    // folder rather than a leaf.
    let url = Url::from_location(&format!("{}/", root.display())).unwrap();
    assert!(url.has_trailing_slash(), "{url}");
    let none: [(&str, &str); 0] = [];
    let mut registry = Instruments::from_url(&url, none).unwrap();
    assert!(registry.is_empty() && !registry.is_dirty());
    assert!(!root.exists(), "nothing is laid out before a commit");
    registry
        .merge(
            entry(HOLCIM, &[(IdType::Ric, "HOLN.S")])
                .try_with_underlying(Some(APPLE))
                .unwrap(),
        )
        .unwrap();
    registry.merge(entry(APPLE, &[])).unwrap();
    assert_eq!(registry.commit().unwrap().written_rows, 2);
    assert!(root.join("part-0.arrows").is_file());
    let back = Instruments::from_url(&url, none).unwrap();
    assert!(back.iter().eq(registry.iter()));
    assert!(!back.is_dirty());
    // Rewritten in place: one part still.
    registry
        .merge(entry(HOLCIM, &[(IdType::Common, "C-1")]))
        .unwrap();
    registry.commit().unwrap();
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
    assert_eq!(
        Instruments::from_url(&url, none)
            .unwrap()
            .get(HOLCIM)
            .unwrap()
            .get(&IdType::Common),
        Some("C-1")
    );
    // Emptied: the part removed, a file that is no record part kept, and
    // read back empty.
    std::fs::write(root.join("README.md"), b"the instrument registry").unwrap();
    registry.clear();
    registry.commit().unwrap();
    assert!(!root.join("part-0.arrows").exists());
    assert!(root.join("README.md").is_file());
    assert!(Instruments::from_url(&url, none).unwrap().is_empty());
    let _ = std::fs::remove_dir_all(root.parent().unwrap());
}

/// A folder laid out in partitions holds exactly the snapshot after every
/// commit: a row removed from the registry is removed from the partition
/// it was stored in, a file that is no record part is never touched, and
/// a folder holding plain text alone is laid out as Arrow IPC.
#[test]
fn a_partitioned_folder_store_holds_exactly_the_snapshot_after_every_commit() {
    crate::install::installed();
    let root = crate::scratch("partitioned").join("isin");
    std::fs::create_dir_all(root.join("isin=CH0012214059")).unwrap();
    std::fs::create_dir_all(root.join("isin=US0378331005")).unwrap();
    std::fs::write(root.join("README.md"), b"the instrument registry").unwrap();
    let url = Url::from_location(&format!("{}/", root.display())).unwrap();
    let none: [(&str, &str); 0] = [];
    let mut registry = Instruments::from_url(&url, none).unwrap();
    assert!(registry.is_empty() && !registry.is_dirty());
    registry
        .merge(
            entry(HOLCIM, &[])
                .with_listing(Listing::new(Some(yggdryl::Mic::new("XSWX").unwrap())))
                .unwrap(),
        )
        .unwrap();
    registry
        .merge(
            entry(APPLE, &[])
                .with_listing(Listing::new(Some(yggdryl::Mic::new("XNAS").unwrap())))
                .unwrap(),
        )
        .unwrap();
    assert_eq!(registry.commit().unwrap().written_rows, 2);
    assert!(root.join("isin=CH0012214059/part-0.arrows").is_file());
    assert!(root.join("isin=US0378331005/part-0.arrows").is_file());
    let back = Instruments::from_url(&url, none).unwrap();
    assert!(back.iter().eq(registry.iter()));
    // A removal reaches the partition the row was stored in.
    assert!(registry.remove(HOLCIM).is_some());
    assert_eq!(registry.commit().unwrap().written_rows, 1);
    assert!(!root.join("isin=CH0012214059/part-0.arrows").exists());
    let back = Instruments::from_url(&url, none).unwrap();
    assert_eq!(back.len(), 1);
    assert!(back.get(APPLE).is_some());
    assert!(root.join("README.md").is_file());
    // Emptied: no part anywhere, the README kept.
    registry.clear();
    registry.commit().unwrap();
    assert!(!root.join("isin=US0378331005/part-0.arrows").exists());
    assert!(root.join("README.md").is_file());
    assert!(Instruments::from_url(&url, none).unwrap().is_empty());
    let _ = std::fs::remove_dir_all(root.parent().unwrap());
}

/// A leaf names its encoding by its name: one this build has no record
/// encoding for is refused at the binding rather than laid out as another.
#[test]
fn a_leaf_of_an_unknown_encoding_is_refused_at_the_binding() {
    crate::install::installed();
    let root = crate::scratch("unknown");
    let url = Url::from_path(root.join("instruments.xyz")).unwrap();
    let none: [(&str, &str); 0] = [];
    assert!(Instruments::from_url(&url, none).is_err());
    assert!(!root.join("instruments.xyz").exists());
    let _ = std::fs::remove_dir_all(&root);
}

/// An Iceberg table at the location works transparently: bound and loaded
/// through its metadata, each dirty commit one atomic snapshot replacing
/// the rows whole, an emptied registry an empty snapshot that keeps the
/// table a table, and a clean commit no call.
#[cfg(feature = "iceberg")]
#[test]
fn an_iceberg_table_store_is_replaced_in_one_snapshot_and_emptied_as_one() {
    crate::install::installed();
    use yggdryl::iceberg::{FormatVersion, IcebergTable, PartitionSpec};

    let (filesystem, _) = counted_folder("table");
    let table = || folder_on(&filesystem, "table");
    IcebergTable::create(
        table(),
        FormatVersion::V3,
        Instrument::field()
            .into_scheme_compat(&yggdryl::Scheme::ICEBERG)
            .unwrap(),
        PartitionSpec::unpartitioned(),
    )
    .unwrap();
    let mut registry = Instruments::from_holder(table()).unwrap();
    assert!(registry.is_empty() && !registry.is_dirty());
    registry
        .merge(
            entry(HOLCIM, &[(IdType::Ric, "HOLN.S")])
                .try_with_underlying(Some(APPLE))
                .unwrap(),
        )
        .unwrap();
    registry.merge(entry(APPLE, &[])).unwrap();
    assert_eq!(registry.commit().unwrap().written_rows, 2);
    assert!(!registry.is_dirty());
    assert_eq!(
        filesystem.costs(|| assert_eq!(registry.commit().unwrap(), IOResult::default())),
        "none",
        "a clean commit"
    );
    let back = Instruments::from_holder(table()).unwrap();
    assert!(back.iter().eq(registry.iter()));
    // Replaced whole by the next commit.
    registry
        .merge(entry(HOLCIM, &[(IdType::Common, "C-1")]))
        .unwrap();
    registry.commit().unwrap();
    let back = Instruments::from_holder(table()).unwrap();
    assert_eq!(back.len(), 2);
    assert_eq!(back.get(HOLCIM).unwrap().get(&IdType::Common), Some("C-1"));
    // Emptied: an empty snapshot, still a table.
    registry.clear();
    registry.commit().unwrap();
    assert!(Instruments::from_holder(table()).unwrap().is_empty());
    assert!(IcebergTable::open(table()).is_ok());
    assert!(table().is_container());

    // A partitioned table is replaced whole too: a row removed from the
    // registry is gone from the partition it was stored in, and an emptied
    // registry empties every partition; a location inside the table - one
    // partition of it - is refused at the binding.
    use yggdryl::iceberg::FIRST_PARTITION_ID;
    let partitioned = || folder_on(&filesystem, "partitioned");
    let mut schema = Instrument::field()
        .into_scheme_compat(&yggdryl::Scheme::ICEBERG)
        .unwrap()
        .with_partition_fields(&["isin"])
        .unwrap();
    yggdryl::iceberg::assign_field_ids(&mut schema, 1).unwrap();
    let spec = PartitionSpec::from_schema(FIRST_PARTITION_ID, &schema).unwrap();
    IcebergTable::create(partitioned(), FormatVersion::V3, schema, spec).unwrap();
    let mut registry = Instruments::from_holder(partitioned()).unwrap();
    registry
        .merge(
            entry(HOLCIM, &[])
                .with_listing(Listing::new(Some(yggdryl::Mic::new("XSWX").unwrap())))
                .unwrap(),
        )
        .unwrap();
    registry
        .merge(
            entry(APPLE, &[])
                .with_listing(Listing::new(Some(yggdryl::Mic::new("XNAS").unwrap())))
                .unwrap(),
        )
        .unwrap();
    assert_eq!(registry.commit().unwrap().written_rows, 2);
    assert_eq!(Instruments::from_holder(partitioned()).unwrap().len(), 2);
    assert!(registry.remove(HOLCIM).is_some());
    assert_eq!(registry.commit().unwrap().written_rows, 1);
    let back = Instruments::from_holder(partitioned()).unwrap();
    assert_eq!(back.len(), 1);
    assert!(back.get(APPLE).is_some());
    registry.clear();
    registry.commit().unwrap();
    assert!(Instruments::from_holder(partitioned()).unwrap().is_empty());
    assert!(IcebergTable::open(partitioned()).is_ok());
    // A location inside a table is refused by name; on a local folder,
    // because the climb from a partition to its table reads `..`, which the
    // memory filesystem does not resolve.
    let root = crate::scratch("inside");
    let local = |name: &str| yggdryl::local::LocalFolder::new(root.join(name)).unwrap();
    let mut schema = Instrument::field()
        .into_scheme_compat(&yggdryl::Scheme::ICEBERG)
        .unwrap()
        .with_partition_fields(&["isin"])
        .unwrap();
    yggdryl::iceberg::assign_field_ids(&mut schema, 1).unwrap();
    let spec = PartitionSpec::from_schema(FIRST_PARTITION_ID, &schema).unwrap();
    IcebergTable::create(local("table"), FormatVersion::V3, schema, spec).unwrap();
    for inside in ["table/isin=US0378331005", "table/data/isin=US0378331005"] {
        let refused = Instruments::from_holder(local(inside))
            .unwrap_err()
            .to_string();
        assert!(
            refused.contains("a partition of one"),
            "{inside}: {refused}"
        );
    }
    assert_eq!(Instruments::from_holder(local("table")).unwrap().len(), 0);
    let _ = std::fs::remove_dir_all(&root);
}

/// The registry's own row partitions an Iceberg table by the ISIN's country
/// prefix: a table created from `Instrument::field()` holds a one-field spec,
/// Iceberg's truncation of `isin` to two characters, over the forty-five
/// columns and no other; every commit replaces every partition in one
/// snapshot - every listing row of it, the listings of one ISIN in one -
/// the live data files' partition values exactly the distinct prefixes of
/// the rows and the table read back the snapshot.
#[cfg(feature = "iceberg")]
#[test]
fn an_iceberg_table_created_from_the_registrys_row_is_partitioned_by_the_country_prefix() {
    crate::install::installed();
    use std::collections::BTreeSet;
    use yggdryl::iceberg::{
        FIRST_PARTITION_ID, FormatVersion, IcebergTable, PartitionSpec, Transform,
    };

    let (filesystem, _) = counted_folder("by-country");
    let table = || folder_on(&filesystem, "instruments");
    let mut schema = Instrument::field()
        .into_scheme_compat(&yggdryl::Scheme::ICEBERG)
        .unwrap();
    yggdryl::iceberg::assign_field_ids(&mut schema, 1).unwrap();
    assert_eq!(schema.field_len(), 25, "no partition column");
    let spec = PartitionSpec::from_schema(FIRST_PARTITION_ID, &schema).unwrap();
    assert_eq!(spec.fields.len(), 1);
    assert_eq!(spec.fields[0].transform, Transform::Truncate(2));
    assert_eq!(
        Some(spec.fields[0].source_id),
        schema
            .get_field_by_path("crosscode")
            .unwrap()
            .parquet_field_id()
            .unwrap()
    );
    IcebergTable::create(table(), FormatVersion::V3, schema, spec).unwrap();

    // The live files of the table, one per partition: their partition
    // values, and how many snapshots the table holds.
    let stored = || {
        let opened = IcebergTable::open(table()).unwrap();
        let files = opened.data_files().unwrap();
        for (_, spec) in &files {
            assert_eq!(spec.fields[0].transform, Transform::Truncate(2));
        }
        let prefixes: Vec<String> = files
            .iter()
            .map(|(file, _)| file.partition[0].as_str().unwrap().to_owned())
            .collect();
        (prefixes, opened.metadata().unwrap().snapshots().len())
    };
    // The distinct country prefixes of the rows `registry` holds.
    let prefixes = |registry: &Instruments| {
        registry
            .iter()
            .map(|row| row.get_crosscode()[..2].to_owned())
            .collect::<BTreeSet<_>>()
    };

    let mut registry = Instruments::seeded_from_holder(table()).unwrap();
    assert!(
        registry
            .merge(
                entry(BAE, &[])
                    .with_listing(Listing::new(Some(Mic::new("XLON").unwrap())))
                    .unwrap()
            )
            .unwrap()
    );
    let expected = prefixes(&registry);
    assert!(expected.len() > 1, "the seed spans countries");
    assert_eq!(registry.rows(), registry.len(), "one row per instrument");
    assert_eq!(
        registry.commit().unwrap().written_rows,
        registry.rows() as u64
    );
    let (files, snapshots) = stored();
    assert_eq!(snapshots, 1, "one snapshot");
    assert_eq!(files.len(), expected.len(), "one file per prefix");
    assert_eq!(files.into_iter().collect::<BTreeSet<_>>(), expected);
    let back = Instruments::from_holder(table()).unwrap();
    assert!(back.iter().eq(registry.iter()), "the snapshot read back");

    // A row whose prefix no other row has, removed: the next commit is one
    // snapshot more, its partition gone with it.
    let alone = registry
        .iter()
        .find(|row| {
            let prefix = &row.get_crosscode()[..2];
            registry
                .iter()
                .filter(|other| other.get_crosscode().starts_with(prefix))
                .count()
                == 1
        })
        .map(|row| row.get_crosscode().to_owned())
        .expect("a country with one instrument");
    assert!(registry.remove(&alone).is_some());
    registry.commit().unwrap();
    let expected = prefixes(&registry);
    assert!(!expected.contains(&alone[..2]));
    let (files, snapshots) = stored();
    assert_eq!(snapshots, 2);
    assert_eq!(files.into_iter().collect::<BTreeSet<_>>(), expected);
    assert!(
        Instruments::from_holder(table())
            .unwrap()
            .iter()
            .eq(registry.iter())
    );

    // Emptied: one snapshot holding no file, still a table.
    registry.clear();
    registry.commit().unwrap();
    let (files, snapshots) = stored();
    assert_eq!((files.len(), snapshots), (0, 3));
}

/// The rows `registry` holds as a store written before the product
/// category was a column: its snapshot less `eusipacode`, twenty-four
/// columns.
fn without_category(registry: &Instruments) -> yggdryl::arrow::BatchReader {
    without(registry, "eusipacode")
}

/// The rows `registry` holds as a store written before `column` was one:
/// its snapshot less that column.
fn without(registry: &Instruments, column: &str) -> yggdryl::arrow::BatchReader {
    let reader = registry.into_arrow_reader().unwrap();
    let schema = reader.schema();
    let kept: Vec<usize> = (0..schema.fields().len())
        .filter(|at| schema.field(*at).name() != column)
        .collect();
    let projected = std::sync::Arc::new(schema.project(&kept).unwrap());
    let batches: Vec<_> = reader.map(|batch| batch.unwrap().project(&kept)).collect();
    Box::new(arrow_array::RecordBatchIterator::new(batches, projected))
}

/// A leaf written before the product category was a column loads with the
/// column null; a commit replaces the rows under the leaf's stored row, as
/// every overwrite of a leaf does, so the category is kept by the registry
/// and not by such a store until the store is laid out afresh - an emptied
/// leaf, or a new one - when a commit writes the row as it is now,
/// twenty-five columns.
#[test]
fn a_leaf_store_without_the_product_category_loads_it_null_and_keeps_its_own_row() {
    crate::install::installed();
    let (_, folder) = counted_folder("isin");
    let leaf = || folder.child_by_path("instruments.arrows").unwrap();
    let mut older = Instruments::new();
    older
        .merge(entry(HOLCIM, &[(IdType::Ric, "HOLN.S")]))
        .unwrap();
    let mut handle = leaf();
    let options = handle.record_options().unwrap();
    handle
        .write_arrow_reader(without_category(&older), IOMode::Overwrite, &options)
        .unwrap();
    assert_eq!(columns(&leaf()), 24);

    let mut registry = Instruments::from_holder(leaf()).unwrap();
    let row = registry.get(HOLCIM).unwrap();
    assert_eq!(row.get(&IdType::Ric), Some("HOLN.S"));
    assert_eq!(row.eusipacode(), None);
    assert!(!registry.is_dirty());
    let category = Some(yggdryl_market::Eusipa::new(2300).unwrap());
    registry
        .merge(entry(HOLCIM, &[]).with_eusipacode(category))
        .unwrap();
    assert_eq!(registry.commit().unwrap().written_rows, 1);
    assert_eq!(columns(&leaf()), 24, "the leaf's own row");
    let back = Instruments::from_holder(leaf()).unwrap();
    let row = back.get(HOLCIM).unwrap();
    assert_eq!(row.get(&IdType::Ric), Some("HOLN.S"));
    assert_eq!(row.eusipacode(), None);
    assert_eq!(registry.get(HOLCIM).unwrap().eusipacode(), category);

    // A store laid out afresh holds the row as it is now.
    let fresh = || folder.child_by_path("fresh.arrows").unwrap();
    let mut moved = Instruments::from_holder(fresh()).unwrap();
    moved.merge(registry.get(HOLCIM).unwrap().clone()).unwrap();
    moved.commit().unwrap();
    assert_eq!(columns(&fresh()), 25);
    let back = Instruments::from_holder(fresh()).unwrap();
    assert_eq!(back.get(HOLCIM).unwrap().eusipacode(), category);
}

/// The number of columns the record stream `handle` holds declares.
fn columns(handle: &dyn IOBase) -> usize {
    handle
        .read_arrow_field(&handle.record_options().unwrap())
        .unwrap()
        .fields()
        .len()
}

/// A leaf written before `lastunix` was a column loads it null and keeps
/// its own row through a commit - a learn that moves only `lastunix` still
/// dirties the registry, which commits the rows under the leaf's row - the
/// instant kept by the registry alone; a store laid out afresh stores it.
#[test]
fn a_leaf_store_without_lastunix_loads_it_null_and_keeps_its_own_row() {
    crate::install::installed();
    use yggdryl_market::graph::OrderEvent;
    use yggdryl_market::{IdKey, Identifier};

    let (_, folder) = counted_folder("isin-lastunix");
    let leaf = || folder.child_by_path("instruments.arrows").unwrap();
    let mut older = Instruments::new();
    older
        .merge(entry(APPLE, &[(IdType::Common, "C-1")]).with_lastunix(Some(5)))
        .unwrap();
    let mut handle = leaf();
    let options = handle.record_options().unwrap();
    handle
        .write_arrow_reader(without(&older, "lastunix"), IOMode::Overwrite, &options)
        .unwrap();
    assert_eq!(columns(&leaf()), 24);

    let mut registry = Instruments::from_holder(leaf()).unwrap();
    let row = registry.get(APPLE).unwrap();
    assert_eq!(row.get(&IdType::Common), Some("C-1"));
    assert_eq!(row.lastunix(), None);
    assert!(!registry.is_dirty());
    let mut met = OrderEvent::at(20);
    yggdryl_market::graph::Market::insert_securityid(
        &mut met,
        Identifier::new(IdKey::base(IdType::Isin), APPLE).unwrap(),
    )
    .unwrap();
    assert!(registry.learn(&met), "met: the instant alone moves");
    assert!(registry.is_dirty());
    assert_eq!(registry.commit().unwrap().written_rows, 1);
    assert_eq!(columns(&leaf()), 24, "the leaf's own row");
    let back = Instruments::from_holder(leaf()).unwrap();
    assert_eq!(back.get(APPLE).unwrap().get(&IdType::Common), Some("C-1"));
    assert_eq!(back.get(APPLE).unwrap().lastunix(), None);
    assert_eq!(registry.get(APPLE).unwrap().lastunix(), Some(20));

    let fresh = || folder.child_by_path("fresh.arrows").unwrap();
    let mut moved = Instruments::from_holder(fresh()).unwrap();
    moved.merge(registry.get(APPLE).unwrap().clone()).unwrap();
    moved.commit().unwrap();
    assert_eq!(columns(&fresh()), 25);
    let back = Instruments::from_holder(fresh()).unwrap();
    assert_eq!(back.get(APPLE).unwrap().lastunix(), Some(20));
}

/// A leaf written before `firstunix` was a column loads it null and keeps
/// its own row through a commit - a learn earlier than any the registry
/// met moves `firstunix` alone and still dirties the registry - the instant
/// kept by the registry alone; a store laid out afresh stores it, and it
/// reads back.
#[test]
fn a_leaf_store_without_firstunix_loads_it_null_and_keeps_its_own_row() {
    crate::install::installed();
    use yggdryl_market::graph::OrderEvent;
    use yggdryl_market::{IdKey, Identifier};

    let (_, folder) = counted_folder("isin-firstunix");
    let leaf = || folder.child_by_path("instruments.arrows").unwrap();
    let mut older = Instruments::new();
    older
        .merge(
            entry(APPLE, &[(IdType::Common, "C-1")])
                .with_firstunix(Some(30))
                .with_lastunix(Some(30)),
        )
        .unwrap();
    let mut handle = leaf();
    let options = handle.record_options().unwrap();
    handle
        .write_arrow_reader(without(&older, "firstunix"), IOMode::Overwrite, &options)
        .unwrap();
    assert_eq!(columns(&leaf()), 24);

    let mut registry = Instruments::from_holder(leaf()).unwrap();
    let row = registry.get(APPLE).unwrap();
    assert_eq!((row.firstunix(), row.lastunix()), (None, Some(30)));
    let mut met = OrderEvent::at(10);
    yggdryl_market::graph::Market::insert_securityid(
        &mut met,
        Identifier::new(IdKey::base(IdType::Isin), APPLE).unwrap(),
    )
    .unwrap();
    assert!(registry.learn(&met), "met earlier: firstunix alone moves");
    assert!(registry.is_dirty());
    assert_eq!(registry.commit().unwrap().written_rows, 1);
    assert_eq!(columns(&leaf()), 24, "the leaf's own row");
    let back = Instruments::from_holder(leaf()).unwrap();
    assert_eq!(back.get(APPLE).unwrap().firstunix(), None);
    assert_eq!(registry.get(APPLE).unwrap().firstunix(), Some(10));

    let fresh = || folder.child_by_path("fresh.arrows").unwrap();
    let mut moved = Instruments::from_holder(fresh()).unwrap();
    moved.merge(registry.get(APPLE).unwrap().clone()).unwrap();
    moved.commit().unwrap();
    assert_eq!(columns(&fresh()), 25);
    let back = Instruments::from_holder(fresh()).unwrap();
    let row = back.get(APPLE).unwrap();
    assert_eq!((row.firstunix(), row.lastunix()), (Some(10), Some(30)));
}

/// Holcim listed on XSWX and on XLON, and Apple on XNAS: two instruments,
/// three listings.
fn listed(registry: &mut Instruments) {
    for (market, ticker, ric) in [("XSWX", "HOLN", "HOLN.S"), ("XLON", "0QKY", "HOLN.L")] {
        registry
            .merge(
                entry(HOLCIM, &[(IdType::Common, "C-1")])
                    .with_listing(
                        Listing::new(Some(Mic::new(market).unwrap()))
                            .with_ticker(Some(ticker.into()))
                            .try_with_code(IdType::Ric, ric)
                            .unwrap(),
                    )
                    .unwrap()
                    .with_lastunix(Some(7)),
            )
            .unwrap();
    }
    registry
        .merge(
            entry(APPLE, &[])
                .with_listing(Listing::new(Some(Mic::new("XNAS").unwrap())))
                .unwrap(),
        )
        .unwrap();
}

/// What `back`, read from a store `registry` was committed to, holds: every
/// instrument as committed, its listings nested in MIC order.
fn assert_listings_round_trip(back: &Instruments, registry: &Instruments) {
    assert_eq!((back.len(), back.rows()), (2, 2));
    assert!(back.iter().eq(registry.iter()), "every instrument");
    let holcim = back.get(HOLCIM).unwrap();
    let markets: Vec<_> = holcim
        .listings()
        .iter()
        .map(|listing| {
            (
                listing.miccode().map(Mic::as_str),
                listing.get(&IdType::Ric),
            )
        })
        .collect();
    assert_eq!(
        markets,
        [
            (Some("XLON"), Some("HOLN.L")),
            (Some("XSWX"), Some("HOLN.S"))
        ]
    );
    assert_eq!(holcim.lastunix(), Some(7));
    assert_eq!(holcim.get(&IdType::Common), Some("C-1"));
}

/// An Arrow IPC leaf, a Parquet leaf and a plain folder carry an
/// instrument's listings as one row each, read back as the same listings.
#[test]
fn a_leaf_and_a_folder_store_carry_two_listings_of_one_isin() {
    crate::install::installed();
    let (_, folder) = counted_folder("isin-listings");
    #[cfg_attr(not(feature = "parquet"), allow(unused_mut))]
    let mut leaves = vec!["instruments.arrows"];
    #[cfg(feature = "parquet")]
    leaves.push("instruments.parquet");
    for name in leaves {
        let leaf = || folder.child_by_path(name).unwrap();
        let mut registry = Instruments::from_holder(leaf()).unwrap();
        listed(&mut registry);
        assert_eq!(registry.commit().unwrap().written_rows, 2, "{name}");
        assert_listings_round_trip(&Instruments::from_holder(leaf()).unwrap(), &registry);
    }
    let root = crate::scratch("listings-folder").join("isin");
    let url = Url::from_location(&format!("{}/", root.display())).unwrap();
    let none: [(&str, &str); 0] = [];
    let mut registry = Instruments::from_url(&url, none).unwrap();
    listed(&mut registry);
    assert_eq!(registry.commit().unwrap().written_rows, 2);
    assert_listings_round_trip(&Instruments::from_url(&url, none).unwrap(), &registry);
    let _ = std::fs::remove_dir_all(root.parent().unwrap());
}

/// An Iceberg table partitioned by the key's first two bytes holds an
/// instrument's listings in its one row: two files for two countries, every
/// instrument read back.
#[cfg(feature = "iceberg")]
#[test]
fn an_iceberg_store_holds_the_listings_of_one_isin_in_its_countrys_partition() {
    crate::install::installed();
    use std::collections::BTreeSet;
    use yggdryl::iceberg::{FIRST_PARTITION_ID, FormatVersion, IcebergTable, PartitionSpec};

    let (filesystem, _) = counted_folder("listings-table");
    let table = || folder_on(&filesystem, "instruments");
    let mut schema = Instrument::field()
        .into_scheme_compat(&yggdryl::Scheme::ICEBERG)
        .unwrap();
    yggdryl::iceberg::assign_field_ids(&mut schema, 1).unwrap();
    let spec = PartitionSpec::from_schema(FIRST_PARTITION_ID, &schema).unwrap();
    IcebergTable::create(table(), FormatVersion::V3, schema, spec).unwrap();
    let mut registry = Instruments::from_holder(table()).unwrap();
    listed(&mut registry);
    assert_eq!(registry.commit().unwrap().written_rows, 2);
    let files = IcebergTable::open(table()).unwrap().data_files().unwrap();
    let prefixes: BTreeSet<String> = files
        .iter()
        .map(|(file, _)| file.partition[0].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(files.len(), 2, "one file per country");
    assert_eq!(prefixes, BTreeSet::from(["CH".to_owned(), "US".to_owned()]));
    assert_listings_round_trip(&Instruments::from_holder(table()).unwrap(), &registry);
}

/// An Iceberg table created before the product category was a column loads
/// it null, and a commit replaces the rows under the table's own schema.
#[cfg(feature = "iceberg")]
#[test]
fn an_iceberg_store_without_the_product_category_loads_it_null_and_keeps_its_schema() {
    crate::install::installed();
    use yggdryl::iceberg::{FormatVersion, IcebergTable, PartitionSpec};

    let (filesystem, _) = counted_folder("table");
    let table = || folder_on(&filesystem, "older");
    let older_row = {
        let mut older = Instruments::new();
        older.merge(entry(HOLCIM, &[])).unwrap();
        without_category(&older).schema()
    };
    let field = yggdryl::Field::from_arrow_schema("instrument", &older_row)
        .unwrap()
        .into_scheme_compat(&yggdryl::Scheme::ICEBERG)
        .unwrap();
    IcebergTable::create(
        table(),
        FormatVersion::V3,
        field,
        PartitionSpec::unpartitioned(),
    )
    .unwrap();
    let mut registry = Instruments::from_holder(table()).unwrap();
    let category = Some(yggdryl_market::Eusipa::new(2300).unwrap());
    registry
        .merge(entry(HOLCIM, &[(IdType::Ric, "HOLN.S")]).with_eusipacode(category))
        .unwrap();
    assert_eq!(registry.commit().unwrap().written_rows, 1);
    let back = Instruments::from_holder(table()).unwrap();
    let row = back.get(HOLCIM).unwrap();
    assert_eq!(row.get(&IdType::Ric), Some("HOLN.S"));
    assert_eq!(row.eusipacode(), None, "the table's own schema");
    assert_eq!(
        IcebergTable::open(table())
            .unwrap()
            .schema()
            .unwrap()
            .fields()
            .len(),
        24
    );
}

/// A plain folder written before the product category was a column is laid
/// out afresh by a commit - its record parts removed, the snapshot written
/// as one part of the row as it is now - so the category is stored.
#[test]
fn a_folder_store_without_the_product_category_is_laid_out_afresh_with_it() {
    crate::install::installed();
    let root = crate::scratch("unstored-folder").join("isin");
    std::fs::create_dir_all(&root).unwrap();
    let url = Url::from_location(&format!("{}/", root.display())).unwrap();
    let mut older = Instruments::new();
    older
        .merge(entry(HOLCIM, &[(IdType::Ric, "HOLN.S")]))
        .unwrap();
    let mut part = yggdryl::holder::Holder::from_url(
        Url::from_location(&root.join("part-0.arrows").display().to_string()).unwrap(),
        [("", ""); 0],
    )
    .unwrap();
    let options = part.record_options().unwrap();
    part.write_arrow_reader(without_category(&older), IOMode::Overwrite, &options)
        .unwrap();
    let none: [(&str, &str); 0] = [];
    let mut registry = Instruments::from_url(&url, none).unwrap();
    assert_eq!(registry.get(HOLCIM).unwrap().eusipacode(), None);
    let category = Some(yggdryl_market::Eusipa::new(2300).unwrap());
    registry
        .merge(entry(HOLCIM, &[]).with_eusipacode(category))
        .unwrap();
    assert_eq!(registry.commit().unwrap().written_rows, 1);
    let back = Instruments::from_url(&url, none).unwrap();
    assert_eq!(back.get(HOLCIM).unwrap().eusipacode(), category);
    assert_eq!(back.get(HOLCIM).unwrap().get(&IdType::Ric), Some("HOLN.S"));
    let _ = std::fs::remove_dir_all(root.parent().unwrap());
}

/// The rows a store states over the seed: Apple, a seed instrument, on its
/// own listing under another currency, and BAE, which the seed lacks.
fn store_rows(registry: &mut Instruments) {
    registry
        .merge(
            Instrument::for_security(Isin::new(APPLE).unwrap())
                .unwrap()
                .with_listing(
                    Listing::new(Some(Mic::new("XNAS").unwrap()))
                        .with_ticker(Some("AAPL".into()))
                        .with_currency(Some(Ccy::new("CHF").unwrap())),
                )
                .unwrap(),
        )
        .unwrap();
    registry
        .merge(
            entry(BAE, &[])
                .with_listing(
                    Listing::new(Some(Mic::new("XLON").unwrap()))
                        .try_with_code(IdType::Ric, "BAES.L")
                        .unwrap(),
                )
                .unwrap(),
        )
        .unwrap();
}

/// What `registry`, `store`'s rows laid over the seed, holds: the store's
/// value where it states one and the seed's fact beside it where it states
/// none, a seed row the store has none of as the seed holds it, the
/// store's own row as the store holds it; bound, and clean.
fn assert_laid_over_the_seed(registry: &Instruments, store: &Instruments) {
    let seed = Instruments::seeded();
    assert!(!registry.is_dirty(), "clean after the load");
    assert!(registry.holder().is_some());
    assert_eq!(
        registry.len(),
        seed.len() + 1,
        "the seed, and the store's other row"
    );
    let apple = registry.get(APPLE).expect("the seed's and the store's");
    let seeded = seed.get(APPLE).expect("a seed instrument");
    let nasdaq = Mic::new("XNAS").unwrap();
    let currency = |held: &Instrument| {
        held.listing(Some(&nasdaq))
            .and_then(Listing::currency)
            .map(|code| code.as_str().to_owned())
    };
    assert_eq!(currency(seeded).as_deref(), Some("USD"));
    assert_eq!(
        currency(apple).as_deref(),
        Some("CHF"),
        "the store's value wins"
    );
    assert!(seeded.fisn().is_some());
    assert_eq!(
        apple.fisn(),
        seeded.fisn(),
        "a fact only the seed states stands"
    );
    assert_eq!(apple.cficode(), seeded.cficode());
    assert_eq!(
        registry.get(MICROSOFT),
        seed.get(MICROSOFT),
        "a seed row the store has none of"
    );
    assert!(seed.get(BAE).is_none());
    assert_eq!(
        registry.get(BAE),
        store.get(BAE),
        "a row only the store holds"
    );
}

/// `seeded_from_url` lays a leaf store's rows over the seed, clean after
/// the load: a clean commit writes nothing, and the first commit after a
/// merge writes the seed's rows with the store's, the store then holding
/// exactly the snapshot.
#[test]
fn seeded_from_url_lays_a_leaf_store_over_the_seed_and_commits_both() {
    crate::install::installed();
    let root = crate::scratch("seeded-url");
    let url = Url::from_path(root.join("instruments.arrows")).unwrap();
    let none: [(&str, &str); 0] = [];
    let mut store = Instruments::from_url(&url, none).unwrap();
    store_rows(&mut store);
    assert_eq!(store.commit().unwrap().written_rows, 2);

    let mut registry = Instruments::seeded_from_url(&url, none).unwrap();
    assert_laid_over_the_seed(&registry, &store);
    assert_eq!(
        registry.holder().unwrap().url().unwrap().to_string(),
        url.to_string()
    );
    assert_eq!(
        registry.commit().unwrap(),
        IOResult::default(),
        "a clean registry writes nothing"
    );
    assert_eq!(
        Instruments::from_url(&url, none).unwrap().len(),
        2,
        "the store as it was"
    );

    assert!(
        registry
            .merge(entry(HOLCIM, &[(IdType::Common, "C-1")]))
            .unwrap()
    );
    let written = registry.commit().unwrap();
    assert_eq!(written.written_rows, registry.rows() as u64);
    assert_eq!(
        written.written_rows,
        Instruments::seeded().rows() as u64 + 1,
        "the seed's rows with the store's"
    );
    assert!(!registry.is_dirty());
    let back = Instruments::from_url(&url, none).unwrap();
    assert!(
        back.iter().eq(registry.iter()),
        "the store holds exactly the snapshot"
    );

    // A leaf this build has no record encoding for is refused at the
    // binding, as `from_url` refuses it.
    let unknown = Url::from_path(root.join("instruments.xyz")).unwrap();
    assert!(Instruments::seeded_from_url(&unknown, none).is_err());
    let _ = std::fs::remove_dir_all(&root);
}

/// `seeded_from_holder` is the same layering over a holder in hand, the
/// store read once as `from_holder` reads it - the seed costs it no call -
/// and a store holding nothing yet loads as the seed bound to it, clean,
/// so a commit writes nothing.
#[test]
fn seeded_from_holder_lays_the_holders_rows_over_the_seed_at_the_cost_of_one_load() {
    crate::install::installed();
    let (filesystem, folder) = counted_folder("isin-seeded");
    let leaf = || folder.child_by_path("instruments.arrows").unwrap();
    let mut store = Instruments::from_holder(leaf()).unwrap();
    store_rows(&mut store);
    store.commit().unwrap();

    let load = filesystem.costs(|| assert_eq!(Instruments::from_holder(leaf()).unwrap().len(), 2));
    let mut registry = None;
    assert_eq!(
        filesystem.costs(|| registry = Some(Instruments::seeded_from_holder(leaf()).unwrap())),
        load,
        "the seed costs the store no call"
    );
    let mut registry = registry.unwrap();
    assert_laid_over_the_seed(&registry, &store);
    assert!(
        registry
            .merge(entry(HOLCIM, &[(IdType::Common, "C-1")]))
            .unwrap()
    );
    assert_eq!(
        registry.commit().unwrap().written_rows,
        registry.rows() as u64
    );
    assert!(
        Instruments::from_holder(leaf())
            .unwrap()
            .iter()
            .eq(registry.iter())
    );

    let fresh = || folder.child_by_path("fresh.arrows").unwrap();
    let mut first = Instruments::seeded_from_holder(fresh()).unwrap();
    assert!(first.iter().eq(Instruments::seeded().iter()));
    assert!(!first.is_dirty() && first.holder().is_some());
    assert_eq!(
        filesystem.costs(|| assert_eq!(first.commit().unwrap(), IOResult::default())),
        "none",
        "a first run's clean commit"
    );
    assert!(Instruments::from_holder(fresh()).unwrap().is_empty());
}

/// `from_url` and `from_holder` bind a store unseeded: the store's rows and
/// nothing else, the seed being `seeded_from_url`'s.
#[test]
fn from_url_and_from_holder_bind_a_store_unseeded() {
    crate::install::installed();
    let root = crate::scratch("unseeded");
    let url = Url::from_path(root.join("instruments.arrows")).unwrap();
    let none: [(&str, &str); 0] = [];
    let mut store = Instruments::from_url(&url, none).unwrap();
    assert!(store.is_empty(), "a first run holds nothing");
    store_rows(&mut store);
    store.commit().unwrap();
    for loaded in [
        Instruments::from_url(&url, none).unwrap(),
        Instruments::from_holder(yggdryl::holder::Holder::from_url(&url, none).unwrap()).unwrap(),
    ] {
        assert!(loaded.iter().eq(store.iter()));
        assert!(loaded.get(MICROSOFT).is_none(), "no seed row");
        assert_eq!(loaded.get(APPLE).unwrap().fisn(), None, "no seed fact");
    }
    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(feature = "internals")]
mod internal {
    use yggdryl::internals::logging_warning::count;
    use yggdryl::{IOBase, IOMedia, IOMode};
    use yggdryl_market::{IdType, Instruments};

    use super::{HOLCIM, entry, without_category};
    use crate::counting_filesystem::counted_folder;

    const SITE: &str = "yggdryl_market::instrument::store";
    const WHAT: &str = "instrument column not stored: the store's row lacks it";

    /// The subject the warning about `column` on `registry`'s store is
    /// counted under: the column and the store.
    fn subject(registry: &Instruments, column: &str) -> String {
        let store = registry.holder().unwrap().url().unwrap();
        format!("{column} at {store}")
    }

    /// A commit to a store whose row was laid out before a column the
    /// registry holds a value for warns, once per commit, naming the column
    /// and the store: the rows are replaced under the store's own row, so
    /// the value stays the registry's. A commit holding no such value, a
    /// clean one, and one to the store laid out afresh say nothing.
    #[test]
    fn a_commit_a_store_keeps_no_column_for_warns_naming_the_column_and_the_store() {
        crate::install::installed();
        let (_, folder) = counted_folder("isin-unstored");
        let leaf = || folder.child_by_path("instruments.arrows").unwrap();
        let mut older = Instruments::new();
        older
            .merge(entry(HOLCIM, &[(IdType::Ric, "HOLN.S")]))
            .unwrap();
        let mut handle = leaf();
        let options = handle.record_options().unwrap();
        handle
            .write_arrow_reader(without_category(&older), IOMode::Overwrite, &options)
            .unwrap();

        let mut registry = Instruments::from_holder(leaf()).unwrap();
        let subject = subject(&registry, "eusipacode");
        let seen = || count(SITE, WHAT, &subject);
        assert_eq!(seen(), 0);
        registry
            .merge(entry(HOLCIM, &[(IdType::Common, "C-1")]))
            .unwrap();
        registry.commit().unwrap();
        assert_eq!(seen(), 0, "no value the store keeps no column for");

        let category = Some(yggdryl_market::Eusipa::new(2300).unwrap());
        registry
            .merge(entry(HOLCIM, &[]).with_eusipacode(category))
            .unwrap();
        assert_eq!(registry.commit().unwrap().written_rows, 1);
        assert_eq!(seen(), 1, "once per commit");
        registry
            .merge(entry(HOLCIM, &[(IdType::Common, "C-2")]))
            .unwrap();
        registry.commit().unwrap();
        assert_eq!(seen(), 2);
        registry.commit().unwrap();
        assert_eq!(seen(), 2, "a clean commit writes nothing");
        // The store keeps its own row all the same: no migration.
        let back = Instruments::from_holder(leaf()).unwrap();
        assert_eq!(back.get(HOLCIM).unwrap().eusipacode(), None);
        assert_eq!(back.get(HOLCIM).unwrap().get(&IdType::Common), Some("C-2"));

        // Emptied, the leaf is laid out afresh by the next commit, which
        // stores the column and says nothing.
        registry.clear();
        registry.commit().unwrap();
        registry
            .merge(entry(HOLCIM, &[]).with_eusipacode(category))
            .unwrap();
        registry.commit().unwrap();
        assert_eq!(seen(), 2);
        let back = Instruments::from_holder(leaf()).unwrap();
        assert_eq!(back.get(HOLCIM).unwrap().eusipacode(), category);

        // A store laid out by the registry from the first says nothing.
        let fresh = || folder.child_by_path("fresh.arrows").unwrap();
        let mut moved = Instruments::from_holder(fresh()).unwrap();
        moved
            .merge(entry(HOLCIM, &[]).with_eusipacode(category))
            .unwrap();
        moved.commit().unwrap();
        moved
            .merge(entry(HOLCIM, &[(IdType::Common, "C-3")]))
            .unwrap();
        moved.commit().unwrap();
        assert_eq!(count(SITE, WHAT, &self::subject(&moved, "eusipacode")), 0);
    }

    /// An Iceberg table created before the product category was a column
    /// warns the same, its own schema kept.
    #[cfg(feature = "iceberg")]
    #[test]
    fn a_commit_an_iceberg_store_keeps_no_column_for_warns_the_same() {
        crate::install::installed();
        use yggdryl::iceberg::{FormatVersion, IcebergTable, PartitionSpec};

        let (filesystem, _) = counted_folder("table-unstored");
        let table = || super::folder_on(&filesystem, "older-unstored");
        let older_row = {
            let mut older = Instruments::new();
            older.merge(entry(HOLCIM, &[])).unwrap();
            without_category(&older).schema()
        };
        let field = yggdryl::Field::from_arrow_schema("instrument", &older_row)
            .unwrap()
            .into_scheme_compat(&yggdryl::Scheme::ICEBERG)
            .unwrap();
        IcebergTable::create(
            table(),
            FormatVersion::V3,
            field,
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        let mut registry = Instruments::from_holder(table()).unwrap();
        let subject = subject(&registry, "eusipacode");
        let category = Some(yggdryl_market::Eusipa::new(2300).unwrap());
        registry
            .merge(entry(HOLCIM, &[]).with_eusipacode(category))
            .unwrap();
        assert_eq!(registry.commit().unwrap().written_rows, 1);
        assert_eq!(count(SITE, WHAT, &subject), 1);
        let back = Instruments::from_holder(table()).unwrap();
        assert_eq!(
            back.get(HOLCIM).unwrap().eusipacode(),
            None,
            "the table's own schema"
        );
    }
}
