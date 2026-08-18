//! The Avro object container: magic, header, blocks, and sync markers.
//!
//! Two reading shapes share one header: [`read_container`] pulls the whole
//! value and decodes every row, which is the right cost for the small
//! self-describing files Avro is usually asked for, and [`read_blocks`]
//! iterates compressed blocks over any [`IOBase`] handle using nothing beyond
//! `pread`, so a large container can be streamed - and whole blocks skipped -
//! without ever holding it in memory.

use std::io::Read;

use smol_str::{SmolStr, format_smolstr};

use crate::io::IOBase;
use crate::{Codec, Level, Limits, Result, Value};

use super::datum::{Cursor, DatumCodec, block_count, codec, invalid, put_bytes, put_long};
use super::resolve::Resolution;
use super::schema::Schema;

/// The four bytes that open every Avro object container.
pub(crate) const MAGIC: [u8; 4] = [b'O', b'b', b'j', 1];

/// The header key naming the writer schema.
pub(crate) const SCHEMA_KEY: &str = "avro.schema";

/// The header key naming the block compression codec.
pub(crate) const CODEC_KEY: &str = "avro.codec";

/// Length of the synchronization marker that closes every block.
pub(crate) const SYNC_LEN: usize = 16;

/// One decoded Avro object container.
#[derive(Debug)]
pub struct Container {
    /// The writer schema the header named.
    pub schema: Schema,
    /// The header's key/value metadata, minus the reserved schema and codec.
    pub metadata: Vec<(SmolStr, SmolStr)>,
    /// Every decoded row, in file order.
    pub rows: Vec<Value>,
}

impl Container {
    /// Return one metadata value by key.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.metadata
            .iter()
            .find_map(|(name, value)| (name == key).then(|| value.as_str()))
    }
}

/// The block compression a container header names.
#[derive(Clone, Copy, Debug)]
pub(crate) enum BlockCoding {
    /// A coding the shared [`Codec`] vocabulary implements.
    Shared(Codec),
    /// Raw Snappy followed by a big-endian CRC-32 of the uncompressed block.
    #[cfg(feature = "parquet")]
    Snappy,
}

impl BlockCoding {
    /// Return the coding one Avro codec name selects.
    pub(crate) fn from_name(name: &str) -> Result<Self> {
        match name {
            "null" => Ok(Self::Shared(Codec::Identity)),
            // Avro's "deflate" is the raw stream, with no zlib wrapper.
            "deflate" => Ok(Self::Shared(Codec::Deflate)),
            "zstandard" => Ok(Self::Shared(Codec::Zstd)),
            #[cfg(feature = "parquet")]
            "snappy" => Ok(Self::Snappy),
            other => Err(invalid(format_smolstr!(
                "expected an Avro block codec this build implements ({}), got {other:?}",
                implemented_codecs()
            ))),
        }
    }

    /// Decode one block payload, bounded by the limits.
    pub(crate) fn load(self, payload: &[u8], limits: Limits) -> Result<Vec<u8>> {
        let mut decoded = Vec::new();
        self.load_into(payload, limits, &mut decoded)?;
        Ok(decoded)
    }

