use super::*;
#[cfg(feature = "arrow")]
use crate::IOMedia;
use crate::holder::local::{File, Folder, Path};

/// A temp root nothing else in this file uses.
fn root(label: &str) -> std::path::PathBuf {
    let mut root = Folder::temporary()
        .expect("the temporary directory")
        .path()
        .expect("a platform path");
    root.push(format!("yggdryl-lifecycle-{label}-{}", std::process::id()));
    let mut folder = Folder::new(&root).expect("a local container");
    folder.remove(true).expect("a removable tree");
    root
}

/// A handle counting every method a probe would go through.
///
/// The point is the assertion this makes possible: `remove` on an absent
/// resource must issue exactly one delete attempt and *zero* probes. A
/// future `if self.kind() == IOKind::Unknown` convenience guard would be
/// invisible in prose and obvious here.
#[derive(Default)]
struct Counted {
    bytes: Buffer,
    kinds: std::cell::Cell<usize>,
    sizes: std::cell::Cell<usize>,
    listings: std::cell::Cell<usize>,
    deletes: std::cell::Cell<usize>,
    clears: std::cell::Cell<usize>,
}

// Written out rather than delegated, because every method the delegation
// would supply is one this double has to count. The counters are
// per-handle and never shared across threads; `IOBase` requires `Send`,
// and `Cell` is `Send` when its contents are.
impl crate::IOMedia for Counted {
    crate::impl_default_iomedia!();
}

impl IOBase for Counted {
    fn pread(&self, offset: u64, buffer: &mut [u8]) -> crate::Result<usize> {
        self.bytes.pread(offset, buffer)
    }

    fn pwrite(&mut self, offset: u64, bytes: &[u8]) -> crate::Result<usize> {
        self.bytes.pwrite(offset, bytes)
    }

    fn capacity(&self) -> u64 {
        self.bytes.capacity()
    }

    fn reserve(&mut self, capacity: u64) -> crate::Result<()> {
        self.bytes.reserve(capacity)
    }

    fn truncate(&mut self, size: u64) -> crate::Result<()> {
        self.bytes.truncate(size)
    }

    fn url(&self) -> Option<&crate::Url> {
        self.bytes.url()
    }

    fn media_type(&self) -> &crate::MediaType {
        self.bytes.media_type()
    }

    fn set_media_type(&mut self, media_type: crate::MediaType) {
        self.bytes.set_media_type(media_type);
    }

    fn kind(&self) -> crate::IOKind {
        self.kinds.set(self.kinds.get() + 1);
        crate::IOKind::Memory
    }

    fn size(&self) -> u64 {
        self.sizes.set(self.sizes.get() + 1);
        self.bytes.size()
    }

    fn ls(&self, _recursive: bool, _include_private: bool) -> crate::Listing {
        self.listings.set(self.listings.get() + 1);
        crate::Listing::empty()
    }

    fn clear(&mut self) -> crate::Result<()> {
        self.clears.set(self.clears.get() + 1);
        self.bytes.clear()
    }

    fn remove(&mut self, recursive: bool) -> crate::Result<()> {
        // Exactly what every backend does: issue the delete, treat the
        // store's own not-found answer as success, probe nothing first.
        self.deletes.set(self.deletes.get() + 1);
        self.bytes.remove(recursive)
    }
}

#[test]
fn removing_an_absent_resource_issues_one_delete_and_no_probe() {
    let mut handle = Counted::default();
    handle.remove(false).expect("absence is a no-op success");

    assert_eq!(handle.deletes.get(), 1, "exactly one delete attempt");
    assert_eq!(handle.kinds.get(), 0, "no kind() probe");
    assert_eq!(handle.sizes.get(), 0, "no size() probe");
    assert_eq!(handle.listings.get(), 0, "no ls() probe");

    handle.clear().expect("absence is a no-op success");
    assert_eq!(handle.clears.get(), 1);
    assert_eq!(handle.kinds.get(), 0, "no kind() probe");
    assert_eq!(handle.sizes.get(), 0, "no size() probe");
}

#[test]
fn a_leaf_clears_empty_and_removes_gone() {
    let root = root("leaf");
    let path = root.join("trades.csv");
    let mut leaf = File::new(&path).expect("a local leaf");
    leaf.write_all_bytes(b"symbol,price\n").expect("a write");
    leaf.flush().expect("a flush");

    leaf.clear().expect("a clearable leaf");
    assert_eq!(leaf.size(), 0, "cleared to empty");
    assert!(path.exists(), "the resource still exists after clear");

    leaf.remove(false).expect("a removable leaf");
    assert!(!path.exists(), "removed");

    // Both succeed a second time: absence is never an error.
    leaf.clear().expect("clearing an absent leaf");
    assert!(!path.exists(), "clearing an absent leaf never creates it");
    leaf.remove(false).expect("removing an absent leaf");

    // The handle stays usable and lazy - a write recreates the resource.
    leaf.write_all_bytes(b"MSFT,2").expect("a write");
    leaf.flush().expect("a flush");
    assert_eq!(leaf.read_all_bytes().expect("a read"), b"MSFT,2");

    Folder::new(&root).expect("a container").remove(true).ok();
}

