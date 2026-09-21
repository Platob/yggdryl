//! `rust/src/iceberg/manifest.rs`: what the manifest readers refuse.
//!
//! A caller writes a manifest and reads it back, so the only way to pin what
//! the readers refuse is to write a manifest they should refuse - the entry
//! schema and the one row built exactly as the writer would build them, then
//! bent. That building is reached through `yggdryl::internals`; the round trip
//! a caller can observe is pinned in `rust/tests/iceberg/mod_.rs`.

use std::sync::atomic::{AtomicUsize, Ordering};

use iceberg_official::spec::{
    FormatVersion as OfficialFormatVersion, Manifest as OfficialManifest,
    ManifestContentType as OfficialManifestContent, ManifestList as OfficialManifestList,
};

use yggdryl::holder::Buffer;
use yggdryl::iceberg::{
    DataFile, EntryStatus, FieldSummary, FormatVersion, ManifestContent, ManifestEntry,
    ManifestFile, PartitionSpec, Transform, assign_field_ids, read_manifest,
    read_manifest_for_plan, read_manifest_list, read_manifest_spec, schema_from_json,
    schema_into_json, write_manifest, write_manifest_list,
};
use yggdryl::internals::iceberg_manifest::{
    avro_map, contains_fixed_uuid, entry_to_value, fixed_uuid_official_reader_view,
    manifest_entry_schema,
};
use yggdryl::{DataType, Error, Field, IOBase, MimeType, Result, Scalar, StructType};

/// A cursor over an Avro object container's own header.
///
/// The header is four bytes of magic, a map of metadata written in blocks, and
/// a sixteen-byte synchronization marker. Two fixtures below have to find
/// where one metadata value starts, so they walk the two primitives that spell
/// it - a zigzag variable-length integer and a length-prefixed byte string -
/// rather than borrowing the reader they are bending.
struct AvroHeader<'bytes> {
    /// The container's bytes.
    bytes: &'bytes [u8],
    /// The next byte to read.
    position: usize,
}

impl<'bytes> AvroHeader<'bytes> {
    /// Start immediately after the container magic.
    fn new(bytes: &'bytes [u8]) -> Self {
        assert_eq!(&bytes[..4], b"Obj\x01", "an Avro object container");
        Self { bytes, position: 4 }
    }

    /// Read one zigzag variable-length integer.
    fn long(&mut self) -> i64 {
        let mut shift = 0_u32;
        let mut accumulated = 0_u64;
        loop {
            let byte = self.bytes[self.position];
            self.position += 1;
            accumulated |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                break;
            }
            shift += 7;
        }
        let magnitude = i64::try_from(accumulated >> 1).unwrap();
        magnitude ^ -i64::try_from(accumulated & 1).unwrap()
    }

    /// Read one length-prefixed byte string, answering where it starts.
    fn bytes(&mut self) -> (usize, &'bytes [u8]) {
        let length = usize::try_from(self.long()).unwrap();
        let start = self.position;
        self.position += length;
        (start, &self.bytes[start..self.position])
    }

    /// Walk every metadata entry, calling `visit` with each key and where its
    /// value's bytes begin.
    fn walk(&mut self, mut visit: impl FnMut(&str, usize, &[u8])) {
        loop {
            let count = self.long();
            if count == 0 {
                break;
            }
            if count < 0 {
                self.long();
            }
            for _ in 0..count.unsigned_abs() {
                let (_, key) = self.bytes();
                let key = std::str::from_utf8(key).unwrap();
                let (start, value) = self.bytes();
                visit(key, start, value);
            }
        }
    }
}

struct OversizedHandle {
    handle: Buffer,
    declared_size: u64,
    reads: AtomicUsize,
}

struct CountingHandle {
    handle: Buffer,
    bytes_read: AtomicUsize,
}

impl yggdryl::IOMedia for OversizedHandle {
    yggdryl::impl_default_iomedia!();
}

/// A handle holding `declared_size` bytes of filler, counting every
/// byte read out of it.
impl IOBase for OversizedHandle {
    yggdryl::delegate_iobase!(handle: pwrite, capacity, reserve, truncate, url,
        media_type, set_media_type);

    fn size(&self) -> u64 {
        self.declared_size
    }

    fn pread(&self, offset: u64, buffer: &mut [u8]) -> Result<usize> {
        let available = usize::try_from(self.declared_size.saturating_sub(offset))
            .unwrap_or(usize::MAX)
            .min(buffer.len());
        buffer[..available].fill(0xAA);
        self.reads.fetch_add(available, Ordering::Relaxed);
        Ok(available)
    }
}

impl yggdryl::IOMedia for CountingHandle {
    yggdryl::impl_default_iomedia!();
}

impl IOBase for CountingHandle {
    yggdryl::delegate_iobase!(handle: pwrite, size, capacity, reserve, truncate, url,
        media_type, set_media_type);

    fn pread(&self, offset: u64, buffer: &mut [u8]) -> Result<usize> {
        let read = self.handle.pread(offset, buffer)?;
        self.bytes_read.fetch_add(read, Ordering::Relaxed);
        Ok(read)
    }
}

fn field() -> Field {
    let mut field = StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("venue"),
    ])
    .map(DataType::from)
    .unwrap()
    .required_field("row");
    assign_field_ids(&mut field, 1).unwrap();
    field.insert_metadata("ICEBERG:schema-id", "0").unwrap();
    field
}

