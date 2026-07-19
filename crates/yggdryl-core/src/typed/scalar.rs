//! [`Scalar`] — the base of the typed-value hierarchy: an **indexed, possibly-null typed value**.
//!
//! A `Scalar` is the common surface [`Serie`](super::Serie) refines (a `Serie` *is* a `Scalar` with
//! more than one element): a length, per-index validity, and a null-aware [`get`](Scalar::get). The
//! concrete [`FixedScalar`] is the degenerate single-element case — one value in an [`IOBase`] data
//! buffer, with an optional validity bit — borrowing the byte layer for its read.

use core::marker::PhantomData;

use super::field::{cast_dtype_error, cast_null_error};
use super::{DataType, Decoder, Encoder, Field, HeaderField};
use crate::datatype_id::DataTypeId;
use crate::headers::Headers;
use crate::io::memory::{Heap, IOBase, IoError};

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
#[derive(Clone)]
pub struct FixedScalar<T: DataType, D: IOBase = Heap> {
    data: D,
    validity: Option<D>,
    index: u64,
    /// The scalar's field **name**, if set — reported by [`field`](FixedScalar::field), set by a
    /// [`cast_field`](FixedScalar::cast_field).
    name: Option<Box<str>>,
    /// Free-form field annotations — carried onto the [`field`](FixedScalar::field) and set by a
    /// [`cast_field`](FixedScalar::cast_field). Empty for a plain scalar.
    metadata: Headers,
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
            name: None,
            metadata: Headers::new(),
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
            name: None,
            metadata: Headers::new(),
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

    /// The scalar's [`Field`](super::Field) metadata — its `name`, `type_id`, `nullable` flag
    /// (whether it carries a validity bit), and any free-form annotations carried by a
    /// [`cast_field`](FixedScalar::cast_field).
    pub fn field(&self) -> HeaderField {
        let nullable = self.validity.is_some();
        let mut field = HeaderField::new(self.name.as_deref(), T::DATA_TYPE_ID, nullable);
        for (name, value) in self.metadata.iter() {
            field.metadata_mut().append_bytes(name, value);
        }
        field
    }

    /// A fresh scalar reshaped to `field` — the non-mutating front door of
    /// [`cast_field_in_place`](FixedScalar::cast_field_in_place) (`clone → cast_field_in_place`).
    pub fn cast_field(&self, field: &HeaderField) -> Result<Self, IoError>
    where
        D: Default + Clone,
    {
        let mut out = self.clone();
        out.cast_field_in_place(field)?;
        Ok(out)
    }

    /// Reshapes this scalar **in place** to match `field`'s metadata, keeping the element type: a
    /// no-op when `field` already matches (same dtype, nullability, name, annotations); otherwise
    /// applies the target **nullability** (non-nullable → nullable marks the element valid;
    /// nullable → non-nullable requires the element be non-null, else the guided
    /// [`IoError::TypedCast`]), the target **name**, and the target's free-form **annotations**. A
    /// **different dtype** is the guided [`IoError::TypedCast`] (the typed scalar keeps its element
    /// type — a runtime dtype change belongs to the erased layer).
    // DESIGN: mirrors `FixedSerie::cast_field_in_place` — a scalar is the one-element column, so its
    // nullability toggles whether the single value is a null.
    pub fn cast_field_in_place(&mut self, field: &HeaderField) -> Result<(), IoError>
    where
        D: Default,
    {
        let target = field.data_type_id();
        if target != T::DATA_TYPE_ID {
            return Err(cast_dtype_error("FixedScalar", T::DATA_TYPE_ID, target));
        }

        let to_nullable = field.nullable();
        let is_nullable = self.validity.is_some();
        let extra = field.extra_annotations();

        // Same dtype, nullability, name, and annotations — nothing to do.
        if is_nullable == to_nullable
            && field.headers().name() == self.name.as_deref()
            && extra == self.metadata
        {
            return Ok(());
        }

        // Validate the fallible step first (a rejected cast leaves `self` untouched).
        if is_nullable && !to_nullable {
            let nulls = self.null_count();
            if nulls > 0 {
                return Err(cast_null_error(nulls));
            }
        }

        if !is_nullable && to_nullable {
            // Add a validity buffer marking the one element valid (it carries a value).
            let mut validity = D::default();
            validity
                .pwrite_bit(self.index, true)
                .expect("bit write into a fresh backing never fails");
            self.validity = Some(validity);
        } else if is_nullable && !to_nullable {
            self.validity = None; // verified non-null above
        }
        self.name = field.headers().name().map(Into::into);
        self.metadata = extra;
        Ok(())
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
