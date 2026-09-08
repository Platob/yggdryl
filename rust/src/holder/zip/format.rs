//! PKWARE APPNOTE 6.3.10 record framing.
//!
//! Every record a ZIP archive is built from is encoded and decoded here and
//! nowhere else: the local file header a member's bytes follow, the central
//! directory record that indexes it, the end-of-central-directory record that
//! finds the index, and the ZIP64 pair that lifts the 32-bit ceilings.
//!
//! The layer knows byte layout and nothing about handles, so a record is
//! exercised directly over a slice.

use smol_str::format_smolstr;

use crate::{Codec, Error, Restarts, Result};

use super::Entry;

/// `PK\x03\x04`: the header one member's bytes follow.
pub(super) const LOCAL_SIGNATURE: u32 = 0x0403_4b50;
/// `PK\x01\x02`: one central directory record.
pub(super) const CENTRAL_SIGNATURE: u32 = 0x0201_4b50;
/// `PK\x05\x06`: the end-of-central-directory record.
pub(super) const END_SIGNATURE: u32 = 0x0605_4b50;
/// `PK\x06\x06`: the ZIP64 end-of-central-directory record.
pub(super) const ZIP64_END_SIGNATURE: u32 = 0x0606_4b50;
/// `PK\x06\x07`: the locator that finds the ZIP64 record.
pub(super) const ZIP64_LOCATOR_SIGNATURE: u32 = 0x0706_4b50;

/// Fixed bytes of a local file header, before its name and extra fields.
pub(super) const LOCAL_LEN: usize = 30;
/// Fixed bytes of a central directory record, before its variable fields.
pub(super) const CENTRAL_LEN: usize = 46;
/// Fixed bytes of an end-of-central-directory record, before its comment.
pub(super) const END_LEN: usize = 22;
/// Bytes of a ZIP64 end-of-central-directory record with no extensible data.
pub(super) const ZIP64_END_LEN: usize = 56;
/// Bytes of a ZIP64 end-of-central-directory locator.
pub(super) const ZIP64_LOCATOR_LEN: usize = 20;

/// The 32-bit value that says "read this from the ZIP64 extra field".
pub(super) const ZIP64_MARK_32: u32 = u32::MAX;
/// The 16-bit counterpart, used for entry counts and disk numbers.
pub(super) const ZIP64_MARK_16: u16 = u16::MAX;

/// Header id of the ZIP64 extended information extra field.
const ZIP64_EXTRA_ID: u16 = 0x0001;
/// Header id of the Info-ZIP extended timestamp extra field.
const TIMESTAMP_EXTRA_ID: u16 = 0x5455;
/// Header id of this crate's restart map, unclaimed by APPNOTE's registry.
///
/// The map says where a compressed member's decode may begin, which is what
/// makes a position inside one addressable without decoding everything before
/// it. It is metadata beside the member rather than part of it: a reader that
/// does not know the id skips the field by its declared length, exactly as
/// this one skips every field it does not know, and reads the member whole as
/// it always would.
const RESTART_EXTRA_ID: u16 = 0x5967;
/// The extended timestamp bit that says a modification time is present.
const TIMESTAMP_MODIFIED: u8 = 0x01;

/// General purpose bit 0: the member is encrypted.
pub(super) const FLAG_ENCRYPTED: u16 = 1 << 0;
/// General purpose bit 3: sizes follow the data instead of preceding it.
pub(super) const FLAG_DATA_DESCRIPTOR: u16 = 1 << 3;
/// General purpose bit 11: the name and comment are UTF-8.
pub(super) const FLAG_UTF8: u16 = 1 << 11;

/// The compression method a stored member declares.
const METHOD_STORE: u16 = 0;
/// The compression method a raw DEFLATE member declares.
const METHOD_DEFLATE: u16 = 8;
/// The compression method a Zstandard member declares.
const METHOD_ZSTD: u16 = 93;

