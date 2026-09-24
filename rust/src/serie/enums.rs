//! The column a dictionary-encoded field is stored in: a key column over a
//! values column.
//!
//! The keys are an integer column of the key datatype, whose validity is
//! the column's; the values are the vocabulary every key points into,
//! shared by every row that reads through it. A row reads through its key,
//! the Arrow array is built on demand from the two, and a write interns:
//! the rows are looked up in the vocabulary, the values it does not hold
//! yet are appended to it, and the keys are spliced in place. The
//! vocabulary never holds a value twice on this column's account, so a
//! push of a value already held moves one key and nothing else.
//!
//! A write never goes through Arrow's dictionary concatenation: that
//! kernel joins two vocabularies naively unless the values are primitive
//! or plain bytes, doubling the vocabulary on every write, and it panics
//! rather than refusing when the join no longer fits the key.

use std::collections::HashMap;
use std::fmt;
use std::ops::Range;
use std::sync::Arc;

use arrow_array::types::{
    Int8Type, Int16Type, Int32Type, Int64Type, UInt8Type, UInt16Type, UInt32Type, UInt64Type,
};
use arrow_array::{Array, ArrayRef, DictionaryArray};
use arrow_buffer::{ArrowNativeType, NullBuffer};
use arrow_schema::DataType as ArrowDataType;

use super::{Serie, require_range, require_row};
use crate::value::SerieValue;
use crate::{DataType, Field, Result, Scalar};

/// The invariant every dictionary column keeps: its keys point into its
/// values, so the Arrow array assembles.
const ALIGNED: &str =
    "a dictionary column's keys point into its values: no public path misaligns them";

/// The invariant a write carries in from `check`: the vocabulary the rows
/// intern into fits the key, and the values column takes what it gains.
const CHECKED: &str = "check ran on these rows: the vocabulary they intern into fits the key";

/// The invariant the door keeps: a dictionary field's keys are an integer
/// column of the key datatype, which the field's contract validated.
const KEYED: &str =
    "a dictionary field's keys are an integer column: the field's contract picked the width";

/// One column of dictionary-encoded rows: the key column and the values it
/// points into.
#[derive(Clone)]
pub struct DictionarySerie {
    field: Arc<Field>,
    keys: Serie,
    values: Serie,
}

/// What a write interns: the key each row reads through, and the values
/// the vocabulary gains, in the order they were met.
struct Interned {
    keys: Vec<Option<usize>>,
    added: Vec<Scalar>,
}

/// How many values a key of `keys`' width indexes: one past the largest
/// key, which is what Arrow lets a values array hold.
fn key_capacity(keys: &Serie) -> usize {
    let bound: u64 = match keys {
        Serie::Int8(_) => 1 << 7,
        Serie::Int16(_) => 1 << 15,
        Serie::Int32(_) => 1 << 31,
        Serie::Int64(_) => 1 << 63,
        Serie::UInt8(_) => 1 << 8,
        Serie::UInt16(_) => 1 << 16,
        Serie::UInt32(_) => 1 << 32,
        Serie::UInt64(_) => u64::MAX,
        _ => unreachable!("{KEYED}"),
    };
    usize::try_from(bound).unwrap_or(usize::MAX)
}

impl DictionarySerie {
    /// Pair a dictionary field with its key column and its values column.
    pub(crate) const fn new(field: Arc<Field>, keys: Serie, values: Serie) -> Self {
        Self {
            field,
            keys,
            values,
        }
    }

    /// Borrow the key column: an integer column of the key datatype, whose
    /// validity is this column's; its field is nullable whatever this one
    /// is, because absence is judged here and not on a key.
    pub const fn keys(&self) -> &Serie {
        &self.keys
    }

    /// Borrow the values column every key points into.
    pub const fn values(&self) -> &Serie {
        &self.values
    }

    /// Build the Arrow dictionary array from the two columns.
    pub fn array(&self) -> ArrayRef {
        let values = self.values.into_arrow_array().expect(ALIGNED);
        macro_rules! dictionary {
            ($column:expr) => {
                Arc::new(DictionaryArray::try_new($column.array().clone(), values).expect(ALIGNED))
                    as ArrayRef
            };
        }
        match &self.keys {
            Serie::Int8(column) => dictionary!(column),
            Serie::Int16(column) => dictionary!(column),
            Serie::Int32(column) => dictionary!(column),
            Serie::Int64(column) => dictionary!(column),
            Serie::UInt8(column) => dictionary!(column),
            Serie::UInt16(column) => dictionary!(column),
            Serie::UInt32(column) => dictionary!(column),
            Serie::UInt64(column) => dictionary!(column),
            _ => unreachable!("{KEYED}"),
        }
    }

