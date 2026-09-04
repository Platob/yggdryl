//! The one place two schemas become one.
//!
//! [`DataType::merge_with`] is the whole rule table, and [`crate::Field`]'s own
//! merge is a thin layer over it: every promotion, every recursion into a
//! nested layout, and every refusal lives here so two callers reading the same
//! pair of types can never disagree about what they meet at.
//!
//! The rules, in the order they are tried:
//!
//! 1. Two equal types are that type.
//! 2. [`DataType::Null`] yields to whatever is defined beside it, in either
//!    position, so a column inferred as all-null takes the shape the other
//!    side gives it.
//! 3. Two nested layouts of the same family recurse into their children. A
//!    struct takes the *union* of its fields; a list, map, or run-end node
//!    merges the children it has.
//! 4. Bytes win. A binary type paired with anything else answers binary,
//!    because bytes are the container every other encoding fits inside. An
//!    ASCII width beside a fixed binary of the same byte width answers that
//!    fixed binary, identical storage; any other pairing is variable bytes.
//! 5. Text wins next, over numbers and temporals. Two ASCII widths meet at
//!    the wider or the narrower; ASCII beside variable text meets at the
//!    variable text when widening and at the ASCII width when narrowing;
//!    text absorbing a non-text side is at least `utf8`, because a number's
//!    rendering does not fit four bytes.
//! 6. Numbers meet by width, and temporals by unit.
//!
//! Anything left is an honest refusal rather than a lossy guess: a boolean and
//! a timestamp have no meeting point that is not a re-encoding.
//!
//! `upscale` picks the direction width is resolved in. Widening is the default
//! and is lossless - `int32` and `int64` meet at `int64`. Narrowing is the
//! deliberate opposite, for a caller who wants the tightest type that names
//! both and accepts that stored values may not fit it.

use smol_str::format_smolstr;

use crate::generic::{TimeUnit, UnionMode};
use crate::{DataType, Error, Field, Result};

/// Whether a pair with no shared family may meet by being re-encoded.
///
/// Answering `Utf8` for an integer beside a string is the right call when two
/// schemas are being unioned - text is the container both fit in. It is the
/// wrong call when a type is being *inferred* from values, or a comparison
/// typed, because there the answer asserts something about the data rather
/// than about the types: `1` and `"AAPL"` become two strings only if someone
/// decides they should.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Recode {
    /// Bytes and text may absorb a type from another family.
    Allowed,
    /// Only types that already share a family meet.
    Refused,
}

/// How the width of two otherwise-compatible types is resolved.
///
/// Passed as a plain `bool` at the boundary, where `true` is widening, so the
/// bindings can spell it `upscale=True` without inventing a second vocabulary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Widening {
    /// Meet at the type that holds both, losing nothing.
    Up,
    /// Meet at the tightest type that names both, which may not hold every
    /// value either side could carry.
    Down,
}

impl Widening {
    /// Read the boundary's `upscale` flag.
    pub const fn upscale(upscale: bool) -> Self {
        if upscale { Self::Up } else { Self::Down }
    }

    /// Pick between two ranked candidates.
    fn pick<T>(self, left: (u8, T), right: (u8, T)) -> T {
        let take_left = match self {
            Self::Up => left.0 >= right.0,
            Self::Down => left.0 <= right.0,
        };
        if take_left { left.1 } else { right.1 }
    }
}

