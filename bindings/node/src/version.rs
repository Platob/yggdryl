//! The `Version` napi class.

use std::collections::HashMap;

use napi::bindgen_prelude::*;
use napi_derive::napi;
use yggdryl_version::{FromInput, ToOutput, Version as CoreVersion};

use crate::to_mapping;

/// A generic `major.minor.patch` version, ordered numerically.
#[napi]
pub struct Version {
    pub(crate) inner: CoreVersion,
}

#[napi]
impl Version {
    /// Construct from components.
    #[napi(constructor)]
    pub fn new(major: u32, minor: u32, patch: u32) -> Self {
        Version {
            inner: CoreVersion::new(major as u64, minor as u64, patch as u64),
        }
    }

    /// Parse a `major[.minor[.patch]]` string, throwing on failure. Components
    /// must be non-negative integers (at most three).
    #[napi(factory, js_name = "fromStr")]
    pub fn from_str(value: String) -> Result<Self> {
        CoreVersion::from_str(&value)
            .map(|inner| Version { inner })
            .map_err(|e| Error::from_reason(e.to_string()))
    }

    /// Build a `Version` from an object of components (`major`, `minor`, `patch`).
    #[napi(factory, js_name = "fromMapping")]
    pub fn from_mapping(fields: HashMap<String, String>) -> Result<Self> {
        CoreVersion::from_mapping(&to_mapping(fields))
            .map(|inner| Version { inner })
            .map_err(|e| Error::from_reason(e.to_string()))
    }

    /// Return a copy, overriding any component passed and keeping the rest.
    #[napi]
    pub fn copy(&self, major: Option<u32>, minor: Option<u32>, patch: Option<u32>) -> Version {
        Version {
            inner: self.inner.copy(
                major.map(u64::from),
                minor.map(u64::from),
                patch.map(u64::from),
            ),
        }
    }

    /// Return a copy with the major component replaced.
    #[napi(js_name = "withMajor")]
    pub fn with_major(&self, major: u32) -> Version {
        Version {
            inner: self.inner.with_major(major as u64),
        }
    }

    /// Return a copy with the minor component replaced.
    #[napi(js_name = "withMinor")]
    pub fn with_minor(&self, minor: u32) -> Version {
        Version {
            inner: self.inner.with_minor(minor as u64),
        }
    }

    /// Return a copy with the patch component replaced.
    #[napi(js_name = "withPatch")]
    pub fn with_patch(&self, patch: u32) -> Version {
        Version {
            inner: self.inner.with_patch(patch as u64),
        }
    }

    /// Render to a component object (the inverse of `fromMapping`).
    #[napi(js_name = "toMapping")]
    pub fn to_mapping(&self) -> std::collections::HashMap<String, String> {
        self.inner.to_mapping().into_iter().collect()
    }

    #[napi(getter)]
    pub fn major(&self) -> u32 {
        self.inner.major() as u32
    }

    #[napi(getter)]
    pub fn minor(&self) -> u32 {
        self.inner.minor() as u32
    }

    #[napi(getter)]
    pub fn patch(&self) -> u32 {
        self.inner.patch() as u32
    }

    /// Compare with another version: `-1`, `0` or `1`.
    #[napi]
    pub fn compare(&self, other: &Version) -> i32 {
        match self.inner.cmp(&other.inner) {
            std::cmp::Ordering::Less => -1,
            std::cmp::Ordering::Equal => 0,
            std::cmp::Ordering::Greater => 1,
        }
    }

    /// `true` if the two versions are equal.
    #[napi]
    pub fn equals(&self, other: &Version) -> bool {
        self.inner == other.inner
    }

    #[napi(js_name = "toString")]
    pub fn to_string_js(&self) -> String {
        self.inner.to_string()
    }
}