    /// Decode one block payload into a reused buffer, bounded by the limits.
    ///
    /// The buffer is cleared but keeps its capacity, so a reader iterating
    /// many blocks - a scan over many manifests - pays one allocation rather
    /// than one per block.
    pub(crate) fn load_into(
        self,
        payload: &[u8],
        limits: Limits,
        decoded: &mut Vec<u8>,
    ) -> Result<()> {
        decoded.clear();
        let bound = limits.max_input_bytes();
        match self {
            Self::Shared(Codec::Identity) => {
                decoded.extend_from_slice(payload);
                Ok(())
            }
            Self::Shared(shared) => {
                // Stream through the codec with a hard ceiling, so a small
                // compressed block cannot decompress the process to death.
                let mut reader = shared.reader(payload).take(bound as u64 + 1);
                reader.read_to_end(decoded).map_err(|error| {
                    invalid(format_smolstr!(
                        "expected a valid {} block, got {error}",
                        self.name()
                    ))
                })?;
                if decoded.len() > bound {
                    return Err(invalid(format_smolstr!(
                        "expected a block of at most {bound} decoded bytes"
                    )));
                }
                Ok(())
            }
            #[cfg(feature = "parquet")]
            Self::Snappy => {
                if payload.len() < 4 {
                    return Err(invalid(format_smolstr!(
                        "expected a snappy block carrying a four-byte CRC-32, got {} bytes",
                        payload.len()
                    )));
                }
                let (body, tail) = payload.split_at(payload.len() - 4);
                let declared = u32::from_be_bytes(tail.try_into().unwrap_or_default());
                let length = snap::raw::decompress_len(body).map_err(|error| {
                    invalid(format_smolstr!(
                        "expected a valid snappy block, got {error}"
                    ))
                })?;
                if length > bound {
                    return Err(invalid(format_smolstr!(
                        "expected a block of at most {bound} decoded bytes, got {length}"
                    )));
                }
                decoded.resize(length, 0);
                snap::raw::Decoder::new()
                    .decompress(body, decoded)
                    .map_err(|error| {
                        invalid(format_smolstr!(
                            "expected a valid snappy block, got {error}"
                        ))
                    })?;
                let mut crc = flate2::Crc::new();
                crc.update(decoded);
                if crc.sum() != declared {
                    return Err(invalid(format_smolstr!(
                        "expected a snappy CRC-32 of {:08x}, got {declared:08x}",
                        crc.sum()
                    )));
                }
                Ok(())
            }
        }
    }

    /// Encode one block payload.
    pub(crate) fn dump(self, payload: &[u8], level: Level) -> Result<Vec<u8>> {
        match self {
            Self::Shared(shared) => shared.dump_with_level(payload, level),
            #[cfg(feature = "parquet")]
            Self::Snappy => {
                let mut encoded =
                    snap::raw::Encoder::new()
                        .compress_vec(payload)
                        .map_err(|error| {
                            invalid(format_smolstr!("expected snappy to encode, got {error}"))
                        })?;
                let mut crc = flate2::Crc::new();
                crc.update(payload);
                encoded.extend_from_slice(&crc.sum().to_be_bytes());
                Ok(encoded)
            }
        }
    }

    /// Return the Avro codec name for this coding.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Shared(Codec::Identity) => "null",
            Self::Shared(Codec::Zstd) => "zstandard",
            Self::Shared(_) => "deflate",
            #[cfg(feature = "parquet")]
            Self::Snappy => "snappy",
        }
    }
}

/// List the codec names this build implements.
fn implemented_codecs() -> &'static str {
    if cfg!(feature = "parquet") {
        "null, deflate, snappy, zstandard"
    } else {
        "null, deflate, zstandard"
    }
}

/// The parsed container header.
pub(crate) struct Header {
    /// The writer schema.
    pub(crate) schema: Schema,
    /// Metadata minus the reserved keys.
    pub(crate) metadata: Vec<(SmolStr, SmolStr)>,
    /// The block compression.
    pub(crate) coding: BlockCoding,
    /// The marker that closes every block.
    pub(crate) sync: [u8; SYNC_LEN],
}

/// The raw header entries, still as bytes.
pub(crate) type HeaderEntries = Vec<(SmolStr, Vec<u8>)>;

/// Parse the raw header entries and the sync marker after the magic.
pub(crate) fn parse_header_entries(
    cursor: &mut Cursor<'_>,
    limits: Limits,
) -> Result<(HeaderEntries, [u8; SYNC_LEN])> {
    let mut entries = Vec::new();
    loop {
        let (count, _) = block_count(cursor)?;
        if count == 0 {
            break;
        }
        for _ in 0..count {
            if entries.len() >= limits.max_nodes() {
                return Err(invalid(format_smolstr!(
                    "expected at most {} header entries",
                    limits.max_nodes()
                )));
            }
            let key = SmolStr::new(std::str::from_utf8(cursor.bytes()?).map_err(|error| {
                codec(
                    cursor.position,
                    format_smolstr!("expected UTF-8 in an Avro header key, got {error}"),
                )
            })?);
            let value = cursor.bytes()?.to_vec();
            entries.push((key, value));
        }
    }
    let sync: [u8; SYNC_LEN] = cursor.take(SYNC_LEN)?.try_into().map_err(|_| {
        codec(
            cursor.position,
            SmolStr::new_static("expected a sixteen-byte Avro synchronization marker"),
        )
    })?;
    Ok((entries, sync))
}

