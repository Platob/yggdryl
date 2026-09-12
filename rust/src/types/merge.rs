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
//! 4. Bytes win. A byte type paired with anything else answers bytes,
//!    because bytes are the container every other encoding fits inside. Two
//!    byte types meet parameter by parameter, exactly as two strings do: the
//!    wider offsets, the variable layout over a fixed one, no bound over a
//!    bound when widening, and the mirror when narrowing. A type storing a
//!    fixed width beside fixed bytes of that same width - a fixed string, a
//!    registered code, a UUID - keeps the storage both already have: widening
//!    answers the plain bytes, narrowing the side that constrains them. Any
//!    other pairing is variable bytes in the byte side's layout.
//! 5. Text wins next, over numbers and temporals. Two strings meet parameter
//!    by parameter: widening takes the wider offsets, the variable layout
//!    over a fixed one, UTF-8 over two different charsets, and no bound over
//!    a bound; narrowing takes the mirror. A registered code is the fixed
//!    US-ASCII width it stores when widening and the code itself when
//!    narrowing, so the tighter type survives the direction that asks for
//!    it; text absorbing a non-text side is at least `utf8`, because a
//!    number's rendering does not fit four bytes.
//! 6. Numbers meet by width, and temporals by unit. An exact decimal keeps
//!    the widest storage either side declared when widening, so a merge never
//!    re-encodes a `decimal128` column into a `decimal64` one.
//!
//! Anything left is an honest refusal rather than a lossy guess: a boolean and
//! a timestamp have no meeting point that is not a re-encoding.
//!
//! `upscale` picks the direction width is resolved in. Widening is the default
//! and is lossless - `int32` and `int64` meet at `int64`. Narrowing is the
//! deliberate opposite, for a caller who wants the tightest type that names
//! both and accepts that stored values may not fit it.

use std::num::NonZeroU32;

use smol_str::format_smolstr;

use crate::{Charset, DataType, Error, Field, Result};
use crate::{TimeUnit, UnionMode};

use super::bytes::{BytesLayout, BytesParameters};
use super::string::{StringLayout, StringParameters};

