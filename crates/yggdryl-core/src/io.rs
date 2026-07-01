//! The positional [`Io`] array abstraction, its [`IoError`], and the in-memory
//! [`Vec`] leaf implementation.

use crate::whence::Whence;

/// An error raised by an [`Io`] operation.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum IoError {
    /// A resolved position fell outside the valid `0..=len` range.
    OutOfBounds,
    /// The source is read-only and cannot be written.
    ReadOnly,
}

impl std::fmt::Display for IoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IoError::OutOfBounds => {
                f.write_str("position out of bounds — expected an index within `0..=len`")
            }
            IoError::ReadOnly => f.write_str("source is read-only — writing is not supported"),
        }
    }
}

impl std::error::Error for IoError {}

/// A random-access array of `T` values addressed by an absolute element position
/// resolved from a [`Whence`] origin — the one abstraction every byte/array source
/// (an in-memory [`Vec`], a local file, a cloud object, an HTTP body) implements.
///
/// An implementor supplies [`len`](Io::len) and the two positional primitives
/// [`pread`](Io::pread) / [`pwrite`](Io::pwrite); the default
/// [`resolve`](Io::resolve) turns a `(position, whence)` pair into an absolute
/// index for them. The array forms [`pread_array`](Io::pread_array) /
/// [`pwrite_array`](Io::pwrite_array) default to looping those primitives, and a
/// memory-resident source overrides them for a bulk copy. A stateful source
/// overrides [`position`](Io::position) / [`seek`](Io::seek) so that
/// [`Whence::Current`] addresses its cursor and a seek retains the move. The size
/// hooks [`capacity`](Io::capacity) / [`with_capacity`](Io::with_capacity) /
/// [`resize`](Io::resize) manage the backing storage, growing it with
/// [`default`](Io::default); a growable source overrides them.
///
/// ```
/// use yggdryl_core::{Io, Whence};
///
/// let mut io = vec![1u8, 2, 3, 4];
/// assert_eq!(io.pread(1, Whence::Start).unwrap(), 2);
/// assert_eq!(io.pread_array(0, Whence::Start, 3).unwrap(), vec![1, 2, 3]);
/// io.pwrite(0, Whence::End, 5).unwrap(); // append one at the end
/// io.pwrite_array(0, Whence::End, &[6, 7]).unwrap(); // append two more
/// assert_eq!(io, vec![1, 2, 3, 4, 5, 6, 7]);
/// assert_eq!(io.seek(1, Whence::End).unwrap(), 6); // one element before the end
/// ```
pub trait Io<T> {
    /// The total number of `T` elements in the source.
    ///
    /// Named `len` for the element count; a leaf reaches through to its own storage
    /// (e.g. [`Vec::as_slice`]`().len()`) to answer it.
    fn len(&self) -> Result<usize, IoError>;

    /// Whether the source holds no elements.
    fn is_empty(&self) -> Result<bool, IoError> {
        Ok(self.len()? == 0)
    }

    /// The number of elements the source can hold before it must grow its storage.
    /// Defaults to [`len`](Io::len) for a source with no spare capacity; a growable
    /// source (e.g. a [`Vec`]) reports its true capacity.
    fn capacity(&self) -> Result<usize, IoError> {
        self.len()
    }

    /// Ensures the source can hold at least `capacity` elements without growing its
    /// storage again, never shrinking it and never changing the elements. The default
    /// is a no-op for a fixed-capacity source; a growable source reserves the room.
    fn with_capacity(&mut self, _capacity: usize) -> Result<(), IoError> {
        Ok(())
    }

    /// The value new slots are filled with when the source grows — [`T::default`].
    /// A source overrides it to fill with a different sentinel.
    fn default(&self) -> T
    where
        T: Default,
    {
        T::default()
    }

    /// The current cursor position (an element index), against which
    /// [`Whence::Current`] is resolved. Defaults to `0` for a cursorless source; a
    /// stateful source overrides it.
    fn position(&self) -> Result<usize, IoError> {
        Ok(0)
    }

    /// Resolves `position` measured from `whence` to an absolute element index in
    /// `0..=len`. A [`Whence::Start`] / [`Whence::Current`] offset counts forward
    /// (from the start / the cursor); a [`Whence::End`] offset counts back from the
    /// end. Errors [`OutOfBounds`](IoError::OutOfBounds) when it falls outside the
    /// source.
    fn resolve(&self, position: usize, whence: Whence) -> Result<usize, IoError> {
        let len = self.len()?;
        let index = match whence {
            Whence::Start => position,
            Whence::Current => self
                .position()?
                .checked_add(position)
                .ok_or(IoError::OutOfBounds)?,
            Whence::End => len.checked_sub(position).ok_or(IoError::OutOfBounds)?,
        };
        if index > len {
            return Err(IoError::OutOfBounds);
        }
        Ok(index)
    }

    /// Moves the cursor to `position` measured from `whence`, returning the new
    /// absolute cursor position (an index in `0..=len`). The default resolves the
    /// target without retaining it — a cursorless source (e.g. a bare [`Vec`]) has a
    /// fixed cursor at the start, so a stateful source overrides this to store the
    /// move. Errors [`OutOfBounds`](IoError::OutOfBounds) when the target falls
    /// outside the source.
    fn seek(&mut self, position: usize, whence: Whence) -> Result<usize, IoError> {
        self.resolve(position, whence)
    }