/// Version 4.5, the first that specifies ZIP64.
const VERSION_ZIP64: u16 = 45;
/// Version 2.0, the first that specifies DEFLATE and directory entries.
const VERSION_BASE: u16 = 20;
/// Version 6.3, the first that specifies Zstandard.
const VERSION_ZSTD: u16 = 63;

/// The most restart points one member's map states.
///
/// A map rides the central directory, which the mount reads whole, so its size
/// is a cost every operation pays rather than one only a seek does. The writer
/// widens the stride instead of passing this, so a large member stays as
/// addressable as a small one at a bounded price: 8 bytes a point, 16 KiB at
/// the ceiling.
pub(super) const MAX_RESTARTS: usize = 2_048;

/// The Unix permissions a written member and directory carry.
const UNIX_FILE_MODE: u32 = 0o100_644;
/// The Unix permissions a written directory entry carries.
const UNIX_DIRECTORY_MODE: u32 = 0o040_755;
/// MS-DOS attribute bit marking a directory, for readers that only read it.
const DOS_DIRECTORY: u32 = 0x10;
/// "Made by" byte 2 (Unix), so the external attributes are read as a mode.
const MADE_BY_UNIX: u16 = 3 << 8;

/// Nanoseconds in one second, the unit every modification time is stated in.
const NANOS_PER_SECOND: i64 = 1_000_000_000;
/// Seconds in one day, for the DOS civil-date conversion.
const SECONDS_PER_DAY: i64 = 86_400;
/// The first year a DOS timestamp can spell.
const DOS_EPOCH_YEAR: i32 = 1980;

/// Report a malformed record, naming the archive offset it was read at.
pub(super) fn malformed(position: usize, reason: impl std::fmt::Display) -> Error {
    Error::Codec {
        format: "zip",
        position,
        reason: format_smolstr!("{reason}"),
    }
}

/// Return the compression method a codec is written as.
///
/// # Errors
///
/// Returns [`Error::Unsupported`] naming a coding no ZIP method spells. Gzip
/// and zlib framing are content codings of a whole resource; a member carries
/// the raw DEFLATE stream inside them instead.
pub(super) fn method_of(codec: Codec) -> Result<u16> {
    match codec {
        Codec::Identity => Ok(METHOD_STORE),
        Codec::Deflate => Ok(METHOD_DEFLATE),
        Codec::Zstd => Ok(METHOD_ZSTD),
        Codec::Gzip | Codec::Zlib => Err(Error::unsupported(
            "a zip member compressed with framing this format has no method for",
            codec.as_str(),
        )),
    }
}

/// Return the codec a compression method decodes with.
///
/// # Errors
///
/// Returns [`Error::Unsupported`] naming the method number this build cannot
/// decode.
pub(super) fn codec_of(method: u16) -> Result<Codec> {
    match method {
        METHOD_STORE => Ok(Codec::Identity),
        METHOD_DEFLATE => Ok(Codec::Deflate),
        METHOD_ZSTD => Ok(Codec::Zstd),
        other => Err(Error::unsupported(
            "decoding a zip member",
            format_smolstr!("compression method {other}"),
        )),
    }
}

/// The version a member needs a reader to implement.
fn version_needed(method: u16, zip64: bool) -> u16 {
    if zip64 {
        VERSION_ZIP64.max(if method == METHOD_ZSTD {
            VERSION_ZSTD
        } else {
            VERSION_BASE
        })
    } else if method == METHOD_ZSTD {
        VERSION_ZSTD
    } else {
        VERSION_BASE
    }
}

/// A bounds-checked little-endian reader over one record.
///
/// `base` is where `bytes` starts in the archive, so a refusal names the
/// offset a caller can actually look at rather than an offset into a copy.
pub(super) struct Scan<'bytes> {
    bytes: &'bytes [u8],
    base: usize,
    position: usize,
}

impl<'bytes> Scan<'bytes> {
    /// Read `bytes`, which begin at archive offset `base`.
    pub(super) const fn new(bytes: &'bytes [u8], base: usize) -> Self {
        Self {
            bytes,
            base,
            position: 0,
        }
    }

    /// The archive offset the next read starts at.
    pub(super) const fn offset(&self) -> usize {
        self.base + self.position
    }

