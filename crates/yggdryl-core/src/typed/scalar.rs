//! [`Scalar`] — the base of the typed-value hierarchy: an **indexed, possibly-null typed value**.
//!
//! A `Scalar` is the common surface [`Serie`](super::Serie) refines (a `Serie` *is* a `Scalar` with
//! more than one element): a length, per-index validity, and a null-aware [`get`](Scalar::get). The
//! concrete [`FixedScalar`] is the degenerate single-element case — one value in an [`IOBase`] data
//! buffer, with an optional validity bit — borrowing the byte layer for its read.

use core::marker::PhantomData;

use super::{DataType, Decoder, Encoder};
use crate::datatype_id::DataTypeId;
use crate::io::memory::{Heap, IOBase};

/// A typed, indexed, possibly-null value surface. A single value has [`len`](Scalar::len) `1`; a
/// [`Serie`](super::Serie) has `n`. `Value` is the native scalar an element decodes to.
pub trait Scalar {
    /// The native scalar one element decodes to.
    type Value;

    /// The element [`DataTypeId`] this value carries.
    fn data_type_id(&self) -> DataTypeId;

    /// The number of elements (a bare scalar is `1`).
    fn len(&self) -> usize;

    /// Whether there are no elements.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Whether the element at `index` is **valid** (non-null). Out-of-range is `false`.
    fn is_valid(&self, index: usize) -> bool;

    /// Whether the element at `index` is **null** (absent).
    fn is_null(&self, index: usize) -> bool {
        !self.is_valid(index)
    }

    /// How many elements are null.
    fn null_count(&self) -> usize {
        (0..self.len()).filter(|&i| self.is_null(i)).count()
    }

    /// The value at `index`, or `None` when it is null or out of range.
    fn get(&self, index: usize) -> Option<Self::Value>;
}

/// A **single typed element** over an [`IOBase`] data buffer (and an optional validity bit) — the
/// one-element [`Scalar`]. It borrows the byte layer: [`value`](FixedScalar::value) decodes on
/// demand via the type's [`Decoder`], never copying eagerly.
pub struct FixedScalar<T: DataType, D: IOBase = Heap> {
    data: D,
    validity: Option<D>,
    index: u64,
    _type: PhantomData<T>,
}

impl<T: Encoder + Decoder> FixedScalar<T, Heap> {
    /// A **non-null** scalar holding `value` (encoded into a fresh one-element heap buffer).
    pub fn of(value: T::Native) -> Self {
        let mut data = Heap::with_capacity(T::byte_width() as usize);
        T::encode(&mut data, 0, value).expect("encode into a fresh heap never fails");
        FixedScalar {
            data,
            validity: None,
            index: 0,
            _type: PhantomData,
        }
    }

    /// A **null** scalar of this type (validity bit clear; no value stored).
    pub fn null() -> Self {
        let mut validity = Heap::new();
        validity
            .pwrite_bit(0, false)
            .expect("bit write into a fresh heap never fails");
        FixedScalar {
            data: Heap::new(),
            validity: Some(validity),
            index: 0,
            _type: PhantomData,
        }
    }

    /// A scalar from an option — [`of`](FixedScalar::of) for `Some`, [`null`](FixedScalar::null)
    /// for `None`.
    pub fn from_option(value: Option<T::Native>) -> Self {
        match value {
            Some(value) => Self::of(value),
            None => Self::null(),
        }
    }
}

impl<T: Decoder, D: IOBase> FixedScalar<T, D> {
    /// Whether the element is valid (non-null) — `true` when there is no validity buffer.
    fn valid(&self) -> bool {
        self.validity
            .as_ref()
            .is_none_or(|bits| bits.pread_bit(self.index).unwrap_or(false))
    }

    /// The value, decoded on demand, or `None` when null.
    pub fn value(&self) -> Option<T::Native> {
        if self.valid() {
            T::decode(&self.data, self.index).ok()
        } else {
            None
        }
    }

    /// The backing data buffer (borrowed).
    pub fn data(&self) -> &D {
        &self.data
    }
}

impl<T: Decoder, D: IOBase> Scalar for FixedScalar<T, D> {
    type Value = T::Native;

    fn data_type_id(&self) -> DataTypeId {
        T::DATA_TYPE_ID
    }

    fn len(&self) -> usize {
        1
    }

    fn is_valid(&self, index: usize) -> bool {
        index == 0 && self.valid()
    }

    fn get(&self, index: usize) -> Option<T::Native> {
        (index == 0).then(|| self.value()).flatten()
    }
}