impl DataType {
    /// Returns the datatype that holds both this one and `other`.
    ///
    /// The rules are tried in order: two equal types are that type; [`Null`]
    /// yields to whatever is defined beside it; two nested layouts of the same
    /// family recurse, a struct taking the *union* of its fields; bytes win
    /// over everything, because every other encoding fits inside them; text
    /// wins next, ASCII widths meeting by width and absorbing a non-text
    /// side at no less than `utf8`; and numbers meet by width, temporals by
    /// unit. Anything left is refused rather than guessed - a boolean and a
    /// timestamp have no meeting point that is not a re-encoding.
    ///
    /// `upscale` chooses the direction width is resolved in: `true` meets at
    /// the type that holds both, `false` at the tightest type that names both.
    ///
    /// [`Null`]: Self::Null
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// // Numbers meet by width, in the direction asked for.
    /// assert_eq!(DataType::Int32.merge_with(&DataType::Int64, true)?, DataType::Int64);
    /// assert_eq!(DataType::Int32.merge_with(&DataType::Int64, false)?, DataType::Int32);
    ///
    /// // Null yields to whatever is defined beside it.
    /// assert_eq!(DataType::Null.merge_with(&DataType::Utf8, true)?, DataType::Utf8);
    ///
    /// // Bytes win over text, and text over numbers.
    /// assert_eq!(DataType::Utf8.merge_with(&DataType::Binary, true)?, DataType::Binary);
    /// assert_eq!(DataType::Int64.merge_with(&DataType::Utf8, true)?, DataType::Utf8);
    ///
    /// // ASCII widths are text, so they meet variable text there when widening.
    /// assert_eq!(DataType::Ascii32.merge_with(&DataType::Utf8, true)?, DataType::Utf8);
    /// assert_eq!(DataType::Ascii32.merge_with(&DataType::Ascii64, false)?, DataType::Ascii32);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error naming both sides when they have no meeting point that
    /// is not a re-encoding, and when a merged child fails its own validation.
    pub fn merge_with(&self, other: &Self, upscale: bool) -> Result<Self> {
        self.merge(other, Widening::upscale(upscale), Recode::Allowed)
    }

    /// Returns the datatype both share without re-encoding either.
    ///
    /// The same table as [`Self::merge_with`] up to the point where it would
    /// answer text or bytes for a pair that is neither. A caller inferring a
    /// type from values, or typing a comparison, wants that refusal: turning
    /// `1` and `"AAPL"` into two strings is a guess about the data, not a fact
    /// about the types.
    ///
    /// # Errors
    ///
    /// Returns an error when the two have no common type that re-encodes
    /// nothing.
    pub(crate) fn merge_exact(&self, other: &Self, how: Widening) -> Result<Self> {
        self.merge(other, how, Recode::Refused)
    }

    /// The recursive worker behind [`Self::merge_with`], shared with
    /// [`Field::merge`] so a nested child never takes a different path.
    pub(crate) fn merge(&self, other: &Self, how: Widening, recode: Recode) -> Result<Self> {
        if self == other {
            return Ok(self.clone());
        }
        // A null column has no shape of its own, so it takes the other's.
        if matches!(self, Self::Null) {
            return Ok(other.clone());
        }
        if matches!(other, Self::Null) {
            return Ok(self.clone());
        }
        if let Some(merged) = merge_encoded(self, other, how, recode)? {
            return Ok(merged);
        }
        if let Some(merged) = merge_nested(self, other, how, recode)? {
            return Ok(merged);
        }
        if let Some(merged) = merge_scalar(self, other, how, recode)? {
            return Ok(merged);
        }
        Err(unmergeable(self, other))
    }
}

/// Merge through a dictionary or run-end wrapper.
///
/// An encoding is a physical choice, not a logical type, so the values are
/// merged and the encoding is kept only where both sides had one. Keeping it
/// otherwise would impose a layout the other side never asked for.
fn merge_encoded(
    left: &DataType,
    right: &DataType,
    how: Widening,
    recode: Recode,
) -> Result<Option<DataType>> {
    match (left, right) {
        (DataType::Dictionary(left_dict), DataType::Dictionary(right_dict)) => {
            let key = left_dict.key().merge(right_dict.key(), how, recode)?;
            let value = left_dict.value().merge(right_dict.value(), how, recode)?;
            DataType::dictionary(key, value).map(Some)
        }
        (DataType::RunEndEncoded(left_run), DataType::RunEndEncoded(right_run)) => {
            let run_ends = left_run
                .run_ends()
                .merge(right_run.run_ends(), how, recode)?;
            let values = left_run.values().merge(right_run.values(), how, recode)?;
            DataType::run_end_encoded(run_ends, values).map(Some)
        }
        // One side encoded and the other not: the logical types meet, and the
        // result is plain, because an encoding one side never had is not
        // something a merge may impose.
        (DataType::Dictionary(_) | DataType::RunEndEncoded(_), _) => {
            decoded(left).merge(right, how, recode).map(Some)
        }
        (_, DataType::Dictionary(_) | DataType::RunEndEncoded(_)) => {
            left.merge(decoded(right), how, recode).map(Some)
        }
        _ => Ok(None),
    }
}