/// Return one raw header entry by key.
pub(crate) fn header_entry<'entries>(
    entries: &'entries [(SmolStr, Vec<u8>)],
    key: &str,
) -> Option<&'entries [u8]> {
    entries
        .iter()
        .find_map(|(name, value)| (name == key).then_some(value.as_slice()))
}

/// Parse the header entries after the magic.
pub(crate) fn parse_header(cursor: &mut Cursor<'_>, limits: Limits) -> Result<Header> {
    let (entries, sync) = parse_header_entries(cursor, limits)?;
    let schema_bytes = header_entry(&entries, SCHEMA_KEY).ok_or_else(|| {
        invalid(format_smolstr!(
            "expected an Avro header carrying {SCHEMA_KEY:?}"
        ))
    })?;
    let schema_json = crate::json::from_slice_with_limits(schema_bytes, limits)?;
    let schema = Schema::from_json_with_limits(&schema_json, limits)?;
    let coding = match header_entry(&entries, CODEC_KEY) {
        Some(value) => BlockCoding::from_name(&String::from_utf8_lossy(value))?,
        None => BlockCoding::Shared(Codec::Identity),
    };

    let metadata = entries
        .into_iter()
        .filter(|(key, _)| key != SCHEMA_KEY && key != CODEC_KEY)
        .map(|(key, value)| (key, SmolStr::new(String::from_utf8_lossy(&value))))
        .collect();

    Ok(Header {
        schema,
        metadata,
        coding,
        sync,
    })
}

/// Check the opening magic.
pub(crate) fn check_magic(magic: &[u8]) -> Result<()> {
    if magic != MAGIC {
        return Err(invalid(format_smolstr!(
            "expected an Avro object container starting with {MAGIC:?}, got {magic:?}"
        )));
    }
    Ok(())
}

/// Read every row of the Avro object container a handle holds.
///
/// The container is read whole, which is the right cost for a small
/// self-describing file - an Iceberg manifest describes files rather than
/// rows, so it is small by construction. [`read_blocks`] is the streaming
/// shape for containers that are not.
///
/// # Errors
///
/// Returns an error when the bytes are not an Avro object container, when the
/// codec is one this build does not implement, or when a row does not decode.
pub fn read_container<H: IOBase + ?Sized>(handle: &H) -> Result<Container> {
    read_container_with_limits(handle, Limits::default())
}

/// Read every row of the Avro object container a handle holds, bounded by
/// explicit limits.
///
/// # Errors
///
/// Returns an error when the bytes are not an Avro object container or exceed
/// the limits.
pub fn read_container_with_limits<H: IOBase + ?Sized>(
    handle: &H,
    limits: Limits,
) -> Result<Container> {
    decode_container(handle, limits, None)
}

/// Read a container, resolving every row from the writer schema it was
/// written with onto the reader schema the caller wants.
///
/// The resolution plan is compiled once for the container and executed per
/// row; extra writer fields are skipped without being decoded, missing reader
/// fields fill from their defaults, and legal promotions widen in place.
///
/// # Errors
///
/// Returns an error when the container is malformed or the schemas do not
/// resolve.
pub fn read_container_resolved<H: IOBase + ?Sized>(
    handle: &H,
    reader: &Schema,
) -> Result<Container> {
    read_container_resolved_with_limits(handle, reader, Limits::default())
}

/// [`read_container_resolved`] with explicit limits.
///
/// # Errors
///
/// Returns an error when the container is malformed, the schemas do not
/// resolve, or a limit is exceeded.
pub fn read_container_resolved_with_limits<H: IOBase + ?Sized>(
    handle: &H,
    reader: &Schema,
    limits: Limits,
) -> Result<Container> {
    decode_container(handle, limits, Some(reader))
}

