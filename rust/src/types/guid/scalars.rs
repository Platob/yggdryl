//! GUID values and the typed scalar alias.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::types::typed::define_scalar_type;
use crate::{Result, types};

/// One RFC 9562 identifier stored as its big-endian 128-bit value.
#[repr(transparent)]
#[derive(
    Clone, Copy, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(transparent)]
pub struct Guid(u128);

const _: () = assert!(std::mem::size_of::<Guid>() == 16);

impl Guid {
    /// Construct from the exact packed identifier.
    pub const fn new(value: u128) -> Self {
        Self(value)
    }

    /// Parse the accepted hyphenated, compact-hex, or 16-byte representation.
    pub fn from_bytes(value: &[u8]) -> Result<Self> {
        Ok(Self(u128::from_be_bytes(types::guid_parse(value)?)))
    }

    /// Return the packed identifier.
    pub const fn get(self) -> u128 {
        self.0
    }

    /// Return the canonical sixteen storage bytes.
    pub const fn into_bytes(self) -> [u8; 16] {
        self.0.to_be_bytes()
    }
}

impl fmt::Display for Guid {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&types::guid_text(&self.0.to_be_bytes()))
    }
}

define_scalar_type!(GuidScalar, super::GuidType, "guid", crate::DataType::Guid);
