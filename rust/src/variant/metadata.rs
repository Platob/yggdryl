//! The Variant metadata dictionary: the field names an object spells by id.
//!
//! Encoding builds a [`Dictionary`] in one walk over the tree - every object
//! field name, deduplicated and sorted lexicographically by unsigned byte
//! order - and writes it with the `sorted_strings` header bit set and the
//! smallest offset width that holds it. Decoding parses a [`MetadataView`]
//! that indexes strings by field id and makes no ordering assumption at all,
//! so a dictionary an older writer left unsorted reads the same way.

use smol_str::{SmolStr, format_smolstr};

use crate::{Limits, Result, Value};

use super::{METADATA_VERSION, byte_width, byte_word, codec, push_unsigned};

/// The sorted, deduplicated field-name dictionary one encode pass writes.
pub(super) struct Dictionary {
    /// Every object field name in the tree, unique, in unsigned-byte
    /// lexicographic order, so a name's index is its field id.
    names: Vec<SmolStr>,
}

impl Dictionary {
    /// Collect every object field name below `value`.
    ///
    /// Only names contribute here: a mapping key that is not a string, a
    /// depth past the encoder's bound, and every other refusal is reported by
    /// the encode walk itself, which carries the `$`-rooted path this
    /// collection pass does not need.
    pub(super) fn build(value: &Value) -> Self {
        let mut names = Vec::new();
        collect_names(value, &mut names);
        names.sort_unstable();
        names.dedup();
        Self { names }
    }

    /// Return the field id of a collected name.
    ///
    /// The encode walk only asks for names [`Self::build`] collected from the
    /// same tree, so absence is an implementation defect, not caller input.
    pub(super) fn id(&self, name: &str) -> u32 {
        let index = self
            .names
            .binary_search_by(|candidate| candidate.as_str().cmp(name))
            .expect("every object field name was collected before encoding");
        u32::try_from(index).expect("the dictionary was bounded at construction")
    }

    /// Spell this dictionary as Variant metadata bytes: the header byte, the
    /// dictionary size, `size + 1` offsets, then the name bytes, all at the
    /// smallest offset width that holds them.
    pub(super) fn encode(&self) -> Result<Vec<u8>> {
        let total: usize = self.names.iter().map(SmolStr::len).sum();
        let total = u32::try_from(total).map_err(|_| {
            codec(
                0,
                format_smolstr!(
                    "expected at most {} dictionary bytes, got {}",
                    u32::MAX,
                    self.names.iter().map(SmolStr::len).sum::<usize>()
                ),
            )
        })?;
        let size = u32::try_from(self.names.len()).map_err(|_| {
            codec(
                0,
                format_smolstr!(
                    "expected at most {} dictionary names, got {}",
                    u32::MAX,
                    self.names.len()
                ),
            )
        })?;
        let offset_size = byte_width(total.max(size));
        let offset_size_minus_one =
            u8::try_from(offset_size - 1).expect("an offset width is between one and four bytes");
        let mut bytes =
            Vec::with_capacity(1 + offset_size * (self.names.len() + 2) + total as usize);
        // Header: version, sorted_strings (this encoder always sorts), and
        // the offset width. VariantEncoding.md "Metadata encoding".
        bytes.push(METADATA_VERSION | 1 << 4 | offset_size_minus_one << 6);
        push_unsigned(&mut bytes, size, offset_size);
        let mut offset = 0_u32;
        push_unsigned(&mut bytes, offset, offset_size);
        for name in &self.names {
            offset += u32::try_from(name.len()).expect("the total length fit u32");
            push_unsigned(&mut bytes, offset, offset_size);
        }
        for name in &self.names {
            bytes.extend_from_slice(name.as_bytes());
        }
        Ok(bytes)
    }
}

/// Push every object field name below `value`, depth-first.
fn collect_names(value: &Value, names: &mut Vec<SmolStr>) {
    match value {
        Value::Mapping(entries) => {
            for (key, child) in entries.iter() {
                if let Value::String(name) = key {
                    names.push(name.clone());
                }
                collect_names(child, names);
            }
        }
        Value::Record(data_type, values) => {
            if let Some(fields) = data_type.as_fields() {
                for field in fields.iter() {
                    names.push(SmolStr::new(field.name()));
                }
            }
            for child in values.iter() {
                collect_names(child, names);
            }
        }
        Value::Sequence(values) => {
            for child in values.iter() {
                collect_names(child, names);
            }
        }
        _ => {}
    }
}

/// The parsed dictionary one decode pass reads field names from.
pub(super) struct MetadataView<'bytes> {
    /// The dictionary strings, indexed by field id, in written order.
    strings: Vec<&'bytes str>,
}