/// The shared whole-container decode path.
fn decode_container<H: IOBase + ?Sized>(
    handle: &H,
    limits: Limits,
    reader: Option<&Schema>,
) -> Result<Container> {
    let bytes = handle.read_all()?;
    if bytes.len() > limits.max_input_bytes() {
        return Err(invalid(format_smolstr!(
            "expected a container of at most {} bytes, got {}",
            limits.max_input_bytes(),
            bytes.len()
        )));
    }
    let mut cursor = Cursor::new(&bytes);
    check_magic(cursor.take(MAGIC.len())?)?;
    let header = parse_header(&mut cursor, limits)?;

    let resolution = reader
        .map(|reader| Resolution::from_schemas(&header.schema, reader))
        .transpose()?;
    let datum = DatumCodec {
        names: &header.schema.names,
        limits,
    };

    let mut rows = Vec::new();
    while !cursor.is_exhausted() {
        let count = cursor.long()?;
        let count = u64::try_from(count).map_err(|_| {
            codec(
                cursor.position,
                format_smolstr!("expected a non-negative Avro block count, got {count}"),
            )
        })?;
        let payload = cursor.bytes()?;
        let marker = cursor.take(SYNC_LEN)?;
        if marker != header.sync {
            return Err(codec(
                cursor.position,
                SmolStr::new_static(
                    "expected the header's synchronization marker after an Avro block",
                ),
            ));
        }
        if count as usize > limits.max_nodes() || rows.len() + count as usize > limits.max_nodes() {
            return Err(invalid(format_smolstr!(
                "expected at most {} rows in a container",
                limits.max_nodes()
            )));
        }
        let decoded = header.coding.load(payload, limits)?;
        let mut block = Cursor::new(&decoded);
        for _ in 0..count {
            let mut budget = limits.max_nodes();
            rows.push(match &resolution {
                Some(plan) => plan.decode(&mut block, limits, &mut budget)?,
                None => datum.decode(&header.schema.node, &mut block, 0, &mut budget)?,
            });
        }
        if !block.is_exhausted() {
            return Err(codec(
                block.position,
                format_smolstr!("expected the block to end after {count} declared rows"),
            ));
        }
    }

    Ok(Container {
        schema: header.schema,
        metadata: header.metadata,
        rows,
    })
}

/// Replace a handle's bytes with an Avro object container holding `rows`.
///
/// The schema is an Avro schema as its JSON [`Value`], and its JSON spelling
/// is written into the header verbatim, so attributes this implementation does
/// not model - Iceberg's `field-id` among them - survive byte for byte. Every
/// row is written as one block, compressed with raw deflate, which is what the
/// `deflate` codec name means and what the reference implementations write by
/// default.
///
/// # Errors
///
/// Returns an error when the schema JSON is not a schema, when a row does not
/// fit it, or when the write fails.
pub fn write_container<H: IOBase + ?Sized>(
    handle: &mut H,
    schema_json: &Value,
    metadata: &[(&str, &str)],
    rows: &[Value],
) -> Result<()> {
    let schema = Schema::from_json(schema_json)?;
    let encoded_schema = crate::json::to_vec(schema_json)?;
    let datum = DatumCodec {
        names: &schema.names,
        limits: Limits::default(),
    };

    let mut payload = Vec::new();
    for row in rows {
        datum.encode(&schema.node, row, &mut payload, 0)?;
    }
    // The marker is derived from the content rather than drawn at random:
    // uniqueness within the file is all the format needs, and a writer whose
    // output is a pure function of its input is what lets a conformance
    // check diff bytes instead of only semantics.
    let sync = derived_sync(&encoded_schema, &payload);
    let coding = BlockCoding::Shared(Codec::Deflate);
    let compressed = coding.dump(&payload, Level::DEFAULT)?;

    let mut output = Vec::with_capacity(compressed.len() + 512);
    output.extend_from_slice(&MAGIC);
    put_long(&mut output, metadata.len() as i64 + 2);
    put_bytes(&mut output, SCHEMA_KEY.as_bytes());
    put_bytes(&mut output, &encoded_schema);
    put_bytes(&mut output, CODEC_KEY.as_bytes());
    put_bytes(&mut output, coding.name().as_bytes());
    for (key, value) in metadata {
        put_bytes(&mut output, key.as_bytes());
        put_bytes(&mut output, value.as_bytes());
    }
    put_long(&mut output, 0);
    output.extend_from_slice(&sync);

    if !rows.is_empty() {
        put_long(&mut output, rows.len() as i64);
        put_bytes(&mut output, &compressed);
        output.extend_from_slice(&sync);
    }

    handle.write_all_bytes(&output)
}