fn official_version(version: FormatVersion) -> OfficialFormatVersion {
    match version {
        FormatVersion::V1 => OfficialFormatVersion::V1,
        FormatVersion::V2 => OfficialFormatVersion::V2,
        FormatVersion::V3 => OfficialFormatVersion::V3,
        // `FormatVersion` is `#[non_exhaustive]`, so a caller - which is what
        // this suite is - spells the arm a new revision would land in.
        other => panic!("no official format version for {other:?}"),
    }
}

fn manifest(sequence_number: i64) -> ManifestFile {
    ManifestFile {
        manifest_path: "s3://warehouse/table/metadata/manifest.avro".into(),
        manifest_length: 1,
        partition_spec_id: 0,
        content: ManifestContent::Data,
        sequence_number,
        min_sequence_number: sequence_number,
        added_snapshot_id: 41,
        added_files_count: Some(1),
        existing_files_count: Some(0),
        deleted_files_count: Some(0),
        added_rows_count: Some(1),
        existing_rows_count: Some(0),
        deleted_rows_count: Some(0),
        partitions: Vec::new(),
        key_metadata: None,
        first_row_id: None,
    }
}

fn entry(status: EntryStatus) -> ManifestEntry {
    ManifestEntry {
        status,
        snapshot_id: None,
        sequence_number: None,
        file_sequence_number: None,
        data_file: DataFile {
            file_path: "s3://warehouse/table/data/part.parquet".into(),
            record_count: 1,
            file_size_in_bytes: 1,
            ..DataFile::default()
        },
    }
}

fn replace_data_file_schema_field(
    schema: &Scalar,
    name: &str,
    replacement_type: Option<Scalar>,
) -> Scalar {
    let fields = schema.get_key_str("fields").unwrap().as_sequence().unwrap();
    let fields = fields.iter().map(|field| {
        if field.get_key_str("name").and_then(Scalar::as_str) != Some("data_file") {
            return field.clone();
        }
        let dtype = field.get_key_str("type").unwrap();
        let data_fields = dtype.get_key_str("fields").unwrap().as_sequence().unwrap();
        let data_fields = data_fields.iter().filter_map(|child| {
            if child.get_key_str("name").and_then(Scalar::as_str) != Some(name) {
                return Some(child.clone());
            }
            replacement_type
                .as_ref()
                .map(|kind| child.with_key("type", kind.clone()).unwrap())
        });
        let dtype = dtype
            .with_key("fields", Scalar::from_sequence(data_fields))
            .unwrap();
        field.with_key("type", dtype).unwrap()
    });
    schema
        .with_key("fields", Scalar::from_sequence(fields))
        .unwrap()
}

fn with_data_file_value(row: &Scalar, name: &str, value: Option<Scalar>) -> Scalar {
    let data_file = row.get_key_str("data_file").unwrap();
    let data_file = value.map_or_else(
        || data_file.without_key(name).unwrap(),
        |value| data_file.with_key(name, value).unwrap(),
    );
    row.with_key("data_file", data_file).unwrap()
}

fn write_nonconforming_manifest(
    handle: &mut Buffer,
    avro_schema: &Scalar,
    field: &Field,
    spec: &PartitionSpec,
    row: Scalar,
) {
    let schema_json = schema_into_json(field).unwrap();
    let schema_text = String::from_utf8(yggdryl::json::into_bytes(&schema_json).unwrap()).unwrap();
    let spec_text = String::from_utf8(
        yggdryl::json::into_bytes(&spec.clone().into_v1_json().unwrap()).unwrap(),
    )
    .unwrap();
    let spec_id = spec.spec_id.to_string();
    let metadata = [
        ("schema", schema_text.as_str()),
        ("schema-id", "0"),
        ("partition-spec", spec_text.as_str()),
        ("partition-spec-id", spec_id.as_str()),
        ("format-version", "2"),
        ("content", "data"),
    ];
    yggdryl::avro::write_container(handle, avro_schema, &metadata, &[row]).unwrap();
}

fn manifest_header_parts(
    field: &Field,
    spec: &PartitionSpec,
    version: FormatVersion,
) -> (Scalar, String, String) {
    let partition = spec.partition_field(field).unwrap();
    let avro_schema = manifest_entry_schema(version, &partition).unwrap();
    let schema = schema_into_json(field).unwrap();
    let schema = yggdryl::json::into_utf8(&schema).unwrap();
    let partition_spec = spec.clone().into_v1_json().unwrap();
    let partition_spec = yggdryl::json::into_utf8(&partition_spec).unwrap();
    (avro_schema, schema, partition_spec)
}

fn with_invalid_utf8_metadata(handle: &Buffer, key: &str) -> Buffer {
    let mut bytes = handle.as_slice().to_vec();
    let position = {
        let mut position = None;
        AvroHeader::new(&bytes).walk(|candidate, start, value| {
            if candidate == key {
                assert!(!value.is_empty(), "metadata value must not be empty");
                assert!(position.is_none(), "metadata key must be unique");
                position = Some(start);
            }
        });
        position.expect("metadata key is present")
    };
    bytes[position] = 0xff;
    Buffer::from(bytes)
}

fn avro_header_end(bytes: &[u8]) -> usize {
    /// Length of the synchronization marker that closes the header.
    const SYNC_LEN: usize = 16;

    let mut header = AvroHeader::new(bytes);
    header.walk(|_, _, _| {});
    header.position + SYNC_LEN
}