/// The logical type under any number of encoding wrappers.
fn decoded(dtype: &DataType) -> &DataType {
    match dtype {
        DataType::Dictionary(dictionary) => decoded(dictionary.value()),
        DataType::RunEndEncoded(encoded) => decoded(encoded.values().dtype()),
        other => other,
    }
}

/// Merge two nested layouts of the same family, recursing into their children.
fn merge_nested(
    left: &DataType,
    right: &DataType,
    how: Widening,
    recode: Recode,
) -> Result<Option<DataType>> {
    match (left, right) {
        (DataType::Struct(left_fields), DataType::Struct(right_fields)) => {
            merge_struct(left_fields.as_ref(), right_fields.as_ref(), how, recode).map(Some)
        }
        (DataType::Map(left_map), DataType::Map(right_map)) => {
            let entries = left_map.entries().merge(right_map.entries(), how, recode)?;
            // Sorted keys are only a promise the merged map can keep if both
            // sides made it.
            DataType::map(entries, left_map.keys_sorted() && right_map.keys_sorted()).map(Some)
        }
        (DataType::Union(left_members, left_mode), DataType::Union(right_members, right_mode)) => {
            merge_union(
                left_members,
                *left_mode,
                right_members,
                *right_mode,
                how,
                recode,
            )
            .map(Some)
        }
        _ => match (list_parts(left), list_parts(right)) {
            (
                Some((left_rank, left_item, left_size)),
                Some((right_rank, right_item, right_size)),
            ) => {
                let item = left_item.merge(right_item, how, recode)?;
                // A fixed size survives only when both sides fix the same one;
                // otherwise the pair is a variable list.
                let rank = if left_size == right_size {
                    how.pick((left_rank, left_rank), (right_rank, right_rank))
                } else {
                    how.pick(
                        (left_rank.max(1), left_rank.max(1)),
                        (right_rank.max(1), right_rank.max(1)),
                    )
                };
                rebuild_list(
                    rank,
                    item,
                    left_size.filter(|size| Some(*size) == right_size),
                )
                .map(Some)
            }
            _ => Ok(None),
        },
    }
}

/// Merge two structs by taking the union of their fields.
///
/// A name both sides carry is merged; a name only one side carries is added
/// and becomes nullable, because the rows the other side described do not have
/// it. Order is the receiver's, then whatever `other` adds, so a merge never
/// reorders columns a caller already depends on.
fn merge_struct(
    left: &[Field],
    right: &[Field],
    how: Widening,
    recode: Recode,
) -> Result<DataType> {
    let mut merged: Vec<Field> = Vec::with_capacity(left.len() + right.len());
    for field in left {
        match right.iter().find(|held| held.name() == field.name()) {
            Some(counterpart) => merged.push(field.merge(counterpart, how, recode)?),
            None => merged.push(optional(field)),
        }
    }
    for field in right {
        if !left.iter().any(|held| held.name() == field.name()) {
            merged.push(optional(field));
        }
    }
    DataType::from_fields(merged)
}

/// The same field, but nullable, because one side never described it.
fn optional(field: &Field) -> Field {
    if field.is_nullable() {
        field.clone()
    } else {
        let mut field = field.clone();
        field.set_nullable(true);
        field
    }
}

/// Merge two unions by taking the union of their members, matched by type id.
fn merge_union(
    left: &crate::datatype::UnionFields,
    left_mode: UnionMode,
    right: &crate::datatype::UnionFields,
    right_mode: UnionMode,
    how: Widening,
    recode: Recode,
) -> Result<DataType> {
    let mut merged: Vec<(i8, Field)> = Vec::new();
    for (id, field) in left.iter() {
        match right.iter().find(|(held, _)| *held == id) {
            Some((_, counterpart)) => merged.push((id, field.merge(counterpart, how, recode)?)),
            None => merged.push((id, field.clone())),
        }
    }
    for (id, field) in right.iter() {
        if !left.iter().any(|(held, _)| held == id) {
            merged.push((id, field.clone()));
        }
    }
    // A sparse union is the layout that can hold either encoding's members.
    let mode = if left_mode == right_mode {
        left_mode
    } else {
        UnionMode::Sparse
    };
    DataType::union(merged, mode)
}

