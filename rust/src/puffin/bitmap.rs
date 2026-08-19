//! The `deletion-vector-v1` blob content: portable 64-bit Roaring + CRC framing.
//!
//! A deletion vector is a set of 64-bit row positions stored as the Roaring
//! bitmap "portable" serialization - a count of buckets, then for each 32-bit
//! high key one standard 32-bit Roaring bitmap of the low words - wrapped in
//! the framing Iceberg's Puffin spec puts around it: a big-endian combined
//! length, the magic `D1 D3 39 64`, the little-endian vector, and a big-endian
//! CRC-32 of the magic and the vector. The mixed endianness is the spec's own
//! (big-endian framing for Delta compatibility, little-endian Roaring), so
//! both are spelled explicitly at every read and write below.
//!
//! [`write_deletion_vector`] and [`read_deletion_vector`] are the two public
//! seams: sorted positions in, framed bytes out, and the exact inverse with
//! every framing rule validated. The Roaring codec beneath them implements
//! array, bitset, and run containers on both sides, choosing per container
//! whatever serializes smallest - a run container only when its run form is
//! strictly smaller than the array or bitset form, an array container at
//! cardinality 4096 or below, a bitset above - which is the choice the
//! reference implementations make, so a vector written here re-reads
//! byte-identically through them.

use smol_str::{SmolStr, format_smolstr};

use crate::{Error, Limits, Result};

/// The four magic bytes that follow a deletion vector's length prefix.
pub(crate) const DELETION_VECTOR_MAGIC: [u8; 4] = [0xD1, 0xD3, 0x39, 0x64];

/// The Roaring cookie of a serialization holding no run containers.
const SERIAL_COOKIE_NO_RUNCONTAINER: u32 = 12346;

/// The Roaring cookie of a serialization that may hold run containers.
const SERIAL_COOKIE: u32 = 12347;

/// The container count at which a run-carrying serialization stores offsets.
const NO_OFFSET_THRESHOLD: usize = 4;

/// The largest cardinality an array container may hold.
const ARRAY_MAX_CARDINALITY: usize = 4096;

/// The exact serialized size of a bitset container.
const BITSET_BYTES: usize = 8192;

/// The framing around the vector: length, magic, and trailing CRC-32.
const FRAMING_BYTES: usize = 12;

/// Serialize sorted row positions as one `deletion-vector-v1` blob content.
///
/// The result is the complete blob: 4-byte big-endian combined length of the
/// magic and the vector, the magic `D1 D3 39 64`, the portable 64-bit Roaring
/// vector little-endian, and a 4-byte big-endian CRC-32 over the magic and the
/// vector.
///
/// # Errors
///
/// Returns an error when `positions` is not strictly increasing or holds a
/// position whose most significant bit is set (the spec supports positive
/// 64-bit positions only).
pub fn write_deletion_vector(positions: &[u64]) -> Result<Vec<u8>> {
    validate_positions(positions)?;
    let vector = encode_portable_64(positions);
    let combined = u32::try_from(DELETION_VECTOR_MAGIC.len() + vector.len()).map_err(|_| {
        invalid(format_smolstr!(
            "expected a vector and magic of at most {} bytes, got {}",
            u32::MAX,
            DELETION_VECTOR_MAGIC.len() + vector.len()
        ))
    })?;
    let mut crc = flate2::Crc::new();
    crc.update(&DELETION_VECTOR_MAGIC);
    crc.update(&vector);
    let mut blob = Vec::with_capacity(FRAMING_BYTES + vector.len());
    blob.extend_from_slice(&combined.to_be_bytes());
    blob.extend_from_slice(&DELETION_VECTOR_MAGIC);
    blob.extend_from_slice(&vector);
    blob.extend_from_slice(&crc.sum().to_be_bytes());
    Ok(blob)
}

/// Deserialize one `deletion-vector-v1` blob content to sorted row positions.
///
/// Every framing rule is validated: the big-endian combined length must equal
/// the length of the magic plus the vector, the magic must be `D1 D3 39 64`,
/// the big-endian CRC-32 must match a checksum recomputed over the magic and
/// the vector, and the vector must decode completely as the portable 64-bit
/// Roaring serialization with its buckets, containers, and values in order.
///
/// # Errors
///
/// Returns an error naming what was expected and what the bytes hold, at the
/// byte position of the failure, when any framing or Roaring rule is broken,
/// or when the decoded positions exceed the default [`Limits`] byte budget.
pub fn read_deletion_vector(bytes: &[u8]) -> Result<Vec<u64>> {
    read_deletion_vector_with_limits(bytes, Limits::default())
}