impl<'bytes> MetadataView<'bytes> {
    /// Parse Variant metadata bytes, validating the version, the offset
    /// monotonicity, and every string's UTF-8 before any value is decoded.
    ///
    /// The `sorted_strings` bit is read past rather than trusted: lookups
    /// index by field id either way, so an unsorted dictionary from an older
    /// writer decodes identically.
    pub(super) fn parse(bytes: &'bytes [u8], limits: Limits) -> Result<Self> {
        let Some(&header) = bytes.first() else {
            return Err(codec(
                0,
                SmolStr::new_static("expected 1 byte of metadata header, got 0"),
            ));
        };
        let version = header & 0x0F;
        if version != METADATA_VERSION {
            return Err(codec(
                0,
                format_smolstr!("expected metadata version {METADATA_VERSION}, got {version}"),
            ));
        }
        let offset_size = usize::from(header >> 6) + 1;
        let mut position = 1;
        let size = read_unsigned(bytes, &mut position, offset_size, "dictionary size")?;
        let size = usize::try_from(size).unwrap_or(usize::MAX);
        if size >= limits.max_nodes() {
            return Err(codec(
                position,
                format_smolstr!(
                    "expected a dictionary of at most {} names, got {size}",
                    limits.max_nodes()
                ),
            ));
        }
        // Bound the promised offset list against the bytes present before
        // allocating for it: `size + 1` offsets of `offset_size` bytes each.
        let offsets_len = (size + 1) * offset_size;
        let remaining = bytes.len() - position;
        if offsets_len > remaining {
            return Err(codec(
                position,
                format_smolstr!(
                    "expected {offsets_len} {} of dictionary offsets, got {remaining}",
                    byte_word(offsets_len)
                ),
            ));
        }
        let offsets_start = position;
        let mut offsets = Vec::with_capacity(size + 1);
        for _ in 0..=size {
            offsets.push(read_unsigned(
                bytes,
                &mut position,
                offset_size,
                "dictionary offset",
            )?);
        }
        let names_bytes = &bytes[position..];
        let total = u64::try_from(names_bytes.len()).unwrap_or(u64::MAX);
        let last = *offsets.last().expect("one more offset than names was read");
        if last != total {
            return Err(codec(
                offsets_start + size * offset_size,
                format_smolstr!("expected a final dictionary offset of {total}, got {last}"),
            ));
        }
        let mut strings = Vec::with_capacity(size);
        for (index, pair) in offsets.windows(2).enumerate() {
            let [start, end] = pair else { unreachable!() };
            // Bound both halves before slicing: an offset past the name
            // bytes, or a decreasing pair, never reaches the byte region.
            let end_position = offsets_start + (index + 1) * offset_size;
            if *end > total {
                return Err(codec(
                    end_position,
                    format_smolstr!("expected a dictionary offset of at most {total}, got {end}"),
                ));
            }
            if start > end {
                return Err(codec(
                    end_position,
                    format_smolstr!(
                        "expected non-decreasing dictionary offsets, got {start} then {end}"
                    ),
                ));
            }
            let start = usize::try_from(*start).expect("the final offset bounded every offset");
            let end = usize::try_from(*end).expect("the final offset bounded every offset");
            let name = std::str::from_utf8(&names_bytes[start..end]).map_err(|error| {
                codec(
                    position + start + error.valid_up_to(),
                    format_smolstr!(
                        "expected UTF-8 dictionary string bytes, got an invalid sequence"
                    ),
                )
            })?;
            strings.push(name);
        }
        Ok(Self { strings })
    }

    /// The number of dictionary strings.
    pub(super) fn len(&self) -> usize {
        self.strings.len()
    }

    /// The string a field id names, when the id is in range.
    pub(super) fn get(&self, id: usize) -> Option<&'bytes str> {
        self.strings.get(id).copied()
    }
}

/// Read one unsigned little-endian value of `width` bytes, advancing
/// `position` and naming `what` when the bytes run out.
fn read_unsigned(
    bytes: &[u8],
    position: &mut usize,
    width: usize,
    what: &'static str,
) -> Result<u64> {
    let Some(taken) = bytes.get(*position..*position + width) else {
        return Err(codec(
            *position,
            format_smolstr!(
                "expected {width} {} of {what}, got {}",
                byte_word(width),
                bytes.len().saturating_sub(*position)
            ),
        ));
    };
    *position += width;
    let mut value = 0_u64;
    for (index, byte) in taken.iter().enumerate() {
        value |= u64::from(*byte) << (8 * index);
    }
    Ok(value)
}
