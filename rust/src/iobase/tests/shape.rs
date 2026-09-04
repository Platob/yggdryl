use super::{Buffer, IOBase};

use crate::coding::Coding;
use crate::holder::Holder;
use crate::holder::buffered::tests::Counting;
use crate::{Codec, IOKind, MediaType, MimeType};

/// A writable temporary root of this test's own.
fn root(label: &str) -> std::path::PathBuf {
    let mut path = crate::holder::local::Folder::temporary()
        .expect("the temporary directory")
        .path()
        .expect("a platform path");
    path.push(format!("yggdryl-shape-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).expect("a writable temporary root");
    path
}

#[test]
fn a_leaf_answers_from_its_representation_and_the_two_are_complements() {
    for (mime, tabular) in [
        (MimeType::PLAIN_TEXT, false),
        (MimeType::JSON, false),
        (MimeType::PARQUET, true),
        (MimeType::ARROW_FILE, true),
        (MimeType::CSV, true),
    ] {
        let mut handle = Buffer::new();
        handle.set_media_type(MediaType::from(mime.clone()));
        assert_eq!(handle.is_tabular(), tabular, "{mime}");
        // Exactly one of the two, because a leaf is read one way or the
        // other and never both.
        assert_eq!(handle.is_atomic(), !tabular, "{mime}");
        assert!(handle.is_io(), "{mime}");
    }
}

#[test]
fn a_default_buffer_is_one_whole_byte_value() {
    let handle = Buffer::from_bytes(b"AAPL".to_vec());
    assert_eq!(handle.kind(), IOKind::Memory);
    assert!(handle.is_atomic());
    assert!(!handle.is_tabular());
}

#[test]
fn a_content_coding_answers_for_the_representation_underneath_it() {
    // `trades.arrows.gz` is an Arrow file that happens to be compressed,
    // so the coding never changes which surface reads it.
    let media = MediaType::from_file_name("trades.arrows.gz");
    assert_eq!(media.base(), &MimeType::ARROW_STREAM);
    assert_eq!(media.encodings(), [MimeType::GZIP]);
    let mut handle = Buffer::new();
    handle.set_media_type(media);
    assert!(handle.is_tabular());
    assert!(!handle.is_atomic());

    let coded = Coding::new(handle, Codec::Gzip);
    assert!(coded.is_tabular());
    assert!(!coded.is_atomic());
}

#[test]
fn a_named_location_answers_before_anything_exists() {
    let path = root("named");

    // Nothing has been written, so the kind is undecided - and the name
    // still says which surface reads it, exactly as the media type does.
    let missing = crate::holder::local::Path::new(path.join("trades.parquet")).unwrap();
    assert_eq!(missing.kind(), IOKind::Unknown);
    assert_eq!(missing.media_type().base(), &MimeType::PARQUET);
    assert!(missing.is_tabular());
    assert!(!missing.is_atomic());

    let notes = crate::holder::local::Path::new(path.join("notes.txt")).unwrap();
    assert_eq!(notes.kind(), IOKind::Unknown);
    assert!(notes.is_atomic());
    assert!(!notes.is_tabular());

    // The leaf implementation answers the same, existing or not.
    let leaf = crate::holder::local::File::new(path.join("trades.arrows")).unwrap();
    assert!(leaf.is_tabular());
    assert!(!leaf.is_atomic());

    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn a_folder_reads_as_the_table_beneath_it() {
    let path = root("folder");
    let lake = path.join("lake");
    std::fs::create_dir_all(lake.join("year=2024/month=01")).unwrap();
    std::fs::write(lake.join("year=2024/month=01/part-0.parquet"), b"PAR1").unwrap();

    let folder = crate::holder::local::Folder::new(&lake).unwrap();
    assert_eq!(folder.kind(), IOKind::Directory);
    assert!(folder.is_container());
    // The probe descends to the first leaf; a folder is never one whole
    // byte value whatever is under it.
    assert!(folder.is_tabular());
    assert!(!folder.is_atomic());

    // A container of plain files is neither: no rows to read, and no one
    // byte value to read whole.
    let logs = path.join("logs");
    std::fs::create_dir_all(&logs).unwrap();
    std::fs::write(logs.join("run.txt"), b"started").unwrap();
    let folder = crate::holder::local::Folder::new(&logs).unwrap();
    assert!(!folder.is_tabular());
    assert!(!folder.is_atomic());
    assert!(!folder.is_io());

    // So is an empty one, and so is a folder that does not exist yet.
    let empty = crate::holder::local::Folder::new(path.join("empty")).unwrap();
    assert!(!empty.is_tabular());
    assert!(!empty.is_atomic());
    assert!(!empty.is_io());

    // A location resolving to that lake answers exactly as the folder did.
    let located = crate::holder::local::Path::new(&lake).unwrap();
    assert_eq!(located.kind(), IOKind::Directory);
    assert!(located.is_tabular());
    assert!(!located.is_atomic());

    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn a_record_encoding_handle_answers_without_touching_its_bytes() {
    // The buffer underneath carries no media type at all, so nothing but
    // the encoding itself can be answering here.
    let plain = Buffer::new();
    assert!(plain.is_atomic());

    let ipc = crate::media::ipc::Ipc::new(Buffer::new());
    assert!(ipc.is_tabular());
    assert!(!ipc.is_atomic());

    #[cfg(feature = "parquet")]
    {
        let parquet = crate::media::parquet::Parquet::new(Buffer::new());
        assert!(parquet.is_tabular());
        assert!(!parquet.is_atomic());
    }

    let avro = crate::media::avro::Avro::new(Buffer::new());
    assert!(avro.is_tabular());
    assert!(!avro.is_atomic());
}

#[test]
fn asking_the_shape_of_a_leaf_reads_nothing() {
    // The counting double is the measuring instrument the page cache uses:
    // it reports every `pread` and every `size` that reaches the bytes.
    let mut handle = Counting::from_bytes(b"PAR1".to_vec());
    handle.set_media_type(MediaType::from(MimeType::PARQUET));

    assert!(handle.is_tabular());
    assert!(!handle.is_atomic());

    // Both answers came from the representation, so nothing was read and
    // nothing was even measured.
    assert_eq!(handle.reads(), 0);
    assert_eq!(handle.sizes(), 0);
}

#[test]
fn wrapping_a_handle_keeps_the_shape_it_wraps() {
    let mut handle = Buffer::new();
    handle.set_media_type(MediaType::from(MimeType::PARQUET));

    // A page cache is invisible: it answers exactly what it wraps.
    let cached = handle.buffered(crate::holder::buffered::BufferedOptions::default());
    assert!(cached.is_tabular());
    assert!(!cached.is_atomic());

    // So is the generic enum every listing hands back.
    let held = Holder::from(Buffer::from_bytes(b"AAPL".to_vec()));
    assert!(held.is_atomic());
    assert!(!held.is_tabular());
}

#[cfg(feature = "arrow")]
#[test]
fn folder_dimensions_sum_only_the_selected_record_encoding() {
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch};

    use crate::IOMedia as _;

    fn rows(values: &[i64]) -> RecordBatch {
        let schema = Arc::new(arrow_schema::Schema::new(vec![arrow_schema::Field::new(
            "id",
            arrow_schema::DataType::Int64,
            false,
        )]));
        RecordBatch::try_new(schema, vec![Arc::new(Int64Array::from(values.to_vec()))])
            .expect("a dimension fixture")
    }

    let path = root("dimensions");
    let lake = path.join("lake");
    for (name, values) in [("a.arrows", vec![1, 2]), ("b.arrows", vec![3])] {
        let mut leaf = crate::holder::local::Path::new(lake.join(name)).expect("a lazy leaf");
        let batch = rows(&values);
        let options = leaf.record_options().expect("IPC options");
        leaf.overwrite_arrow_reader(
            crate::arrow::batch_reader(batch.schema(), [batch]),
            &options,
        )
        .expect("a published IPC leaf");
    }
    crate::holder::local::Path::new(lake.join("notes.txt"))
        .expect("a text leaf")
        .write_all_bytes(b"not a table row")
        .expect("a published unrelated leaf");

    let folder = crate::holder::local::Folder::new(&lake).expect("the lake folder");
    assert_eq!(folder.row_size().expect("metadata row count"), 3);
    assert_eq!(folder.column_size().expect("metadata field width"), 1);

    let _ = std::fs::remove_dir_all(&path);
}