    /// Read row `index`'s key off the key buffer: `None` when the key is
    /// absent; the caller keeps `index` below the length.
    fn key_at(&self, index: usize) -> Option<usize> {
        macro_rules! key {
            ($column:expr) => {
                $column.value(index).map(|key| key.as_usize())
            };
        }
        match &self.keys {
            Serie::Int8(column) => key!(column),
            Serie::Int16(column) => key!(column),
            Serie::Int32(column) => key!(column),
            Serie::Int64(column) => key!(column),
            Serie::UInt8(column) => key!(column),
            Serie::UInt16(column) => key!(column),
            Serie::UInt32(column) => key!(column),
            Serie::UInt64(column) => key!(column),
            _ => unreachable!("{KEYED}"),
        }
    }

    /// Intern canonical `rows` into the vocabulary: the key each row reads
    /// through, and the values the vocabulary gains.
    ///
    /// The vocabulary is read once, one row per value it holds; a value a
    /// foreign array spelled twice is reached through its first key. An
    /// absent row interns to no key.
    fn intern(&self, rows: &[Scalar]) -> Interned {
        let held = self.values.len();
        // `Scalar`'s hash reads canonical content only, never the
        // interior-mutable caches a datatype holds, so the key is stable.
        #[allow(clippy::mutable_key_type)]
        let mut vocabulary: HashMap<Scalar, usize> = HashMap::with_capacity(held + rows.len());
        for (key, value) in self.values.rows().into_owned().into_iter().enumerate() {
            vocabulary.entry(value).or_insert(key);
        }
        let mut added = Vec::new();
        let keys = rows
            .iter()
            .map(|row| {
                if row.is_null() {
                    return None;
                }
                let next = held + added.len();
                Some(*vocabulary.entry(row.clone()).or_insert_with(|| {
                    added.push(row.clone());
                    next
                }))
            })
            .collect();
        Interned { keys, added }
    }

    /// Refuse what applying `interned` could not do: a vocabulary past what
    /// the key indexes, or what the values column refuses for what it gains.
    fn require_fit(&self, interned: &Interned) -> Result<()> {
        let held = self.values.len();
        let total = held + interned.added.len();
        let capacity = key_capacity(&self.keys);
        if total > capacity {
            return Err(crate::Error::InvalidRecord {
                path: smol_str::SmolStr::new(self.field.name()),
                reason: smol_str::format_smolstr!(
                    "{total} distinct values are past the {capacity} a {} key indexes in {}",
                    self.keys.require_field()?.dtype(),
                    self.field.name()
                ),
            });
        }
        self.values.check(&(held..held), &interned.added)
    }

    /// Apply `interned` over a checked `range`: the vocabulary appended,
    /// the keys spliced in place.
    fn apply(&mut self, range: Range<usize>, interned: Interned) {
        let held = self.values.len();
        self.values.write(held..held, interned.added);
        macro_rules! keys {
            ($column:expr, $native:ty) => {
                $column
                    .splice_values(
                        range,
                        interned
                            .keys
                            .into_iter()
                            .map(|key| key.map(|key| <$native>::from_usize(key).expect(CHECKED)))
                            .collect(),
                    )
                    .expect(CHECKED)
            };
        }
        match &mut self.keys {
            Serie::Int8(column) => keys!(Arc::make_mut(column), i8),
            Serie::Int16(column) => keys!(Arc::make_mut(column), i16),
            Serie::Int32(column) => keys!(Arc::make_mut(column), i32),
            Serie::Int64(column) => keys!(Arc::make_mut(column), i64),
            Serie::UInt8(column) => keys!(Arc::make_mut(column), u8),
            Serie::UInt16(column) => keys!(Arc::make_mut(column), u16),
            Serie::UInt32(column) => keys!(Arc::make_mut(column), u32),
            Serie::UInt64(column) => keys!(Arc::make_mut(column), u64),
            _ => unreachable!("{KEYED}"),
        }
    }

    /// Refuse what a write could not do: a vocabulary past what the key
    /// indexes, or what the values column refuses for the values it gains.
    pub(crate) fn check(&self, range: &Range<usize>, rows: &[Scalar]) -> Result<()> {
        let _ = range;
        self.require_fit(&self.intern(rows))
    }

    /// Write canonical `rows` over a checked `range`: the rows interned,
    /// the vocabulary appended, the keys spliced in place.
    pub(crate) fn write(&mut self, range: Range<usize>, rows: Vec<Scalar>) {
        let interned = self.intern(&rows);
        self.apply(range, interned);
    }

    /// Append `other`'s rows, whose field agrees with this one's, answering
    /// whether the vocabulary could take its values; when it cannot, nothing
    /// moved.
    ///
    /// `other`'s vocabulary is interned once and its keys are remapped
    /// through it; no row of `other` is built.
    pub(crate) fn append(&mut self, other: &Self) -> bool {
        let vocabulary = self.intern(&other.values.rows());
        let keys = (0..other.len())
            .map(|index| other.key_at(index).and_then(|key| vocabulary.keys[key]))
            .collect();
        let interned = Interned {
            keys,
            added: vocabulary.added,
        };
        if self.require_fit(&interned).is_err() {
            return false;
        }
        let len = self.len();
        self.apply(len..len, interned);
        true
    }
}

