//! An instant coupled with an xxHash digest, and the configuration that
//! answers many.
//!
//! [`JsTxHash`] is the value: a unix count, UTC, at a clock resolution, and
//! the digest behind it. [`JsTxHasher`] is one configuration - resolution,
//! algorithm, seed, secret - applied to bytes, values, and batches. The
//! JavaScript loader owns the instant intake: a `bigint` or an integer
//! `number` is the count already, and a `Date`, a string, or a `Scalar`
//! crosses as a native value the core's one intake reads, so the accepted
//! spellings are its.

use napi::JsDate;
use napi::bindgen_prelude::{BigInt, Buffer, ClassInstance, Either, Result, Uint8Array};
use napi_derive::napi;

use yggdryl::txhash::{self, TxHash, TxHasher};
use yggdryl::xxhash::{Xxh3, Xxh32, Xxh64, Xxh128};
use yggdryl::{Scalar, TimeUnit};

use crate::napi_error;
use crate::text::codec::JsScalar;
use crate::types::datatype::JsDataType;
use crate::types::field::JsField;
use crate::xxhash::{
    JsDigest, JsXxh3, JsXxh32, JsXxh64, JsXxh128, algorithm_from_str, apply_arrow_batch_ipc,
    content_bytes, seed_from_bigint,
};

/// Anything the loader hands a native entry point as an instant.
///
/// Spelled out at every boundary rather than used as the parameter type, for
/// the reason the digest content is: NAPI emits no alias for one.
pub type UnixContent<'content> = Either<
    BigInt,
    Either<f64, Either<String, Either<JsDate<'content>, ClassInstance<'content, JsScalar>>>>,
>;

/// The widest integer a JavaScript `number` states exactly.
const SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;

/// Parse a clock resolution, the microsecond default when none is named.
fn unit_from_js(value: Option<String>) -> Result<TimeUnit> {
    match value {
        Some(value) => TimeUnit::from_str(&value).map_err(napi_error),
        None => Ok(txhash::DEFAULT_UNIT),
    }
}

/// Read an instant as a unix count of `unit`.
///
/// A `bigint` and an integer `number` are the count already; a string, a
/// `Date`, and a `Scalar` take the core's one intake, the `Date` as the UTC
/// millisecond instant every crossing reads it as. Nothing here builds a
/// `Scalar` object on the JavaScript side: each shape crosses as itself.
fn unix_from_js(value: UnixContent<'_>, unit: TimeUnit) -> Result<i64> {
    let count = match value {
        Either::A(count) => {
            let (count, lossless) = count.get_i64();
            if !lossless {
                return Err(napi_error("instant must fit a signed 64-bit unix count"));
            }
            count
        }
        Either::B(Either::A(number)) => {
            if number.fract() != 0.0 || number.abs() > SAFE_INTEGER {
                return Err(napi_error(
                    "instant must be a safe integer, a bigint, a Date, a string, or a Scalar",
                ));
            }
            #[allow(clippy::cast_possible_truncation)]
            {
                number as i64
            }
        }
        Either::B(Either::B(Either::A(text))) => {
            return txhash::unix_from_scalar(&Scalar::from(text.as_str()), unit)
                .map_err(napi_error);
        }
        Either::B(Either::B(Either::B(Either::A(date)))) => {
            let instant = crate::text::codec::scalar_from_date_millis(date.value_of()?)?;
            return txhash::unix_from_scalar(&instant, unit).map_err(napi_error);
        }
        Either::B(Either::B(Either::B(Either::B(scalar)))) => {
            return txhash::unix_from_scalar(&scalar.inner, unit).map_err(napi_error);
        }
    };
    txhash::unix_from_scalar(&Scalar::from(count), unit).map_err(napi_error)
}