    /// The bytes not yet read.
    pub(super) const fn remaining(&self) -> usize {
        self.bytes.len() - self.position
    }

    /// Take `length` bytes.
    fn take(&mut self, length: usize) -> Result<&'bytes [u8]> {
        let end = self.position.checked_add(length).ok_or_else(|| {
            malformed(
                self.offset(),
                "a record field length exceeds the address space",
            )
        })?;
        let taken = self.bytes.get(self.position..end).ok_or_else(|| {
            malformed(
                self.offset(),
                format_smolstr!(
                    "expected {length} more bytes of the record, got {}",
                    self.remaining()
                ),
            )
        })?;
        self.position = end;
        Ok(taken)
    }

    /// Read one little-endian `u16`.
    fn u16(&mut self) -> Result<u16> {
        let bytes = self.take(2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    /// Read one little-endian `u32`.
    fn u32(&mut self) -> Result<u32> {
        let bytes = self.take(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    /// Read one little-endian `u64`.
    fn u64(&mut self) -> Result<u64> {
        let bytes = self.take(8)?;
        Ok(u64::from_le_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]))
    }

    /// Read one little-endian `i32`.
    fn i32(&mut self) -> Result<i32> {
        self.u32().map(|value| value as i32)
    }
}

/// Append one little-endian `u16`.
fn put_u16(target: &mut Vec<u8>, value: u16) {
    target.extend_from_slice(&value.to_le_bytes());
}

/// Append one little-endian `u32`.
fn put_u32(target: &mut Vec<u8>, value: u32) {
    target.extend_from_slice(&value.to_le_bytes());
}

/// Append one little-endian `u64`.
fn put_u64(target: &mut Vec<u8>, value: u64) {
    target.extend_from_slice(&value.to_le_bytes());
}

/// Narrow a 64-bit value, or answer the ZIP64 marker when it does not fit.
const fn marked(value: u64) -> u32 {
    if value >= ZIP64_MARK_32 as u64 {
        ZIP64_MARK_32
    } else {
        value as u32
    }
}

/// Convert UTC nanoseconds into the MS-DOS date and time pair.
///
/// The DOS clock starts in 1980 and counts two-second ticks, so a value
/// outside its range is clamped to the nearest representable instant rather
/// than refused: a timestamp is member metadata, never member content.
pub(super) fn dos_datetime(nanos: i64) -> (u16, u16) {
    let seconds = nanos.div_euclid(NANOS_PER_SECOND);
    let days = seconds.div_euclid(SECONDS_PER_DAY);
    let within = seconds.rem_euclid(SECONDS_PER_DAY);
    let (year, month, day) = crate::timezone::civil_from_days(days);
    if year < DOS_EPOCH_YEAR {
        // 1980-01-01T00:00:00, the earliest instant the format can state.
        return (0x0021, 0);
    }
    if year > DOS_EPOCH_YEAR + 127 {
        // 2107-12-31T23:59:58, the latest.
        return (0xFF9F, 0xBF7D);
    }
    let date = (((year - DOS_EPOCH_YEAR) as u16) << 9) | ((month as u16) << 5) | (day as u16);
    let hour = (within / 3_600) as u16;
    let minute = ((within % 3_600) / 60) as u16;
    let second = (within % 60) as u16;
    (date, (hour << 11) | (minute << 5) | (second / 2))
}

/// Convert an MS-DOS date and time pair into UTC nanoseconds.
///
/// The pair carries no zone, so it is read as UTC. An archive written by this
/// crate also carries an Info-ZIP extended timestamp, which is read instead
/// wherever it is present.
pub(super) fn dos_nanos(date: u16, time: u16) -> i64 {
    let year = DOS_EPOCH_YEAR + i32::from(date >> 9);
    let month = u32::from((date >> 5) & 0x0F).clamp(1, 12);
    let day = u32::from(date & 0x1F).clamp(1, 31);
    let hour = i64::from(time >> 11).min(23);
    let minute = i64::from((time >> 5) & 0x3F).min(59);
    let second = i64::from((time & 0x1F) * 2).min(59);
    let days = crate::timezone::days_from_civil(year, month, day);
    (days * SECONDS_PER_DAY + hour * 3_600 + minute * 60 + second) * NANOS_PER_SECOND
}

/// The 32-bit fields a ZIP64 extra field may replace, in the format's order.
struct Zip64Fields {
    size: u64,
    compressed_size: u64,
    header_offset: u64,
}

/// Read the ZIP64 and timestamp extras a central record carries.
///
/// Only the fields whose 32-bit slot holds the marker are present in the
/// ZIP64 extra, and they appear in a fixed order, so the sentinels decide what
/// to read. An extra field this crate does not know is skipped by its declared
/// length, which is what keeps an archive written by another tool readable.
fn read_extras(
    extra: &[u8],
    base: usize,
    fields: &mut Zip64Fields,
    modified: &mut Option<i64>,
    restarts: &mut Restarts,
) -> Result<()> {
    let mut scan = Scan::new(extra, base);
    while scan.remaining() >= 4 {
        let id = scan.u16()?;
        let length = usize::from(scan.u16()?);
        let body = scan.take(length)?;
        match id {
            ZIP64_EXTRA_ID => {
                let mut body = Scan::new(body, base);
                if fields.size == u64::from(ZIP64_MARK_32) && body.remaining() >= 8 {
                    fields.size = body.u64()?;
                }
                if fields.compressed_size == u64::from(ZIP64_MARK_32) && body.remaining() >= 8 {
                    fields.compressed_size = body.u64()?;
                }
                if fields.header_offset == u64::from(ZIP64_MARK_32) && body.remaining() >= 8 {
                    fields.header_offset = body.u64()?;
                }
            }
            TIMESTAMP_EXTRA_ID => {
                let mut body = Scan::new(body, base);
                if body.remaining() >= 5 {
                    let present = body.take(1)?[0];
                    if present & TIMESTAMP_MODIFIED != 0 {
                        *modified = Some(i64::from(body.i32()?) * NANOS_PER_SECOND);
                    }
                }
            }
            RESTART_EXTRA_ID => {
                let mut body = Scan::new(body, base);
                if body.remaining() >= 8 {
                    let stride = body.u64()?;
                    let mut points = Vec::with_capacity(body.remaining() / 8);
                    while body.remaining() >= 8 {
                        points.push(body.u64()?);
                    }
                    *restarts = Restarts::new(stride, points);
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// Build the restart map extra field, or nothing when the member has none.
fn restart_extra(entry: &Entry) -> Vec<u8> {
    let restarts = entry.restarts();
    if restarts.is_empty() {
        return Vec::new();
    }
    let mut extra = Vec::with_capacity(8 * restarts.points().len() + 12);
    put_u16(&mut extra, RESTART_EXTRA_ID);
    put_u16(&mut extra, (8 * restarts.points().len() + 8) as u16);
    put_u64(&mut extra, restarts.stride());
    for point in restarts.points() {
        put_u64(&mut extra, *point);
    }
    extra
}

/// Build the ZIP64 extra field a record needs, or nothing when it needs none.
///
/// A local header that states either size in the extra states *both*, which
/// APPNOTE 6.3.10 requires of it: a reader that takes the pair as one unit
/// would otherwise read the field behind it as the second half. A central
/// record has no such rule and carries only the fields its own slots marked.
fn zip64_extra(entry: &Entry, local: bool) -> Vec<u8> {
    let mut body = Vec::new();
    let both = local && settles_extra(entry);
    if both || entry.size() >= u64::from(ZIP64_MARK_32) {
        put_u64(&mut body, entry.size());
    }
    if both || entry.compressed_size() >= u64::from(ZIP64_MARK_32) {
        put_u64(&mut body, entry.compressed_size());
    }
    // A local header states no offset of its own, so the field never travels
    // in one even when the central record needs it.
    if !local && entry.header_offset() >= u64::from(ZIP64_MARK_32) {
        put_u64(&mut body, entry.header_offset());
    }
    if body.is_empty() {
        return body;
    }
    let mut extra = Vec::with_capacity(body.len() + 4);
    put_u16(&mut extra, ZIP64_EXTRA_ID);
    put_u16(&mut extra, body.len() as u16);
    extra.extend_from_slice(&body);
    extra
}

/// Build the ZIP64 extra a header reserves before it knows the sizes.
///
/// A streamed member's sizes are only known once its last byte is encoded,
/// and a header that grew to state them would move the bytes it introduces.
/// So the room is taken up front and filled in afterwards, at the one length
/// both spellings share.
fn reserved_zip64_extra(entry: &Entry) -> Vec<u8> {
    let mut extra = Vec::with_capacity(20);
    put_u16(&mut extra, ZIP64_EXTRA_ID);
    put_u16(&mut extra, 16);
    put_u64(&mut extra, entry.size());
    put_u64(&mut extra, entry.compressed_size());
    extra
}

/// Build the Info-ZIP extended timestamp extra carrying a modification time.
///
/// The field states whole UTC seconds, which is what makes a round trip exact
/// where the two-second DOS pair beside it cannot be.
fn timestamp_extra(entry: &Entry) -> Vec<u8> {
    let seconds = entry.modified().div_euclid(NANOS_PER_SECOND);
    let Ok(seconds) = i32::try_from(seconds) else {
        return Vec::new();
    };
    let mut extra = Vec::with_capacity(9);
    put_u16(&mut extra, TIMESTAMP_EXTRA_ID);
    put_u16(&mut extra, 5);
    extra.push(TIMESTAMP_MODIFIED);
    extra.extend_from_slice(&seconds.to_le_bytes());
    extra
}

/// Read one central directory record, returning it and the bytes it spanned.
///
/// The scan starts at the record's signature. A record whose signature does
/// not match ends the directory rather than failing, which is what lets the
/// caller stop at the first record that is not one.
///
/// # Errors
///
/// Returns [`Error::Codec`] naming the offset when the record is truncated or
/// its name is not UTF-8.
pub(super) fn read_central(scan: &mut Scan<'_>) -> Result<Entry> {
    let offset = scan.offset();
    let signature = scan.u32()?;
    if signature != CENTRAL_SIGNATURE {
        return Err(malformed(
            offset,
            format_smolstr!("expected a central directory record, got {signature:#010x}"),
        ));
    }
    let _version_made_by = scan.u16()?;
    let _version_needed = scan.u16()?;
    let flags = scan.u16()?;
    let method = scan.u16()?;
    let time = scan.u16()?;
    let date = scan.u16()?;
    let crc32 = scan.u32()?;
    let compressed_size = u64::from(scan.u32()?);
    let size = u64::from(scan.u32()?);
    let name_len = usize::from(scan.u16()?);
    let extra_len = usize::from(scan.u16()?);
    let comment_len = usize::from(scan.u16()?);
    let _disk = scan.u16()?;
    let _internal = scan.u16()?;
    let external_attributes = scan.u32()?;
    let header_offset = u64::from(scan.u32()?);
    let name = scan.take(name_len)?;
    let extra = scan.take(extra_len)?;
    let comment = scan.take(comment_len)?;

    let mut fields = Zip64Fields {
        size,
        compressed_size,
        header_offset,
    };
    let mut modified = None;
    let mut restarts = Restarts::default();
    read_extras(extra, offset, &mut fields, &mut modified, &mut restarts)?;

    let name = decode_text(name, offset, "name")?;
    let comment = decode_text(comment, offset, "comment")?;
    Ok(Entry::from_parts(
        name,
        flags,
        method,
        modified.unwrap_or_else(|| dos_nanos(date, time)),
        crc32,
        fields.compressed_size,
        fields.size,
        fields.header_offset,
        external_attributes,
        comment,
    )
    .with_restarts(restarts))
}

/// Decode one record's text field, which the format states is UTF-8.
///
/// General purpose bit 11 says so explicitly; an archive that predates the bit
/// spells names in a code page nothing identifies, so the one interpretation
/// that can be checked is the one that is used.
fn decode_text(bytes: &[u8], offset: usize, field: &str) -> Result<smol_str::SmolStr> {
    std::str::from_utf8(bytes)
        .map(smol_str::SmolStr::new)
        .map_err(|error| {
            malformed(
                offset,
                format_smolstr!("expected a UTF-8 member {field}, {error}"),
            )
        })
}

/// Encode one central directory record.
pub(super) fn write_central(entry: &Entry, target: &mut Vec<u8>) {
    let zip64 = zip64_extra(entry, false);
    let timestamp = timestamp_extra(entry);
    // The map rides the central record alone: the index is what reads it, and
    // a local header that never carries it is one compaction copies verbatim.
    let restarts = restart_extra(entry);
    let extra_len = zip64.len() + timestamp.len() + restarts.len();
    let (date, time) = dos_datetime(entry.modified());
    put_u32(target, CENTRAL_SIGNATURE);
    put_u16(target, MADE_BY_UNIX | VERSION_ZIP64);
    put_u16(target, version_needed(entry.method(), !zip64.is_empty()));
    put_u16(target, entry.flags());
    put_u16(target, entry.method());
    put_u16(target, time);
    put_u16(target, date);
    put_u32(target, entry.crc32());
    put_u32(target, marked(entry.compressed_size()));
    put_u32(target, marked(entry.size()));
    put_u16(target, entry.name().len() as u16);
    put_u16(target, extra_len as u16);
    put_u16(target, entry.comment().len() as u16);
    // One disk, and no internal attributes: a member is opaque bytes here.
    put_u16(target, 0);
    put_u16(target, 0);
    put_u32(target, entry.external_attributes());
    put_u32(target, marked(entry.header_offset()));
    target.extend_from_slice(entry.name().as_bytes());
    target.extend_from_slice(&zip64);
    target.extend_from_slice(&timestamp);
    target.extend_from_slice(&restarts);
    target.extend_from_slice(entry.comment().as_bytes());
}

/// Encode the local file header a member's bytes follow.
///
/// A write that does not yet know how long the member will be reserves the
/// room, writes the bytes, and states the sizes into the header it already
/// placed - which is only possible because the reserved shape has the length
/// the settled one will have.
pub(super) fn write_local_with(entry: &Entry, reserve: bool, target: &mut Vec<u8>) {
    let zip64 = if reserve {
        reserved_zip64_extra(entry)
    } else {
        zip64_extra(entry, true)
    };
    let timestamp = timestamp_extra(entry);
    let extra_len = zip64.len() + timestamp.len();
    let (date, time) = dos_datetime(entry.modified());
    put_u32(target, LOCAL_SIGNATURE);
    put_u16(target, version_needed(entry.method(), !zip64.is_empty()));
    put_u16(target, entry.flags());
    put_u16(target, entry.method());
    put_u16(target, time);
    put_u16(target, date);
    put_u32(target, entry.crc32());
    put_u32(target, marked(entry.compressed_size()));
    put_u32(target, marked(entry.size()));
    put_u16(target, entry.name().len() as u16);
    put_u16(target, extra_len as u16);
    target.extend_from_slice(entry.name().as_bytes());
    target.extend_from_slice(&zip64);
    target.extend_from_slice(&timestamp);
}

/// Return where a member's bytes start, given its local header.
///
/// The local header repeats the name and may carry different extra fields from
/// the central record, so the data offset is only knowable from the header
/// itself - which is why this is the one read a member's first byte costs.
///
/// # Errors
///
/// Returns [`Error::Codec`] when the header is not one.
pub(super) fn local_data_offset(header: &[u8], offset: u64) -> Result<u64> {
    let base = usize::try_from(offset).unwrap_or(usize::MAX);
    let mut scan = Scan::new(header, base);
    let signature = scan.u32()?;
    if signature != LOCAL_SIGNATURE {
        return Err(malformed(
            base,
            format_smolstr!("expected a local file header, got {signature:#010x}"),
        ));
    }
    // Version, flags, method, time, date, crc, and both sizes are already
    // stated by the central record; only the two lengths are needed here.
    let mut scan = Scan::new(header, base);
    let _ = scan.take(26)?;
    let name_len = u64::from(scan.u16()?);
    let extra_len = u64::from(scan.u16()?);
    Ok(offset + LOCAL_LEN as u64 + name_len + extra_len)
}

/// What the end-of-central-directory records say about the index.
#[derive(Clone, Copy, Debug)]
pub(super) struct End {
    /// Where the central directory starts, as the record declares it.
    pub(super) directory_offset: u64,
    /// How many bytes of central directory records there are.
    pub(super) directory_size: u64,
    /// How many records that is.
    pub(super) entries: u64,
}

/// Read the end-of-central-directory record `bytes` starts with.
///
/// # Errors
///
/// Returns [`Error::Codec`] naming the offset when the record is truncated.
pub(super) fn read_end(bytes: &[u8], base: usize) -> Result<(End, Vec<u8>)> {
    let mut scan = Scan::new(bytes, base);
    let signature = scan.u32()?;
    if signature != END_SIGNATURE {
        return Err(malformed(
            base,
            format_smolstr!("expected an end of central directory record, got {signature:#010x}"),
        ));
    }
    let _disk = scan.u16()?;
    let _directory_disk = scan.u16()?;
    let _disk_entries = scan.u16()?;
    let entries = u64::from(scan.u16()?);
    let directory_size = u64::from(scan.u32()?);
    let directory_offset = u64::from(scan.u32()?);
    let comment_len = usize::from(scan.u16()?);
    let comment = scan.take(comment_len.min(scan.remaining()))?.to_vec();
    Ok((
        End {
            directory_offset,
            directory_size,
            entries,
        },
        comment,
    ))
}

/// Read the ZIP64 locator `bytes` starts with, answering the record's offset.
///
/// # Errors
///
/// Returns [`Error::Codec`] when the locator is truncated or is not one.
pub(super) fn read_zip64_locator(bytes: &[u8], base: usize) -> Result<Option<u64>> {
    let mut scan = Scan::new(bytes, base);
    if scan.u32()? != ZIP64_LOCATOR_SIGNATURE {
        return Ok(None);
    }
    let _disk = scan.u32()?;
    let offset = scan.u64()?;
    Ok(Some(offset))
}

/// Read the ZIP64 end-of-central-directory record `bytes` starts with.
///
/// # Errors
///
/// Returns [`Error::Codec`] when the record is truncated or is not one.
pub(super) fn read_zip64_end(bytes: &[u8], base: usize) -> Result<End> {
    let mut scan = Scan::new(bytes, base);
    let signature = scan.u32()?;
    if signature != ZIP64_END_SIGNATURE {
        return Err(malformed(
            base,
            format_smolstr!(
                "expected a zip64 end of central directory record, got {signature:#010x}"
            ),
        ));
    }
    let _record_size = scan.u64()?;
    let _version_made_by = scan.u16()?;
    let _version_needed = scan.u16()?;
    let _disk = scan.u32()?;
    let _directory_disk = scan.u32()?;
    let _disk_entries = scan.u64()?;
    let entries = scan.u64()?;
    let directory_size = scan.u64()?;
    let directory_offset = scan.u64()?;
    Ok(End {
        directory_offset,
        directory_size,
        entries,
    })
}

/// Encode the trailer that closes an archive: ZIP64 pair when needed, then the
/// end-of-central-directory record.
///
/// `end.directory_offset` is where the records this trailer indexes begin, and
/// the trailer itself is appended after them.
pub(super) fn write_end(end: End, comment: &[u8], target: &mut Vec<u8>) {
    let zip64 = end.entries > u64::from(ZIP64_MARK_16)
        || end.directory_offset >= u64::from(ZIP64_MARK_32)
        || end.directory_size >= u64::from(ZIP64_MARK_32);
    if zip64 {
        let record = end.directory_offset + end.directory_size;
        put_u32(target, ZIP64_END_SIGNATURE);
        // The record's own size, counted from the field that follows it.
        put_u64(target, ZIP64_END_LEN as u64 - 12);
        put_u16(target, MADE_BY_UNIX | VERSION_ZIP64);
        put_u16(target, VERSION_ZIP64);
        put_u32(target, 0);
        put_u32(target, 0);
        put_u64(target, end.entries);
        put_u64(target, end.entries);
        put_u64(target, end.directory_size);
        put_u64(target, end.directory_offset);

        put_u32(target, ZIP64_LOCATOR_SIGNATURE);
        put_u32(target, 0);
        put_u64(target, record);
        put_u32(target, 1);
    }
    put_u32(target, END_SIGNATURE);
    put_u16(target, 0);
    put_u16(target, 0);
    let entries = u16::try_from(end.entries).unwrap_or(ZIP64_MARK_16);
    put_u16(target, entries);
    put_u16(target, entries);
    put_u32(target, marked(end.directory_size));
    put_u32(target, marked(end.directory_offset));
    put_u16(target, comment.len() as u16);
    target.extend_from_slice(comment);
}

/// Whether settling a moved record has to reach past its fixed head.
///
/// Only a size the 32-bit slot cannot hold is stated in the ZIP64 extra, so
/// only that case makes compaction read the variable part of a local header.
pub(super) fn settles_extra(entry: &Entry) -> bool {
    entry.size() >= u64::from(ZIP64_MARK_32) || entry.compressed_size() >= u64::from(ZIP64_MARK_32)
}

/// Make a copied local header state what its central record states.
///
/// A streaming writer leaves the digest and both sizes out of the local header
/// and sets the data-descriptor bit, promising them after the member's bytes
/// instead. Compaction moves the record but not that trailer, so the header it
/// writes has to carry the values itself - which the central record already
/// knows. The header keeps its exact length, because that is what lets a
/// record only ever move earlier.
pub(super) fn settle_local(header: &mut [u8], entry: &Entry) {
    if header.len() < LOCAL_LEN {
        return;
    }
    let flags = entry.flags() & !FLAG_DATA_DESCRIPTOR;
    header[6..8].copy_from_slice(&flags.to_le_bytes());
    header[14..18].copy_from_slice(&entry.crc32().to_le_bytes());
    header[18..22].copy_from_slice(&marked(entry.compressed_size()).to_le_bytes());
    header[22..26].copy_from_slice(&marked(entry.size()).to_le_bytes());

    // A size the 32-bit slot cannot hold is stated in the ZIP64 extra beside
    // it, in the same order the record was read in.
    if !settles_extra(entry) {
        return;
    }
    let name_len = usize::from(u16::from_le_bytes([header[26], header[27]]));
    let extra_len = usize::from(u16::from_le_bytes([header[28], header[29]]));
    let Some(extra) = header.get_mut(LOCAL_LEN + name_len..LOCAL_LEN + name_len + extra_len) else {
        return;
    };
    let mut at = 0;
    while at + 4 <= extra.len() {
        let id = u16::from_le_bytes([extra[at], extra[at + 1]]);
        let length = usize::from(u16::from_le_bytes([extra[at + 2], extra[at + 3]]));
        let body = at + 4;
        if id == ZIP64_EXTRA_ID {
            let mut slot = body;
            for value in [entry.size(), entry.compressed_size()] {
                if slot + 8 > body + length || slot + 8 > extra.len() {
                    break;
                }
                extra[slot..slot + 8].copy_from_slice(&value.to_le_bytes());
                slot += 8;
            }
            return;
        }
        at = body + length;
    }
}

/// The external attributes a written member carries.
///
/// The high half is the Unix mode the "made by" byte promises; the low half is
/// the MS-DOS attribute byte, which is all a reader that ignores the mode
/// looks at.
pub(super) const fn external_attributes(directory: bool) -> u32 {
    if directory {
        (UNIX_DIRECTORY_MODE << 16) | DOS_DIRECTORY
    } else {
        UNIX_FILE_MODE << 16
    }
}