    /// Reads the single `T` at `position` measured from `whence`. Errors
    /// [`OutOfBounds`](IoError::OutOfBounds) if the resolved position is at or past
    /// the end, where no element lives.
    fn pread(&self, position: usize, whence: Whence) -> Result<T, IoError>;

    /// Reads up to `len` elements starting at `position` measured from `whence`,
    /// returning those actually available there (fewer than `len` near the end,
    /// empty at the end). The default loops [`pread`](Io::pread); a memory-resident
    /// source overrides it for a bulk copy. Errors
    /// [`OutOfBounds`](IoError::OutOfBounds) if the resolved position is past the end.
    fn pread_array(&self, position: usize, whence: Whence, len: usize) -> Result<Vec<T>, IoError> {
        let start = self.resolve(position, whence)?;
        let count = len.min(self.len()?.saturating_sub(start));
        let mut out = Vec::with_capacity(count);
        for offset in 0..count {
            out.push(self.pread(start + offset, Whence::Start)?);
        }
        Ok(out)
    }

    /// Writes `value` at `position` measured from `whence` — overwriting the element
    /// there, or appending it when the position is exactly the end. Errors
    /// [`ReadOnly`](IoError::ReadOnly) on a read-only source or
    /// [`OutOfBounds`](IoError::OutOfBounds) if the resolved position is past the end.
    fn pwrite(&mut self, position: usize, whence: Whence, value: T) -> Result<(), IoError>;

    /// Writes `values` at `position` measured from `whence` — overwriting, and
    /// extending the source when the write runs past the end — and returns the number
    /// of elements written. The default loops [`pwrite`](Io::pwrite); a
    /// memory-resident source overrides it for a bulk copy. Errors as
    /// [`pwrite`](Io::pwrite) does.
    fn pwrite_array(
        &mut self,
        position: usize,
        whence: Whence,
        values: &[T],
    ) -> Result<usize, IoError>
    where
        T: Clone,
    {
        let start = self.resolve(position, whence)?;
        for (offset, value) in values.iter().enumerate() {
            self.pwrite(start + offset, Whence::Start, value.clone())?;
        }
        Ok(values.len())
    }

    /// Resizes the source to `len` elements, filling the new slots with
    /// [`default`](Io::default) when it grows. The default can only grow (it appends
    /// through [`pwrite_array`](Io::pwrite_array)); a shrink is skipped, as the base
    /// trait cannot truncate, so a source whose storage supports truncation overrides
    /// this to also shrink.
    fn resize(&mut self, len: usize) -> Result<(), IoError>
    where
        T: Default + Clone,
    {
        let current = self.len()?;
        if len > current {
            let fill = vec![self.default(); len - current];
            self.pwrite_array(current, Whence::Start, &fill)?;
        } else if len < current {
            crate::log_event!(
                warn,
                "Io::resize skipping shrink {current} -> {len}; override resize to truncate"
            );
        }
        Ok(())
    }
}

/// [`Vec`] is the in-memory array leaf: a read clones the element at the position; a
/// write overwrites it, or appends when the position is exactly the end (the
/// resolved position is `<= len`, so a write never opens a gap).
impl<T: Clone> Io<T> for Vec<T> {
    fn len(&self) -> Result<usize, IoError> {
        Ok(self.as_slice().len())
    }

    fn capacity(&self) -> Result<usize, IoError> {
        Ok(Vec::capacity(self))
    }

    fn with_capacity(&mut self, capacity: usize) -> Result<(), IoError> {
        self.reserve(capacity.saturating_sub(self.as_slice().len()));
        Ok(())
    }

    fn resize(&mut self, len: usize) -> Result<(), IoError>
    where
        T: Default,
    {
        let fill = self.default();
        crate::log_event!(trace, "Vec::resize len={len}");
        Vec::resize(self, len, fill);
        Ok(())
    }

    fn pread(&self, position: usize, whence: Whence) -> Result<T, IoError> {
        let at = self.resolve(position, whence)?;
        crate::log_event!(trace, "Vec::pread at={at}");
        self.get(at).cloned().ok_or(IoError::OutOfBounds)
    }

    fn pread_array(&self, position: usize, whence: Whence, len: usize) -> Result<Vec<T>, IoError> {
        let start = self.resolve(position, whence)?;
        crate::log_event!(trace, "Vec::pread_array start={start} len={len}");
        let end = start.saturating_add(len).min(self.as_slice().len());
        Ok(self[start..end].to_vec())
    }

    fn pwrite(&mut self, position: usize, whence: Whence, value: T) -> Result<(), IoError> {
        let at = self.resolve(position, whence)?;
        crate::log_event!(trace, "Vec::pwrite at={at}");
        if at == self.as_slice().len() {
            self.push(value);
        } else {
            self[at] = value;
        }
        Ok(())
    }

    fn pwrite_array(
        &mut self,
        position: usize,
        whence: Whence,
        values: &[T],
    ) -> Result<usize, IoError> {
        let start = self.resolve(position, whence)?;
        crate::log_event!(
            trace,
            "Vec::pwrite_array start={start} len={}",
            values.len()
        );
        let overlap = values.len().min(self.as_slice().len() - start);
        self[start..start + overlap].clone_from_slice(&values[..overlap]);
        self.extend_from_slice(&values[overlap..]);
        Ok(values.len())
    }
}
