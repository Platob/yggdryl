//! Node wrapper for [`yggdryl_core::Whence`].

use napi_derive::napi;
use yggdryl_core::Whence as CoreWhence;

/// Where a `Binary.seek` offset is measured from.
#[napi]
pub enum Whence {
    /// From the start of the buffer (absolute).
    Start,
    /// From the current cursor position.
    Current,
    /// From the end of the buffer.
    End,
}

impl From<Whence> for CoreWhence {
    fn from(whence: Whence) -> Self {
        match whence {
            Whence::Start => CoreWhence::Start,
            Whence::Current => CoreWhence::Current,
            Whence::End => CoreWhence::End,
        }
    }
}
