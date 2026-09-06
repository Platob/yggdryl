//! xxHash digests over bytes, values, and handles.
//!
//! The one-shot functions answer `number` for XXH32 - a 32-bit value always
//! fits a JS number exactly - and `bigint` for the 64- and 128-bit results,
//! which do not. [`JsDigest`] is the answer that carries its algorithm with
//! it, which is what keeps `xxh64` and `xxh3-64`, both 64 bits wide, from
//! being confused for one another.
//!
//! Input is any `Buffer`, `Uint8Array`, `ArrayBuffer`, or `string`; the
//! wrapper in `binding.js` narrows every array-buffer view to a window over
//! the same memory rather than copying it.

use napi::bindgen_prelude::{BigInt, Buffer, Either, Result, Uint8Array};
use napi_derive::napi;

use yggdryl::xxhash::{Xxh3, Xxh32, Xxh64, Xxh128};
use yggdryl::{Digest, DigestAlgorithm};

use crate::napi_error;
use crate::text::codec::JsScalar;
use crate::types::field::JsField;

/// Decode exactly one Arrow batch, transform it, and return one IPC batch.
fn fill_arrow_batch_ipc(
    bytes: &Uint8Array,
    fill: impl FnOnce(arrow_array::RecordBatch) -> yggdryl::arrow::Result<arrow_array::RecordBatch>,
) -> Result<Buffer> {
    use arrow_ipc::reader::StreamReader;
    use arrow_ipc::writer::StreamWriter;

    let mut reader =
        StreamReader::try_new(std::io::Cursor::new(bytes.to_vec()), None).map_err(napi_error)?;
    let batch = reader
        .next()
        .transpose()
        .map_err(napi_error)?
        .ok_or_else(|| napi_error("Arrow IPC must contain exactly one record batch, got 0"))?;
    if reader.next().transpose().map_err(napi_error)?.is_some() {
        return Err(napi_error(
            "Arrow IPC must contain exactly one record batch, got more than 1",
        ));
    }
    let filled = fill(batch).map_err(napi_error)?;
    let mut writer =
        StreamWriter::try_new(Vec::new(), filled.schema().as_ref()).map_err(napi_error)?;
    writer.write(&filled).map_err(napi_error)?;
    writer.finish().map_err(napi_error)?;
    Ok(writer.into_inner().map_err(napi_error)?.into())
}

/// Anything JavaScript can hand a digest.
///
/// Spelled out at every `#[napi]` boundary rather than used as the parameter
/// type: NAPI emits no alias for one, so a named type would reach the
/// generated declarations undefined.
pub type DigestContent<'content> = Either<Buffer, Either<Uint8Array, String>>;

/// Borrow the bytes `content` holds.
///
/// Every shape here is already contiguous native memory or a `string`, so the
/// slice is the caller's own bytes: nothing is copied to hash it.
fn content_bytes<'content>(content: &'content DigestContent<'content>) -> &'content [u8] {
    match content {
        Either::A(buffer) => buffer.as_ref(),
        Either::B(Either::A(array)) => array.as_ref(),
        Either::B(Either::B(text)) => text.as_bytes(),
    }
}

/// Parse an algorithm token, keeping the core's message.
pub(crate) fn algorithm_from_str(value: &str) -> Result<DigestAlgorithm> {
    DigestAlgorithm::from_str(value).map_err(napi_error)
}

/// Render a `u128` as the two's-complement-free `BigInt` JavaScript reads.
fn bigint_from_u128(value: u128) -> BigInt {
    let bytes = value.to_le_bytes();
    let mut low = [0_u8; 8];
    let mut high = [0_u8; 8];
    low.copy_from_slice(&bytes[..8]);
    high.copy_from_slice(&bytes[8..]);
    BigInt {
        sign_bit: false,
        words: vec![u64::from_le_bytes(low), u64::from_le_bytes(high)],
    }
}

/// Read a `BigInt` seed, refusing a value wider than the seed it names.
fn seed_from_bigint(value: Option<BigInt>) -> Result<u64> {
    let Some(value) = value else { return Ok(0) };
    let (signed, value, lossless) = value.get_u64();
    if signed || !lossless {
        return Err(napi_error("seed must be an unsigned 64-bit integer"));
    }
    Ok(value)
}