/// The item field, width rank, and fixed size of a list-shaped layout.
fn list_parts(dtype: &DataType) -> Option<(u8, &Field, Option<i32>)> {
    match dtype {
        DataType::List(item) => Some((0, item, None)),
        DataType::ListView(item) => Some((1, item, None)),
        DataType::FixedSizeList(item, size) => Some((0, item, Some(*size))),
        DataType::LargeList(item) => Some((2, item, None)),
        DataType::LargeListView(item) => Some((3, item, None)),
        _ => None,
    }
}

/// Rebuild a list-shaped layout from a width rank and an item.
fn rebuild_list(rank: u8, item: Field, size: Option<i32>) -> Result<DataType> {
    if let Some(size) = size {
        return DataType::fixed_size_list(item, size);
    }
    Ok(match rank {
        1 => DataType::list_view(item),
        2 => DataType::large_list(item),
        3 => DataType::large_list_view(item),
        _ => DataType::list(item),
    })
}

/// Merge two leaf types: bytes, then text, then numbers, then temporals.
fn merge_scalar(
    left: &DataType,
    right: &DataType,
    how: Widening,
    recode: Recode,
) -> Result<Option<DataType>> {
    // Bytes hold every other encoding, so a binary side decides the pair.
    if let Some(rank) = binary_rank(left) {
        return Ok(Some(match binary_rank(right) {
            Some(other) => rebuild_binary(how.pick((rank, rank), (other, other)), left, right),
            None if recode == Recode::Allowed && is_mergeable_into_bytes(right) => {
                rebuild_binary(rank, left, right)
            }
            None => return Ok(None),
        }));
    }
    if let Some(rank) = binary_rank(right) {
        return Ok(
            if recode == Recode::Allowed && is_mergeable_into_bytes(left) {
                Some(rebuild_binary(rank, left, right))
            } else {
                None
            },
        );
    }
    // Text is next, over numbers and temporals.
    if let Some(rank) = text_rank(left) {
        return Ok(match text_rank(right) {
            Some(other) => Some(rebuild_text(how.pick((rank, rank), (other, other)))),
            None if recode == Recode::Allowed && is_mergeable_into_text(right) => {
                Some(rebuild_text(rank.max(UTF8_RANK)))
            }
            None => None,
        });
    }
    if let Some(rank) = text_rank(right) {
        return Ok(
            if recode == Recode::Allowed && is_mergeable_into_text(left) {
                Some(rebuild_text(rank.max(UTF8_RANK)))
            } else {
                None
            },
        );
    }
    if let Some(merged) = merge_numeric(left, right, how)? {
        return Ok(Some(merged));
    }
    Ok(merge_temporal(left, right, how))
}

/// Whether a type has a byte rendering a merge may fall back to.
fn is_mergeable_into_bytes(dtype: &DataType) -> bool {
    !matches!(
        dtype,
        DataType::Struct(_) | DataType::Union(..) | DataType::Map(_)
    ) && list_parts(dtype).is_none()
}

/// Whether a type has a text rendering a merge may fall back to.
fn is_mergeable_into_text(dtype: &DataType) -> bool {
    is_mergeable_into_bytes(dtype)
}

/// How wide a binary layout is, if it is one.
const fn binary_rank(dtype: &DataType) -> Option<u8> {
    match dtype {
        DataType::FixedSizeBinary(_) => Some(0),
        DataType::Binary => Some(1),
        DataType::BinaryView => Some(2),
        DataType::LargeBinary => Some(3),
        _ => None,
    }
}