/// [`read_deletion_vector`], bounded by explicit limits.
///
/// The Roaring form is a compression, so a few bytes of run containers can
/// describe billions of positions; the decoded positions are capped at
/// `limits.max_input_bytes()` of memory (eight bytes per position), the same
/// ceiling the other codecs place on a decompressed payload.
///
/// # Errors
///
/// Returns an error when the framing or the vector is invalid, or when the
/// decoded positions exceed the byte budget.
pub fn read_deletion_vector_with_limits(bytes: &[u8], limits: Limits) -> Result<Vec<u64>> {
    if bytes.len() < FRAMING_BYTES {
        return Err(invalid(format_smolstr!(
            "expected at least {FRAMING_BYTES} bytes of deletion-vector framing, got {}",
            bytes.len()
        )));
    }
    let combined = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    let content = bytes.len() - 8;
    if combined as usize != content {
        return Err(invalid(format_smolstr!(
            "expected a combined magic-and-vector length of {content}, got {combined}"
        )));
    }
    if bytes[4..8] != DELETION_VECTOR_MAGIC {
        return Err(codec(
            4,
            format_smolstr!(
                "expected deletion-vector magic d1d33964, got {:02x}{:02x}{:02x}{:02x}",
                bytes[4],
                bytes[5],
                bytes[6],
                bytes[7]
            ),
        ));
    }
    let vector = &bytes[8..bytes.len() - 4];
    let mut crc = flate2::Crc::new();
    crc.update(&bytes[4..bytes.len() - 4]);
    let stored = u32::from_be_bytes([
        bytes[bytes.len() - 4],
        bytes[bytes.len() - 3],
        bytes[bytes.len() - 2],
        bytes[bytes.len() - 1],
    ]);
    if crc.sum() != stored {
        return Err(codec(
            bytes.len() - 4,
            format_smolstr!("expected CRC-32 {:#010x}, got {:#010x}", crc.sum(), stored),
        ));
    }
    let mut cursor = Cursor {
        bytes: vector,
        // Positions in errors are blob-relative: the vector starts at byte 8.
        base: 8,
        position: 0,
    };
    let positions = decode_portable_64(&mut cursor, limits)?;
    if cursor.position != vector.len() {
        return Err(codec(
            8 + cursor.position,
            format_smolstr!(
                "expected the vector to end at byte {}, got {} trailing bytes",
                8 + cursor.position,
                vector.len() - cursor.position
            ),
        ));
    }
    Ok(positions)
}

/// Reject unordered or sign-bit positions before anything is encoded.
fn validate_positions(positions: &[u64]) -> Result<()> {
    for (index, window) in positions.windows(2).enumerate() {
        if window[1] <= window[0] {
            return Err(invalid(format_smolstr!(
                "expected strictly increasing positions, got {} after {} at index {}",
                window[1],
                window[0],
                index + 1
            )));
        }
    }
    if let Some(position) = positions
        .iter()
        .find(|position| **position > i64::MAX as u64)
    {
        return Err(invalid(format_smolstr!(
            "expected a position with a zero most significant bit, got {position}"
        )));
    }
    Ok(())
}

/// Encode sorted positions as the portable 64-bit Roaring serialization.
///
/// The caller has validated ordering, so bucketing is one linear pass.
pub(crate) fn encode_portable_64(positions: &[u64]) -> Vec<u8> {
    // Split into buckets of equal high 32 bits. The slice is sorted, so each
    // bucket is one contiguous window; what is held is one bucket's low words,
    // bounded by the 32-bit value space.
    let mut buckets: Vec<(u32, Vec<u32>)> = Vec::new();
    for position in positions {
        let key = u32::try_from(position >> 32).unwrap_or(u32::MAX);
        let low = u32::try_from(position & 0xFFFF_FFFF).unwrap_or(u32::MAX);
        match buckets.last_mut() {
            Some((last, values)) if *last == key => values.push(low),
            _ => buckets.push((key, vec![low])),
        }
    }
    let mut output = Vec::new();
    output.extend_from_slice(&(buckets.len() as u64).to_le_bytes());
    for (key, values) in &buckets {
        output.extend_from_slice(&key.to_le_bytes());
        encode_portable_32(values, true, &mut output);
    }
    output
}

/// One 16-bit container's chosen serialized shape.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ContainerKind {
    Array,
    Bitset,
    Run,
}