fn manifest_with_data_file_value(name: &str, value: Scalar) -> Buffer {
    let field = field();
    let spec = PartitionSpec::identity(3, &field, &["venue"]).unwrap();
    let partition = spec.partition_field(&field).unwrap();
    let schema = manifest_entry_schema(FormatVersion::V2, &partition).unwrap();
    let input = ManifestEntry::added(
        41,
        DataFile {
            file_path: "s3://warehouse/table/data/part.parquet".into(),
            partition: vec![Scalar::from("XNAS")],
            record_count: 7,
            file_size_in_bytes: 128,
            ..DataFile::default()
        },
    );
    let row = with_data_file_value(
        &entry_to_value(&input, FormatVersion::V2, &partition).unwrap(),
        name,
        Some(value),
    );
    let mut handle = Buffer::new();
    write_nonconforming_manifest(&mut handle, &schema, &field, &spec, row);
    handle
}

fn metric_pairs(entries: impl IntoIterator<Item = (i64, Scalar)>) -> Scalar {
    Scalar::from_sequence(entries.into_iter().map(|(id, value)| {
        Scalar::from_mapping([
            (Scalar::from("key"), Scalar::from(id)),
            (Scalar::from("value"), value),
        ])
        .unwrap()
    }))
}

fn assert_preflight_error(handle: &Buffer, expected: &str) {
    let error = read_manifest(handle).unwrap_err();
    let Error::Codec { format, reason, .. } = error else {
        panic!("expected a raw Iceberg preflight error, got {error}");
    };
    assert_eq!(format, "iceberg");
    assert!(reason.contains(expected), "{reason}");
}

#[test]
fn manifest_entry_inheritance_matches_official_rules() {
    let current = manifest(7);
    let mut added = entry(EntryStatus::Added);
    added.inherit(&current).unwrap();
    assert_eq!(added.snapshot_id, Some(41));
    assert_eq!(added.sequence_number, Some(7));
    assert_eq!(added.file_sequence_number, Some(7));

    let mut existing = entry(EntryStatus::Existing);
    existing.sequence_number = Some(3);
    existing.file_sequence_number = Some(4);
    existing.inherit(&current).unwrap();
    assert_eq!(existing.snapshot_id, Some(41));
    assert_eq!(existing.sequence_number, Some(3));
    assert_eq!(existing.file_sequence_number, Some(4));

    for status in [EntryStatus::Existing, EntryStatus::Deleted] {
        let mut initial = entry(status);
        initial.inherit(&manifest(0)).unwrap();
        assert_eq!(initial.snapshot_id, Some(41));
        assert_eq!(initial.sequence_number, Some(0));
        assert_eq!(initial.file_sequence_number, Some(0));

        let mut missing = entry(status);
        assert!(missing.inherit(&current).is_err());
        assert_eq!(missing.snapshot_id, Some(41));
    }
}

#[test]
fn manifest_rejects_an_oversized_container_where_the_limit_is_crossed() {
    let limit = yggdryl::Limits::default().max_input_bytes();
    let handle = OversizedHandle {
        handle: Buffer::new(),
        declared_size: u64::try_from(limit).unwrap() + 1,
        reads: AtomicUsize::new(0),
    };

    // The container is refused as soon as more than the limit has
    // arrived - the one read stops there rather than draining it - and
    // its size was never asked for first, because on a store that is a
    // request of its own.
    let error = read_manifest(&handle).unwrap_err();
    assert!(error.to_string().contains("at most"));
    assert!(error.to_string().contains(&(limit + 1).to_string()));
    let read = handle.reads.load(Ordering::Relaxed);
    assert!(read > limit, "the limit was crossed: {read}");
    assert!(
        read <= limit + yggdryl::DEFAULT_STREAM_BATCH_SIZE,
        "the read stopped at the limit: {read}"
    );
}

#[test]
fn manifest_spec_reads_only_the_bounded_avro_header() {
    let field = field();
    let spec = PartitionSpec::identity(3, &field, &["venue"]).unwrap();
    let entry = ManifestEntry::added(
        41,
        DataFile {
            file_path: "s3://warehouse/table/data/part.parquet".into(),
            partition: vec![Scalar::from("XNAS")],
            record_count: 1,
            file_size_in_bytes: 1,
            ..DataFile::default()
        },
    );
    let mut buffer = Buffer::new();
    let entries = (0..4_096_u64)
        .map(|index| {
            let mut entry = entry.clone();
            let mut state = index.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut key = Vec::with_capacity(32);
            for _ in 0..4 {
                state ^= state >> 12;
                state ^= state << 25;
                state ^= state >> 27;
                state = state.wrapping_mul(0x2545_f491_4f6c_dd1d);
                key.extend_from_slice(&state.to_le_bytes());
            }
            entry.data_file.key_metadata = Some(key);
            entry
        })
        .collect::<Vec<_>>();
    write_manifest(&mut buffer, FormatVersion::V2, &field, &spec, &entries).unwrap();
    assert!(buffer.size() > 64 * 1024);
    let handle = CountingHandle {
        handle: buffer,
        bytes_read: AtomicUsize::new(0),
    };

    assert_eq!(read_manifest_spec(&handle).unwrap(), spec);
    let bytes_read = handle.bytes_read.load(Ordering::Relaxed);
    assert!(
        bytes_read < 72 * 1024,
        "header read used {bytes_read} bytes"
    );
    assert!(u64::try_from(bytes_read).unwrap() < handle.size());
}

