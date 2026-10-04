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

/// The row of `text` listed on `mic`.
fn listed(text: &str, mic: &str) -> IsinEntry {
    entry(text, &[]).with_miccode(Some(yggdryl::Mic::new(mic).unwrap()))
}

/// The registry the location `url` names, bound with no property.
fn at(url: &Url) -> yggdryl::Result<IsinRegistry> {
    IsinRegistry::from_url(url, std::iter::empty::<(&str, &str)>())
}

/// An Iceberg table at `root` over the registry's row, partitioned by
/// market.
#[cfg(feature = "iceberg")]
fn create_by_market<H: IOBase>(root: H) {
    use yggdryl::iceberg::{FIRST_PARTITION_ID, FormatVersion, IcebergTable, PartitionSpec};
    let mut schema = IsinEntry::field()
        .with_partition_fields(&["miccode"])
        .unwrap();
    yggdryl::iceberg::assign_field_ids(&mut schema, 1).unwrap();
    let spec = PartitionSpec::from_schema(FIRST_PARTITION_ID, &schema).unwrap();
    IcebergTable::create(root, FormatVersion::V3, schema, spec).unwrap();
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
        .merge(entry(HOLCIM, &[(IdType::Ric, "HOLN.S")]))
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
    let mut registry = at(&url).unwrap();
    assert!(registry.is_empty() && !registry.is_dirty());
    assert!(!root.exists(), "nothing is laid out before a commit");
    registry
        .merge(entry(HOLCIM, &[(IdType::Ric, "HOLN.S")]))
        .unwrap();
    registry.merge(entry(APPLE, &[])).unwrap();
    assert_eq!(registry.commit().unwrap().written_rows, 2);
    assert!(root.join("part-0.arrows").is_file());
    let back = at(&url).unwrap();
    assert!(back.iter().eq(registry.iter()));
    assert!(!back.is_dirty());
    // Rewritten in place: one part still.
    registry
        .merge(entry(HOLCIM, &[(IdType::Common, "C-1")]))
        .unwrap();
    registry.commit().unwrap();
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
    assert_eq!(
        at(&url).unwrap().get(HOLCIM).unwrap().get(&IdType::Common),
        Some("C-1")
    );
    // Emptied: the part removed, a file that is no record part kept, and
    // read back empty.
    std::fs::write(root.join("README.md"), b"the instrument registry").unwrap();
    registry.clear();
    registry.commit().unwrap();
    assert!(!root.join("part-0.arrows").exists());
    assert!(root.join("README.md").is_file());
    assert!(at(&url).unwrap().is_empty());
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
    let mut registry = at(&url).unwrap();
    assert!(registry.is_empty() && !registry.is_dirty());
    registry.merge(listed(HOLCIM, "XSWX")).unwrap();
    registry.merge(listed(APPLE, "XNAS")).unwrap();
    assert_eq!(registry.commit().unwrap().written_rows, 2);
    assert!(root.join("miccode=XSWX/part-0.arrows").is_file());
    assert!(root.join("miccode=XNAS/part-0.arrows").is_file());
    let back = at(&url).unwrap();
    assert!(back.iter().eq(registry.iter()));
    // A removal reaches the partition the row was stored in.
    registry.remove(HOLCIM).unwrap();
    assert_eq!(registry.commit().unwrap().written_rows, 1);
    assert!(!root.join("miccode=XSWX/part-0.arrows").exists());
    let back = at(&url).unwrap();
    assert_eq!(back.len(), 1);
    assert!(back.get(APPLE).is_some());
    assert!(root.join("README.md").is_file());
    // Emptied: no part anywhere, the README kept.
    registry.clear();
    registry.commit().unwrap();
    assert!(!root.join("miccode=XNAS/part-0.arrows").exists());
    assert!(root.join("README.md").is_file());
    assert!(at(&url).unwrap().is_empty());
    let _ = std::fs::remove_dir_all(root.parent().unwrap());
}

/// A leaf names its encoding by its name: one this build has no record
/// encoding for is refused at the binding rather than laid out as another.
#[test]
fn a_leaf_of_an_unknown_encoding_is_refused_at_the_binding() {
    let root = crate::scratch("unknown");
    let url = Url::from_path(root.join("instruments.xyz")).unwrap();
    assert!(at(&url).is_err());
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
        .merge(entry(HOLCIM, &[(IdType::Ric, "HOLN.S")]))
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
    let partitioned = || folder_on(&filesystem, "partitioned");
    create_by_market(partitioned());
    let mut registry = IsinRegistry::from_holder(partitioned()).unwrap();
    registry.merge(listed(HOLCIM, "XSWX")).unwrap();
    registry.merge(listed(APPLE, "XNAS")).unwrap();
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
    create_by_market(local("table"));
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