/// One container of a 32-bit bitmap: its high key and its sorted low values.
struct Container<'values> {
    key: u16,
    values: &'values [u32],
    kind: ContainerKind,
    runs: usize,
}

impl Container<'_> {
    /// The exact byte size of this container's serialized body.
    fn serialized_len(&self) -> usize {
        match self.kind {
            ContainerKind::Array => 2 * self.values.len(),
            ContainerKind::Bitset => BITSET_BYTES,
            ContainerKind::Run => 2 + 4 * self.runs,
        }
    }
}

/// Count the maximal runs of consecutive values in a sorted slice.
fn count_runs(values: &[u32]) -> usize {
    let mut runs = 0;
    let mut previous: Option<u32> = None;
    for value in values {
        if previous != Some(value.wrapping_sub(1)) {
            runs += 1;
        }
        previous = Some(*value);
    }
    runs
}

/// Encode one standard 32-bit Roaring bitmap from sorted distinct values.
///
/// `allow_runs` exists so a caller can force the `SERIAL_COOKIE_NO_RUNCONTAINER`
/// form; the production path always allows runs and lets each container's
/// sizes decide, which is exactly the reference implementations' `runOptimize`.
pub(crate) fn encode_portable_32(values: &[u32], allow_runs: bool, output: &mut Vec<u8>) {
    // Split into 16-bit containers and choose each one's cheapest shape.
    let mut containers: Vec<Container<'_>> = Vec::new();
    let mut start = 0;
    while start < values.len() {
        let key = (values[start] >> 16) as u16;
        let mut end = start;
        while end < values.len() && (values[end] >> 16) as u16 == key {
            end += 1;
        }
        let slice = &values[start..end];
        let runs = count_runs(slice);
        let run_len = 2 + 4 * runs;
        let flat_len = if slice.len() <= ARRAY_MAX_CARDINALITY {
            2 * slice.len()
        } else {
            BITSET_BYTES
        };
        let kind = if allow_runs && run_len < flat_len {
            ContainerKind::Run
        } else if slice.len() <= ARRAY_MAX_CARDINALITY {
            ContainerKind::Array
        } else {
            ContainerKind::Bitset
        };
        containers.push(Container {
            key,
            values: slice,
            kind,
            runs,
        });
        start = end;
    }

    let has_run = containers
        .iter()
        .any(|container| container.kind == ContainerKind::Run);
    let size = containers.len();
    let mut body_offset;
    if has_run {
        let cookie = SERIAL_COOKIE | ((size as u32 - 1) << 16);
        output.extend_from_slice(&cookie.to_le_bytes());
        let mut run_bitset = vec![0_u8; size.div_ceil(8)];
        for (index, container) in containers.iter().enumerate() {
            if container.kind == ContainerKind::Run {
                run_bitset[index / 8] |= 1 << (index % 8);
            }
        }
        output.extend_from_slice(&run_bitset);
        body_offset = 4 + run_bitset.len() + 4 * size;
        if size >= NO_OFFSET_THRESHOLD {
            body_offset += 4 * size;
        }
    } else {
        output.extend_from_slice(&SERIAL_COOKIE_NO_RUNCONTAINER.to_le_bytes());
        output.extend_from_slice(&(size as u32).to_le_bytes());
        body_offset = 4 + 4 + 4 * size + 4 * size;
    }
    // Descriptive header: key and cardinality minus one per container.
    for container in &containers {
        output.extend_from_slice(&container.key.to_le_bytes());
        output.extend_from_slice(&((container.values.len() as u32 - 1) as u16).to_le_bytes());
    }
    // Offset header: byte offsets from the cookie, present without runs
    // always and with runs only at NO_OFFSET_THRESHOLD containers or more.
    if !has_run || size >= NO_OFFSET_THRESHOLD {
        let mut offset = body_offset;
        for container in &containers {
            output.extend_from_slice(&(offset as u32).to_le_bytes());
            offset += container.serialized_len();
        }
    }
    for container in &containers {
        match container.kind {
            ContainerKind::Array => {
                for value in container.values {
                    output.extend_from_slice(&((value & 0xFFFF) as u16).to_le_bytes());
                }
            }
            ContainerKind::Bitset => {
                let mut bitset = [0_u8; BITSET_BYTES];
                for value in container.values {
                    let low = (value & 0xFFFF) as usize;
                    bitset[low / 8] |= 1 << (low % 8);
                }
                output.extend_from_slice(&bitset);
            }
            ContainerKind::Run => {
                output.extend_from_slice(&(container.runs as u16).to_le_bytes());
                let mut index = 0;
                while index < container.values.len() {
                    let run_start = container.values[index];
                    let mut end = index;
                    while end + 1 < container.values.len()
                        && container.values[end + 1] == container.values[end] + 1
                    {
                        end += 1;
                    }
                    let length_minus_one = container.values[end] - run_start;
                    output.extend_from_slice(&((run_start & 0xFFFF) as u16).to_le_bytes());
                    output.extend_from_slice(&((length_minus_one & 0xFFFF) as u16).to_le_bytes());
                    index = end + 1;
                }
            }
        }
    }
}