impl SerieValue for DictionarySerie {
    fn field(&self) -> &Field {
        &self.field
    }

    fn field_ref(&self) -> &Arc<Field> {
        &self.field
    }

    fn len(&self) -> usize {
        self.keys.len()
    }

    fn null_count(&self) -> usize {
        if self.values.null_count() == 0 {
            return self.keys.null_count();
        }
        (0..self.keys.len())
            .filter(|index| {
                self.key_at(*index)
                    .is_none_or(|key| self.values.is_null(key).expect(ALIGNED))
            })
            .count()
    }

    fn is_null(&self, index: usize) -> Result<bool> {
        require_row(self.field.name(), index, self.keys.len())?;
        Ok(self
            .key_at(index)
            .is_none_or(|key| self.values.is_null(key).expect(ALIGNED)))
    }

    fn scalar(&self, index: usize) -> Result<Scalar> {
        require_row(self.field.name(), index, self.keys.len())?;
        match self.key_at(index) {
            Some(key) => self.values.scalar(key),
            None => Ok(Scalar::Null),
        }
    }

    fn slice(&self, offset: usize, length: usize) -> Result<Self> {
        Ok(Self::new(
            Arc::clone(&self.field),
            self.keys.slice(offset, length)?,
            self.values.clone(),
        ))
    }

    fn splice(&mut self, range: Range<usize>, rows: Vec<Scalar>) -> Result<()> {
        require_range(self.field.name(), &range, self.keys.len())?;
        let canonical = rows
            .into_iter()
            .map(|row| self.field.scalar(row))
            .collect::<Result<Vec<Scalar>>>()?;
        let interned = self.intern(&canonical);
        self.require_fit(&interned)?;
        self.apply(range, interned);
        Ok(())
    }

    fn into_arrow_array(&self) -> ArrayRef {
        self.array()
    }

    fn into_serie(self) -> Serie {
        super::Leaf::root(self)
    }

    fn from_serie(value: &Serie) -> Option<&Self> {
        super::Leaf::narrow(value)
    }
}

impl fmt::Debug for DictionarySerie {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        super::debug_column(self, "DictionarySerie", formatter)
    }
}

serie_leaf!(DictionarySerie);

/// Build the column `field` types out of a dictionary array, or answer
/// `None` for a layout that is not one.
///
/// Both halves take the door nullable under this column's name: absence
/// is judged once, at this level, on the logical nulls the keys and the
/// values spell together - and a key hidden under an absent record, or
/// written for a placeholder, selects no value for validation. Only vocabulary
/// values referenced by visible keys are proven. Unused narrow slots become
/// null placeholders while their payload buffers remain shared.
pub(crate) fn column_of(
    field: Arc<Field>,
    array: ArrayRef,
    parent: Option<&NullBuffer>,
    proof: &super::arrow::Proof,
    budget: &mut crate::budget::MaterializationBudget,
) -> crate::arrow::Result<Option<Serie>> {
    if !matches!(array.data_type(), ArrowDataType::Dictionary(..)) {
        return Ok(None);
    }
    let internal = || crate::arrow::Error::Internal {
        site: "serie::enums::column_of",
    };
    let DataType::Dictionary(dictionary) = field.dtype() else {
        return Err(internal());
    };
    macro_rules! parts {
        ($key:ty) => {{
            let held = super::arrow::held::<DictionaryArray<$key>>(&array)?;
            let needs_mask = parent.is_some_and(|above| above.null_count() != 0)
                || !dictionary.value().layout_is_contract();
            let hidden = if needs_mask {
                crate::cast::columns::selected_index_exposure(
                    held.values().len(),
                    held.len(),
                    parent.map(NullBuffer::inner),
                    |row| held.key(row),
                    budget,
                )?
                .map(NullBuffer::new)
            } else {
                None
            };
            let (keys, values) = held.into_parts();
            (Arc::new(keys) as ArrayRef, values, hidden)
        }};
    }
    let (keys, values, hidden) = match dictionary.key() {
        DataType::Int8 => parts!(Int8Type),
        DataType::Int16 => parts!(Int16Type),
        DataType::Int32 => parts!(Int32Type),
        DataType::Int64 => parts!(Int64Type),
        DataType::UInt8 => parts!(UInt8Type),
        DataType::UInt16 => parts!(UInt16Type),
        DataType::UInt32 => parts!(UInt32Type),
        DataType::UInt64 => parts!(UInt64Type),
        _ => return Err(internal()),
    };
    let keys = super::arrow::child_of(
        Arc::new(Field::new(field.name(), dictionary.key().clone(), true)),
        keys,
        None,
        &super::arrow::Proof::Proven,
        budget,
    )?;
    let values = super::arrow::child_of(
        Arc::new(Field::new(field.name(), dictionary.value().clone(), true)),
        values,
        hidden.as_ref(),
        proof.child(0),
        budget,
    )?;
    Ok(Some(DictionarySerie::new(field, keys, values).into_serie()))
}