/// One instant coupled with one digest.
///
/// `bytes()` is the canonical layout - the instant big-endian, then the
/// digest - and `toString()` the `<unix>@<unit>:<algorithm>:<hex>` spelling
/// `TxHash.from` reads back. Values order by unit, instant, then digest,
/// which is the order their bytes sort in from the epoch on.
#[napi(js_name = "TxHash")]
pub struct JsTxHash {
    inner: TxHash,
}

impl Clone for JsTxHash {
    fn clone(&self) -> Self {
        Self::from_core(self.inner)
    }
}

impl JsTxHash {
    pub(crate) const fn from_core(inner: TxHash) -> Self {
        Self { inner }
    }
}

#[napi]
impl JsTxHash {
    /// Parse the canonical `<unix>@<unit>:<algorithm>:<hex>` spelling.
    #[napi(constructor)]
    pub fn new(value: String) -> Result<Self> {
        TxHash::from_str(&value)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// Parse the canonical spelling, or clone a native value.
    #[napi(factory, js_name = "from")]
    pub fn from_js(value: Either<String, &JsTxHash>) -> Result<Self> {
        match value {
            Either::A(value) => Self::new(value),
            Either::B(value) => Ok(value.clone()),
        }
    }

    /// Rebuild a value from its canonical bytes.
    #[napi(factory)]
    pub fn from_bytes(unit: String, algorithm: String, data: Uint8Array) -> Result<Self> {
        TxHash::from_bytes(
            unit_from_js(Some(unit))?,
            algorithm_from_str(&algorithm)?,
            data.as_ref(),
        )
        .map(Self::from_core)
        .map_err(napi_error)
    }

    /// Couple an instant with a digest already computed.
    ///
    /// The loader owns the instant intake and publishes this as `fromParts`.
    #[napi(factory, js_name = "_fromPartsNative", skip_typescript)]
    pub fn from_parts_native(
        instant: UnixContent<'_>,
        digest: &JsDigest,
        unit: Option<String>,
    ) -> Result<Self> {
        let unit = unit_from_js(unit)?;
        let count = unix_from_js(instant, unit)?;
        TxHash::new_in(count, unit, digest.inner())
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// The instant as a unix count of `unit`.
    #[napi(getter)]
    pub fn unix(&self) -> BigInt {
        BigInt::from(self.inner.unix())
    }

    /// The clock resolution the instant is counted in.
    #[napi(getter)]
    pub fn unit(&self) -> String {
        self.inner.unit().as_str().to_owned()
    }

    /// The canonical algorithm token of the digest half.
    #[napi(getter)]
    pub fn algorithm(&self) -> String {
        self.inner.algorithm().as_str().to_owned()
    }

    /// The width of the canonical bytes.
    #[napi(getter)]
    pub fn width(&self) -> u32 {
        u32::try_from(self.inner.width())
            .unwrap_or_else(|_| unreachable!("no coupled value is wider than 24 bytes"))
    }

    /// The digest half, carrying its algorithm.
    #[napi(getter)]
    pub fn digest(&self) -> JsDigest {
        JsDigest::from_core(self.inner.digest())
    }

    /// The datatype a column of values like this one is stored under.
    #[napi(getter)]
    pub fn dtype(&self) -> JsDataType {
        JsDataType::from_core(self.inner.dtype())
    }

    /// Restate the instant at another clock resolution, keeping the digest.
    #[napi]
    pub fn with_unit(&self, unit: String) -> Result<JsTxHash> {
        self.inner
            .with_unit(unit_from_js(Some(unit))?)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// The canonical bytes: the instant big-endian, then the digest.
    #[napi]
    pub fn bytes(&self) -> Uint8Array {
        Uint8Array::new(self.inner.into_bytes().to_vec())
    }

    /// The instant as a UTC datetime `Scalar` at this value's resolution.
    #[napi]
    #[allow(clippy::wrong_self_convention)] // Binding `into_*` methods do not consume wrappers.
    pub fn into_datetime(&self) -> JsScalar {
        JsScalar::from_core(self.inner.into_datetime())
    }

    /// The canonical bytes as a fixed-width byte `Scalar`.
    #[napi]
    #[allow(clippy::wrong_self_convention)] // Binding `into_*` methods do not consume wrappers.
    pub fn into_scalar(&self) -> JsScalar {
        JsScalar::from_core(self.inner.into_scalar())
    }

    /// Exact equality: another unit or algorithm is another value.
    #[napi]
    pub fn equals(&self, other: &JsTxHash) -> bool {
        self.inner == other.inner
    }

    /// Total native ordering: `-1`, `0`, or `1`.
    #[napi]
    pub fn compare(&self, other: &JsTxHash) -> i32 {
        crate::ordering_value(self.inner.cmp(&other.inner))
    }

    /// A deterministic cross-language hash of this value.
    #[napi]
    pub fn stable_hash(&self) -> u64 {
        self.inner.stable_hash()
    }

    /// Make a cheap native clone.
    #[napi(js_name = "clone")]
    pub fn clone_js(&self) -> Self {
        self.clone()
    }

    /// Return the canonical spelling, accepted losslessly by `TxHash.from`.
    #[napi(js_name = "toString")]
    pub fn js_string(&self) -> String {
        self.inner.to_string()
    }

    /// Serialize as the canonical spelling, so a value survives
    /// `JSON.stringify`.
    #[napi(js_name = "toJSON")]
    pub fn js_json(&self) -> String {
        self.inner.to_string()
    }
}

/// One resolution and one digest configuration, applied to many values.
///
/// The one-shot functions answer at microseconds with the default seed; this
/// is the form for another resolution, a seed, an XXH3 secret carried in
/// through a configured state, or an algorithm read from configuration.
#[napi(js_name = "TxHasher")]
pub struct JsTxHasher {
    inner: TxHasher,
}

impl Clone for JsTxHasher {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

#[napi]
impl JsTxHasher {
    /// Start a hasher for one algorithm at one resolution, optionally seeded.
    #[napi(constructor)]
    pub fn new(
        algorithm: Option<String>,
        unit: Option<String>,
        seed: Option<BigInt>,
    ) -> Result<Self> {
        let algorithm = match algorithm {
            Some(algorithm) => algorithm_from_str(&algorithm)?,
            None => yggdryl::DigestAlgorithm::Xxh3,
        };
        let hasher = TxHasher::new_in(unit_from_js(unit)?, algorithm).map_err(napi_error)?;
        Ok(Self {
            inner: match seed {
                Some(seed) => hasher.with_seed(seed_from_bigint(Some(seed))?),
                None => hasher,
            },
        })
    }

    /// Start from a configured state, keeping its algorithm, seed, and secret.
    ///
    /// This is how a custom XXH3 secret reaches a hasher: build `Xxh3` or
    /// `Xxh128` with it and hand it over. Bytes already fed to the state are
    /// discarded.
    #[napi(factory)]
    pub fn from_state(
        state: Either<&JsXxh32, Either<&JsXxh64, Either<&JsXxh3, &JsXxh128>>>,
        unit: Option<String>,
    ) -> Result<Self> {
        let digester = match state {
            Either::A(state) => state.digester(),
            Either::B(Either::A(state)) => state.digester(),
            Either::B(Either::B(Either::A(state))) => state.digester(),
            Either::B(Either::B(Either::B(state))) => state.digester(),
        };
        TxHasher::from_digester(unit_from_js(unit)?, digester)
            .map(|inner| Self { inner })
            .map_err(napi_error)
    }

    /// The clock resolution every answer counts its instant in.
    #[napi(getter)]
    pub fn unit(&self) -> String {
        self.inner.unit().as_str().to_owned()
    }

    /// The canonical algorithm token every answer's digest half computes.
    #[napi(getter)]
    pub fn algorithm(&self) -> String {
        self.inner.algorithm().as_str().to_owned()
    }

    /// The width of every answer's canonical bytes.
    #[napi(getter)]
    pub fn width(&self) -> u32 {
        u32::try_from(self.inner.width())
            .unwrap_or_else(|_| unreachable!("no coupled value is wider than 24 bytes"))
    }

    /// The datatype a column of answers is stored under.
    #[napi(getter)]
    pub fn dtype(&self) -> JsDataType {
        JsDataType::from_core(self.inner.dtype())
    }

    /// Couple an instant with the digest of raw bytes or a string's UTF-8.
    ///
    /// The loader owns the instant intake and publishes this as `digest`.
    #[napi(js_name = "_digestNative", skip_typescript)]
    pub fn digest_native(
        &self,
        data: Either<Buffer, Either<Uint8Array, String>>,
        unix: UnixContent<'_>,
    ) -> Result<JsTxHash> {
        let unix = unix_from_js(unix, self.inner.unit())?;
        Ok(JsTxHash::from_core(
            self.inner.digest(content_bytes(&data), unix),
        ))
    }

    /// Couple an instant with a value's canonical feed.
    #[napi(js_name = "_digestScalarNative", skip_typescript)]
    pub fn digest_scalar_native(
        &self,
        value: &JsScalar,
        unix: UnixContent<'_>,
    ) -> Result<JsTxHash> {
        let unix = unix_from_js(unix, self.inner.unit())?;
        Ok(JsTxHash::from_core(
            self.inner.digest_scalar(&value.inner, unix),
        ))
    }

    /// Read any instant as a unix count of this hasher's resolution.
    #[napi(js_name = "_unixOfNative", skip_typescript)]
    pub fn unix_of_native(&self, unix: UnixContent<'_>) -> Result<BigInt> {
        unix_from_js(unix, self.inner.unit()).map(BigInt::from)
    }

    /// Fill this schema's default digest holders in one Arrow batch.
    ///
    /// The JavaScript loader owns the copied Arrow IPC boundary and removes
    /// this private method from the published class.
    #[napi(js_name = "_applyArrowBatchIpcNative", skip_typescript)]
    pub fn apply_arrow_batch_ipc(
        &self,
        root: &JsField,
        bytes: Uint8Array,
        force: bool,
    ) -> Result<Buffer> {
        apply_arrow_batch_ipc(&bytes, |batch| {
            self.inner.apply_arrow_batch(&root.inner, batch, force)
        })
    }

    /// Make a cheap native clone.
    #[napi(js_name = "clone")]
    pub fn clone_js(&self) -> Self {
        self.clone()
    }
}

/// Couple a microsecond instant with a one-shot digest of `data`.
fn couple(unix: UnixContent<'_>, digest: yggdryl::Digest) -> Result<JsTxHash> {
    let unix = unix_from_js(unix, txhash::DEFAULT_UNIT)?;
    Ok(JsTxHash::from_core(TxHash::new(unix, digest)))
}

/// Couple a microsecond instant with XXH32 of a complete value.
#[napi(js_name = "_txh32Native", skip_typescript)]
pub fn txh32_native(
    data: Either<Buffer, Either<Uint8Array, String>>,
    unix: UnixContent<'_>,
    seed: Option<u32>,
) -> Result<JsTxHash> {
    let mut state = Xxh32::with_seed(seed.unwrap_or(0));
    state.write_bytes(content_bytes(&data));
    couple(unix, state.as_digest())
}

/// Couple a microsecond instant with XXH64 of a complete value.
#[napi(js_name = "_txh64Native", skip_typescript)]
pub fn txh64_native(
    data: Either<Buffer, Either<Uint8Array, String>>,
    unix: UnixContent<'_>,
    seed: Option<BigInt>,
) -> Result<JsTxHash> {
    let mut state = Xxh64::with_seed(seed_from_bigint(seed)?);
    state.write_bytes(content_bytes(&data));
    couple(unix, state.as_digest())
}

/// Couple a microsecond instant with XXH3-64 of a complete value.
#[napi(js_name = "_txh3Native", skip_typescript)]
pub fn txh3_native(
    data: Either<Buffer, Either<Uint8Array, String>>,
    unix: UnixContent<'_>,
    seed: Option<BigInt>,
    secret: Option<Uint8Array>,
) -> Result<JsTxHash> {
    let seed = seed_from_bigint(seed)?;
    let mut state = match secret {
        Some(secret) => Xxh3::from_seed_and_secret(seed, secret.as_ref()).map_err(napi_error)?,
        None => Xxh3::with_seed(seed),
    };
    state.write_bytes(content_bytes(&data));
    couple(unix, state.as_digest())
}

/// Couple a microsecond instant with XXH3-128 of a complete value.
#[napi(js_name = "_txh128Native", skip_typescript)]
pub fn txh128_native(
    data: Either<Buffer, Either<Uint8Array, String>>,
    unix: UnixContent<'_>,
    seed: Option<BigInt>,
    secret: Option<Uint8Array>,
) -> Result<JsTxHash> {
    let seed = seed_from_bigint(seed)?;
    let mut state = match secret {
        Some(secret) => Xxh128::from_seed_and_secret(seed, secret.as_ref()).map_err(napi_error)?,
        None => Xxh128::with_seed(seed),
    };
    state.write_bytes(content_bytes(&data));
    couple(unix, state.as_digest())
}

/// Couple a microsecond instant with a complete value's digest.
#[napi(js_name = "_txhashDigestNative", skip_typescript)]
pub fn txhash_digest_native(
    data: Either<Buffer, Either<Uint8Array, String>>,
    unix: UnixContent<'_>,
    algorithm: String,
) -> Result<JsTxHash> {
    let algorithm = algorithm_from_str(&algorithm)?;
    couple(unix, algorithm.digest(content_bytes(&data)))
}

/// Read the system clock as a unix count of `unit`.
#[napi(js_name = "_txhashUnixNowNative", skip_typescript)]
pub fn txhash_unix_now_native(unit: Option<String>) -> Result<BigInt> {
    txhash::unix_now(unit_from_js(unit)?)
        .map(BigInt::from)
        .map_err(napi_error)
}

/// Read any instant as a unix count of `unit`.
#[napi(js_name = "_txhashUnixOfNative", skip_typescript)]
pub fn txhash_unix_of_native(instant: UnixContent<'_>, unit: Option<String>) -> Result<BigInt> {
    unix_from_js(instant, unit_from_js(unit)?).map(BigInt::from)
}

/// Restate a count of one resolution as a count of another.
#[napi(js_name = "_txhashRestateUnixNative", skip_typescript)]
pub fn txhash_restate_unix_native(count: BigInt, from: String, into: String) -> Result<BigInt> {
    let (count, lossless) = count.get_i64();
    if !lossless {
        return Err(napi_error("count must fit a signed 64-bit unix count"));
    }
    txhash::restate_unix(count, unit_from_js(Some(from))?, unit_from_js(Some(into))?)
        .map(BigInt::from)
        .map_err(napi_error)
}

/// The width of a value coupling an instant with an algorithm's digest.
#[napi(js_name = "_txhashWidthNative", skip_typescript)]
pub fn txhash_width_native(algorithm: String) -> Result<u32> {
    Ok(
        u32::try_from(txhash::width(algorithm_from_str(&algorithm)?))
            .unwrap_or_else(|_| unreachable!("no coupled value is wider than 24 bytes")),
    )
}

/// The datatype a column of coupled values is stored under.
#[napi(js_name = "_txhashDtypeNative", skip_typescript)]
pub fn txhash_dtype_native(algorithm: String) -> Result<JsDataType> {
    Ok(JsDataType::from_core(txhash::dtype(algorithm_from_str(
        &algorithm,
    )?)))
}

/// The resolution a unix count carries when a caller names none.
#[napi(js_name = "_txhashDefaultUnitNative", skip_typescript)]
pub fn txhash_default_unit_native() -> String {
    txhash::DEFAULT_UNIT.as_str().to_owned()
}
