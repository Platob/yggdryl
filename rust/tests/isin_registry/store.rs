//! `rust/src/isin_registry/store.rs`: the store a registry is bound to -
//! loaded once at the binding under options resolved once, committed back
//! as one snapshot only where the table moved, a leaf, a plain folder or
//! an Iceberg table alike.

use yggdryl::media::IORecordOptions;
use yggdryl::{IOBase, IOMedia, IOMode, IOResult, IdType, Isin, IsinEntry, IsinRegistry, Url};

use crate::counting_filesystem::counted_folder;

const HOLCIM: &str = "CH0012214059";
const APPLE: &str = "US0378331005";

/// The row of `text` stating each of `codes`.
fn entry(text: &str, codes: &[(IdType, &str)]) -> IsinEntry {
    codes.iter().fold(
        IsinEntry::new(Isin::new(text).unwrap()),
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
    let mut registry = IsinRegistry::new();
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
    let (filesystem, folder) = counted_folder("isin");
    let leaf = || folder.child_by_path("instruments.arrows").unwrap();
    let mut registry = IsinRegistry::from_holder(leaf()).unwrap();
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
                .with_underlyingisin(Some(Isin::new(APPLE).unwrap())),
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
        .with_field(IsinEntry::field());
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
    let back = IsinRegistry::from_holder(leaf()).unwrap();
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
        filesystem.costs(|| assert_eq!(IsinRegistry::from_holder(leaf()).unwrap().len(), 2)),
        reading,
        "a load"
    );

    // Emptied: the leaf is cleared, and a reload is empty.
    registry.clear();
    assert!(registry.is_dirty());
    registry.commit().unwrap();
    assert!(!registry.is_dirty());
    assert!(IsinRegistry::from_holder(leaf()).unwrap().is_empty());
}

