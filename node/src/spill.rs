//! JavaScript's native view of the shared [`SpillOptions`]: the bound a
//! column stays resident under, and the local folder it spills to.
//!
//! [`JsSpillOptions`] owns only the core value. The bound crosses as a
//! `bigint` both ways, because [`SpillOptions::NEVER`] is `u64::MAX`, which
//! no JavaScript number holds; a whole `number` is read as well. The folder
//! crosses through the package's one location door - a path, a `Url`, any
//! identifier naming a location, or an `IOBase` - held as a container, and
//! only a local one is a spill folder.

use napi::bindgen_prelude::{BigInt, ClassInstance, Either3, Either6, Either7, Null, Result};
use napi_derive::napi;
use yggdryl::holder::Holder;
use yggdryl::{DEFAULT_SPILL_BYTE_SIZE, IOBase as _, SpillOptions};

use crate::holder::fs::exact_bigint_u64;
use crate::iobase::{JsIOBase, folder_from_input};
use crate::napi_error;
use crate::uri::{JsArn, JsUri, JsUrl, JsUrn};

/// What a `SpillOptions` is built from, each slot `undefined` or `null` where
/// not given: the bound and the folder.
#[napi(object, object_to_js = false)]
#[derive(Default)]
pub struct SpillOptionsInit<'env> {
    /// The resident bytes a column may hold before it spills: a whole
    /// `number` or a `bigint`; `SpillOptions.NEVER` spills nothing and `0`
    /// everything. The core's 64 MiB when unstated.
    #[napi(ts_type = "number | bigint | null")]
    pub byte_size: Option<Either3<BigInt, f64, Null>>,
    /// The local folder spill files are created in: a path, a `Url`, a
    /// `Uri`, `Urn` or `Arn` naming one, or an `IOBase`. The platform
    /// temporary folder when unstated.
    #[napi(ts_type = "IOBase | Url | Uri | Urn | Arn | string | null")]
    pub folder: Option<
        Either7<
            ClassInstance<'env, JsIOBase>,
            ClassInstance<'env, JsUrl>,
            ClassInstance<'env, JsUri>,
            ClassInstance<'env, JsUrn>,
            ClassInstance<'env, JsArn>,
            String,
            Null,
        >,
    >,
}

/// The bound a column stays resident under, and the folder it spills to.
///
/// `byteSize` is the resident bytes a column may hold before it spills;
/// `SpillOptions.NEVER` spills nothing and `0` everything. `folder` is where
/// the files are created, the platform temporary folder when `null`. Every
/// spill file is private to the process and gone from the folder as soon as
/// it is opened, so a crash leaves nothing behind.
#[napi(js_name = "SpillOptions")]
#[derive(Clone)]
pub struct JsSpillOptions {
    pub(crate) inner: SpillOptions,
}

impl JsSpillOptions {
    /// Wrap a value the core built.
    pub(crate) const fn from_core(inner: SpillOptions) -> Self {
        Self { inner }
    }
}

/// The bound a spill verb settles under: the options given, else the process
/// default ([`SpillOptions::from_env`]), read once per process.
pub(crate) fn spill_bound(options: Option<&JsSpillOptions>) -> Result<&SpillOptions> {
    match options {
        Some(options) => Ok(&options.inner),
        None => SpillOptions::from_env().map_err(napi_error),
    }
}