/// The byte width of a fixed-width byte layout: a fixed binary or an ASCII
/// width, whose storage is the fixed binary of the same width.
fn fixed_width(dtype: &DataType) -> Option<i32> {
    match dtype {
        DataType::FixedSizeBinary(width) => Some(*width),
        other => other.ascii_width(),
    }
}

/// Rebuild a binary layout from a width rank, keeping a shared fixed width.
fn rebuild_binary(rank: u8, left: &DataType, right: &DataType) -> DataType {
    if let (Some(left_width), Some(right_width)) = (fixed_width(left), fixed_width(right)) {
        if left_width == right_width {
            return DataType::FixedSizeBinary(left_width);
        }
    }
    match rank {
        2 => DataType::BinaryView,
        3 => DataType::LargeBinary,
        // A fixed width the two sides do not share becomes variable bytes.
        _ => DataType::Binary,
    }
}

/// The rank of `utf8`: the narrowest text a non-text side re-encodes into.
const UTF8_RANK: u8 = 6;

/// How wide a text layout is, if it is one.
///
/// The ASCII widths rank below every variable layout, so widening beside
/// variable text answers the variable text and narrowing the ASCII width.
const fn text_rank(dtype: &DataType) -> Option<u8> {
    match dtype {
        DataType::Ascii16 => Some(0),
        DataType::Ascii24 => Some(1),
        DataType::Ascii32 => Some(2),
        DataType::Ascii64 => Some(3),
        DataType::Ascii96 => Some(4),
        DataType::Ascii128 => Some(5),
        DataType::Utf8 => Some(UTF8_RANK),
        DataType::Utf8View => Some(7),
        DataType::LargeUtf8 => Some(8),
        _ => None,
    }
}

/// Rebuild a text layout from a width rank.
const fn rebuild_text(rank: u8) -> DataType {
    match rank {
        0 => DataType::Ascii16,
        1 => DataType::Ascii24,
        2 => DataType::Ascii32,
        3 => DataType::Ascii64,
        4 => DataType::Ascii96,
        5 => DataType::Ascii128,
        7 => DataType::Utf8View,
        8 => DataType::LargeUtf8,
        _ => DataType::Utf8,
    }
}

/// Merge two numbers: decimals, then floats, then integers.
fn merge_numeric(left: &DataType, right: &DataType, how: Widening) -> Result<Option<DataType>> {
    let left_decimal = decimal_parts(left);
    let right_decimal = decimal_parts(right);
    if left_decimal.is_some() || right_decimal.is_some() {
        // A decimal only meets another exact number. Pairing it with a float
        // would trade exactness for range without saying so.
        let (Some(left_parts), Some(right_parts)) = (
            left_decimal.or_else(|| integer_as_decimal(left)),
            right_decimal.or_else(|| integer_as_decimal(right)),
        ) else {
            return Ok(None);
        };
        return merge_decimal(left_parts, right_parts, how).map(Some);
    }
    if let (Some(left_rank), Some(right_rank)) = (float_rank(left), float_rank(right)) {
        return Ok(Some(rebuild_float(
            how.pick((left_rank, left_rank), (right_rank, right_rank)),
        )));
    }
    // A float beside a whole number answers the float, whichever direction
    // width is resolved in: an integer has no fractional part to lose.
    if let Some(rank) = float_rank(left) {
        return Ok(integer_rank(right).map(|_| rebuild_float(rank)));
    }
    if let Some(rank) = float_rank(right) {
        return Ok(integer_rank(left).map(|_| rebuild_float(rank)));
    }
    match (integer_rank(left), integer_rank(right)) {
        (Some(left_rank), Some(right_rank)) => Ok(Some(rebuild_integer(
            how.pick((left_rank, left_rank), (right_rank, right_rank)),
        ))),
        _ => Ok(None),
    }
}

/// The widest decimal that names both, capped at what the backing width holds.
fn merge_decimal(left: (u8, i8), right: (u8, i8), how: Widening) -> Result<DataType> {
    let (left_precision, left_scale) = left;
    let (right_precision, right_scale) = right;
    let scale = match how {
        Widening::Up => left_scale.max(right_scale),
        Widening::Down => left_scale.min(right_scale),
    };
    let integral = left_precision
        .saturating_sub(u8::try_from(left_scale.max(0)).unwrap_or(0))
        .max(right_precision.saturating_sub(u8::try_from(right_scale.max(0)).unwrap_or(0)));
    let precision = integral
        .saturating_add(u8::try_from(scale.max(0)).unwrap_or(0))
        .clamp(1, MAX_DECIMAL_PRECISION);
    DataType::decimal(precision, scale)
}

