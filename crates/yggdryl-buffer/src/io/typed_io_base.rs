//! [`TypedIOBase<T>`] — cursor IO of an arbitrary element type.

use crate::{IOBase, IoError, IoPrimitive, Whence};

/// The wire width of `T` in bytes ([`IoPrimitive::WIDTH`], floored at 1) — the
/// serialized element width, which for `i96` differs from `size_of::<i96>()`.
fn width<T: IoPrimitive>() -> u64 {
    T::WIDTH.max(1) as u64
}

/// Reads and writes of `T` values over a cursor, layered on [`IOBase`].
///
/// Like [`IOBase`], operations resolve their start via `whence` and **advance** the
/// cursor. `TypedIOBase<T>` generalises the byte surface to a single `T`
/// ([`pread_one`](TypedIOBase::pread_one) / [`pwrite_one`](TypedIOBase::pwrite_one))
/// or an array of `T` ([`pread_array`](TypedIOBase::pread_array) /
/// [`pwrite_array`](TypedIOBase::pwrite_array)), reports its extent in `T`
/// units ([`size`](TypedIOBase::size) / [`capacity`](TypedIOBase::capacity)), and
/// carries a `T`-unit position ([`tell`](TypedIOBase::tell) /
/// [`seek`](TypedIOBase::seek)) alongside [`IOBase`]'s byte position. The `T = u8`
/// case coincides with [`IOBase`].
///
/// ```
/// use yggdryl_buffer::{ByteBuffer, IOBase, TypedIOBase, Whence};
///
/// let mut cursor = ByteBuffer::new().byte_cursor(); // TypedIOBase<u8>
/// cursor.pwrite_array(&[1, 2, 3], Whence::Start).unwrap();
/// assert_eq!(cursor.pread_array(3, Whence::Start).unwrap(), vec![1, 2, 3]);
/// ```
#[allow(clippy::upper_case_acronyms)] // `IO` matches the project's IO-trait naming.
pub trait TypedIOBase<T: IoPrimitive>: IOBase {
    /// Creates a cursor over a fresh resource able to hold `capacity` `T` values.
    fn with_capacity(capacity: usize) -> Self
    where
        Self: Sized,
    {
        Self::with_byte_capacity(capacity.saturating_mul(T::WIDTH.max(1)))
    }

    /// Reads a single `T` at `whence`, advancing the cursor past it.
    fn pread_one(&mut self, whence: Whence) -> Result<T, IoError>;

    /// Writes a single `T` at `whence`, advancing the cursor; returns the number of
    /// `T` values written (`1` on success).
    fn pwrite_one(&mut self, value: T, whence: Whence) -> Result<usize, IoError>;

    /// Reads up to `count` `T` values at `whence`, advancing the cursor.
    fn pread_array(&mut self, count: usize, whence: Whence) -> Result<Vec<T>, IoError>;

    /// Writes the `T` values in `data` at `whence`, advancing the cursor; returns
    /// the number of `T` values written.
    fn pwrite_array(&mut self, data: &[T], whence: Whence) -> Result<usize, IoError>;

    /// The number of `T` values **remaining** from the current position to the end
    /// ([`byte_size`](IOBase::byte_size) — the remaining bytes — divided by the width
    /// of `T`).
    fn size(&self) -> Result<usize, IoError> {
        Ok(self.byte_size()? / T::WIDTH.max(1))
    }

    /// The number of `T` values the resource can hold without reallocating
    /// ([`byte_capacity`](IOBase::byte_capacity) divided by the width of `T`).
    fn capacity(&self) -> Result<usize, IoError> {
        Ok(self.byte_capacity()? / T::WIDTH.max(1))
    }

    /// The current position in `T` units from the start
    /// ([`byte_tell`](IOBase::byte_tell) divided by the width of `T`). The `T = u8`
    /// case coincides with [`byte_tell`](IOBase::byte_tell).
    fn tell(&self) -> Result<u64, IoError> {
        Ok(self.byte_tell()? / width::<T>())
    }

    /// Moves the cursor to `offset` `T` values relative to `whence`, returning the
    /// new absolute position in `T` units. A negative `offset` seeks backward (from
    /// `Current` / `End`); a resolved position before the start is an
    /// [`IoError::InvalidSeek`]. The `End` origin resolves against the resource's
    /// **total** extent (like [`byte_seek`](IOBase::byte_seek)), not the remaining.
    fn seek(&mut self, offset: i64, whence: Whence) -> Result<u64, IoError> {
        let width = width::<T>();
        // Convert the `T`-unit offset to bytes and delegate, so the `End` origin
        // resolves against the total extent (`size` reports the *remaining* `T`s).
        let byte_offset = i128::from(offset)
            .checked_mul(i128::from(width))
            .and_then(|b| i64::try_from(b).ok())
            .ok_or(IoError::InvalidSeek { offset, whence })?;
        let byte = self.byte_seek(byte_offset, whence)?;
        Ok(byte / width)
    }

    /// The default `T` value used to fill a gap opened past the end on a grow —
    /// [`IoPrimitive::ZERO`], zero for every primitive.
    fn default_value(&self) -> T {
        T::ZERO
    }

    /// The little-endian bytes of `count` [`default_value`](TypedIOBase::default_value)
    /// values — the pattern a typed write fills into any gap it opens past the end of
    /// the resource. For every native primitive the default is zero, so this is
    /// zero-fill.
    fn default_byte_array(&self, count: usize) -> Vec<u8> {
        self.default_value().to_le_vec().repeat(count)
    }
}