/// Derive a synchronization marker from the container's own content.
///
/// The marker only has to be constant within one file and improbable in its
/// data, and a fingerprint of the schema and the encoded rows is exactly
/// that - while making the whole container a pure function of its input, so
/// two writers given the same rows produce the same bytes.
pub(crate) fn derived_sync(schema_bytes: &[u8], payload: &[u8]) -> [u8; SYNC_LEN] {
    let mut marker = [0_u8; SYNC_LEN];
    let head = super::schema::rabin(schema_bytes);
    // Salting the payload fingerprint with the schema's keeps the two halves
    // independent even when the payload is empty.
    let mut salted = Vec::with_capacity(payload.len() + 8);
    salted.extend_from_slice(&head.to_le_bytes());
    salted.extend_from_slice(payload);
    let tail = super::schema::rabin(&salted);
    marker[..8].copy_from_slice(&head.to_le_bytes());
    marker[8..].copy_from_slice(&tail.to_le_bytes());
    marker
}

/// Produce a synchronization marker unlikely to occur inside a block.
///
/// The marker only has to be constant within one file and improbable in its
/// data, so hashing process-seeded state is enough and avoids a dependency
/// whose only job would be sixteen bytes.
pub(crate) fn sync_marker() -> [u8; SYNC_LEN] {
    use std::hash::{BuildHasher, Hasher};

    let state = std::collections::hash_map::RandomState::new();
    let mut marker = [0_u8; SYNC_LEN];
    for (half, chunk) in marker.chunks_mut(8).enumerate() {
        let mut hasher = state.build_hasher();
        hasher.write_usize(half);
        hasher.write_u128(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or_default(),
        );
        chunk.copy_from_slice(&hasher.finish().to_le_bytes()[..chunk.len()]);
    }
    marker
}

/// A buffered positional reader over any handle: `pread` is the only
/// assumption, which is the storage contract's floor.
struct Pread<'handle, H: IOBase + ?Sized> {
    /// The handle being read.
    handle: &'handle H,
    /// The next byte to serve.
    position: u64,
    /// The handle's size when iteration began.
    size: u64,
    /// The buffered window.
    buffer: Vec<u8>,
    /// The absolute offset of the window's first byte.
    start: u64,
}

/// How many bytes one buffered window pulls at a time.
const CHUNK: usize = 64 * 1024;

impl<'handle, H: IOBase + ?Sized> Pread<'handle, H> {
    /// Start at the beginning of the handle.
    fn new(handle: &'handle H) -> Self {
        let size = handle.size();
        Self {
            handle,
            position: 0,
            size,
            buffer: Vec::new(),
            start: 0,
        }
    }

    /// Return whether every byte has been served.
    const fn is_exhausted(&self) -> bool {
        self.position >= self.size
    }

    /// Serve one byte.
    fn byte(&mut self) -> Result<u8> {
        if self.position >= self.size {
            return Err(codec(
                self.position as usize,
                SmolStr::new_static("expected another byte, got the end of the container"),
            ));
        }
        let offset = (self.position - self.start) as usize;
        if self.position < self.start || offset >= self.buffer.len() {
            let want = CHUNK.min((self.size - self.position) as usize);
            let mut chunk = vec![0; want];
            self.handle.pread_exact(self.position, &mut chunk)?;
            self.buffer = chunk;
            self.start = self.position;
        }
        let byte = self.buffer[(self.position - self.start) as usize];
        self.position += 1;
        Ok(byte)
    }

    /// Read a zig-zag variable-length integer.
    fn long(&mut self) -> Result<i64> {
        let mut shift = 0_u32;
        let mut accumulated = 0_u64;
        loop {
            let byte = self.byte()?;
            if shift > 63 {
                return Err(codec(
                    self.position as usize,
                    SmolStr::new_static("expected a variable-length integer of at most 10 bytes"),
                ));
            }
            accumulated |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                break;
            }
            shift += 7;
        }
        Ok(((accumulated >> 1) as i64) ^ -((accumulated & 1) as i64))
    }

    /// Read exactly `count` bytes.
    fn exact(&mut self, count: usize) -> Result<Vec<u8>> {
        let remaining = self.size.saturating_sub(self.position);
        if count as u64 > remaining {
            return Err(codec(
                self.position as usize,
                format_smolstr!("expected {count} bytes, got {remaining} bytes"),
            ));
        }
        let mut bytes = vec![0; count];
        self.handle.pread_exact(self.position, &mut bytes)?;
        self.position += count as u64;
        Ok(bytes)
    }

    /// Read a length-prefixed byte run bounded by `bound`.
    fn sized(&mut self, bound: usize) -> Result<Vec<u8>> {
        let length = self.long()?;
        let length = usize::try_from(length).map_err(|_| {
            codec(
                self.position as usize,
                format_smolstr!("expected a non-negative byte length, got {length}"),
            )
        })?;
        if length > bound {
            return Err(codec(
                self.position as usize,
                format_smolstr!("expected at most {bound} bytes, got {length}"),
            ));
        }
        self.exact(length)
    }
}