/// Digest a complete value with XXH32.
#[napi(js_name = "_xxh32Native", skip_typescript)]
pub fn xxh32_native(data: Either<Buffer, Either<Uint8Array, String>>, seed: Option<u32>) -> u32 {
    yggdryl::xxhash::xxh32_with_seed(content_bytes(&data), seed.unwrap_or(0))
}

/// Digest a complete value with XXH64.
#[napi(js_name = "_xxh64Native", skip_typescript)]
pub fn xxh64_native(
    data: Either<Buffer, Either<Uint8Array, String>>,
    seed: Option<BigInt>,
) -> Result<BigInt> {
    let seed = seed_from_bigint(seed)?;
    Ok(BigInt::from(yggdryl::xxhash::xxh64_with_seed(
        content_bytes(&data),
        seed,
    )))
}

/// Digest a complete value with XXH3, answering 64 bits.
#[napi(js_name = "_xxh3Native", skip_typescript)]
pub fn xxh3_native(
    data: Either<Buffer, Either<Uint8Array, String>>,
    seed: Option<BigInt>,
    secret: Option<Uint8Array>,
) -> Result<BigInt> {
    let seed = seed_from_bigint(seed)?;
    let value = match secret {
        Some(secret) => {
            yggdryl::xxhash::xxh3_with_seed_and_secret(content_bytes(&data), seed, secret.as_ref())
                .map_err(napi_error)?
        }
        None => yggdryl::xxhash::xxh3_with_seed(content_bytes(&data), seed),
    };
    Ok(BigInt::from(value))
}

/// Digest a complete value with XXH3, answering 128 bits.
#[napi(js_name = "_xxh128Native", skip_typescript)]
pub fn xxh128_native(
    data: Either<Buffer, Either<Uint8Array, String>>,
    seed: Option<BigInt>,
    secret: Option<Uint8Array>,
) -> Result<BigInt> {
    let seed = seed_from_bigint(seed)?;
    let value = match secret {
        Some(secret) => yggdryl::xxhash::xxh128_with_seed_and_secret(
            content_bytes(&data),
            seed,
            secret.as_ref(),
        )
        .map_err(napi_error)?,
        None => yggdryl::xxhash::xxh128_with_seed(content_bytes(&data), seed),
    };
    Ok(bigint_from_u128(value))
}

/// Digest a complete value, carrying the algorithm with the answer.
#[napi(js_name = "_xxhashDigestNative", skip_typescript)]
pub fn xxhash_digest_native(
    data: Either<Buffer, Either<Uint8Array, String>>,
    algorithm: String,
) -> Result<JsDigest> {
    let algorithm = algorithm_from_str(&algorithm)?;
    Ok(JsDigest::from_core(algorithm.digest(content_bytes(&data))))
}

/// The shortest custom secret XXH3 accepts, in bytes.
#[napi(js_name = "_xxhashSecretMinimumLengthNative", skip_typescript)]
pub fn xxhash_secret_minimum_length_native() -> u32 {
    u32::try_from(yggdryl::xxhash::SECRET_MINIMUM_LENGTH)
        .unwrap_or_else(|_| unreachable!("the reference minimum is 136 bytes"))
}

/// One digest: the algorithm that produced it and the value it produced.
///
/// Two digests of different algorithms are never equal, whatever their
/// payloads. `toString()` is the `<algorithm>:<hex>` spelling `Digest.from`
/// reads back, and `bytes()` the canonical big-endian representation.
#[napi(js_name = "Digest")]
pub struct JsDigest {
    inner: Digest,
}

impl Clone for JsDigest {
    fn clone(&self) -> Self {
        Self::from_core(self.inner)
    }
}

impl JsDigest {
    pub(crate) const fn from_core(inner: Digest) -> Self {
        Self { inner }
    }
}