#[test]
fn manifest_spec_ignores_truncated_and_corrupt_bodies() {
    let field = field();
    let spec = PartitionSpec::identity(3, &field, &["venue"]).unwrap();
    let entry = ManifestEntry::added(
        41,
        DataFile {
            file_path: "s3://warehouse/table/data/part.parquet".into(),
            partition: vec![Scalar::from("XNAS")],
            record_count: 1,
            file_size_in_bytes: 1,
            ..DataFile::default()
        },
    );
    let mut encoded = Buffer::new();
    write_manifest(&mut encoded, FormatVersion::V2, &field, &spec, &[entry]).unwrap();
    let header_end = avro_header_end(encoded.as_slice());
    assert!(header_end < encoded.as_slice().len());

    let truncated = Buffer::from(encoded.as_slice()[..=header_end].to_vec());
    assert_eq!(read_manifest_spec(&truncated).unwrap(), spec);
    assert!(read_manifest(&truncated).is_err());

    let mut corrupt = encoded.as_slice().to_vec();
    *corrupt.last_mut().unwrap() ^= 0xff;
    let corrupt = Buffer::from(corrupt);
    assert_eq!(read_manifest_spec(&corrupt).unwrap(), spec);
    assert!(read_manifest(&corrupt).is_err());
}

#[test]
fn manifest_spec_rejects_missing_and_malformed_required_metadata() {
    let field = field();
    let spec = PartitionSpec::identity(3, &field, &["venue"]).unwrap();
    let (avro_schema, schema, partition_spec) =
        manifest_header_parts(&field, &spec, FormatVersion::V1);

    let mut missing_schema = Buffer::new();
    yggdryl::avro::write_container(
        &mut missing_schema,
        &avro_schema,
        &[("partition-spec", partition_spec.as_str())],
        &[],
    )
    .unwrap();
    let message = read_manifest_spec(&missing_schema).unwrap_err().to_string();
    assert!(message.contains("schema is required"), "{message}");

    let mut missing_spec = Buffer::new();
    yggdryl::avro::write_container(
        &mut missing_spec,
        &avro_schema,
        &[("schema", schema.as_str())],
        &[],
    )
    .unwrap();
    let message = read_manifest_spec(&missing_spec).unwrap_err().to_string();
    assert!(message.contains("partition-spec is required"), "{message}");

    let mut malformed_schema = Buffer::new();
    yggdryl::avro::write_container(
        &mut malformed_schema,
        &avro_schema,
        &[("schema", "{"), ("partition-spec", partition_spec.as_str())],
        &[],
    )
    .unwrap();
    let message = read_manifest_spec(&malformed_schema)
        .unwrap_err()
        .to_string();
    assert!(message.contains("parse schema"), "{message}");

    let mut malformed_spec = Buffer::new();
    yggdryl::avro::write_container(
        &mut malformed_spec,
        &avro_schema,
        &[("schema", schema.as_str()), ("partition-spec", "{")],
        &[],
    )
    .unwrap();
    let message = read_manifest_spec(&malformed_spec).unwrap_err().to_string();
    assert!(message.contains("parse partition spec"), "{message}");
}

#[test]
fn manifest_spec_rejects_invalid_utf8_required_metadata() {
    let field = field();
    let spec = PartitionSpec::identity(3, &field, &["venue"]).unwrap();
    let (avro_schema, schema, partition_spec) =
        manifest_header_parts(&field, &spec, FormatVersion::V1);
    let mut encoded = Buffer::new();
    yggdryl::avro::write_container(
        &mut encoded,
        &avro_schema,
        &[
            ("schema", schema.as_str()),
            ("partition-spec", partition_spec.as_str()),
        ],
        &[],
    )
    .unwrap();

    for key in ["schema", "partition-spec"] {
        let malformed = with_invalid_utf8_metadata(&encoded, key);
        let message = read_manifest_spec(&malformed).unwrap_err().to_string();
        assert!(message.contains("parse"), "{key}: {message}");
    }
}

#[test]
fn manifest_header_defaults_preserve_official_v1_semantics() {
    let field = field();
    let spec = PartitionSpec::identity(3, &field, &["venue"]).unwrap();
    let (avro_schema, schema, partition_spec) =
        manifest_header_parts(&field, &spec, FormatVersion::V1);
    let partition = spec.partition_field(&field).unwrap();
    let entry = ManifestEntry::added(
        41,
        DataFile {
            file_path: "s3://warehouse/table/data/part.parquet".into(),
            partition: vec![Scalar::from("XNAS")],
            record_count: 1,
            file_size_in_bytes: 1,
            ..DataFile::default()
        },
    );
    let row = entry_to_value(&entry, FormatVersion::V1, &partition).unwrap();
    let mut encoded = Buffer::new();
    yggdryl::avro::write_container(
        &mut encoded,
        &avro_schema,
        &[
            ("schema", schema.as_str()),
            ("partition-spec", partition_spec.as_str()),
        ],
        &[row],
    )
    .unwrap();

    let official = OfficialManifest::parse_avro(encoded.as_slice()).unwrap();
    assert_eq!(official.metadata().schema_id(), 0);
    assert_eq!(official.metadata().partition_spec().spec_id(), 0);
    assert_eq!(
        official.metadata().format_version(),
        &OfficialFormatVersion::V1
    );
    assert_eq!(
        official.metadata().content(),
        &OfficialManifestContent::Data
    );

    let mut expected_spec = spec;
    expected_spec.spec_id = 0;
    assert_eq!(read_manifest_spec(&encoded).unwrap(), expected_spec);
    let decoded = read_manifest(&encoded).unwrap();
    assert_eq!(decoded.len(), 1);
    assert_eq!(decoded[0].snapshot_id, Some(41));
    assert_eq!(decoded[0].sequence_number, Some(0));
    assert_eq!(decoded[0].file_sequence_number, Some(0));
    assert_eq!(decoded[0].data_file.content, 0);
}