/// Whether a pair with no shared family may meet by being re-encoded.
///
/// Answering `utf8` for an integer beside a string is the right call when two
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
    /// wins next, strings meeting parameter by parameter and absorbing a
    /// non-text side at no less than `utf8`; and numbers meet by width,
    /// temporals by unit. Anything left is refused rather than guessed - a
    /// boolean and a timestamp have no meeting point that is not a
    /// re-encoding.
    ///
    /// A type that names fewer values than the shape it stores in - a
    /// registered code, a UUID, a decimal's declared backing - survives
    /// the direction that asks for it: widening answers the shape holding
    /// both, narrowing answers the tighter type.
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
    /// assert_eq!(DataType::Null.merge_with(&DataType::utf8(), true)?, DataType::utf8());
    ///
    /// // Bytes win over text, and text over numbers.
    /// assert_eq!(DataType::utf8().merge_with(&DataType::binary(), true)?, DataType::binary());
    /// assert_eq!(DataType::Int64.merge_with(&DataType::utf8(), true)?, DataType::utf8());
    ///
    /// // A fixed width is text, so it meets variable text there when widening.
    /// assert_eq!(DataType::fixed_ascii(4)?.merge_with(&DataType::utf8(), true)?, DataType::utf8());
    /// assert_eq!(DataType::fixed_ascii(4)?.merge_with(&DataType::fixed_ascii(8)?, false)?, DataType::fixed_ascii(4)?);
    ///
    /// // Narrowing keeps the tighter type: the code over the width it stores
    /// // in, and the decimal's own backing over the one precision needs.
    /// assert_eq!(DataType::Currency.merge_with(&DataType::utf8(), false)?, DataType::Currency);
    /// assert_eq!(DataType::Currency.merge_with(&DataType::utf8(), true)?, DataType::utf8());
    /// assert_eq!(
    ///     DataType::decimal128(10, 2)?.merge_with(&DataType::Int16, true)?,
    ///     DataType::decimal128(10, 2)?,
    /// );
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
        // A version has numeric ordering semantics that a text or numeric
        // merge cannot preserve, and a URL carries a validation a text merge
        // would silently drop. Only the equal-type arm above may merge either.
        if matches!(self, Self::Version | Self::Url) || matches!(other, Self::Version | Self::Url) {
            return Err(unmergeable(self, other));
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
    left: &crate::types::UnionFields,
    left_mode: UnionMode,
    right: &crate::types::UnionFields,
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
    // Bytes hold every other encoding, so a byte side decides the pair.
    if let Some(parameters) = left.bytes_parameters() {
        return match right.bytes_parameters() {
            Some(other) => DataType::bytes(merge_bytes(parameters, other, how)?).map(Some),
            None if recode == Recode::Allowed && is_mergeable_into_bytes(right) => {
                rebuild_binary(parameters, how, left, right).map(Some)
            }
            None => Ok(None),
        };
    }
    if let Some(parameters) = right.bytes_parameters() {
        return if recode == Recode::Allowed && is_mergeable_into_bytes(left) {
            rebuild_binary(parameters, how, left, right).map(Some)
        } else {
            Ok(None)
        };
    }
    // Text is next, over numbers and temporals.
    if let Some(parameters) = text_parameters(left) {
        return match text_parameters(right) {
            Some(other) => merge_text((left, parameters), (right, other), how).map(Some),
            None if recode == Recode::Allowed && is_mergeable_into_text(right) => {
                absorbing_text(parameters).map(Some)
            }
            None => Ok(None),
        };
    }
    if let Some(parameters) = text_parameters(right) {
        return if recode == Recode::Allowed && is_mergeable_into_text(left) {
            absorbing_text(parameters).map(Some)
        } else {
            Ok(None)
        };
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

/// Meet two byte types parameter by parameter.
///
/// The same table as [`merge_parameters`] without the charset: widening takes
/// the wider offsets and the variable layout over a fixed one, a view staying
/// a view only beside another view, and no bound unless both have one, and
/// then the larger; narrowing is the mirror. Two fixed widths that agree are
/// one type and never reach here, and two that disagree are variable bytes
/// when widening, because a byte value is never padded to a wider slot.
fn merge_bytes(
    left: BytesParameters,
    right: BytesParameters,
    how: Widening,
) -> Result<BytesParameters> {
    let layout = match how {
        Widening::Up => match (left.is_fixed(), right.is_fixed()) {
            (true, true) => BytesLayout::Binary,
            (true, false) => right.layout(),
            (false, true) => left.layout(),
            (false, false) => variable_bytes_layout(
                left.layout().is_view() && right.layout().is_view(),
                left.layout().is_large() || right.layout().is_large(),
            ),
        },
        Widening::Down if left.is_fixed() || right.is_fixed() => BytesLayout::FixedSizeBinary,
        Widening::Down => variable_bytes_layout(
            left.layout().is_view() && right.layout().is_view(),
            left.layout().is_large() && right.layout().is_large(),
        ),
    };
    let bound = match (how, left.bound(), right.bound()) {
        (Widening::Up, Some(left), Some(right)) => Some(left.max(right)),
        (Widening::Up, _, _) => None,
        (Widening::Down, Some(left), Some(right)) => Some(left.min(right)),
        (Widening::Down, left, right) => left.or(right),
    };
    let parameters = BytesParameters::new(layout);
    match bound {
        Some(bound) => parameters.try_with_bound(bound),
        None => Ok(parameters),
    }
}

/// The variable byte layout with the given view and offsets declarations.
const fn variable_bytes_layout(view: bool, large: bool) -> BytesLayout {
    match (view, large) {
        (true, _) => BytesLayout::BinaryView,
        (false, true) => BytesLayout::LargeBinary,
        (false, false) => BytesLayout::Binary,
    }
}

/// The byte width of a fixed-width byte layout: fixed bytes, a fixed string,
/// a registered code, or a UUID, each of whose storage is the fixed binary of
/// that width. A number's width is its own encoding and never bytes it
/// shares, so `int32` beside `fixed_size_binary(4)` is variable bytes.
fn fixed_width(dtype: &DataType) -> Option<usize> {
    match dtype {
        DataType::Bytes(_) | DataType::String(_) | DataType::Uuid => dtype.fixed_byte_width(),
        _ if dtype.is_code() => dtype.fixed_byte_width(),
        _ => None,
    }
}

/// Rebuild the byte side beside a non-byte one, keeping a shared fixed width.
///
/// Two sides storing the same number of bytes keep that storage, and the
/// direction decides which of the two names it. Widening answers the plain
/// bytes, which hold every value either side can carry; narrowing answers the
/// side that constrains them - a fixed string, a registered code, a UUID -
/// because that is the tightest type naming both and the storage is identical
/// either way. Any other pairing is variable bytes in the byte side's layout:
/// the other side's rendering fits no fixed width and no maximum.
fn rebuild_binary(
    parameters: BytesParameters,
    how: Widening,
    left: &DataType,
    right: &DataType,
) -> Result<DataType> {
    if let (Some(left_width), Some(right_width)) = (fixed_width(left), fixed_width(right)) {
        if left_width == right_width {
            if how == Widening::Down {
                // The side that is not the bytes is the one constraining them.
                return Ok(match left.bytes_parameters() {
                    Some(_) => right.clone(),
                    None => left.clone(),
                });
            }
            if let Ok(width) = u32::try_from(left_width) {
                return DataType::fixed_size_binary(width);
            }
        }
    }
    let layout = match parameters.is_fixed() {
        true => BytesLayout::Binary,
        false => parameters.layout(),
    };
    Ok(DataType::Bytes(BytesParameters::new(layout)))
}

/// The parameters a text datatype merges as, if it is text at all.
///
/// A registered code is the fixed US-ASCII width it stores. Two schemas that
/// agree on a code never reach here - the merge answers an equal pair before
/// reading anything - so this decides only the pairs that disagree, and
/// [`merge_text`] is what says when the code identity survives.
fn text_parameters(dtype: &DataType) -> Option<StringParameters> {
    match dtype {
        DataType::String(parameters) => Some(*parameters),
        _ if dtype.is_code() => {
            let width = u32::try_from(dtype.fixed_byte_width()?).ok()?;
            Some(
                StringParameters::ascii(StringLayout::FixedString)
                    .with_bound(NonZeroU32::new(width)?),
            )
        }
        _ => None,
    }
}

/// Meet two text types, conserving a registered code where the direction can.
///
/// Widening never answers a code: a code names fewer values than the width it
/// stores in, so the type holding both sides is the plain one - `currency`
/// beside `fixed_ascii(3)` is `fixed_ascii(3)`. Narrowing asks the opposite
/// question, for the tightest type that names both, and there the code is the
/// answer whenever the other side is at least as general: `currency` beside
/// `utf8` or `fixed_ascii(3)` narrows to `currency`, and only a side narrower
/// still, such as `fixed_ascii(2)`, outranks it.
///
/// Two *different* codes are the one pair neither direction answers with a
/// code, because neither standard names the other's values: a currency merged
/// with a country is `fixed_ascii(3)` widening and `fixed_ascii(2)` narrowing,
/// never one standard's code carrying the other's values.
fn merge_text(
    left: (&DataType, StringParameters),
    right: (&DataType, StringParameters),
    how: Widening,
) -> Result<DataType> {
    let ((left_type, left_parameters), (right_type, right_parameters)) = (left, right);
    if how == Widening::Down && !(left_type.is_code() && right_type.is_code()) {
        if left_type.is_code() && holds_width(right_parameters, left_parameters) {
            return Ok(left_type.clone());
        }
        if right_type.is_code() && holds_width(left_parameters, right_parameters) {
            return Ok(right_type.clone());
        }
    }
    DataType::string(merge_parameters(left_parameters, right_parameters, how)?)
}

/// Whether one string's bound leaves room for every value of a fixed width.
fn holds_width(parameters: StringParameters, fixed: StringParameters) -> bool {
    parameters
        .bound()
        .is_none_or(|bound| Some(bound) >= fixed.bound())
}

/// Meet two strings parameter by parameter.
///
/// Widening takes, for the layout, the wider offsets and the variable layout
/// over a fixed one, a view staying a view only beside another view; for the
/// charset, UTF-8 unless both agree; for the bound, none unless both have
/// one, and then the larger. Narrowing is the mirror: the narrower layout,
/// the narrower repertoire, the smaller bound.
fn merge_parameters(
    left: StringParameters,
    right: StringParameters,
    how: Widening,
) -> Result<StringParameters> {
    let layout = match how {
        Widening::Up => match (left.is_fixed(), right.is_fixed()) {
            (true, true) => StringLayout::FixedString,
            (true, false) => right.layout(),
            (false, true) => left.layout(),
            (false, false) => variable_layout(
                left.layout().is_view() && right.layout().is_view(),
                left.layout().is_large() || right.layout().is_large(),
            ),
        },
        Widening::Down if left.is_fixed() || right.is_fixed() => StringLayout::FixedString,
        Widening::Down => variable_layout(
            left.layout().is_view() && right.layout().is_view(),
            left.layout().is_large() && right.layout().is_large(),
        ),
    };
    let charset = match how {
        _ if left.charset() == right.charset() => left.charset(),
        Widening::Up => Charset::Utf8,
        // Two different repertoires of one rank name neither's values; the
        // left one is the deterministic pick, as with a decimal's backing.
        Widening::Down if repertoire(right.charset()) < repertoire(left.charset()) => {
            right.charset()
        }
        Widening::Down => left.charset(),
    };
    let bound = match (how, left.bound(), right.bound()) {
        (Widening::Up, Some(left), Some(right)) => Some(left.max(right)),
        (Widening::Up, _, _) => None,
        (Widening::Down, Some(left), Some(right)) => Some(left.min(right)),
        (Widening::Down, left, right) => left.or(right),
    };
    let parameters = StringParameters::new(layout, charset);
    match bound {
        Some(bound) => parameters.try_with_bound(bound),
        None => Ok(parameters),
    }
}

/// The variable layout with the given view and offsets declarations.
const fn variable_layout(view: bool, large: bool) -> StringLayout {
    match (view, large) {
        (true, true) => StringLayout::LargeStringView,
        (true, false) => StringLayout::StringView,
        (false, true) => StringLayout::LargeString,
        (false, false) => StringLayout::String,
    }
}

/// How much a charset names: US-ASCII, then one byte per scalar, then all
/// of Unicode.
const fn repertoire(charset: Charset) -> u8 {
    match charset {
        Charset::Ascii => 0,
        _ if charset.is_single_byte() => 1,
        _ => 2,
    }
}

/// The text a non-text side re-encodes into beside `parameters`: at least
/// `utf8`, because a number's rendering fits no fixed width and no bound.
fn absorbing_text(parameters: StringParameters) -> Result<DataType> {
    DataType::string(merge_parameters(
        parameters,
        StringParameters::default(),
        Widening::Up,
    )?)
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
        // Widening keeps the widest storage either side declared. A column
        // saying `decimal128` is not re-encoded to `decimal64` because the
        // merged precision happens to fit there: that is a layout change the
        // other side never asked for, and the same reason a dictionary does
        // not survive a merge with a plain column. Narrowing wants the
        // tightest type instead, so it takes whatever the precision needs.
        let backing = match how {
            Widening::Up => decimal_backing(left).max(decimal_backing(right)),
            Widening::Down => None,
        };
        return merge_decimal(left_parts, right_parts, how, backing).map(Some);
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
///
/// `backing` is the storage the pair has already agreed on, when either side
/// declared one wider than the merged precision needs.
fn merge_decimal(
    left: (u8, i8),
    right: (u8, i8),
    how: Widening,
    backing: Option<u8>,
) -> Result<DataType> {
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
    rebuild_decimal(
        backing.unwrap_or(0).max(required_backing(precision)),
        precision,
        scale,
    )
}

/// Which of the four backings a decimal declares, as a width rank.
const fn decimal_backing(dtype: &DataType) -> Option<u8> {
    match dtype {
        DataType::Decimal32 { .. } => Some(0),
        DataType::Decimal64 { .. } => Some(1),
        DataType::Decimal128 { .. } => Some(2),
        DataType::Decimal256 { .. } => Some(3),
        _ => None,
    }
}

/// The narrowest backing a precision fits in, the one [`DataType::decimal`]
/// picks for it.
const fn required_backing(precision: u8) -> u8 {
    match precision {
        0..=9 => 0,
        10..=18 => 1,
        19..=38 => 2,
        _ => 3,
    }
}

/// Rebuild an exact decimal at a backing rank.
fn rebuild_decimal(backing: u8, precision: u8, scale: i8) -> Result<DataType> {
    match backing {
        0 => DataType::decimal32(precision, scale),
        1 => DataType::decimal64(precision, scale),
        2 => DataType::decimal128(precision, scale),
        _ => DataType::decimal256(precision, scale),
    }
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
            let timezone = match (left, right) {
                (DataType::DateTime64 { timezone, .. }, _) if !timezone.is_naive() => *timezone,
                (_, DataType::DateTime64 { timezone, .. }) if !timezone.is_naive() => *timezone,
                _ => crate::Timezone::NAIVE,
            };
            DataType::DateTime64 { unit, timezone }
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
        DataType::DateTime64 { unit, .. } => Some((2, *unit)),
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
    fn fixed_widths_meet_text_by_width_in_the_direction_asked_for() {
        let up = |left: &DataType, right: &DataType| left.merge_with(right, true).unwrap();
        let down = |left: &DataType, right: &DataType| left.merge_with(right, false).unwrap();

        assert_eq!(
            up(&DataType::fixed_ascii(4).unwrap(), &DataType::utf8()),
            DataType::utf8()
        );
        assert_eq!(
            down(&DataType::fixed_ascii(4).unwrap(), &DataType::utf8()),
            DataType::fixed_ascii(4).unwrap()
        );
        assert_eq!(
            up(
                &DataType::fixed_ascii(4).unwrap(),
                &DataType::fixed_ascii(8).unwrap()
            ),
            DataType::fixed_ascii(8).unwrap()
        );
        assert_eq!(
            down(
                &DataType::fixed_ascii(4).unwrap(),
                &DataType::fixed_ascii(8).unwrap()
            ),
            DataType::fixed_ascii(4).unwrap()
        );
        assert_eq!(
            up(&DataType::large_utf8(), &DataType::fixed_ascii(16).unwrap()),
            DataType::large_utf8()
        );
        assert_eq!(
            down(&DataType::large_utf8(), &DataType::fixed_ascii(16).unwrap()),
            DataType::fixed_ascii(16).unwrap()
        );
        assert_eq!(
            DataType::fixed_ascii(4)
                .unwrap()
                .merge_exact(&DataType::utf8(), Widening::Up)
                .unwrap(),
            DataType::utf8()
        );
    }

    #[test]
    fn a_fixed_string_absorbs_a_number_at_no_less_than_utf8_and_only_when_allowed() {
        assert_eq!(
            DataType::fixed_ascii(4)
                .unwrap()
                .merge_with(&DataType::Int32, true)
                .unwrap(),
            DataType::utf8()
        );
        assert_eq!(
            DataType::Int32
                .merge_with(&DataType::fixed_ascii(4).unwrap(), false)
                .unwrap(),
            DataType::utf8()
        );
        let refused = DataType::fixed_ascii(4)
            .unwrap()
            .merge_exact(&DataType::Int32, Widening::Up)
            .unwrap_err()
            .to_string();
        assert!(refused.contains("fixed_ascii(4)"), "{refused}");
        assert!(refused.contains("int32"), "{refused}");
    }

    #[test]
    fn bytes_win_over_text_and_keep_only_an_identical_fixed_width() {
        assert_eq!(
            DataType::fixed_ascii(4)
                .unwrap()
                .merge_with(&DataType::fixed_size_binary(4).unwrap(), true)
                .unwrap(),
            DataType::fixed_size_binary(4).unwrap()
        );
        assert_eq!(
            DataType::fixed_size_binary(8)
                .unwrap()
                .merge_with(&DataType::fixed_ascii(4).unwrap(), true)
                .unwrap(),
            DataType::binary()
        );
        assert_eq!(
            DataType::fixed_ascii(4)
                .unwrap()
                .merge_with(&DataType::binary(), true)
                .unwrap(),
            DataType::binary()
        );
        // The byte side keeps its layout and drops a maximum the other side
        // never declared.
        assert_eq!(
            DataType::from_str("large_binary(16)")
                .unwrap()
                .merge_with(&DataType::utf8(), true)
                .unwrap(),
            DataType::large_binary()
        );
    }

    #[test]
    fn a_number_never_shares_a_fixed_byte_width() {
        // Four bytes of `int32` are an encoding, not a slot the bytes side
        // stores, so the pair is variable bytes in either direction.
        let fixed = DataType::fixed_size_binary(4).unwrap();
        assert_eq!(
            DataType::Int32.merge_with(&fixed, true).unwrap(),
            DataType::binary()
        );
        assert_eq!(
            fixed.merge_with(&DataType::Int32, false).unwrap(),
            DataType::binary()
        );
    }

    #[test]
    fn two_byte_types_meet_parameter_by_parameter() {
        let up = |left: &str, right: &str| {
            DataType::from_str(left)
                .unwrap()
                .merge_with(&DataType::from_str(right).unwrap(), true)
                .unwrap()
                .to_string()
        };
        let down = |left: &str, right: &str| {
            DataType::from_str(left)
                .unwrap()
                .merge_with(&DataType::from_str(right).unwrap(), false)
                .unwrap()
                .to_string()
        };
        // Widening: the wider offsets, a view only beside a view, the
        // variable layout over a fixed one, no bound over a bound.
        assert_eq!(up("binary", "large_binary"), "large_binary");
        assert_eq!(up("binary_view", "binary"), "binary");
        assert_eq!(up("binary_view", "large_binary"), "large_binary");
        assert_eq!(up("fixed_size_binary(4)", "binary_view"), "binary_view");
        assert_eq!(up("fixed_size_binary(4)", "binary(2)"), "binary(4)");
        assert_eq!(
            up("fixed_size_binary(4)", "fixed_size_binary(8)"),
            "binary(8)"
        );
        assert_eq!(up("binary(16)", "binary"), "binary");
        assert_eq!(up("binary(16)", "binary(32)"), "binary(32)");
        // Narrowing: the mirror.
        assert_eq!(down("binary", "large_binary"), "binary");
        assert_eq!(down("binary_view", "large_binary"), "binary");
        assert_eq!(
            down("fixed_size_binary(4)", "binary_view"),
            "fixed_size_binary(4)"
        );
        assert_eq!(
            down("fixed_size_binary(4)", "fixed_size_binary(8)"),
            "fixed_size_binary(4)"
        );
        assert_eq!(down("binary(16)", "binary"), "binary(16)");
        assert_eq!(down("binary(16)", "binary(32)"), "binary(16)");
    }

    #[test]
    fn narrowing_keeps_the_type_that_constrains_a_shared_fixed_width() {
        // The storage is the same either way, so the direction is free to
        // answer the tighter of the two types.
        for (left, right) in [
            (
                DataType::fixed_ascii(4).unwrap(),
                DataType::fixed_size_binary(4).unwrap(),
            ),
            (DataType::Currency, DataType::fixed_size_binary(3).unwrap()),
            (DataType::Uuid, DataType::fixed_size_binary(16).unwrap()),
        ] {
            assert_eq!(
                left.merge_with(&right, false).unwrap(),
                left,
                "narrowing answers {left}"
            );
            assert_eq!(
                right.merge_with(&left, false).unwrap(),
                left,
                "in either position"
            );
            assert_eq!(
                left.merge_with(&right, true).unwrap(),
                right,
                "widening answers the bytes"
            );
        }

        // A width neither side shares is variable bytes, as before.
        assert_eq!(
            DataType::Uuid
                .merge_with(&DataType::fixed_size_binary(8).unwrap(), true)
                .unwrap(),
            DataType::binary()
        );
    }

    #[test]
    fn narrowing_keeps_a_registered_code_over_the_width_it_stores_in() {
        let up = |left: &DataType, right: &DataType| left.merge_with(right, true).unwrap();
        let down = |left: &DataType, right: &DataType| left.merge_with(right, false).unwrap();

        // The code is the tighter type, so narrowing answers it in either
        // position and widening answers the shape that holds both.
        for other in [
            DataType::fixed_ascii(3).unwrap(),
            DataType::ascii(),
            DataType::utf8(),
            DataType::large_utf8(),
        ] {
            assert_eq!(down(&DataType::Currency, &other), DataType::Currency);
            assert_eq!(down(&other, &DataType::Currency), DataType::Currency);
            assert_ne!(up(&DataType::Currency, &other), DataType::Currency);
        }
        assert_eq!(
            down(&DataType::Cfi, &DataType::fixed_ascii(6).unwrap()),
            DataType::Cfi
        );

        // A side narrower than the code still outranks it: narrowing is the
        // tightest type that names both, not the most specific one.
        assert_eq!(
            down(&DataType::Currency, &DataType::fixed_ascii(2).unwrap()),
            DataType::fixed_ascii(2).unwrap()
        );

        // Two different codes are the pair neither direction answers with a
        // code: neither standard names the other's values.
        assert_eq!(
            down(&DataType::Currency, &DataType::Country),
            DataType::fixed_ascii(2).unwrap()
        );
        assert_eq!(
            up(&DataType::Currency, &DataType::Country),
            DataType::fixed_ascii(3).unwrap()
        );

        // A number's rendering does not fit a code, so absorbing one is still
        // no less than `utf8`.
        assert_eq!(
            down(&DataType::Currency, &DataType::Int32),
            DataType::utf8()
        );
    }

    #[test]
    fn widening_a_decimal_keeps_the_widest_backing_either_side_declared() {
        let decimal128 = DataType::decimal128(10, 2).unwrap();
        let decimal256 = DataType::Decimal256 {
            precision: 20,
            scale: 2,
        };

        // The merged precision fits a narrower backing, but re-encoding the
        // storage is not something a widening merge may impose.
        assert_eq!(
            decimal128.merge_with(&DataType::Int16, true).unwrap(),
            decimal128
        );
        assert_eq!(
            decimal256
                .merge_with(&DataType::decimal128(20, 2).unwrap(), true)
                .unwrap(),
            decimal256
        );
        assert_eq!(
            decimal256.merge_with(&DataType::Int32, true).unwrap(),
            decimal256
        );

        // A precision wider than either backing still moves up to the one
        // that holds it.
        assert_eq!(
            DataType::decimal32(9, 2)
                .unwrap()
                .merge_with(&DataType::Int32, true)
                .unwrap(),
            DataType::decimal64(12, 2).unwrap()
        );

        // Narrowing wants the tightest type, so it takes what precision needs.
        assert_eq!(
            decimal128.merge_with(&DataType::Int16, false).unwrap(),
            DataType::decimal32(8, 0).unwrap()
        );
        assert_eq!(
            decimal256
                .merge_with(&DataType::decimal128(20, 2).unwrap(), false)
                .unwrap(),
            DataType::decimal128(20, 2).unwrap()
        );
    }

    #[test]
    fn strings_meet_parameter_by_parameter() {
        let up = |left: &DataType, right: &DataType| left.merge_with(right, true).unwrap();
        let down = |left: &DataType, right: &DataType| left.merge_with(right, false).unwrap();
        let dtype = |spelling: &str| DataType::from_str(spelling).unwrap();

        // Offsets widen and narrow; a view stays a view only beside a view.
        assert_eq!(
            up(&DataType::utf8(), &DataType::large_utf8()),
            DataType::large_utf8()
        );
        assert_eq!(
            down(&DataType::utf8(), &DataType::large_utf8()),
            DataType::utf8()
        );
        assert_eq!(
            up(&DataType::utf8_view(), &DataType::utf8()),
            DataType::utf8()
        );
        assert_eq!(
            up(&DataType::utf8_view(), &dtype("large_utf8_view")),
            dtype("large_utf8_view")
        );
        assert_eq!(
            down(&DataType::utf8_view(), &dtype("large_utf8_view")),
            DataType::utf8_view()
        );

        // Two charsets widen to UTF-8 and narrow to the smaller repertoire.
        assert_eq!(
            up(&DataType::ascii(), &dtype("string(windows-1252)")),
            DataType::utf8()
        );
        assert_eq!(
            down(&DataType::utf8(), &dtype("string(windows-1252)")),
            dtype("string(windows-1252)")
        );
        assert_eq!(
            down(&dtype("string(windows-1252)"), &DataType::ascii()),
            DataType::ascii()
        );
        assert_eq!(
            down(&dtype("string(latin1)"), &dtype("string(windows-1252)")),
            dtype("string(latin1)")
        );

        // No bound beats a maximum widening; the smaller bound wins narrowing.
        assert_eq!(up(&dtype("utf8(32)"), &DataType::utf8()), DataType::utf8());
        assert_eq!(up(&dtype("utf8(32)"), &dtype("utf8(8)")), dtype("utf8(32)"));
        assert_eq!(
            down(&dtype("utf8(32)"), &DataType::utf8()),
            dtype("utf8(32)")
        );
        assert_eq!(
            down(&dtype("utf8(32)"), &dtype("utf8(8)")),
            dtype("utf8(8)")
        );
        assert_eq!(
            up(&dtype("fixed_utf8(40)"), &dtype("utf8(32)")),
            dtype("utf8(40)")
        );
        assert_eq!(
            down(&dtype("fixed_utf8(40)"), &dtype("utf8(32)")),
            dtype("fixed_utf8(32)")
        );
    }
}
