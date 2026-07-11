//! [`TypedIOSlice<T>`] — an element-typed bounded window.

use crate::{IOSlice, IoPrimitive, TypedIOBase};

/// A slice that is both a [`TypedIOBase<T>`] and an [`IOSlice`]: typed reads and writes
/// confined to a fixed window, reporting the window's byte bounds.
///
/// The blanket impl makes every type that is both `TypedIOBase<T>` and `IOSlice` a
/// `TypedIOSlice<T>` automatically — e.g. [`TypedSlice<T>`](crate::TypedSlice) and
/// [`ByteSlice`](crate::ByteSlice) (a `TypedIOSlice<u8>`).
#[allow(clippy::upper_case_acronyms)] // `IO` matches the project's IO-trait naming.
pub trait TypedIOSlice<T: IoPrimitive>: TypedIOBase<T> + IOSlice {}

impl<T: IoPrimitive, S> TypedIOSlice<T> for S where S: TypedIOBase<T> + IOSlice {}
