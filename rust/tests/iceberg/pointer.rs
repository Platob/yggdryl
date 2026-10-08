//! `rust/src/iceberg/pointer.rs`: a table whose current document a
//! [`MetadataPointer`] names, created, opened and committed to through it.
//!
//! The pointer here is a value in memory; the table's folder is a counting
//! filesystem, so every pin below states what the store was asked - and that
//! it was never asked to list, nor to delete, nor for a version hint. A
//! removal is the pointer's own to answer: the folder is asked for nothing
//! either way.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use arrow_array::{Array, Int64Array, RecordBatch, StringArray};

use crate::counting_filesystem::{CountingFileSystem, counted_folder};
use yggdryl::arrow::BatchReader;
use yggdryl::fs::FileSystem;
use yggdryl::iceberg::{
    FormatVersion, IcebergTable, MetadataPointer, PartitionSpec, PointerState, assign_field_ids,
};
use yggdryl::{DataType, Error, Field, IOBase, IOKind, StructType, Url};

/// A pointer held in memory: the location it names and a counter for its
/// token, with a tally of what it was asked.
#[derive(Debug, Default)]
struct MemoryPointer {
    held: Mutex<(Option<Url>, u64)>,
    reads: AtomicUsize,
    publications: AtomicUsize,
    /// Take the next publication, then answer it as a transport failure.
    lose_next_answer: Mutex<bool>,
}

impl MemoryPointer {
    /// Move the token as a change that writes no document does - a rename.
    fn bump(&self) {
        self.held.lock().unwrap().1 += 1;
    }

    /// Name `location` current, as a concurrent writer's commit does.
    fn force(&self, location: Url) {
        let mut held = self.held.lock().unwrap();
        *held = (Some(location), held.1 + 1);
    }

    fn named(&self) -> Option<Url> {
        self.held.lock().unwrap().0.clone()
    }

    fn calls(&self) -> (usize, usize) {
        (
            self.reads.load(Ordering::SeqCst),
            self.publications.load(Ordering::SeqCst),
        )
    }
}

impl MetadataPointer for MemoryPointer {
    fn current(&self) -> yggdryl::Result<PointerState> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        let held = self.held.lock().unwrap();
        Ok(PointerState::new(held.0.clone(), held.1.to_string()))
    }

    fn publish(&self, token: &str, location: &Url) -> yggdryl::Result<PointerState> {
        self.publications.fetch_add(1, Ordering::SeqCst);
        let mut held = self.held.lock().unwrap();
        if token != held.1.to_string() {
            return Err(Error::conflict("table version", "table version", location));
        }
        *held = (Some(location.clone()), held.1 + 1);
        if std::mem::take(&mut *self.lose_next_answer.lock().unwrap()) {
            return Err(Error::Io(std::io::Error::new(
                std::io::ErrorKind::ConnectionReset,
                "the answer was lost",
            )));
        }
        Ok(PointerState::new(held.0.clone(), held.1.to_string()))
    }
}

/// A pointer whose catalog drops a table: [`MemoryPointer`], counting the
/// drops it was asked for.
#[derive(Debug, Default)]
struct DroppingPointer {
    pointer: MemoryPointer,
    drops: AtomicUsize,
}

impl MetadataPointer for DroppingPointer {
    fn current(&self) -> yggdryl::Result<PointerState> {
        self.pointer.current()
    }

    fn publish(&self, token: &str, location: &Url) -> yggdryl::Result<PointerState> {
        self.pointer.publish(token, location)
    }