#[test]
fn planning_reader_rejects_missing_required_fields() {
    let field = field();
    let spec = PartitionSpec::identity(3, &field, &["venue"]).unwrap();
    let partition = spec.partition_field(&field).unwrap();
    let schema = replace_data_file_schema_field(
        &manifest_entry_schema(FormatVersion::V2, &partition).unwrap(),
        "record_count",
        None,
    );
    let input = ManifestEntry::added(
        41,
        DataFile {
            file_path: "s3://warehouse/table/data/part.parquet".into(),
            partition: vec![Scalar::from("XNAS")],
            record_count: 7,
            file_size_in_bytes: 128,
            ..DataFile::default()
        },
    );
    let row = with_data_file_value(
        &entry_to_value(&input, FormatVersion::V2, &partition).unwrap(),
        "record_count",
        None,
    );
    let mut handle = Buffer::new();
    write_nonconforming_manifest(&mut handle, &schema, &field, &spec, row);

    assert!(
        fixed_uuid_official_reader_view(handle.as_slice())
            .unwrap()
            .is_none()
    );
    assert!(read_manifest(&handle).is_err());
    assert!(read_manifest_for_plan(&handle, false).is_err());
}

#[test]
fn planning_reader_rejects_malformed_statistics_members() {
    let field = field();
    let spec = PartitionSpec::identity(3, &field, &["venue"]).unwrap();
    let partition = spec.partition_field(&field).unwrap();
    let string_counts =
        Scalar::from_sequence([Scalar::from("null"), avro_map(119, 120, "string").unwrap()]);
    let schema = replace_data_file_schema_field(
        &manifest_entry_schema(FormatVersion::V2, &partition).unwrap(),
        "value_counts",
        Some(string_counts),
    );
    let input = ManifestEntry::added(
        41,
        DataFile {
            file_path: "s3://warehouse/table/data/part.parquet".into(),
            partition: vec![Scalar::from("XNAS")],
            record_count: 7,
            file_size_in_bytes: 128,
            ..DataFile::default()
        },
    );
    let malformed_counts = Scalar::from_sequence([Scalar::from_mapping([
        (Scalar::from("key"), Scalar::from(1_i64)),
        (Scalar::from("value"), Scalar::from("seven")),
    ])
    .unwrap()]);
    let row = with_data_file_value(
        &entry_to_value(&input, FormatVersion::V2, &partition).unwrap(),
        "value_counts",
        Some(malformed_counts),
    );
    let mut handle = Buffer::new();
    write_nonconforming_manifest(&mut handle, &schema, &field, &spec, row);

    assert!(read_manifest(&handle).is_err());
    assert!(read_manifest_for_plan(&handle, true).is_err());
}

#[test]
fn raw_manifest_preflight_rejects_duplicate_metric_ids_before_official_maps() {
    for name in [
        "column_sizes",
        "value_counts",
        "null_value_counts",
        "nan_value_counts",
        "lower_bounds",
        "upper_bounds",
    ] {
        let value = if matches!(name, "lower_bounds" | "upper_bounds") {
            Scalar::from(1_i64.to_le_bytes().to_vec())
        } else {
            Scalar::from(1_i64)
        };
        let handle =
            manifest_with_data_file_value(name, metric_pairs([(1, value.clone()), (1, value)]));
        assert!(
            OfficialManifest::parse_avro(handle.as_slice()).is_ok(),
            "official map conversion should demonstrate the lossy duplicate case for {name}"
        );
        assert_preflight_error(&handle, &format!("unique field ids in {name}"));
    }
}

#[test]
fn raw_manifest_preflight_rejects_negative_metric_counts() {
    for name in [
        "column_sizes",
        "value_counts",
        "null_value_counts",
        "nan_value_counts",
    ] {
        let handle = manifest_with_data_file_value(name, metric_pairs([(1, Scalar::from(-1_i64))]));
        assert_preflight_error(&handle, &format!("non-negative count in {name}"));
    }
}

#[test]
fn raw_manifest_preflight_rejects_invalid_split_offsets() {
    for offsets in [vec![-1_i64], vec![4, 4], vec![8, 4]] {
        let handle = manifest_with_data_file_value(
            "split_offsets",
            Scalar::from_sequence(offsets.into_iter().map(Scalar::from)),
        );
        assert_preflight_error(&handle, "non-negative strictly ascending split_offsets");
    }
}

