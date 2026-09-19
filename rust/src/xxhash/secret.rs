//! Validation for the custom secret the XXH3 pair accepts.

use crate::{DigestAlgorithm, Error, Result};

/// The shortest custom secret XXH3 accepts, in bytes.
///
/// This is the reference implementation's `XXH3_SECRET_SIZE_MIN`. A shorter
/// secret does not seed enough accumulator lanes to reach the algorithm's
/// stated dispersion, so it is rejected rather than padded.
pub const SECRET_MINIMUM_LENGTH: usize = 136;

/// Accept a custom secret for an XXH3 algorithm.
///
/// # Errors
///
/// Returns [`Error::InvalidSecret`] naming the required and actual lengths.
pub(crate) fn validate(algorithm: DigestAlgorithm, secret: &[u8]) -> Result<()> {
    if secret.len() >= SECRET_MINIMUM_LENGTH {
        return Ok(());
    }
    Err(Error::InvalidSecret {
        algorithm: algorithm.as_str(),
        required: SECRET_MINIMUM_LENGTH,
        actual: secret.len(),
    })
}