#[napi]
impl JsDigest {
    /// Parse the canonical `<algorithm>:<hex>` spelling.
    #[napi(constructor)]
    pub fn new(value: String) -> Result<Self> {
        Digest::from_str(&value)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// Parse the canonical spelling, or clone a native digest.
    #[napi(factory, js_name = "from")]
    pub fn from_js(value: Either<String, &JsDigest>) -> Result<Self> {
        match value {
            Either::A(value) => Self::new(value),
            Either::B(digest) => Ok(digest.clone()),
        }
    }

    /// Rebuild a digest from its canonical big-endian bytes.
    #[napi(factory)]
    pub fn from_bytes(algorithm: String, data: Uint8Array) -> Result<Self> {
        let algorithm = algorithm_from_str(&algorithm)?;
        Digest::from_bytes(algorithm, data.as_ref())
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// The canonical algorithm token.
    #[napi(getter)]
    pub fn algorithm(&self) -> String {
        self.inner.algorithm().as_str().to_owned()
    }

    /// The digest width in bytes.
    #[napi(getter)]
    pub fn width(&self) -> u32 {
        u32::try_from(self.inner.algorithm().width())
            .unwrap_or_else(|_| unreachable!("no digest is wider than 16 bytes"))
    }

    /// The digest width in bits.
    #[napi(getter)]
    pub fn bits(&self) -> u32 {
        self.inner.algorithm().bits()
    }

    /// The native value: a `number` for XXH32, a `bigint` otherwise.
    #[napi]
    pub fn value(&self) -> Either<u32, BigInt> {
        match self.inner.as_u32() {
            Some(value) => Either::A(value),
            None => Either::B(self.inner.as_u64().map_or_else(
                || bigint_from_u128(self.inner.as_u128().unwrap_or_default()),
                BigInt::from,
            )),
        }
    }

    /// The canonical big-endian bytes at the algorithm's exact width.
    #[napi]
    pub fn bytes(&self) -> Uint8Array {
        Uint8Array::new(self.inner.into_bytes().to_vec())
    }

    /// Exact equality: a different algorithm is a different digest.
    #[napi]
    pub fn equals(&self, other: &JsDigest) -> bool {
        self.inner == other.inner
    }

    /// Total native ordering: `-1`, `0`, or `1`.
    #[napi]
    pub fn compare(&self, other: &JsDigest) -> i32 {
        crate::ordering_value(self.inner.cmp(&other.inner))
    }

    /// A deterministic cross-language hash of this digest.
    #[napi]
    pub fn stable_hash(&self) -> u64 {
        self.inner.stable_hash()
    }

    /// Make a cheap native clone.
    #[napi(js_name = "clone")]
    pub fn clone_js(&self) -> Self {
        self.clone()
    }

    /// Return the canonical spelling, accepted losslessly by `Digest.from`.
    #[napi(js_name = "toString")]
    pub fn js_string(&self) -> String {
        self.inner.to_string()
    }

    /// Serialize as the canonical spelling, so a digest survives
    /// `JSON.stringify`.
    #[napi(js_name = "toJSON")]
    pub fn js_json(&self) -> String {
        self.inner.to_string()
    }
}

/// Declare one streaming state class over one core state.
///
/// The four algorithms differ in the width of their seed and in whether they
/// accept a custom secret, and nothing else. Only the shared half is generated
/// here: a doc comment passed through a macro reaches NAPI as a raw string
/// literal and lands mangled in the generated declarations, so each
/// constructor is written out below in its own block.
macro_rules! state {
    ($class:ident, $name:literal, $core:ty, $doc:literal) => {
        #[doc = $doc]
        #[napi(js_name = $name)]
        pub struct $class {
            inner: $core,
        }

        impl Clone for $class {
            fn clone(&self) -> Self {
                Self {
                    inner: self.inner.clone(),
                }
            }
        }

        #[napi]
        impl $class {
            /// The canonical algorithm token.
            #[napi(getter)]
            pub fn algorithm(&self) -> String {
                self.inner.as_digest().algorithm().as_str().to_owned()
            }

            /// Feed raw bytes or a string's UTF-8.
            #[napi]
            pub fn write_bytes(&mut self, data: Either<Buffer, Either<Uint8Array, String>>) {
                self.inner.write_bytes(content_bytes(&data));
            }

            /// Feed one value's canonical byte representation.
            #[napi]
            pub fn write_scalar(&mut self, value: &JsScalar) {
                self.inner.write_scalar(&value.inner);
            }

            /// Fill this schema's default digest holders in one Arrow batch.
            ///
            /// The JavaScript loader owns the copied Arrow IPC boundary and
            /// removes this private method from the published class.
            #[napi(js_name = "_fillArrowBatchIpcNative", skip_typescript)]
            pub fn fill_arrow_batch_ipc(
                &self,
                root: &JsField,
                bytes: Uint8Array,
                force: bool,
            ) -> Result<Buffer> {
                fill_arrow_batch_ipc(&bytes, |batch| {
                    self.inner.fill_arrow_batch(&root.inner, batch, force)
                })
            }

            /// Answer the digest of everything fed so far.
            ///
            /// Answering never consumes the state, so a running digest can be
            /// read at every commit boundary rather than only at the end.
            #[napi]
            pub fn as_digest(&self) -> JsDigest {
                JsDigest::from_core(self.inner.as_digest())
            }

            /// Reset to the constructed seed and secret, not to a fresh state.
            #[napi]
            pub fn clear(&mut self) {
                self.inner.clear();
            }

            /// Make a cheap native clone, carrying everything fed so far.
            #[napi(js_name = "clone")]
            pub fn clone_js(&self) -> Self {
                self.clone()
            }
        }
    };
}

state!(JsXxh32, "Xxh32", Xxh32, "A resumable XXH32 state.");
state!(JsXxh64, "Xxh64", Xxh64, "A resumable XXH64 state.");
state!(
    JsXxh3,
    "Xxh3",
    Xxh3,
    "A resumable XXH3 state answering 64 bits."
);
state!(
    JsXxh128,
    "Xxh128",
    Xxh128,
    "A resumable XXH3 state answering 128 bits."
);

#[napi]
impl JsXxh32 {
    /// Start a state, optionally seeded.
    ///
    /// XXH32 takes a seed and never a secret; only the XXH3 pair is
    /// secretable.
    #[napi(constructor)]
    pub fn new(seed: Option<u32>) -> Self {
        Self {
            inner: Xxh32::with_seed(seed.unwrap_or(0)),
        }
    }

    /// The seed this state was constructed with.
    #[napi(getter)]
    pub fn seed(&self) -> u32 {
        self.inner.seed()
    }
}

#[napi]
impl JsXxh64 {
    /// Start a state, optionally seeded.
    ///
    /// XXH64 takes a seed and never a secret; only the XXH3 pair is
    /// secretable.
    #[napi(constructor)]
    pub fn new(seed: Option<BigInt>) -> Result<Self> {
        Ok(Self {
            inner: Xxh64::with_seed(seed_from_bigint(seed)?),
        })
    }

    /// The seed this state was constructed with.
    #[napi(getter)]
    pub fn seed(&self) -> BigInt {
        BigInt::from(self.inner.seed())
    }
}

#[napi]
impl JsXxh3 {
    /// Start a state, optionally seeded and with a custom secret.
    ///
    /// A secret shorter than `xxhash.SECRET_MINIMUM_LENGTH` is rejected by
    /// length whatever the payload: the reference only consults a secret past
    /// its 240-byte cutoff, and a secret that is sometimes used is worse than
    /// one that is refused.
    #[napi(constructor)]
    pub fn new(seed: Option<BigInt>, secret: Option<Uint8Array>) -> Result<Self> {
        let seed = seed_from_bigint(seed)?;
        let inner = match secret {
            Some(secret) => {
                Xxh3::from_seed_and_secret(seed, secret.as_ref()).map_err(napi_error)?
            }
            None => Xxh3::with_seed(seed),
        };
        Ok(Self { inner })
    }

    /// The seed this state was constructed with.
    #[napi(getter)]
    pub fn seed(&self) -> BigInt {
        BigInt::from(self.inner.seed())
    }

    /// The custom secret this state was constructed with, if any.
    #[napi(getter)]
    pub fn secret(&self) -> Option<Uint8Array> {
        self.inner
            .secret()
            .map(|secret| Uint8Array::new(secret.to_vec()))
    }
}

#[napi]
impl JsXxh128 {
    /// Start a state, optionally seeded and with a custom secret.
    ///
    /// A secret shorter than `xxhash.SECRET_MINIMUM_LENGTH` is rejected by
    /// length whatever the payload, for the reason `Xxh3` states.
    #[napi(constructor)]
    pub fn new(seed: Option<BigInt>, secret: Option<Uint8Array>) -> Result<Self> {
        let seed = seed_from_bigint(seed)?;
        let inner = match secret {
            Some(secret) => {
                Xxh128::from_seed_and_secret(seed, secret.as_ref()).map_err(napi_error)?
            }
            None => Xxh128::with_seed(seed),
        };
        Ok(Self { inner })
    }

    /// The seed this state was constructed with.
    #[napi(getter)]
    pub fn seed(&self) -> BigInt {
        BigInt::from(self.inner.seed())
    }

    /// The custom secret this state was constructed with, if any.
    #[napi(getter)]
    pub fn secret(&self) -> Option<Uint8Array> {
        self.inner
            .secret()
            .map(|secret| Uint8Array::new(secret.to_vec()))
    }
}
