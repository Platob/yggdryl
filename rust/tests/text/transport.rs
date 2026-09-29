//! `rust/src/text/transport.rs`: the one decoded stream a text-shaped read
//! takes over a handle - owned, where the reader outlives the borrow.
//!
//! An owned read reopens the handle's location for a stream of its own. A
//! handle that applies a coding presents its bytes decoded, so what lies at
//! the location is coded and the reopened stream has that coding to peel
//! again; this is the shape `Holder::from_url` and both bindings build for
//! every `*.gz`, `*.zst` and `*.zz` name.

use std::path::PathBuf;
use std::sync::Arc;

use yggdryl::coding::Coded;
use yggdryl::fs::{BoundLocation, FileSystem, FsFile, MemoryFileSystem};
use yggdryl::holder::{Buffer, Holder};
use yggdryl::text::{Text, TextOptions, read_text_lines};
use yggdryl::{Codec, IOBase, IOMedia, Url};

const LINES: &[u8] = b"alpha\nbravo\ncharlie\n";

/// One of the two doors a path opens as a holder through.
type Opener = fn(&std::path::Path) -> yggdryl::Result<Holder>;

/// A fresh folder under the temporary directory, named after `label`.
fn temporary(label: &str) -> PathBuf {
    let mut root = yggdryl::local::LocalFolder::temporary()
        .expect("a temporary directory")
        .path()
        .expect("a path");
    root.push(format!(
        "yggdryl-text-transport-{label}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("the folder is created");
    root
}

/// The bodies `read_text_lines` answers over `handle` under the defaults.
fn bodies(handle: &(impl IOBase + ?Sized)) -> Vec<String> {
    read_text_lines(handle, &TextOptions::new())
        .expect("a settled configuration")
        .map(|line| line.expect("a line").body().to_owned())
        .collect()
}

/// The rows the record surface of `holder` answers, counted.
fn rows(holder: &Holder) -> usize {
    let options = holder.record_options().expect("the options");
    holder
        .read_arrow_reader(&options)
        .expect("a reader")
        .map(|batch| batch.expect("a batch").num_rows())
        .sum()
}

#[test]
fn a_composed_local_handle_reads_its_lines_through_the_coding_it_applies() {
    let root = temporary("composed");
    let openers: [(&str, Opener); 2] = [
        ("local", |path| Holder::local(path)),
        ("file", |path| Holder::file(path)),
    ];
    for (label, open) in openers {
        let path = root.join(format!("{label}.log.gz"));
        // Text records over a gzip view over the location: the write goes
        // out through the coding.
        let mut holder = open(&path).expect("a location").into_declared_media();
        assert!(matches!(holder, Holder::Text(_)), "{label}");
        holder.write_all_bytes(LINES).expect("written");
        let stored = std::fs::read(&path).expect("on disk");
        assert_eq!(&stored[..2], &[0x1F, 0x8B], "{label}");
        assert_eq!(
            Codec::Gzip.load(&stored).expect("decodes"),
            LINES,
            "{label}"
        );

        // And the same composition reopened reads the lines back, whether
        // the reader borrows the handle or reopens its location for a
        // stream of its own.
        let holder = open(&path).expect("a location").into_declared_media();
        let Holder::Text(text) = &holder else {
            panic!("{label}: expected text media, got {holder:?}");
        };
        assert_eq!(text.row_size().expect("the rows"), 3, "{label}");
        assert_eq!(
            bodies(text.as_ref()),
            ["alpha", "bravo", "charlie"],
            "{label}"
        );
        assert_eq!(
            text.read_text_lines()
                .expect("a settled configuration")
                .map(|line| line.expect("a line").body().to_owned())
                .collect::<Vec<_>>(),
            ["alpha", "bravo", "charlie"],
            "{label}"
        );
        assert_eq!(rows(&holder), 3, "{label}");
    }
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_coded_view_over_a_bound_location_reopens_the_stored_bytes_under_its_coding() {
    let filesystem: Arc<dyn FileSystem> = Arc::new(MemoryFileSystem::new());
    filesystem
        .open_output_stream("app.log.gz", None)
        .expect("a stream")
        .write(&Codec::Gzip.dump(LINES).expect("encodes"))
        .expect("written");
    let mut file = FsFile::new(
        BoundLocation::new(Arc::clone(&filesystem), "app.log.gz", None).expect("a location"),
    );
    file.set_media_type(
        Url::from_str("file:///app.log.gz")
            .expect("a url")
            .media_type(),
    );

    // The view presents the lines and declares no coding.
    let coded = Coded::infer(Holder::FsFile(file));
    assert_eq!(coded.applied_codec(), Codec::Gzip);
    assert!(coded.media_type().encodings().is_empty());
    assert_eq!(coded.read_all_bytes().expect("the bytes"), LINES);

    assert_eq!(bodies(&coded), ["alpha", "bravo", "charlie"]);
    let text = Text::new(coded);
    assert_eq!(text.row_size().expect("the rows"), 3);
    assert_eq!(
        text.read_text_lines()
            .expect("a settled configuration")
            .map(|line| line.expect("a line").body().to_owned())
            .collect::<Vec<_>>(),
        ["alpha", "bravo", "charlie"]
    );
}

#[test]
fn an_unlocated_coded_view_is_copied_decoded() {
    // A buffer has no location to reopen, so the owned stream is a copy of
    // what the view presents, under the media type it presents it as.
    let mut held = Buffer::new().with_media_type(
        Url::from_str("file:///app.log.gz")
            .expect("a url")
            .media_type(),
    );
    held.write_all_bytes(&Codec::Gzip.dump(LINES).expect("encodes"))
        .expect("written");
    let coded = Coded::infer(Holder::buffer(held));
    assert_eq!(bodies(&coded), ["alpha", "bravo", "charlie"]);
    assert_eq!(bodies(&Text::new(coded)), ["alpha", "bravo", "charlie"]);
}