/// Arrow's widest exact decimal precision.
const MAX_DECIMAL_PRECISION: u8 = 76;

/// A whole number as the decimal that names every value it holds.
const fn integer_as_decimal(dtype: &DataType) -> Option<(u8, i8)> {
    match dtype {
        DataType::Int8 | DataType::UInt8 => Some((3, 0)),
        DataType::Int16 | DataType::UInt16 => Some((5, 0)),
        DataType::Int32 | DataType::UInt32 => Some((10, 0)),
        DataType::Int64 | DataType::UInt64 => Some((20, 0)),
        _ => None,
    }
}

/// The precision and scale of an exact decimal, if it is one.
const fn decimal_parts(dtype: &DataType) -> Option<(u8, i8)> {
    match dtype {
        DataType::Decimal32 { precision, scale }
        | DataType::Decimal64 { precision, scale }
        | DataType::Decimal128 { precision, scale }
        | DataType::Decimal256 { precision, scale } => Some((*precision, *scale)),
        _ => None,
    }
}

/// How wide a float is, if it is one.
const fn float_rank(dtype: &DataType) -> Option<u8> {
    match dtype {
        DataType::Float16 => Some(0),
        DataType::Float32 => Some(1),
        DataType::Float64 => Some(2),
        _ => None,
    }
}

/// Rebuild a float from a width rank.
const fn rebuild_float(rank: u8) -> DataType {
    match rank {
        0 => DataType::Float16,
        1 => DataType::Float32,
        _ => DataType::Float64,
    }
}

/// How wide a whole number is, if it is one.
///
/// Signedness is part of the rank: an unsigned type ranks above the signed one
/// of the same width, because widening to hold both needs the next size up.
const fn integer_rank(dtype: &DataType) -> Option<u8> {
    match dtype {
        DataType::Int8 => Some(0),
        DataType::UInt8 => Some(1),
        DataType::Int16 => Some(2),
        DataType::UInt16 => Some(3),
        DataType::Int32 => Some(4),
        DataType::UInt32 => Some(5),
        DataType::Int64 => Some(6),
        DataType::UInt64 => Some(7),
        _ => None,
    }
}

/// Rebuild a whole number from a width rank.
const fn rebuild_integer(rank: u8) -> DataType {
    match rank {
        0 => DataType::Int8,
        1 => DataType::UInt8,
        2 => DataType::Int16,
        3 => DataType::UInt16,
        4 => DataType::Int32,
        5 => DataType::UInt32,
        6 => DataType::Int64,
        _ => DataType::UInt64,
    }
}

/// Merge two temporals of the same family, meeting at one unit.
fn merge_temporal(left: &DataType, right: &DataType, how: Widening) -> Option<DataType> {
    let (left_family, left_unit) = temporal_parts(left)?;
    let (right_family, right_unit) = temporal_parts(right)?;
    if left_family != right_family {
        return None;
    }
    let unit = how.pick(
        (unit_rank(left_unit), left_unit),
        (unit_rank(right_unit), right_unit),
    );
    Some(match left_family {
        0 => {
            if matches!(left, DataType::Date64) || matches!(right, DataType::Date64) {
                DataType::Date64
            } else {
                DataType::Date32
            }
        }
        1 => DataType::time(unit).ok()?,
        2 => {
            // A zone one side declares is kept: a naive reading of a zoned
            // column loses the offset, which is not a merge but a cast.
            let zone = match (left, right) {
                (DataType::Timestamp(_, Some(zone)), _)
                | (_, DataType::Timestamp(_, Some(zone))) => Some(zone.clone()),
                _ => None,
            };
            DataType::Timestamp(unit, zone)
        }
        _ => {
            if matches!(left, DataType::Duration64(_)) || matches!(right, DataType::Duration64(_)) {
                DataType::Duration64(unit)
            } else {
                DataType::Duration32(unit)
            }
        }
    })
}