#[test]
fn a_container_clears_empty_and_removes_by_recursion() {
    let root = root("container");
    let mut folder = Folder::new(&root).expect("a local container");
    folder.truncate(0).expect("a created container");
    for name in ["a.log", "b.log"] {
        folder
            .child_by_path(name)
            .expect("a child")
            .write_all_bytes(b"line\n")
            .expect("a write");
    }
    let mut nested = Folder::new(root.join("deep")).expect("a local container");
    nested.truncate(0).expect("a created container");
    nested
        .child_by_path("c.log")
        .expect("a child")
        .write_all_bytes(b"line\n")
        .expect("a write");

    folder.clear().expect("a clearable container");
    assert!(root.exists(), "the container still exists after clear");
    assert!(
        folder
            .ls(true, true)
            .collect::<crate::Result<Vec<_>>>()
            .expect("a listing")
            .is_empty(),
        "and is empty"
    );

    // An empty container is removable without recursion.
    folder.remove(false).expect("an empty container removes");
    assert!(!root.exists());

    // A populated one is not.
    folder.truncate(0).expect("a created container");
    folder
        .child_by_path("a.log")
        .expect("a child")
        .write_all_bytes(b"line\n")
        .expect("a write");
    let refused = folder.remove(false).expect_err("a populated container");
    let message = refused.to_string();
    assert!(message.contains("children"), "{message}");
    assert!(
        message.contains(root.file_name().expect("a name").to_string_lossy().as_ref()),
        "the refusal names the location: {message}"
    );
    assert!(root.exists(), "and nothing was deleted");

    folder.remove(true).expect("a recursive removal");
    assert!(!root.exists());
    folder.remove(true).expect("absence is a no-op success");
}

#[test]
fn a_generic_path_routes_on_the_kind_it_already_resolved() {
    let root = root("path");
    Folder::new(&root)
        .expect("a container")
        .truncate(0)
        .expect("a created container");

    let leaf = root.join("one.log");
    Path::new(&leaf)
        .expect("a location")
        .write_all_bytes(b"line\n")
        .expect("a write");

    let mut path = Path::new(&leaf).expect("a location");
    assert_eq!(path.kind(), crate::IOKind::File);
    path.remove(false).expect("a removable leaf");
    assert!(!leaf.exists());

    let mut container = Path::new(&root).expect("a location");
    assert_eq!(container.kind(), crate::IOKind::Directory);
    container.remove(true).expect("a removable container");
    assert!(!root.exists());

    // Undecided is absence, which is a no-op success.
    let mut absent = Path::new(root.join("never")).expect("a location");
    assert_eq!(absent.kind(), crate::IOKind::Unknown);
    absent.clear().expect("a no-op clear");
    absent.remove(true).expect("a no-op removal");
}

#[test]
fn a_pending_write_cannot_survive_a_removal() {
    let root = root("pending");
    let path = root.join("staged.bin");
    let mut leaf = File::new(&path).expect("a local leaf");

    // Write, do not flush, remove, then flush.
    leaf.pwrite(0, b"unflushed").expect("a write");
    leaf.remove(false).expect("a removable leaf");
    leaf.flush().expect("a flush after removal");

    assert!(
        !path.exists(),
        "a flush after a removal must not recreate the resource"
    );
    Folder::new(&root).expect("a container").remove(true).ok();
}

#[test]
fn a_buffer_gives_its_allocation_back() {
    let mut buffer = Buffer::from_bytes(vec![7_u8; 4096]);
    buffer.clear().expect("a clearable buffer");
    assert_eq!(buffer.size(), 0);
    assert!(
        buffer.capacity() >= 4096,
        "clearing keeps the allocation for the next write"
    );

    buffer.remove(false).expect("a removable buffer");
    assert_eq!(buffer.size(), 0);
    assert_eq!(buffer.capacity(), 0, "removing gives the memory back");

    // Still usable and lazy afterwards.
    buffer.write_all_bytes(b"AAPL").expect("a write");
    assert_eq!(buffer.read_all_bytes().expect("a read"), b"AAPL");
}

#[test]
fn a_coding_handle_removes_the_encoded_resource() {
    let root = root("coded");
    let path = root.join("trades.csv.gz");
    let mut coded = crate::coding::gzip::Gzip::new(File::new(&path).expect("a local leaf"));
    coded
        .write_all_bytes(b"symbol,price\n")
        .expect("a decoded write");
    coded.close().expect("a published value");
    assert!(path.exists(), "the encoded bytes are on disk");

    coded.remove(false).expect("a removable coded resource");
    assert!(!path.exists(), "the .gz resource itself is gone");
    assert_eq!(coded.read_all_bytes().expect("a read").len(), 0);

    // A later flush must not resurrect it from a held decoded buffer.
    coded.flush().expect("a flush after removal");
    assert!(!path.exists());

    Folder::new(&root).expect("a container").remove(true).ok();
}

#[cfg(feature = "arrow")]
#[test]
fn a_media_handle_drops_its_cache_as_part_of_the_removal() {
    use crate::arrow::batch_reader;
    use crate::media::ipc::Ipc;

    let field = crate::DataType::from_fields([crate::DataType::Int64.required_field("id")])
        .expect("a struct root")
        .required_field("row");
    let schema = crate::arrow::arrow_schema_from_field(&field).expect("an Arrow schema");

    let mut media = Ipc::new(Buffer::new());
    let options = media.record_options().expect("IPC options");
    media
        .overwrite_arrow_reader(batch_reader(schema, []), &options)
        .expect("a write");
    media.open().expect("an opened media");
    assert!(media.opened(), "the schema is cached");

    media.remove(false).expect("a removable media");
    assert!(
        !media.opened(),
        "the cache is invalidated as part of the removal, not on the next open"
    );
    assert_eq!(media.size(), 0, "and the encoded bytes are gone");
}
