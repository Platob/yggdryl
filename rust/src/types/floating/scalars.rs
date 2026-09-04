//! Floating scalar canonicalization.

use smol_str::SmolStr;

use crate::{Error, Result, Scalar};

pub(crate) enum FloatWidth {
    Float16,
    Float32,
    Float64,
}

pub(crate) fn canonical_float(value: &Scalar, width: FloatWidth) -> Result<(Scalar, bool)> {
    let Some(number) = value.as_f64() else {
        return Err(Error::InvalidRecord {
            path: SmolStr::new_static("$"),
            reason: SmolStr::new_static("validated float value could not be canonicalized"),
        });
    };
    let canonical = match width {
        FloatWidth::Float16 => Scalar::from(half::f16::from_f64(number)),
        FloatWidth::Float32 => Scalar::from(number as f32),
        FloatWidth::Float64 => Scalar::from(number),
    };
    let changed = value != &canonical;
    Ok((canonical, changed))
}