/// The temporal family and unit of a datatype, if it has one.
const fn temporal_parts(dtype: &DataType) -> Option<(u8, TimeUnit)> {
    match dtype {
        DataType::Date32 => Some((0, TimeUnit::Day)),
        DataType::Date64 => Some((0, TimeUnit::Millisecond)),
        DataType::Time32(unit) | DataType::Time64(unit) => Some((1, *unit)),
        DataType::Timestamp(unit, _) => Some((2, *unit)),
        DataType::Duration32(unit) | DataType::Duration64(unit) => Some((3, *unit)),
        _ => None,
    }
}

/// How fine a unit is, so two temporals can meet at one of them.
const fn unit_rank(unit: TimeUnit) -> u8 {
    match unit {
        TimeUnit::Day => 0,
        TimeUnit::Second => 1,
        TimeUnit::Millisecond => 2,
        TimeUnit::Microsecond => 3,
        TimeUnit::Nanosecond => 4,
        TimeUnit::YearMonth | TimeUnit::DayTime | TimeUnit::MonthDayNano => 5,
    }
}

/// Report a pair with no meeting point that is not a re-encoding.
fn unmergeable(left: &DataType, right: &DataType) -> Error {
    Error::InvalidDataType {
        kind: "DataType",
        reason: crate::text::expected_got(
            format_smolstr!("a datatype that merges with {left}"),
            format_smolstr!("{right}"),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::Widening;
    use crate::DataType;

    #[test]
    fn ascii_widths_meet_text_by_width_in_the_direction_asked_for() {
        let up = |left: &DataType, right: &DataType| left.merge_with(right, true).unwrap();
        let down = |left: &DataType, right: &DataType| left.merge_with(right, false).unwrap();

        assert_eq!(up(&DataType::Ascii32, &DataType::Utf8), DataType::Utf8);
        assert_eq!(down(&DataType::Ascii32, &DataType::Utf8), DataType::Ascii32);
        assert_eq!(
            up(&DataType::Ascii32, &DataType::Ascii64),
            DataType::Ascii64
        );
        assert_eq!(
            down(&DataType::Ascii32, &DataType::Ascii64),
            DataType::Ascii32
        );
        assert_eq!(
            up(&DataType::LargeUtf8, &DataType::Ascii128),
            DataType::LargeUtf8
        );
        assert_eq!(
            down(&DataType::LargeUtf8, &DataType::Ascii128),
            DataType::Ascii128
        );
        assert_eq!(
            DataType::Ascii32
                .merge_exact(&DataType::Utf8, Widening::Up)
                .unwrap(),
            DataType::Utf8
        );
    }

    #[test]
    fn ascii_absorbs_a_number_at_no_less_than_utf8_and_only_when_allowed() {
        assert_eq!(
            DataType::Ascii32
                .merge_with(&DataType::Int32, true)
                .unwrap(),
            DataType::Utf8
        );
        assert_eq!(
            DataType::Int32
                .merge_with(&DataType::Ascii32, false)
                .unwrap(),
            DataType::Utf8
        );
        let refused = DataType::Ascii32
            .merge_exact(&DataType::Int32, Widening::Up)
            .unwrap_err()
            .to_string();
        assert!(refused.contains("ascii32"), "{refused}");
        assert!(refused.contains("int32"), "{refused}");
    }

    #[test]
    fn bytes_win_over_ascii_and_keep_only_an_identical_fixed_width() {
        assert_eq!(
            DataType::Ascii32
                .merge_with(&DataType::FixedSizeBinary(4), true)
                .unwrap(),
            DataType::FixedSizeBinary(4)
        );
        assert_eq!(
            DataType::FixedSizeBinary(8)
                .merge_with(&DataType::Ascii32, true)
                .unwrap(),
            DataType::Binary
        );
        assert_eq!(
            DataType::Ascii32
                .merge_with(&DataType::Binary, true)
                .unwrap(),
            DataType::Binary
        );
    }
}