#[test]
fn public_manifest_reads_are_official_parser_views() {
    let field = field();
    let spec = PartitionSpec::identity(3, &field, &["venue"]).unwrap();
    for version in [FormatVersion::V1, FormatVersion::V2, FormatVersion::V3] {
        let input = ManifestEntry::added(
            41,
            DataFile {
                file_path: "s3://warehouse/table/data/part.parquet".into(),
                partition: vec![Scalar::from("XNAS")],
                record_count: 7,
                file_size_in_bytes: 512,
                value_counts: vec![(1, 7)],
                null_value_counts: vec![(1, 0)],
                lower_bounds: vec![(1, 1_i64.to_le_bytes().to_vec())],
                upper_bounds: vec![(1, 7_i64.to_le_bytes().to_vec())],
                key_metadata: Some(vec![1, 2, 3]),
                split_offsets: vec![4],
                sort_order_id: Some(0),
                first_row_id: (version == FormatVersion::V3).then_some(70),
                ..DataFile::default()
            },
        );
        let mut handle = Buffer::new();
        write_manifest(
            &mut handle,
            version,
            &field,
            &spec,
            std::slice::from_ref(&input),
        )
        .unwrap();

        let official = OfficialManifest::parse_avro(handle.as_slice()).unwrap();
        assert_eq!(
            *official.metadata().format_version(),
            official_version(version)
        );
        assert_eq!(official.entries().len(), 1);

        let output = read_manifest(&handle).unwrap();
        assert_eq!(output.len(), 1);
        assert_eq!(output[0].status, EntryStatus::Added);
        assert_eq!(output[0].snapshot_id, Some(41));
        assert_eq!(output[0].data_file, input.data_file);
        if version == FormatVersion::V1 {
            assert_eq!(output[0].sequence_number, Some(0));
            assert_eq!(output[0].file_sequence_number, Some(0));
        }
        assert_eq!(read_manifest_spec(&handle).unwrap(), spec);
    }
}

#[test]
fn planning_reader_preserves_nonlexical_partition_spec_order() {
    let mut field = StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().required_field("z"),
        DataType::utf8().required_field("a"),
    ])
    .map(DataType::from)
    .unwrap()
    .required_field("row");
    assign_field_ids(&mut field, 1).unwrap();
    field.insert_metadata("ICEBERG:schema-id", "0").unwrap();
    let spec = PartitionSpec::identity(3, &field, &["z", "a"]).unwrap();
    let input = ManifestEntry::added(
        41,
        DataFile {
            file_path: "s3://warehouse/table/data/part.parquet".into(),
            partition: vec![Scalar::from("Z"), Scalar::from("A")],
            record_count: 1,
            file_size_in_bytes: 128,
            column_sizes: vec![(1, 8)],
            value_counts: vec![(1, 1)],
            null_value_counts: vec![(1, 0)],
            lower_bounds: vec![(1, 1_i64.to_le_bytes().to_vec())],
            upper_bounds: vec![(1, 1_i64.to_le_bytes().to_vec())],
            split_offsets: vec![4],
            ..DataFile::default()
        },
    );
    let mut handle = Buffer::new();
    write_manifest(&mut handle, FormatVersion::V2, &field, &spec, &[input]).unwrap();

    let official_partition = read_manifest(&handle).unwrap()[0]
        .data_file
        .partition
        .clone();
    assert_eq!(
        official_partition,
        vec![Scalar::from("Z"), Scalar::from("A")]
    );
    for with_stats in [false, true] {
        let planned = read_manifest_for_plan(&handle, with_stats).unwrap();
        assert_eq!(planned[0].data_file.partition, official_partition);
        assert!(planned[0].data_file.column_sizes.is_empty());
        assert!(planned[0].data_file.split_offsets.is_empty());
        assert_eq!(
            planned[0].data_file.value_counts,
            if with_stats { vec![(1, 1)] } else { Vec::new() }
        );
    }
}

#[test]
fn official_manifest_reads_preserve_unknown_partition_transforms() {
    let field = field();
    let mut spec = PartitionSpec::identity(3, &field, &["venue"]).unwrap();
    spec.fields[0].name = "venue_opaque".into();
    spec.fields[0].transform = Transform::Unknown;
    let input = ManifestEntry::added(
        41,
        DataFile {
            file_path: "s3://warehouse/table/data/part.parquet".into(),
            partition: vec![Scalar::from("opaque")],
            record_count: 1,
            file_size_in_bytes: 128,
            ..DataFile::default()
        },
    );
    let mut handle = Buffer::new();
    write_manifest(&mut handle, FormatVersion::V2, &field, &spec, &[input]).unwrap();

    assert_eq!(read_manifest_spec(&handle).unwrap(), spec);
    assert_eq!(
        read_manifest(&handle).unwrap()[0].data_file.partition,
        vec![Scalar::from("opaque")]
    );
}

#[test]
fn official_uuid_partition_literals_use_the_exact_uuid_shape() {
    let document = yggdryl::json::from_utf8(
        r#"{"type":"struct","schema-id":0,"fields":[
            {"id":1,"name":"id","required":true,"type":"long"},
            {"id":2,"name":"token","required":true,"type":"uuid"}
        ]}"#,
    )
    .unwrap();
    let field = schema_from_json("row", &document).unwrap();
    let spec = PartitionSpec::identity(4, &field, &["token"]).unwrap();
    let token = 0x0db3_e2a8_9d1d_42b9_aa7b_74eb_e558_dcebu128
        .to_be_bytes()
        .to_vec();
    let expected = DataType::uuid()
        .scalar(Scalar::from(token.clone()))
        .unwrap();
    let input = ManifestEntry::added(
        41,
        DataFile {
            file_path: "s3://warehouse/table/data/part.parquet".into(),
            partition: vec![Scalar::from(token.clone())],
            record_count: 1,
            file_size_in_bytes: 128,
            ..DataFile::default()
        },
    );
    let mut handle = Buffer::new();
    write_manifest(&mut handle, FormatVersion::V2, &field, &spec, &[input]).unwrap();
    let container = yggdryl::avro::read_container(&handle).unwrap();
    assert!(contains_fixed_uuid(&container.schema.into_json()));
    assert_eq!(
        container.rows[0].path("data_file.partition.token"),
        Some(&expected)
    );
    let read = read_manifest(&handle).unwrap();
    assert_eq!(read[0].data_file.partition, vec![expected]);

    let mut rewritten = Buffer::new();
    write_manifest(&mut rewritten, FormatVersion::V2, &field, &spec, &read).unwrap();
    assert_eq!(read_manifest(&rewritten).unwrap(), read);
}