/// Binding a registry already holding rows loads the store's and folds the
/// held ones over them, dirty exactly where a held row moved something; a
/// binding that fails leaves the registry as it was, bound to nothing.
#[test]
fn binding_a_registry_holding_rows_folds_them_over_the_stores() {
    let (_, folder) = counted_folder("isin");
    let leaf = || folder.child_by_path("instruments.arrows").unwrap();
    let mut stored = IsinRegistry::from_holder(leaf()).unwrap();
    stored
        .merge(entry(
            HOLCIM,
            &[(IdType::Ric, "HOLN.S"), (IdType::Common, "C-1")],
        ))
        .unwrap();
    stored.commit().unwrap();

    let mut same = IsinRegistry::new();
    same.merge(entry(
        HOLCIM,
        &[(IdType::Ric, "HOLN.S"), (IdType::Common, "C-1")],
    ))
    .unwrap();
    assert_eq!(same.set_holder(leaf()).unwrap(), 1);
    assert!(!same.is_dirty(), "the held row moved nothing");
    assert_eq!(same.len(), 1);
    assert!(same.holder().is_some());

    let mut more = IsinRegistry::new();
    more.merge(entry(HOLCIM, &[(IdType::Common, "C-2")]))
        .unwrap();
    more.merge(entry(APPLE, &[])).unwrap();
    let bound = more.try_with_holder(leaf()).unwrap();
    assert!(bound.is_dirty(), "the held rows moved the store's");
    assert_eq!(bound.len(), 2);
    let holcim = bound.get(HOLCIM).unwrap();
    assert_eq!(holcim.get(&IdType::Common), Some("C-2"));
    assert_eq!(holcim.get(&IdType::Ric), Some("HOLN.S"));

    let mut bounded = IsinRegistry::new().with_max_instruments(1);
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

/// The default layout: a plain folder that is not there yet is an empty
/// first run, laid out as one Arrow IPC part by the first dirty commit,
/// read back whole, rewritten in place and cleared when emptied.
#[test]
fn a_folder_store_is_laid_out_by_the_first_dirty_commit_and_read_back_whole() {
    let root = crate::scratch("folder").join("isin");
    // A trailing slash is what makes a location that is not there yet a
    // folder rather than a leaf.
    let url = Url::from_location(&format!("{}/", root.display())).unwrap();
    assert!(url.has_trailing_slash(), "{url}");
    let none: [(&str, &str); 0] = [];
    let mut registry = IsinRegistry::from_url(&url, none).unwrap();
    assert!(registry.is_empty() && !registry.is_dirty());
    assert!(!root.exists(), "nothing is laid out before a commit");
    registry
        .merge(
            entry(HOLCIM, &[(IdType::Ric, "HOLN.S")])
                .with_underlyingisin(Some(Isin::new(APPLE).unwrap())),
        )
        .unwrap();
    registry.merge(entry(APPLE, &[])).unwrap();
    assert_eq!(registry.commit().unwrap().written_rows, 2);
    assert!(root.join("part-0.arrows").is_file());
    let back = IsinRegistry::from_url(&url, none).unwrap();
    assert!(back.iter().eq(registry.iter()));
    assert!(!back.is_dirty());
    // Rewritten in place: one part still.
    registry
        .merge(entry(HOLCIM, &[(IdType::Common, "C-1")]))
        .unwrap();
    registry.commit().unwrap();
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
    assert_eq!(
        IsinRegistry::from_url(&url, none)
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
    assert!(IsinRegistry::from_url(&url, none).unwrap().is_empty());
    let _ = std::fs::remove_dir_all(root.parent().unwrap());
}

/// A folder laid out in partitions holds exactly the snapshot after every
/// commit: a row removed from the registry is removed from the partition
/// it was stored in, a file that is no record part is never touched, and
/// a folder holding plain text alone is laid out as Arrow IPC.
#[test]
fn a_partitioned_folder_store_holds_exactly_the_snapshot_after_every_commit() {
    let root = crate::scratch("partitioned").join("isin");
    std::fs::create_dir_all(root.join("miccode=XSWX")).unwrap();
    std::fs::create_dir_all(root.join("miccode=XNAS")).unwrap();
    std::fs::write(root.join("README.md"), b"the instrument registry").unwrap();
    let url = Url::from_location(&format!("{}/", root.display())).unwrap();
    let none: [(&str, &str); 0] = [];
    let mut registry = IsinRegistry::from_url(&url, none).unwrap();
    assert!(registry.is_empty() && !registry.is_dirty());
    registry
        .merge(entry(HOLCIM, &[]).with_miccode(Some(yggdryl::Mic::new("XSWX").unwrap())))
        .unwrap();
    registry
        .merge(entry(APPLE, &[]).with_miccode(Some(yggdryl::Mic::new("XNAS").unwrap())))
        .unwrap();
    assert_eq!(registry.commit().unwrap().written_rows, 2);
    assert!(root.join("miccode=XSWX/part-0.arrows").is_file());
    assert!(root.join("miccode=XNAS/part-0.arrows").is_file());
    let back = IsinRegistry::from_url(&url, none).unwrap();
    assert!(back.iter().eq(registry.iter()));
    // A removal reaches the partition the row was stored in.
    registry.remove(HOLCIM).unwrap();
    assert_eq!(registry.commit().unwrap().written_rows, 1);
    assert!(!root.join("miccode=XSWX/part-0.arrows").exists());
    let back = IsinRegistry::from_url(&url, none).unwrap();
    assert_eq!(back.len(), 1);
    assert!(back.get(APPLE).is_some());
    assert!(root.join("README.md").is_file());
    // Emptied: no part anywhere, the README kept.
    registry.clear();
    registry.commit().unwrap();
    assert!(!root.join("miccode=XNAS/part-0.arrows").exists());
    assert!(root.join("README.md").is_file());
    assert!(IsinRegistry::from_url(&url, none).unwrap().is_empty());
    let _ = std::fs::remove_dir_all(root.parent().unwrap());
}

/// A leaf names its encoding by its name: one this build has no record
/// encoding for is refused at the binding rather than laid out as another.
#[test]
fn a_leaf_of_an_unknown_encoding_is_refused_at_the_binding() {
    let root = crate::scratch("unknown");
    let url = Url::from_path(root.join("instruments.xyz")).unwrap();
    let none: [(&str, &str); 0] = [];
    assert!(IsinRegistry::from_url(&url, none).is_err());
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
    use yggdryl::iceberg::{FormatVersion, IcebergTable, PartitionSpec};

    let (filesystem, _) = counted_folder("table");
    let table = || folder_on(&filesystem, "table");
    IcebergTable::create(
        table(),
        FormatVersion::V3,
        IsinEntry::field(),
        PartitionSpec::unpartitioned(),
    )
    .unwrap();
    let mut registry = IsinRegistry::from_holder(table()).unwrap();
    assert!(registry.is_empty() && !registry.is_dirty());
    registry
        .merge(
            entry(HOLCIM, &[(IdType::Ric, "HOLN.S")])
                .with_underlyingisin(Some(Isin::new(APPLE).unwrap())),
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
    let back = IsinRegistry::from_holder(table()).unwrap();
    assert!(back.iter().eq(registry.iter()));
    // Replaced whole by the next commit.
    registry
        .merge(entry(HOLCIM, &[(IdType::Common, "C-1")]))
        .unwrap();
    registry.commit().unwrap();
    let back = IsinRegistry::from_holder(table()).unwrap();
    assert_eq!(back.len(), 2);
    assert_eq!(back.get(HOLCIM).unwrap().get(&IdType::Common), Some("C-1"));
    // Emptied: an empty snapshot, still a table.
    registry.clear();
    registry.commit().unwrap();
    assert!(IsinRegistry::from_holder(table()).unwrap().is_empty());
    assert!(IcebergTable::open(table()).is_ok());
    assert!(table().is_container());

    // A partitioned table is replaced whole too: a row removed from the
    // registry is gone from the partition it was stored in, and an emptied
    // registry empties every partition; a location inside the table - one
    // partition of it - is refused at the binding.
    use yggdryl::iceberg::FIRST_PARTITION_ID;
    let partitioned = || folder_on(&filesystem, "partitioned");
    let mut schema = IsinEntry::field()
        .with_partition_fields(&["miccode"])
        .unwrap();
    yggdryl::iceberg::assign_field_ids(&mut schema, 1).unwrap();
    let spec = PartitionSpec::from_schema(FIRST_PARTITION_ID, &schema).unwrap();
    IcebergTable::create(partitioned(), FormatVersion::V3, schema, spec).unwrap();
    let mut registry = IsinRegistry::from_holder(partitioned()).unwrap();
    registry
        .merge(entry(HOLCIM, &[]).with_miccode(Some(yggdryl::Mic::new("XSWX").unwrap())))
        .unwrap();
    registry
        .merge(entry(APPLE, &[]).with_miccode(Some(yggdryl::Mic::new("XNAS").unwrap())))
        .unwrap();
    assert_eq!(registry.commit().unwrap().written_rows, 2);
    assert_eq!(IsinRegistry::from_holder(partitioned()).unwrap().len(), 2);
    registry.remove(HOLCIM).unwrap();
    assert_eq!(registry.commit().unwrap().written_rows, 1);
    let back = IsinRegistry::from_holder(partitioned()).unwrap();
    assert_eq!(back.len(), 1);
    assert!(back.get(APPLE).is_some());
    registry.clear();
    registry.commit().unwrap();
    assert!(IsinRegistry::from_holder(partitioned()).unwrap().is_empty());
    assert!(IcebergTable::open(partitioned()).is_ok());
    // A location inside a table is refused by name; on a local folder,
    // because the climb from a partition to its table reads `..`, which the
    // memory filesystem does not resolve.
    let root = crate::scratch("inside");
    let local = |name: &str| yggdryl::local::LocalFolder::new(root.join(name)).unwrap();
    let mut schema = IsinEntry::field()
        .with_partition_fields(&["miccode"])
        .unwrap();
    yggdryl::iceberg::assign_field_ids(&mut schema, 1).unwrap();
    let spec = PartitionSpec::from_schema(FIRST_PARTITION_ID, &schema).unwrap();
    IcebergTable::create(local("table"), FormatVersion::V3, schema, spec).unwrap();
    for inside in ["table/miccode=XNAS", "table/data/miccode=XNAS"] {
        let refused = IsinRegistry::from_holder(local(inside))
            .unwrap_err()
            .to_string();
        assert!(
            refused.contains("a partition of one"),
            "{inside}: {refused}"
        );
    }
    assert_eq!(IsinRegistry::from_holder(local("table")).unwrap().len(), 0);
    let _ = std::fs::remove_dir_all(&root);
}

/// The rows `registry` holds as a store written before the product
/// category was a column: its snapshot less `eusipacode`, forty-one
/// columns.
fn without_category(registry: &IsinRegistry) -> yggdryl::arrow::BatchReader {
    let reader = registry.into_arrow_reader().unwrap();
    let schema = reader.schema();
    let kept: Vec<usize> = (0..schema.fields().len())
        .filter(|at| schema.field(*at).name() != "eusipacode")
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
/// forty-two columns.
#[test]
fn a_leaf_store_without_the_product_category_loads_it_null_and_keeps_its_own_row() {
    let (_, folder) = counted_folder("isin");
    let leaf = || folder.child_by_path("instruments.arrows").unwrap();
    let mut older = IsinRegistry::new();
    older
        .merge(entry(HOLCIM, &[(IdType::Ric, "HOLN.S")]))
        .unwrap();
    let mut handle = leaf();
    let options = handle.record_options().unwrap();
    handle
        .write_arrow_reader(without_category(&older), IOMode::Overwrite, &options)
        .unwrap();
    let columns = |handle: &dyn IOBase| {
        handle
            .read_arrow_field(&handle.record_options().unwrap())
            .unwrap()
            .fields()
            .len()
    };
    assert_eq!(columns(&leaf()), 41);

    let mut registry = IsinRegistry::from_holder(leaf()).unwrap();
    let row = registry.get(HOLCIM).unwrap();
    assert_eq!(row.get(&IdType::Ric), Some("HOLN.S"));
    assert_eq!(row.eusipacode(), None);
    assert!(!registry.is_dirty());
    let category = Some(yggdryl::Eusipa::new(2300).unwrap());
    registry
        .merge(entry(HOLCIM, &[]).with_eusipacode(category))
        .unwrap();
    assert_eq!(registry.commit().unwrap().written_rows, 1);
    assert_eq!(columns(&leaf()), 41, "the leaf's own row");
    let back = IsinRegistry::from_holder(leaf()).unwrap();
    let row = back.get(HOLCIM).unwrap();
    assert_eq!(row.get(&IdType::Ric), Some("HOLN.S"));
    assert_eq!(row.eusipacode(), None);
    assert_eq!(registry.get(HOLCIM).unwrap().eusipacode(), category);

    // A store laid out afresh holds the row as it is now.
    let fresh = || folder.child_by_path("fresh.arrows").unwrap();
    let mut moved = IsinRegistry::from_holder(fresh()).unwrap();
    moved.merge(registry.get(HOLCIM).unwrap().clone()).unwrap();
    moved.commit().unwrap();
    assert_eq!(columns(&fresh()), 42);
    let back = IsinRegistry::from_holder(fresh()).unwrap();
    assert_eq!(back.get(HOLCIM).unwrap().eusipacode(), category);
}

/// An Iceberg table created before the product category was a column loads
/// it null, and a commit replaces the rows under the table's own schema.
#[cfg(feature = "iceberg")]
#[test]
fn an_iceberg_store_without_the_product_category_loads_it_null_and_keeps_its_schema() {
    use yggdryl::iceberg::{FormatVersion, IcebergTable, PartitionSpec};

    let (filesystem, _) = counted_folder("table");
    let table = || folder_on(&filesystem, "older");
    let older_row = {
        let mut older = IsinRegistry::new();
        older.merge(entry(HOLCIM, &[])).unwrap();
        without_category(&older).schema()
    };
    let field = yggdryl::Field::from_arrow_schema("isinregistry", &older_row).unwrap();
    IcebergTable::create(
        table(),
        FormatVersion::V3,
        field,
        PartitionSpec::unpartitioned(),
    )
    .unwrap();
    let mut registry = IsinRegistry::from_holder(table()).unwrap();
    let category = Some(yggdryl::Eusipa::new(2300).unwrap());
    registry
        .merge(entry(HOLCIM, &[(IdType::Ric, "HOLN.S")]).with_eusipacode(category))
        .unwrap();
    assert_eq!(registry.commit().unwrap().written_rows, 1);
    let back = IsinRegistry::from_holder(table()).unwrap();
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
        41
    );
}

/// A plain folder written before the product category was a column is laid
/// out afresh by a commit - its record parts removed, the snapshot written
/// as one part of the row as it is now - so the category is stored.
#[test]
fn a_folder_store_without_the_product_category_is_laid_out_afresh_with_it() {
    let root = crate::scratch("unstored-folder").join("isin");
    std::fs::create_dir_all(&root).unwrap();
    let url = Url::from_location(&format!("{}/", root.display())).unwrap();
    let mut older = IsinRegistry::new();
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
    let mut registry = IsinRegistry::from_url(&url, none).unwrap();
    assert_eq!(registry.get(HOLCIM).unwrap().eusipacode(), None);
    let category = Some(yggdryl::Eusipa::new(2300).unwrap());
    registry
        .merge(entry(HOLCIM, &[]).with_eusipacode(category))
        .unwrap();
    assert_eq!(registry.commit().unwrap().written_rows, 1);
    let back = IsinRegistry::from_url(&url, none).unwrap();
    assert_eq!(back.get(HOLCIM).unwrap().eusipacode(), category);
    assert_eq!(back.get(HOLCIM).unwrap().get(&IdType::Ric), Some("HOLN.S"));
    let _ = std::fs::remove_dir_all(root.parent().unwrap());
}

#[cfg(feature = "internals")]
mod internal {
    use yggdryl::internals::logging_warning::count;
    use yggdryl::{IOBase, IOMedia, IOMode, IdType, IsinRegistry};

    use super::{HOLCIM, entry, without_category};
    use crate::counting_filesystem::counted_folder;

    const SITE: &str = "yggdryl::isin_registry::store";
    const WHAT: &str = "instrument registry column not stored: the store's row lacks it";

    /// The subject the warning about `column` on `registry`'s store is
    /// counted under: the column and the store.
    fn subject(registry: &IsinRegistry, column: &str) -> String {
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
        let (_, folder) = counted_folder("isin-unstored");
        let leaf = || folder.child_by_path("instruments.arrows").unwrap();
        let mut older = IsinRegistry::new();
        older
            .merge(entry(HOLCIM, &[(IdType::Ric, "HOLN.S")]))
            .unwrap();
        let mut handle = leaf();
        let options = handle.record_options().unwrap();
        handle
            .write_arrow_reader(without_category(&older), IOMode::Overwrite, &options)
            .unwrap();

        let mut registry = IsinRegistry::from_holder(leaf()).unwrap();
        let subject = subject(&registry, "eusipacode");
        let seen = || count(SITE, WHAT, &subject);
        assert_eq!(seen(), 0);
        registry
            .merge(entry(HOLCIM, &[(IdType::Common, "C-1")]))
            .unwrap();
        registry.commit().unwrap();
        assert_eq!(seen(), 0, "no value the store keeps no column for");

        let category = Some(yggdryl::Eusipa::new(2300).unwrap());
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
        let back = IsinRegistry::from_holder(leaf()).unwrap();
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
        let back = IsinRegistry::from_holder(leaf()).unwrap();
        assert_eq!(back.get(HOLCIM).unwrap().eusipacode(), category);

        // A store laid out by the registry from the first says nothing.
        let fresh = || folder.child_by_path("fresh.arrows").unwrap();
        let mut moved = IsinRegistry::from_holder(fresh()).unwrap();
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
        use yggdryl::iceberg::{FormatVersion, IcebergTable, PartitionSpec};

        let (filesystem, _) = counted_folder("table-unstored");
        let table = || super::folder_on(&filesystem, "older-unstored");
        let older_row = {
            let mut older = IsinRegistry::new();
            older.merge(entry(HOLCIM, &[])).unwrap();
            without_category(&older).schema()
        };
        let field = yggdryl::Field::from_arrow_schema("isinregistry", &older_row).unwrap();
        IcebergTable::create(
            table(),
            FormatVersion::V3,
            field,
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        let mut registry = IsinRegistry::from_holder(table()).unwrap();
        let subject = subject(&registry, "eusipacode");
        let category = Some(yggdryl::Eusipa::new(2300).unwrap());
        registry
            .merge(entry(HOLCIM, &[]).with_eusipacode(category))
            .unwrap();
        assert_eq!(registry.commit().unwrap().written_rows, 1);
        assert_eq!(count(SITE, WHAT, &subject), 1);
        let back = IsinRegistry::from_holder(table()).unwrap();
        assert_eq!(
            back.get(HOLCIM).unwrap().eusipacode(),
            None,
            "the table's own schema"
        );
    }
}
