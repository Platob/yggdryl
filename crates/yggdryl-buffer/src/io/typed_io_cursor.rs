//! [`TypedIOCursor<T>`] — an element-typed cursor.

use crate::{IOCursor, IoPrimitive, TypedIOBase};

/// A cursor that is both a [`TypedIOBase<T>`] and an [`IOCursor`]: typed reads and
/// writes over an inner resource, tracking a position.
///
/// The blanket impl makes every type that is both `TypedIOBase<T>` and `IOCursor`
/// a `TypedIOCursor<T>` automatically — e.g. [`ByteCursor`](crate::ByteCursor) is a
/// `TypedIOCursor<u8>`.
#[allow(clippy::upper_case_acronyms)] // `IO` matches the project's IO-trait naming.
pub trait TypedIOCursor<T: IoPrimitive>: TypedIOBase<T> + IOCursor {}

impl<T: IoPrimitive, C> TypedIOCursor<T> for C where C: TypedIOBase<T> + IOCursor {}
