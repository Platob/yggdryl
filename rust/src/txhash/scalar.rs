//! A value's time-coupled digest.

use crate::{DigestAlgorithm, Scalar};

use super::value::TxHash;

impl Scalar {
    /// Couple a microsecond instant with this value's digest.
    ///
    /// The digest half is exactly [`Scalar::digest`], so a value and the row
    /// it sits in answer the same digest here as anywhere else.
    ///
    /// ```
    /// use yggdryl::{DigestAlgorithm, Scalar};
    ///
    /// let value = Scalar::from("AAPL");
    /// let coupled = value.txhash(1_700_000_000_000_000, DigestAlgorithm::Xxh3);
    /// assert_eq!(coupled.unix(), 1_700_000_000_000_000);
    /// assert_eq!(coupled.digest(), value.digest(DigestAlgorithm::Xxh3));
    /// ```
    pub fn txhash(&self, unix: i64, algorithm: DigestAlgorithm) -> TxHash {
        TxHash::new(unix, self.digest(algorithm))
    }
}

impl crate::TypedScalar<'_> {
    /// Couple a microsecond instant with this value's digest.
    ///
    /// The field is proof, not content: the answer is the value's, so a
    /// `TypedScalar` and the `Scalar` inside it answer the same.
    pub fn txhash(&self, unix: i64, algorithm: DigestAlgorithm) -> TxHash {
        self.value().txhash(unix, algorithm)
    }
}

impl crate::TypedRecord<'_> {
    /// Couple a microsecond instant with this row's digest.
    ///
    /// The digest half is exactly [`crate::TypedRecord::digest`], so a row
    /// answers here what its canonical sequence answers.
    pub fn txhash(&self, unix: i64, algorithm: DigestAlgorithm) -> TxHash {
        TxHash::new(unix, self.digest(algorithm))
    }
}