    fn remove(&self) -> yggdryl::Result<()> {
        self.drops.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

/// A pointer whose catalog refuses a drop for a reason of its own:
/// [`MemoryPointer`], its `remove` another refusal than the provided one.
#[derive(Debug, Default)]
struct RefusingPointer(MemoryPointer);

impl MetadataPointer for RefusingPointer {
    fn current(&self) -> yggdryl::Result<PointerState> {
        self.0.current()
    }

    fn publish(&self, token: &str, location: &Url) -> yggdryl::Result<PointerState> {
        self.0.publish(token, location)
    }

    fn remove(&self) -> yggdryl::Result<()> {
        Err(Error::unsupported(
            "a SOCKS proxy, which this transport does not speak",
            "the catalog's transport",
        ))
    }
}

/// `(venue, id)` rows under an unpartitioned schema.
fn schema() -> Field {
    let mut schema = StructType::from_fields([
        DataType::utf8().required_field("venue"),
        DataType::Int64.required_field("id"),
    ])
    .map(DataType::from)
    .unwrap()
    .required_field("row");
    assign_field_ids(&mut schema, 1).unwrap();
    schema
}

fn rows(rows: &[(&str, i64)]) -> BatchReader {
    let schema = Arc::new(arrow_schema::Schema::new(vec![
        arrow_schema::Field::new("venue", arrow_schema::DataType::Utf8, false),
        arrow_schema::Field::new("id", arrow_schema::DataType::Int64, false),
    ]));
    let batch = RecordBatch::try_new(
        Arc::clone(&schema),
        vec![
            Arc::new(StringArray::from(
                rows.iter().map(|row| row.0).collect::<Vec<_>>(),
            )),
            Arc::new(Int64Array::from(
                rows.iter().map(|row| row.1).collect::<Vec<_>>(),
            )),
        ],
    )
    .unwrap();
    yggdryl::arrow::batch_reader(schema, [batch])
}

/// Every id the current snapshot holds, ascending.
fn ids<H: yggdryl::IOBase>(table: &IcebergTable<H>) -> Vec<i64> {
    let mut ids: Vec<i64> = table
        .scan(None)
        .unwrap()
        .flat_map(|batch| {
            let batch = batch.unwrap();
            let column = batch.column_by_name("id").unwrap();
            let column = column.as_any().downcast_ref::<Int64Array>().unwrap();
            (0..column.len())
                .map(|row| column.value(row))
                .collect::<Vec<_>>()
        })
        .collect();
    ids.sort_unstable();
    ids
}

/// A pointed table created over a fresh counting folder.
fn created(
    name: &str,
) -> (
    Arc<CountingFileSystem>,
    Arc<MemoryPointer>,
    IcebergTable<yggdryl::fs::FsFolder>,
) {
    let (filesystem, folder) = counted_folder(name);
    let pointer = Arc::new(MemoryPointer::default());
    let table = IcebergTable::create_pointed(
        folder,
        FormatVersion::V3,
        schema(),
        PartitionSpec::unpartitioned(),
        Arc::clone(&pointer) as Arc<dyn MetadataPointer>,
    )
    .unwrap();
    (filesystem, pointer, table)
}

/// Whether the store holds anything at `path`.
fn holds(filesystem: &CountingFileSystem, path: &str) -> bool {
    filesystem.file_info(path).unwrap().kind != IOKind::Unknown
}

/// No call the pointed contract forbids: a listing, a delete, a move.
fn assert_forbids_nothing(costs: &str) {
    for forbidden in [
        "list",
        "delete_file",
        "delete_dir",
        "delete_dir_contents",
        "move_file",
    ] {
        assert!(
            !costs
                .split(' ')
                .any(|call| call.starts_with(&format!("{forbidden}="))),
            "{forbidden} in {costs}"
        );
    }
}

#[test]
fn a_created_table_publishes_version_zero_and_writes_no_hint() {
    let (filesystem, folder) = counted_folder("created");
    let pointer = Arc::new(MemoryPointer::default());
    let mut table = None;
    let costs = filesystem.costs(|| {
        table = Some(
            IcebergTable::create_pointed(
                folder,
                FormatVersion::V3,
                schema(),
                PartitionSpec::unpartitioned(),
                Arc::clone(&pointer) as Arc<dyn MetadataPointer>,
            )
            .unwrap(),
        );
    });
    let table = table.unwrap();
    // One read of the pointer, one document written - the write that finds
    // no folder makes it and opens again - and one publication.
    assert_eq!(costs, "create_dir=1 open_output_stream=2");
    assert_eq!(pointer.calls(), (1, 1));
    assert_eq!(table.metadata_version().unwrap(), 0);
    let name = table.metadata_file_name().unwrap();
    assert!(
        name.starts_with("00000-") && name.ends_with(".metadata.json"),
        "{name}"
    );
    assert_eq!(
        pointer.named().map(|url| url.to_string()),
        Some(table.metadata_location().unwrap())
    );
    assert!(holds(&filesystem, &format!("created/metadata/{name}")));
    assert!(!holds(&filesystem, "created/metadata/version-hint.text"));
}

#[test]
fn a_pointer_that_names_a_document_refuses_a_second_creation() {
    let (_, pointer, _) = created("twice");
    let (_, folder) = counted_folder("twice-again");
    let error = IcebergTable::create_pointed(
        folder,
        FormatVersion::V3,
        schema(),
        PartitionSpec::unpartitioned(),
        pointer as Arc<dyn MetadataPointer>,
    )
    .unwrap_err();
    assert!(error.is_conflict(), "{error}");
}

#[test]
fn open_reads_the_one_document_the_pointer_names() {
    let (filesystem, pointer, mut table) = created("opened");
    table.commit_append(rows(&[("XNAS", 1)])).unwrap();
    assert_eq!(table.metadata_version().unwrap(), 1);

    // A hint planted beside the documents names nothing a pointed table
    // reads: the pointer is the one answer.
    let folder = yggdryl::fs::FsFolder::from_path(
        Arc::clone(&filesystem) as Arc<dyn FileSystem>,
        "opened",
        None,
    )
    .unwrap();
    folder
        .child_by_path("metadata/version-hint.text")
        .unwrap()
        .write_all_bytes(b"7")
        .unwrap();
    let mut opened = None;
    let costs = filesystem.costs(|| {
        opened = Some(
            IcebergTable::open_pointed(folder, Arc::clone(&pointer) as Arc<dyn MetadataPointer>)
                .unwrap(),
        );
    });
    assert_eq!(costs, "open_input_stream=1");
    let opened = opened.unwrap();
    assert_eq!(opened.metadata_version().unwrap(), 1);
    assert_eq!(
        opened.metadata_file_name().unwrap(),
        table.metadata_file_name().unwrap()
    );
    assert_eq!(ids(&opened), [1]);
}

#[test]
fn appends_and_overwrites_publish_through_the_pointer_and_remove_nothing() {
    let (filesystem, pointer, mut table) = created("written");
    let costs = filesystem.costs(|| {
        table
            .commit_append(rows(&[("XNAS", 1), ("XLON", 2)]))
            .unwrap();
        table.commit_append(rows(&[("XNYS", 3)])).unwrap();
        table.commit_overwrite(rows(&[("XPAR", 4)])).unwrap();
    });
    assert_forbids_nothing(&costs);
    assert_eq!(table.metadata_version().unwrap(), 3);
    assert!(
        table.metadata_file_name().unwrap().starts_with("00003-"),
        "{}",
        table.metadata_file_name().unwrap()
    );
    assert_eq!(ids(&table), [4]);
    // Every commit is one publication and no read of the pointer: the
    // token the last publication answered is the next one's condition.
    assert_eq!(pointer.calls(), (1, 4));
    assert!(!holds(&filesystem, "written/metadata/version-hint.text"));

    // The document before names the one before it, as Iceberg's log does.
    assert_eq!(table.metadata().unwrap().metadata_log().len(), 3);

    // The folder contract, for contrast, claims its version with one
    // exclusive create of `v{n}.metadata.json` and then writes the hint
    // whole, because its folder names the document where the pointer does
    // here; like the pointed path, it lists and removes nothing.
    let (plain_filesystem, plain) = counted_folder("plain");
    let mut plain = IcebergTable::create(
        plain,
        FormatVersion::V3,
        schema(),
        PartitionSpec::unpartitioned(),
    )
    .unwrap();
    let plain_costs = plain_filesystem.costs(|| {
        plain.commit_append(rows(&[("XNAS", 1)])).unwrap();
    });
    assert!(
        plain_costs.contains("create_file=1")
            && !plain_costs.contains("delete_file=")
            && !plain_costs.contains("list="),
        "{plain_costs}"
    );
    assert!(holds(&plain_filesystem, "plain/metadata/version-hint.text"));
}

#[test]
fn a_moved_pointer_rebases_an_append_onto_the_winner() {
    let (filesystem, pointer, mut first) = created("rebased");
    let reopen = || {
        IcebergTable::open_pointed(
            yggdryl::fs::FsFolder::from_path(
                Arc::clone(&filesystem) as Arc<dyn FileSystem>,
                "rebased",
                None,
            )
            .unwrap(),
            Arc::clone(&pointer) as Arc<dyn MetadataPointer>,
        )
        .unwrap()
    };
    let mut second = reopen();
    second.commit_append(rows(&[("XLON", 2)])).unwrap();

    // The first handle still holds version 0 and its token: its publication
    // is refused, it reads where the pointer stands and applies again.
    let costs = filesystem.costs(|| {
        first.commit_append(rows(&[("XNAS", 1)])).unwrap();
    });
    assert_forbids_nothing(&costs);
    assert_eq!(first.metadata_version().unwrap(), 2);
    assert_eq!(ids(&first), [1, 2]);
    assert_eq!(ids(&reopen()), [1, 2]);
}

#[test]
fn a_metadata_only_commit_rebases_too() {
    let (_, pointer, mut table) = created("properties");
    let winner = pointer.named().unwrap();
    pointer.force(winner);
    table
        .commit_metadata_changes(|metadata| {
            metadata.set_property("owner", "desk")?;
            Ok(())
        })
        .unwrap();
    assert_eq!(table.metadata_version().unwrap(), 1);
    assert_eq!(table.metadata().unwrap().property("owner"), Some("desk"));
}

#[test]
fn a_moved_pointer_is_a_conflict_for_an_overwrite_and_leaves_its_files() {
    let (filesystem, pointer, mut first) = created("conflicted");
    first.commit_append(rows(&[("XNAS", 1)])).unwrap();
    let mut second = IcebergTable::open_pointed(
        yggdryl::fs::FsFolder::from_path(
            Arc::clone(&filesystem) as Arc<dyn FileSystem>,
            "conflicted",
            None,
        )
        .unwrap(),
        Arc::clone(&pointer) as Arc<dyn MetadataPointer>,
    )
    .unwrap();
    second.commit_append(rows(&[("XLON", 2)])).unwrap();

    let held = first.metadata_file_name().unwrap();
    let mut error = None;
    let costs = filesystem.costs(|| {
        error = Some(first.commit_overwrite(rows(&[("XPAR", 3)])).unwrap_err());
    });
    let error = error.unwrap();
    assert!(error.is_conflict(), "{error}");
    assert!(error.to_string().contains("last saw version 2"), "{error}");
    // The data file, the manifest, the list and the document it wrote stay:
    // nothing is removed on the way out.
    assert_forbids_nothing(&costs);
    assert!(costs.contains("open_output_stream=4"), "{costs}");
    // The handle is where it was before the commit.
    assert_eq!(first.metadata_file_name().unwrap(), held);
    assert_eq!(ids(&first), [1]);
}

#[test]
fn a_token_moved_by_no_document_publishes_an_overwrite_again() {
    let (_, pointer, mut table) = created("renamed");
    table.commit_append(rows(&[("XNAS", 1)])).unwrap();
    pointer.bump();
    table.commit_overwrite(rows(&[("XLON", 2)])).unwrap();
    assert_eq!(table.metadata_version().unwrap(), 2);
    assert_eq!(ids(&table), [2]);
}

#[test]
fn a_publication_in_doubt_that_took_is_the_commit() {
    let (_, pointer, mut table) = created("doubted");
    *pointer.lose_next_answer.lock().unwrap() = true;
    table.commit_append(rows(&[("XNAS", 1)])).unwrap();
    assert_eq!(table.metadata_version().unwrap(), 1);
    assert_eq!(
        pointer.named().map(|url| url.to_string()),
        Some(table.metadata_location().unwrap())
    );
    // The next commit publishes under the token the pointer stands at.
    table.commit_append(rows(&[("XLON", 2)])).unwrap();
    assert_eq!(ids(&table), [1, 2]);
}

#[test]
fn a_pointed_table_neither_lists_nor_removes_its_folder() {
    let (filesystem, pointer, mut table) = created("kept");
    table.commit_append(rows(&[("XNAS", 1)])).unwrap();
    let name = table.metadata_file_name().unwrap();

    // The catalog keeps the table: its folder is neither listed nor
    // removed, however the table is asked, and nothing reaches the store.
    // A removal is the pointer's to answer, and one that drops nothing
    // refuses - restated by the table naming where it is, as the listing's
    // refusal does.
    let mut listed = None;
    let mut removed = None;
    let costs = filesystem.costs(|| {
        listed = Some(table.ls(true, false).collect::<Vec<_>>());
        removed = Some((table.remove(true), table.remove(false)));
    });
    assert_eq!(costs, "none");
    let listed = listed.unwrap();
    assert_eq!(listed.len(), 1, "one refusal");
    let error = listed.into_iter().next().unwrap().unwrap_err();
    assert!(error.to_string().contains("listing the files"), "{error}");
    let (recursive, flat) = removed.unwrap();
    for error in [recursive.unwrap_err(), flat.unwrap_err()] {
        assert!(matches!(error, Error::Unsupported { .. }), "{error:?}");
        assert!(
            error
                .to_string()
                .contains("dropping a table through its pointer"),
            "{error}"
        );
        assert!(error.to_string().contains("kept"), "{error}");
    }
    let listing = table.ls(false, false).next().unwrap().unwrap_err();
    let removal = table.remove(true).unwrap_err();
    let location = IOBase::url(table.root()).unwrap().to_string();
    for error in [listing, removal] {
        assert!(error.to_string().contains(&location), "{error}");
    }

    // The table is where it was, and still reads.
    assert!(holds(&filesystem, &format!("kept/metadata/{name}")));
    assert_eq!(
        pointer.named().map(|url| url.to_string()),
        Some(table.metadata_location().unwrap())
    );
    assert_eq!(ids(&table), [1]);
}

#[test]
fn removing_a_pointed_table_asks_its_pointer_and_never_its_folder() {
    let (filesystem, folder) = counted_folder("dropped");
    let pointer = Arc::new(DroppingPointer::default());
    let mut table = IcebergTable::create_pointed(
        folder,
        FormatVersion::V3,
        schema(),
        PartitionSpec::unpartitioned(),
        Arc::clone(&pointer) as Arc<dyn MetadataPointer>,
    )
    .unwrap();
    table.commit_append(rows(&[("XNAS", 1)])).unwrap();
    let name = table.metadata_file_name().unwrap();

    // The catalog drops the table whole, so the pointer is asked once
    // whatever `recursive` says, and the store is asked for nothing: the
    // files are the catalog's to collect.
    let costs = filesystem.costs(|| {
        table.remove(false).unwrap();
        table.remove(true).unwrap();
    });
    assert_eq!(costs, "none");
    assert_eq!(pointer.drops.load(Ordering::SeqCst), 2);
    assert!(holds(&filesystem, &format!("dropped/metadata/{name}")));

    // The document the value had read went with the table: what it is
    // asked next is asked of the pointer again - one answer and one read,
    // here of a pointer that still names the document - so a catalog that
    // dropped the table is what answers, never a document held from before.
    let (asked, _) = pointer.pointer.calls();
    let costs = filesystem.costs(|| {
        table.metadata_version().unwrap();
    });
    assert_eq!(costs, "open_input_stream=1");
    assert_eq!(pointer.pointer.calls().0, asked + 1);
}

/// Only the provided refusal - a pointer that drops nothing - is restated
/// naming the table; a refusal of the pointer's own reaches the caller as
/// it came, and a drop that did not take forgets nothing.
#[test]
fn a_pointers_own_refusal_of_a_drop_reaches_the_caller_as_it_came() {
    let (filesystem, folder) = counted_folder("refused");
    let mut table = IcebergTable::create_pointed(
        folder,
        FormatVersion::V3,
        schema(),
        PartitionSpec::unpartitioned(),
        Arc::new(RefusingPointer::default()) as Arc<dyn MetadataPointer>,
    )
    .unwrap();
    let costs = filesystem.costs(|| {
        let error = table.remove(true).unwrap_err();
        assert!(error.is_unsupported(), "{error}");
        let text = error.to_string();
        assert!(text.contains("a SOCKS proxy"), "{text}");
        assert!(!text.contains("drops none"), "{text}");
    });
    assert_eq!(costs, "none");
    let costs = filesystem.costs(|| {
        assert!(ids(&table).is_empty());
    });
    assert_eq!(costs, "none");
}

#[test]
fn a_pointer_naming_no_document_opens_nothing() {
    let (_, folder) = counted_folder("unnamed");
    let pointer = Arc::new(MemoryPointer::default());
    let error =
        IcebergTable::open_pointed(folder, pointer as Arc<dyn MetadataPointer>).unwrap_err();
    assert!(error.to_string().contains("to name a document"), "{error}");
}