/// A little-endian read cursor whose error positions are blob-relative.
pub(crate) struct Cursor<'bytes> {
    bytes: &'bytes [u8],
    /// Offset of `bytes[0]` within the enclosing blob, for error positions.
    base: usize,
    position: usize,
}

impl<'bytes> Cursor<'bytes> {
    /// A cursor over a bare vector, for decoding outside blob framing.
    #[cfg(test)]
    pub(crate) fn over(bytes: &'bytes [u8]) -> Self {
        Self {
            bytes,
            base: 0,
            position: 0,
        }
    }

    /// The number of bytes consumed so far.
    #[cfg(test)]
    pub(crate) fn consumed(&self) -> usize {
        self.position
    }

    fn take(&mut self, length: usize) -> Result<&'bytes [u8]> {
        let remaining = self.bytes.len() - self.position;
        if remaining < length {
            return Err(codec(
                self.base + self.position,
                format_smolstr!("expected {length} bytes, got {remaining}"),
            ));
        }
        let slice = &self.bytes[self.position..self.position + length];
        self.position += length;
        Ok(slice)
    }

    fn u16(&mut self) -> Result<u16> {
        let bytes = self.take(2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    fn u32(&mut self) -> Result<u32> {
        let bytes = self.take(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn u64(&mut self) -> Result<u64> {
        let bytes = self.take(8)?;
        let mut fixed = [0_u8; 8];
        fixed.copy_from_slice(bytes);
        Ok(u64::from_le_bytes(fixed))
    }

    fn error(&self, reason: SmolStr) -> Error {
        codec(self.base + self.position, reason)
    }
}

/// Decode the portable 64-bit Roaring serialization to sorted positions.
pub(crate) fn decode_portable_64(cursor: &mut Cursor<'_>, limits: Limits) -> Result<Vec<u64>> {
    let buckets = cursor.u64()?;
    // Eight bytes of memory per decoded position, capped like a decompressed
    // payload: the vector is not preallocated from the untrusted count.
    let budget = limits.max_input_bytes() / 8;
    let mut positions = Vec::new();
    let mut previous_key: Option<u32> = None;
    for _ in 0..buckets {
        let key = cursor.u32()?;
        if let Some(previous) = previous_key {
            if key <= previous {
                return Err(cursor.error(format_smolstr!(
                    "expected bucket keys in increasing order, got {key} after {previous}"
                )));
            }
        }
        previous_key = Some(key);
        decode_portable_32(cursor, u64::from(key) << 32, budget, &mut positions)?;
    }
    Ok(positions)
}

/// Decode one standard 32-bit Roaring bitmap, offsetting values by `base`.
pub(crate) fn decode_portable_32(
    cursor: &mut Cursor<'_>,
    base: u64,
    budget: usize,
    positions: &mut Vec<u64>,
) -> Result<()> {
    let cookie = cursor.u32()?;
    let (size, run_bitset) = if cookie == SERIAL_COOKIE_NO_RUNCONTAINER {
        (cursor.u32()? as usize, None)
    } else if cookie & 0xFFFF == SERIAL_COOKIE {
        let size = (cookie >> 16) as usize + 1;
        (size, Some(cursor.take(size.div_ceil(8))?))
    } else {
        return Err(cursor.error(format_smolstr!(
            "expected Roaring cookie {SERIAL_COOKIE_NO_RUNCONTAINER} or {SERIAL_COOKIE}, got {}",
            cookie & 0xFFFF
        )));
    };
    if size > 65536 {
        return Err(cursor.error(format_smolstr!(
            "expected at most 65536 containers, got {size}"
        )));
    }
    // Descriptive header: key and cardinality minus one per container.
    let mut descriptors = Vec::with_capacity(size.min(4096));
    let mut previous_key: Option<u16> = None;
    for _ in 0..size {
        let key = cursor.u16()?;
        let cardinality = cursor.u16()? as usize + 1;
        if let Some(previous) = previous_key {
            if key <= previous {
                return Err(cursor.error(format_smolstr!(
                    "expected container keys in increasing order, got {key} after {previous}"
                )));
            }
        }
        previous_key = Some(key);
        descriptors.push((key, cardinality));
    }
    // Offset header: a random-access aid this sequential decode does not
    // need, so it is consumed without being consulted.
    let has_run = run_bitset.is_some();
    if !has_run || size >= NO_OFFSET_THRESHOLD {
        cursor.take(4 * size)?;
    }
    for (index, (key, cardinality)) in descriptors.iter().enumerate() {
        if positions.len() + cardinality > budget {
            return Err(cursor.error(format_smolstr!(
                "expected at most {budget} decoded positions within the limits' byte budget, got more"
            )));
        }
        let container_base = base | (u64::from(*key) << 16);
        let is_run = run_bitset.is_some_and(|bitset| bitset[index / 8] & (1 << (index % 8)) != 0);
        if is_run {
            decode_run_container(cursor, container_base, *cardinality, positions)?;
        } else if *cardinality <= ARRAY_MAX_CARDINALITY {
            decode_array_container(cursor, container_base, *cardinality, positions)?;
        } else {
            decode_bitset_container(cursor, container_base, *cardinality, positions)?;
        }
    }
    Ok(())
}

/// Decode one array container: sorted 16-bit values.
fn decode_array_container(
    cursor: &mut Cursor<'_>,
    base: u64,
    cardinality: usize,
    positions: &mut Vec<u64>,
) -> Result<()> {
    let mut previous: Option<u16> = None;
    for _ in 0..cardinality {
        let value = cursor.u16()?;
        if let Some(previous) = previous {
            if value <= previous {
                return Err(cursor.error(format_smolstr!(
                    "expected array values in increasing order, got {value} after {previous}"
                )));
            }
        }
        previous = Some(value);
        positions.push(base | u64::from(value));
    }
    Ok(())
}

/// Decode one bitset container: 8 KiB of 64-bit words, low bit first.
fn decode_bitset_container(
    cursor: &mut Cursor<'_>,
    base: u64,
    cardinality: usize,
    positions: &mut Vec<u64>,
) -> Result<()> {
    let start = cursor.base + cursor.position;
    let bitset = cursor.take(BITSET_BYTES)?;
    let before = positions.len();
    for (byte_index, byte) in bitset.iter().enumerate() {
        let mut bits = *byte;
        while bits != 0 {
            let bit = bits.trailing_zeros();
            positions.push(base | ((byte_index as u64) * 8 + u64::from(bit)));
            bits &= bits - 1;
        }
    }
    let decoded = positions.len() - before;
    if decoded != cardinality {
        return Err(codec(
            start,
            format_smolstr!("expected a bitset of cardinality {cardinality}, got {decoded}"),
        ));
    }
    Ok(())
}

/// Decode one run container: a run count, then sorted non-overlapping runs.
fn decode_run_container(
    cursor: &mut Cursor<'_>,
    base: u64,
    cardinality: usize,
    positions: &mut Vec<u64>,
) -> Result<()> {
    let start = cursor.base + cursor.position;
    let runs = cursor.u16()? as usize;
    let mut decoded = 0_usize;
    let mut previous_end: Option<u32> = None;
    for _ in 0..runs {
        let run_start = u32::from(cursor.u16()?);
        let length_minus_one = u32::from(cursor.u16()?);
        if let Some(previous) = previous_end {
            if run_start <= previous {
                return Err(cursor.error(format_smolstr!(
                    "expected non-overlapping sorted runs, got a run starting at {run_start} after one ending at {previous}"
                )));
            }
        }
        let end = run_start + length_minus_one;
        if end > 0xFFFF {
            return Err(cursor.error(format_smolstr!(
                "expected a run within the 16-bit value space, got {run_start}..={end}"
            )));
        }
        previous_end = Some(end);
        for value in run_start..=end {
            positions.push(base | u64::from(value));
        }
        decoded += length_minus_one as usize + 1;
    }
    if decoded != cardinality {
        return Err(codec(
            start,
            format_smolstr!("expected runs of total cardinality {cardinality}, got {decoded}"),
        ));
    }
    Ok(())
}

/// Report a malformed deletion vector at a byte position within the blob.
pub(crate) fn codec(position: usize, reason: SmolStr) -> Error {
    Error::Codec {
        format: "puffin",
        position,
        reason,
    }
}

/// Report a malformed deletion vector whose position is the blob itself.
pub(crate) fn invalid(reason: SmolStr) -> Error {
    codec(0, reason)
}
