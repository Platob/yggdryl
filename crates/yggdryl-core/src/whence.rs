//! The [`Whence`] seek origin.

/// The origin a positional offset is measured from — the start, the current cursor,
/// or the end of an [`Io`](crate::Io) source (mirroring POSIX `SEEK_SET` /
/// `SEEK_CUR` / `SEEK_END`).
///
/// ```
/// use yggdryl_core::Whence;
///
/// assert_eq!(Whence::default(), Whence::Start);
/// assert_eq!(Whence::End as u8, 2);
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum Whence {
    /// From the start of the source (offset `0`).
    #[default]
    Start = 0,
    /// From the current cursor position.
    Current = 1,
    /// From the end of the source.
    End = 2,
}

// Hand-rolled serde (as the `u8` discriminant) keeps `core`'s `serde` dependency
// free of the `derive` feature.
#[cfg(feature = "serde")]
impl serde::Serialize for Whence {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        (*self as u8).serialize(serializer)
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for Whence {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        match u8::deserialize(deserializer)? {
            0 => Ok(Whence::Start),
            1 => Ok(Whence::Current),
            2 => Ok(Whence::End),
            other => Err(serde::de::Error::custom(format!(
                "invalid Whence discriminant {other}, expected 0, 1 or 2"
            ))),
        }
    }
}