/// A lazy iterator over the blocks of an Avro object container.
///
/// Each step hands back one [`Block`] still compressed; decoding it is the
/// caller's choice, so a consumer can stream a large container and skip whole
/// blocks for free.
pub struct Blocks<'handle, H: IOBase + ?Sized> {
    /// The positional reader.
    source: Pread<'handle, H>,
    /// The writer schema the header named.
    schema: Schema,
    /// The header's metadata, minus the reserved keys.
    metadata: Vec<(SmolStr, SmolStr)>,
    /// The block compression.
    coding: BlockCoding,
    /// The marker that closes every block.
    sync: [u8; SYNC_LEN],
    /// The bounds every block decode honors.
    limits: Limits,
}

/// One block of an Avro object container, still compressed.
#[derive(Debug)]
pub struct Block {
    /// Rows the block declares.
    count: u64,
    /// The compressed payload.
    payload: Vec<u8>,
    /// The block compression.
    coding: BlockCoding,
    /// The writer schema.
    schema: Schema,
    /// The bounds decoding honors.
    limits: Limits,
}

/// Open the blocks of the Avro object container a handle holds.
///
/// # Errors
///
/// Returns an error when the header is not an Avro object container header.
pub fn read_blocks<H: IOBase + ?Sized>(handle: &H) -> Result<Blocks<'_, H>> {
    read_blocks_with_limits(handle, Limits::default())
}

/// Open the blocks of a container with explicit limits.
///
/// # Errors
///
/// Returns an error when the header is not an Avro object container header or
/// exceeds the limits.
pub fn read_blocks_with_limits<H: IOBase + ?Sized>(
    handle: &H,
    limits: Limits,
) -> Result<Blocks<'_, H>> {
    let mut source = Pread::new(handle);
    check_magic(&source.exact(MAGIC.len())?)?;

    // The header is small by construction; buffer it through the same parser
    // the whole-container path uses by reading entries incrementally.
    let mut entries = Vec::new();
    loop {
        let count = source.long()?;
        if count < 0 {
            // The size-carrying block form: the byte size is not needed when
            // every entry is read anyway.
            source.long()?;
        }
        let count = count.unsigned_abs();
        if count == 0 {
            break;
        }
        for _ in 0..count {
            if entries.len() >= limits.max_nodes() {
                return Err(invalid(format_smolstr!(
                    "expected at most {} header entries",
                    limits.max_nodes()
                )));
            }
            let key_bytes = source.sized(limits.max_input_bytes())?;
            let key = SmolStr::new(std::str::from_utf8(&key_bytes).map_err(|error| {
                codec(
                    source.position as usize,
                    format_smolstr!("expected UTF-8 in an Avro header key, got {error}"),
                )
            })?);
            let value = source.sized(limits.max_input_bytes())?;
            entries.push((key, value));
        }
    }
    let sync: [u8; SYNC_LEN] = source.exact(SYNC_LEN)?.as_slice().try_into().map_err(|_| {
        codec(
            source.position as usize,
            SmolStr::new_static("expected a sixteen-byte Avro synchronization marker"),
        )
    })?;

    let lookup = |key: &str| -> Option<&[u8]> {
        entries
            .iter()
            .find_map(|(name, value)| (name == key).then_some(value.as_slice()))
    };
    let schema_bytes = lookup(SCHEMA_KEY).ok_or_else(|| {
        invalid(format_smolstr!(
            "expected an Avro header carrying {SCHEMA_KEY:?}"
        ))
    })?;
    let schema_json = crate::json::from_slice_with_limits(schema_bytes, limits)?;
    let schema = Schema::from_json_with_limits(&schema_json, limits)?;
    let coding = match lookup(CODEC_KEY) {
        Some(value) => BlockCoding::from_name(&String::from_utf8_lossy(value))?,
        None => BlockCoding::Shared(Codec::Identity),
    };
    let metadata = entries
        .into_iter()
        .filter(|(key, _)| key != SCHEMA_KEY && key != CODEC_KEY)
        .map(|(key, value)| (key, SmolStr::new(String::from_utf8_lossy(&value))))
        .collect();

    Ok(Blocks {
        source,
        schema,
        metadata,
        coding,
        sync,
        limits,
    })
}