#[napi]
impl JsSpillOptions {
    /// The bound and the folder `options` states, each `undefined` or `null`
    /// the default: 64 MiB over the platform temporary folder. A folder that
    /// is not local is refused naming it.
    #[napi(constructor)]
    pub fn new(options: Option<SpillOptionsInit<'_>>) -> Result<Self> {
        let options = options.unwrap_or_default();
        let mut inner = SpillOptions::new();
        match options.byte_size {
            Some(Either3::A(count)) => {
                inner =
                    inner.with_byte_size(exact_bigint_u64(&count, "byteSize").map_err(napi_error)?);
            }
            Some(Either3::B(count)) => {
                inner = inner.with_byte_size(crate::exact_u64(count, "byteSize")?);
            }
            Some(Either3::C(Null)) | None => {}
        }
        let location = match options.folder {
            Some(Either7::A(handle)) => Some(Either6::A(handle)),
            Some(Either7::B(url)) => Some(Either6::B(url)),
            Some(Either7::C(uri)) => Some(Either6::C(uri)),
            Some(Either7::D(urn)) => Some(Either6::D(urn)),
            Some(Either7::E(arn)) => Some(Either6::E(arn)),
            Some(Either7::F(path)) => Some(Either6::F(path)),
            Some(Either7::G(Null)) | None => None,
        };
        if let Some(location) = location {
            match folder_from_input(location)? {
                Holder::LocalFolder(folder) => inner = inner.with_folder(folder),
                other => {
                    return Err(napi_error(format!(
                        "expected a local folder to spill to, got {}",
                        other.url().map_or_else(
                            || "a location with no URL".to_owned(),
                            ToString::to_string
                        )
                    )));
                }
            }
        }
        Ok(Self::from_core(inner))
    }

    /// The options the process environment states, read on the first call
    /// and the same value after: `YGGDRYL_SPILL_BYTE_SIZE` the bound - a byte
    /// count, or `never` in any case - and `YGGDRYL_SPILL_FOLDER` the folder,
    /// either unset or empty the default. A refused variable is named, and
    /// the next call reads the environment again.
    #[napi(factory)]
    pub fn from_env() -> Result<Self> {
        SpillOptions::from_env()
            .cloned()
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// State the options every later `fromEnv` - and every verb settling
    /// under the process default - answers, before anything resolves them;
    /// refused once they were read or installed, so the value every caller
    /// saw cannot change underneath them.
    #[napi]
    pub fn install_env(options: &JsSpillOptions) -> Result<()> {
        SpillOptions::install_env(options.inner.clone()).map_err(napi_error)
    }

    /// The resident bytes a column may hold before it spills.
    #[napi(getter)]
    pub fn byte_size(&self) -> BigInt {
        BigInt::from(self.inner.byte_size())
    }

    /// The folder spill files are created in, as the local container handle
    /// it is, or `null` for the platform temporary folder.
    #[napi(getter)]
    pub fn folder(&self) -> Option<JsIOBase> {
        self.inner
            .folder()
            .map(|folder| JsIOBase::from_core(Holder::LocalFolder(folder.clone())))
    }

    /// Whether the bound is `SpillOptions.NEVER`: nothing spills.
    #[napi]
    pub fn is_never(&self) -> bool {
        self.inner.is_never()
    }

    /// Whether `other` states the same bound over the same folder: two
    /// folders are one when they name one location.
    #[napi]
    pub fn equals(&self, other: &JsSpillOptions) -> bool {
        self.inner == other.inner
    }

    /// A cheap native clone.
    #[napi(js_name = "clone")]
    pub fn clone_js(&self) -> Self {
        self.clone()
    }

    /// The bound - `never` for `SpillOptions.NEVER` - then the folder's URL
    /// where one is stated: `SpillOptions(byteSize=0, folder=file:///spill)`.
    #[napi(js_name = "toString")]
    pub fn js_string(&self) -> String {
        let bound = if self.inner.is_never() {
            "never".to_owned()
        } else {
            self.inner.byte_size().to_string()
        };
        match self.inner.folder() {
            Some(folder) => format!("SpillOptions(byteSize={bound}, folder={})", folder.url()),
            None => format!("SpillOptions(byteSize={bound})"),
        }
    }
}

/// `SpillOptions.NEVER`: the bound under which nothing spills, `u64::MAX`.
#[allow(dead_code)] // Reached through NAPI's registration inventory.
#[napi(js_name = "_spillNeverNative", skip_typescript)]
pub fn spill_never_native() -> BigInt {
    BigInt::from(SpillOptions::NEVER)
}

/// `DEFAULT_SPILL_BYTE_SIZE`: the bound one column stays resident under by
/// default, 64 MiB.
#[allow(dead_code)] // Reached through NAPI's registration inventory.
#[napi(js_name = "_defaultSpillByteSizeNative", skip_typescript)]
pub fn default_spill_byte_size_native() -> BigInt {
    BigInt::from(DEFAULT_SPILL_BYTE_SIZE)
}
