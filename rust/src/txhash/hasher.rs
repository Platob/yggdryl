//! One configuration - resolution, algorithm, seed, secret - answering many
//! values.

use crate::{DataType, Digest, DigestAlgorithm, Digester, Result, Scalar, TimeUnit};

use super::time::{DEFAULT_UNIT, unix_from_scalar, validate_unit};
use super::value::{TxHash, dtype, width};

/// A clock resolution and a digest configuration, held once.
///
/// The one-shot functions answer at microseconds with the default seed. A
/// hasher is the form for everything else: another resolution, a seed, an
/// XXH3 secret, or a runtime-chosen algorithm - settled once, then applied
/// to every value, row, or batch the same way. The state it carries is a
/// prototype: bytes are never accumulated in it, so answering never changes
/// it and one hasher serves any number of threads' clones.
///
/// ```
/// use yggdryl::txhash::TxHasher;
/// use yggdryl::{DigestAlgorithm, Scalar, TimeUnit, xxhash};
///
/// # fn main() -> yggdryl::Result<()> {
/// let seconds = TxHasher::new_in(TimeUnit::Second, DigestAlgorithm::Xxh64)?.with_seed(7);
/// let value = seconds.digest(b"AAPL", 1_700_000_000);
/// assert_eq!(value.unit(), TimeUnit::Second);
/// assert_eq!(value.digest().as_u64(), Some(xxhash::xxh64_with_seed(b"AAPL", 7)));
///
/// // A value's canonical feed, under the same configuration.
/// let row = seconds.digest_scalar(&Scalar::from_sequence([Scalar::from("AAPL")]), 1_700_000_000);
/// assert_eq!(row.unix(), 1_700_000_000);
/// assert_ne!(row.digest(), value.digest());
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug)]
pub struct TxHasher {
    unit: TimeUnit,
    prototype: Digester,
}

impl TxHasher {
    /// Start at microseconds with the default seed.
    pub fn new(algorithm: DigestAlgorithm) -> Self {
        Self {
            unit: DEFAULT_UNIT,
            prototype: algorithm.digester(),
        }
    }

    /// Start at an explicit clock resolution with the default seed.
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::InvalidDataType`] when `unit` is not a clock
    /// resolution.
    pub fn new_in(unit: TimeUnit, algorithm: DigestAlgorithm) -> Result<Self> {
        validate_unit(unit)?;
        Ok(Self {
            unit,
            prototype: algorithm.digester(),
        })
    }

    /// Start from a configured state, keeping its algorithm, seed, and secret.
    ///
    /// This is how a custom XXH3 secret reaches a hasher: build the concrete
    /// state with it and hand it over. Bytes already fed to the state are
    /// discarded, because a hasher is a configuration rather than a running
    /// digest.
    ///
    /// ```
    /// use yggdryl::TimeUnit;
    /// use yggdryl::txhash::TxHasher;
    /// use yggdryl::xxhash::{self, Xxh3};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let secret = vec![0x5a_u8; xxhash::SECRET_MINIMUM_LENGTH];
    /// let mut state = Xxh3::from_seed_and_secret(3, &secret)?;
    /// state.write_bytes(b"already fed, and forgotten");
    /// let hasher = TxHasher::from_digester(TimeUnit::Millisecond, state.into())?;
    ///
    /// let long = vec![0x11_u8; 241];
    /// assert_eq!(
    ///     hasher.digest(&long, 5).digest().as_u64(),
    ///     Some(xxhash::xxh3_with_seed_and_secret(&long, 3, &secret)?),
    /// );
    /// assert_eq!(hasher.unit(), TimeUnit::Millisecond);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::InvalidDataType`] when `unit` is not a clock
    /// resolution.
    pub fn from_digester(unit: TimeUnit, digester: Digester) -> Result<Self> {
        validate_unit(unit)?;
        let mut prototype = digester;
        prototype.clear();
        Ok(Self { unit, prototype })
    }

    /// Return this hasher digesting under an explicit seed.
    ///
    /// A secret does not survive: a seed and a secret are one configuration,
    /// so a seeded secret is built as one state and given to
    /// [`Self::from_digester`].
    #[must_use]
    pub fn with_seed(self, seed: u64) -> Self {
        Self {
            unit: self.unit,
            prototype: self.algorithm().digester_with_seed(seed),
        }
    }