#[test]
fn manifest_writer_rejects_non_iceberg_mime_types_without_writing() {
    let field = field();
    let spec = PartitionSpec::unpartitioned();
    let entry = ManifestEntry::added(
        41,
        DataFile {
            file_path: "s3://warehouse/table/data/part.json".into(),
            mime_type: MimeType::JSON,
            record_count: 1,
            file_size_in_bytes: 2,
            ..DataFile::default()
        },
    );
    let mut handle = Buffer::new();

    let message = write_manifest(&mut handle, FormatVersion::V2, &field, &spec, &[entry])
        .unwrap_err()
        .to_string();

    assert!(message.contains("Iceberg MIME type"), "{message}");
    assert!(message.contains(MimeType::JSON.as_str()), "{message}");
    assert!(handle.as_slice().is_empty());
    assert!(OfficialManifest::parse_avro(handle.as_slice()).is_err());
}

#[test]
fn delete_manifest_metadata_is_official_and_mixed_content_is_refused() {
    let field = field();
    let spec = PartitionSpec::identity(3, &field, &["venue"]).unwrap();
    for version in [FormatVersion::V2, FormatVersion::V3] {
        let deleted = ManifestEntry::added(
            41,
            DataFile {
                content: 2,
                file_path: "s3://warehouse/table/data/delete.parquet".into(),
                partition: vec![Scalar::from("XNAS")],
                record_count: 1,
                file_size_in_bytes: 128,
                equality_ids: Some(vec![1]),
                ..DataFile::default()
            },
        );
        let mut handle = Buffer::new();
        write_manifest(
            &mut handle,
            version,
            &field,
            &spec,
            std::slice::from_ref(&deleted),
        )
        .unwrap();
        let official = OfficialManifest::parse_avro(handle.as_slice()).unwrap();
        assert_eq!(
            official.metadata().content(),
            &OfficialManifestContent::Deletes
        );
        assert_eq!(read_manifest(&handle).unwrap(), vec![deleted.clone()]);

        let data = ManifestEntry::added(
            41,
            DataFile {
                file_path: "s3://warehouse/table/data/part.parquet".into(),
                partition: vec![Scalar::from("XNAS")],
                record_count: 1,
                file_size_in_bytes: 128,
                ..DataFile::default()
            },
        );
        let message = write_manifest(&mut Buffer::new(), version, &field, &spec, &[data, deleted])
            .unwrap_err()
            .to_string();
        assert!(message.contains("mixed data and delete files"), "{message}");
    }
}

#[test]
fn versioned_manifest_fields_are_preserved_or_rejected_before_write() {
    let field = field();
    let spec = PartitionSpec::identity(3, &field, &["venue"]).unwrap();
    let vector = ManifestEntry::added(
        41,
        DataFile {
            content: 1,
            file_path: "s3://warehouse/table/data/deletes.puffin".into(),
            mime_type: MimeType::PUFFIN,
            partition: vec![Scalar::from("XNAS")],
            record_count: 1,
            file_size_in_bytes: 128,
            referenced_data_file: Some("s3://warehouse/table/data/part.parquet".into()),
            content_offset: Some(4),
            content_size_in_bytes: Some(64),
            ..DataFile::default()
        },
    );
    let mut handle = Buffer::new();
    write_manifest(
        &mut handle,
        FormatVersion::V3,
        &field,
        &spec,
        std::slice::from_ref(&vector),
    )
    .unwrap();
    assert_eq!(read_manifest(&handle).unwrap(), vec![vector.clone()]);

    let message = write_manifest(
        &mut Buffer::new(),
        FormatVersion::V2,
        &field,
        &spec,
        &[vector],
    )
    .unwrap_err()
    .to_string();
    assert!(
        message.contains("referenced_data_file") && message.contains("v2"),
        "{message}"
    );

    let v1_delete = ManifestEntry::added(
        41,
        DataFile {
            content: 2,
            file_path: "s3://warehouse/table/data/delete.parquet".into(),
            partition: vec![Scalar::from("XNAS")],
            record_count: 1,
            file_size_in_bytes: 128,
            equality_ids: Some(vec![1]),
            ..DataFile::default()
        },
    );
    let message = write_manifest(
        &mut Buffer::new(),
        FormatVersion::V1,
        &field,
        &spec,
        &[v1_delete],
    )
    .unwrap_err()
    .to_string();
    assert!(
        message.contains("only data files") && message.contains("v1"),
        "{message}"
    );

    let manifest = ManifestFile {
        manifest_path: "s3://warehouse/table/metadata/delete.avro".into(),
        manifest_length: 1,
        partition_spec_id: 0,
        content: ManifestContent::Deletes,
        sequence_number: 1,
        min_sequence_number: 1,
        added_snapshot_id: 41,
        added_files_count: Some(0),
        existing_files_count: Some(0),
        deleted_files_count: Some(0),
        added_rows_count: Some(0),
        existing_rows_count: Some(0),
        deleted_rows_count: Some(0),
        partitions: Vec::new(),
        key_metadata: None,
        first_row_id: None,
    };
    let message = write_manifest_list(
        &mut Buffer::new(),
        FormatVersion::V1,
        41,
        None,
        0,
        None,
        &[manifest],
    )
    .unwrap_err()
    .to_string();
    assert!(message.contains("v1 data manifest"), "{message}");
}