impl<H: IOBase + ?Sized> Blocks<'_, H> {
    /// Return the writer schema the header named.
    pub fn schema(&self) -> &Schema {
        &self.schema
    }

    /// Return the header's metadata, minus the reserved schema and codec.
    pub fn metadata(&self) -> &[(SmolStr, SmolStr)] {
        &self.metadata
    }

    /// Return one metadata value by key.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.metadata
            .iter()
            .find_map(|(name, value)| (name == key).then(|| value.as_str()))
    }

    /// Read the next block, still compressed, or `None` past the last one.
    ///
    /// # Errors
    ///
    /// Returns an error when the container ends mid-block or a marker does
    /// not match the header's.
    pub fn next_block(&mut self) -> Result<Option<Block>> {
        if self.source.is_exhausted() {
            return Ok(None);
        }
        let count = self.source.long()?;
        let count = u64::try_from(count).map_err(|_| {
            codec(
                self.source.position as usize,
                format_smolstr!("expected a non-negative Avro block count, got {count}"),
            )
        })?;
        if count as usize > self.limits.max_nodes() {
            return Err(invalid(format_smolstr!(
                "expected at most {} rows in a block",
                self.limits.max_nodes()
            )));
        }
        let payload = self.source.sized(self.limits.max_input_bytes())?;
        let marker = self.source.exact(SYNC_LEN)?;
        if marker != self.sync {
            return Err(codec(
                self.source.position as usize,
                SmolStr::new_static(
                    "expected the header's synchronization marker after an Avro block",
                ),
            ));
        }
        Ok(Some(Block {
            count,
            payload,
            coding: self.coding,
            schema: self.schema.clone(),
            limits: self.limits,
        }))
    }
}

impl Block {
    /// Return how many rows the block declares.
    pub const fn count(&self) -> u64 {
        self.count
    }

    /// Return the compressed payload size in bytes.
    pub fn size(&self) -> usize {
        self.payload.len()
    }

    /// Decompress and decode every row of the block with the writer schema.
    ///
    /// # Errors
    ///
    /// Returns an error when the payload does not decompress or a row does
    /// not decode.
    pub fn rows(&self) -> Result<Vec<Value>> {
        let decoded = self.coding.load(&self.payload, self.limits)?;
        let mut cursor = Cursor::new(&decoded);
        let datum = DatumCodec {
            names: &self.schema.names,
            limits: self.limits,
        };
        let mut rows = Vec::new();
        for _ in 0..self.count {
            let mut budget = self.limits.max_nodes();
            rows.push(datum.decode(&self.schema.node, &mut cursor, 0, &mut budget)?);
        }
        self.ended(&cursor)?;
        Ok(rows)
    }

    /// Check that a decoded block consumed its whole payload.
    fn ended(&self, cursor: &Cursor<'_>) -> Result<()> {
        if !cursor.is_exhausted() {
            return Err(codec(
                cursor.position,
                format_smolstr!(
                    "expected the block to end after {} declared rows",
                    self.count
                ),
            ));
        }
        Ok(())
    }

    /// Decompress and decode every row through a resolution plan.
    ///
    /// # Errors
    ///
    /// Returns an error when the payload does not decompress or a row does
    /// not resolve.
    pub fn rows_resolved(&self, resolution: &Resolution) -> Result<Vec<Value>> {
        let decoded = self.coding.load(&self.payload, self.limits)?;
        let mut cursor = Cursor::new(&decoded);
        let mut rows = Vec::new();
        for _ in 0..self.count {
            let mut budget = self.limits.max_nodes();
            rows.push(resolution.decode(&mut cursor, self.limits, &mut budget)?);
        }
        self.ended(&cursor)?;
        Ok(rows)
    }
}