    /// Return the clock resolution every answer counts its instant in.
    pub const fn unit(&self) -> TimeUnit {
        self.unit
    }

    /// Return the algorithm every answer's digest half computes.
    pub const fn algorithm(&self) -> DigestAlgorithm {
        self.prototype.algorithm()
    }

    /// Return the width of every answer's canonical bytes.
    pub const fn width(&self) -> usize {
        width(self.algorithm())
    }

    /// Return the datatype a column of answers is stored under.
    pub const fn dtype(&self) -> DataType {
        dtype(self.algorithm())
    }

    /// Borrow the configured state.
    ///
    /// A caller feeding bytes by hand clones it; the prototype itself stays
    /// untouched.
    pub const fn digester(&self) -> &Digester {
        &self.prototype
    }

    /// Couple an instant, already counted in [`Self::unit`], with a buffer's
    /// digest.
    pub fn digest(&self, input: &[u8], unix: i64) -> TxHash {
        let mut state = self.prototype.clone();
        state.write_bytes(input);
        self.couple(unix, state.as_digest())
    }

    /// Couple an instant with a value's canonical feed.
    ///
    /// See [`Scalar::write_bytes`] for the encoding and what it guarantees.
    pub fn digest_scalar(&self, value: &Scalar, unix: i64) -> TxHash {
        let mut state = self.prototype.clone();
        state.write_scalar(value);
        self.couple(unix, state.as_digest())
    }

    /// Read any instant as a unix count of [`Self::unit`].
    ///
    /// [`unix_from_scalar`] carries the rule and the spellings it accepts.
    ///
    /// # Errors
    ///
    /// Returns an error when the value names no instant.
    pub fn unix_of(&self, time: &Scalar) -> Result<i64> {
        unix_from_scalar(time, self.unit)
    }

    /// Couple an instant with a digest this hasher already answered.
    fn couple(&self, unix: i64, digest: Digest) -> TxHash {
        TxHash::new_in(unix, self.unit, digest)
            .unwrap_or_else(|_| unreachable!("the unit was validated at construction"))
    }

    /// Fill the digest holders in one Arrow batch under `root`.
    ///
    /// The configured state is the prototype the fill reads its algorithm,
    /// seed, and secret from; [`Digester::apply_arrow_batch`] carries the
    /// rule. A holder coupling an instant reads its own `digest:unit`, not
    /// this hasher's, because the resolution a column stores is a fact about
    /// the schema.
    ///
    /// # Errors
    ///
    /// Returns an error when `root` is not a usable Struct schema, a holder
    /// has the wrong width, a digest or time path cannot be resolved, or the
    /// batch cannot be cast to `root`.
    #[cfg(feature = "arrow")]
    pub fn apply_arrow_batch(
        &self,
        root: &crate::Field,
        batch: arrow_array::RecordBatch,
        force: bool,
    ) -> crate::arrow::Result<arrow_array::RecordBatch> {
        self.prototype.apply_arrow_batch(root, batch, force)
    }

    /// Couple every row's digest with the instant beside it.
    ///
    /// The seeded form of [`super::arrow::row_txhashes`], which carries the
    /// rule.
    ///
    /// # Errors
    ///
    /// Returns an error when `times` is not an instant column of the batch's
    /// length, or a value cannot be represented.
    #[cfg(feature = "arrow")]
    pub fn row_txhashes(
        &self,
        batch: &arrow_array::RecordBatch,
        times: &dyn arrow_array::Array,
    ) -> crate::arrow::Result<arrow_array::ArrayRef> {
        super::arrow::row_txhashes_with(&self.prototype, batch, times, self.unit)
    }

    /// Couple every cell's digest with the instant beside it.
    ///
    /// The seeded form of [`super::arrow::column_txhashes`], which carries
    /// the rule.
    ///
    /// # Errors
    ///
    /// Returns an error when `times` is not an instant column of the array's
    /// length, the array cannot be reconciled to `field`, or a value cannot
    /// be represented.
    #[cfg(feature = "arrow")]
    pub fn column_txhashes(
        &self,
        times: &dyn arrow_array::Array,
        values: arrow_array::ArrayRef,
        field: &crate::Field,
    ) -> crate::arrow::Result<arrow_array::ArrayRef> {
        super::arrow::column_txhashes_with(&self.prototype, times, values, field, self.unit)
    }
}