#[test]
fn public_manifest_list_reads_are_official_parser_views() {
    for version in [FormatVersion::V1, FormatVersion::V2, FormatVersion::V3] {
        let input = ManifestFile {
            manifest_path: "s3://warehouse/table/metadata/manifest.avro".into(),
            manifest_length: 1_024,
            partition_spec_id: 3,
            content: ManifestContent::Data,
            sequence_number: if version == FormatVersion::V1 { 0 } else { 5 },
            min_sequence_number: if version == FormatVersion::V1 { 0 } else { 2 },
            added_snapshot_id: 41,
            added_files_count: Some(1),
            existing_files_count: Some(2),
            deleted_files_count: Some(3),
            added_rows_count: Some(7),
            existing_rows_count: Some(11),
            deleted_rows_count: Some(13),
            partitions: vec![FieldSummary {
                contains_null: false,
                contains_nan: Some(false),
                lower_bound: Some(vec![1]),
                upper_bound: Some(vec![9]),
            }],
            key_metadata: Some(vec![3, 1, 4]),
            first_row_id: (version == FormatVersion::V3).then_some(70),
        };
        let mut handle = Buffer::new();
        write_manifest_list(
            &mut handle,
            version,
            41,
            None,
            5,
            (version == FormatVersion::V3).then_some(70),
            std::slice::from_ref(&input),
        )
        .unwrap();

        let official =
            OfficialManifestList::parse_with_version(handle.as_slice(), official_version(version))
                .unwrap();
        assert_eq!(official.entries().len(), 1);
        assert_eq!(read_manifest_list(&handle).unwrap(), vec![input]);

        let mut unknown = read_manifest_list(&handle).unwrap().remove(0);
        unknown.added_files_count = None;
        unknown.existing_files_count = None;
        unknown.deleted_files_count = None;
        unknown.added_rows_count = None;
        unknown.existing_rows_count = None;
        unknown.deleted_rows_count = None;
        let mut handle = Buffer::new();
        if version == FormatVersion::V1 {
            write_manifest_list(&mut handle, version, 41, None, 5, None, &[unknown.clone()])
                .unwrap();
            assert_eq!(read_manifest_list(&handle).unwrap(), vec![unknown]);
        } else {
            let message = write_manifest_list(
                &mut handle,
                version,
                41,
                None,
                5,
                (version == FormatVersion::V3).then_some(70),
                &[unknown],
            )
            .unwrap_err()
            .to_string();
            assert!(
                message.contains("required added_files_count") && message.contains("got null"),
                "{message}"
            );
        }
    }
}

#[test]
fn v3_manifest_lists_assign_contiguous_row_ranges_and_header() {
    let manifest = |name: &str, added: i64, existing: i64| ManifestFile {
        manifest_path: format!("s3://warehouse/table/metadata/{name}").into(),
        manifest_length: 1_024,
        partition_spec_id: 0,
        content: ManifestContent::Data,
        sequence_number: 5,
        min_sequence_number: 5,
        added_snapshot_id: 41,
        added_files_count: Some(1),
        existing_files_count: Some(1),
        deleted_files_count: Some(0),
        added_rows_count: Some(added),
        existing_rows_count: Some(existing),
        deleted_rows_count: Some(0),
        partitions: Vec::new(),
        key_metadata: None,
        first_row_id: None,
    };
    let manifests = [manifest("a.avro", 3, 2), manifest("b.avro", 7, 0)];
    let mut handle = Buffer::new();
    assert_eq!(
        write_manifest_list(
            &mut handle,
            FormatVersion::V3,
            41,
            None,
            5,
            Some(10),
            &manifests,
        )
        .unwrap(),
        Some(22)
    );

    let container = yggdryl::avro::read_container(&handle).unwrap();
    assert_eq!(container.get("first-row-id"), Some("10"));
    let read = read_manifest_list(&handle).unwrap();
    assert_eq!(read[0].first_row_id, Some(10));
    assert_eq!(read[1].first_row_id, Some(15));
}

#[test]
fn v3_row_range_assignment_fails_before_writing() {
    let manifest = ManifestFile {
        manifest_path: "s3://warehouse/table/metadata/a.avro".into(),
        manifest_length: 1,
        partition_spec_id: 0,
        content: ManifestContent::Data,
        sequence_number: 1,
        min_sequence_number: 1,
        added_snapshot_id: 41,
        added_files_count: Some(1),
        existing_files_count: Some(0),
        deleted_files_count: Some(0),
        added_rows_count: None,
        existing_rows_count: Some(0),
        deleted_rows_count: Some(0),
        partitions: Vec::new(),
        key_metadata: None,
        first_row_id: None,
    };
    let mut handle = Buffer::new();
    let message = write_manifest_list(
        &mut handle,
        FormatVersion::V3,
        41,
        None,
        1,
        Some(0),
        &[manifest],
    )
    .unwrap_err()
    .to_string();
    assert!(message.contains("added_rows_count") && message.contains("null"));
    assert!(handle.as_slice().is_empty());
}
