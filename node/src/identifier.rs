//! Native identifiers: the name a source gave a thing, as a type of name,
//! and the sorted map an element states them in, keyed `src:type`. A source
//! and a type cross as the words they fold to - `fix`, `clordid` - read by
//! the core's [`IdSource`] and [`IdType`].

use napi::bindgen_prelude::{ClassInstance, Result};
use napi_derive::napi;
use yggdryl::{IdSource, IdType, Identifier, Identifiers};

use crate::{napi_error, ordering_value};

/// The source `text` folds to.
fn source_of(text: &str) -> Result<IdSource> {
    text.parse().map_err(napi_error)
}

/// The type `text` folds to.
fn type_of(text: &str) -> Result<IdType> {
    text.parse().map_err(napi_error)
}

/// One identifier: a source, a type and a value, unique by its key
/// `src:type`.
#[napi(js_name = "Identifier")]
#[derive(Clone)]
pub struct JsIdentifier {
    pub(crate) inner: Identifier,
}

#[napi]
impl JsIdentifier {
    /// A source and a type are words, folded to lower case without their
    /// breaks; the value is trimmed text that states something, held as its
    /// type stores it.
    #[napi(constructor)]
    pub fn new(src: String, kind: String, value: String) -> Result<Self> {
        Identifier::new(source_of(&src)?, type_of(&kind)?, &value)
            .map(|inner| Self { inner })
            .map_err(napi_error)
    }

    /// The identifier a key names, or `null` where it names none, the value
    /// states nothing or its type refuses the value.
    ///
    /// An explicit `src:type` is read as it is. Otherwise a whole name a
    /// security type is spelled by - `ISINCode`, `security_cusip` - is that
    /// type from `base`, and a security type is never read off a key that
    /// names another instrument's (`underlyingisin`, `legisin`). Otherwise
    /// the key folds - lower case, no `_`, `-`, space or `#` - and the
    /// longest identifier name it ends with is the type: a type the crate
    /// names whose spelling ends with `id`, `account`, `isin`, `cusip`,
    /// `sedol` or `figi`, a parentage word (`parent`, `orig`, `origin`,
    /// `original`) right before it kept inside the type. The source is the
    /// rest of the folded key with its dots trimmed at both ends and kept
    /// inside, `base` where nothing is left or where it folds to a source
    /// the crate reserves - `base`, `derived`, `fix` - which names no
    /// namespace: `Derived_ISIN` is `base:isin`.
    ///
    /// `firm.x.ParentOrderID` is `firm.x:parentorderid`, `OMS_InstrumentID`
    /// `oms:instrumentid`, `marketorderid` `market:orderid`, `ISINCode`
    /// `base:isin`; `underlyingisin` and `transversalkey` name none.
    #[napi]
    pub fn from_key(key: String, value: String) -> Option<JsIdentifier> {
        Identifier::from_key(&key, &value).map(|inner| Self { inner })
    }

    /// Who gave the name: `fix`, `oms`, `derived`, `base` where no source
    /// is named.
    #[napi(getter)]
    pub fn src(&self) -> String {
        self.inner.src().to_string()
    }

    /// The type of name this is: `isin`, `executingtrader`, `clordid`.
    #[napi(getter, js_name = "type")]
    pub fn kind(&self) -> String {
        self.inner.kind().to_string()
    }

    /// The name itself.
    #[napi(getter)]
    pub fn value(&self) -> String {
        self.inner.value().to_owned()
    }

    /// The unique key, `src:type`, an `Identifiers` keys it by.
    #[napi(getter)]
    pub fn key(&self) -> String {
        self.inner.key().to_string()
    }

    /// Whether this identifier's key is `src:type`, each folded.
    #[napi]
    pub fn is_of(&self, src: String, kind: String) -> Result<bool> {
        Ok(self.inner.is_of(&source_of(&src)?, &type_of(&kind)?))
    }

    /// Compare the complete native values.
    #[napi]
    pub fn equals(&self, other: &JsIdentifier) -> bool {
        self.inner == other.inner
    }

    /// Compare native values in canonical order: the key as spelled
    /// (`src:type`), then the value.
    #[napi]
    pub fn compare(&self, other: &JsIdentifier) -> i32 {
        ordering_value(self.inner.cmp(&other.inner))
    }

    /// Render `src:type=value`.
    #[napi(js_name = "toString")]
    pub fn js_string(&self) -> String {
        self.inner.to_string()
    }
}

/// A sorted map of identifiers, one per unique key `src:type`, iterated in
/// key order.
#[napi(js_name = "Identifiers")]
#[derive(Clone, Default)]
pub struct JsIdentifiers {
    pub(crate) inner: Identifiers,
}

impl JsIdentifiers {
    pub(crate) fn from_core(inner: &Identifiers) -> Self {
        Self {
            inner: inner.clone(),
        }
    }
}

#[napi]
impl JsIdentifiers {
    /// The map `ids` fill, the first identifier of a key standing.
    #[napi(constructor)]
    pub fn new(ids: Option<Vec<ClassInstance<'_, JsIdentifier>>>) -> Self {
        Self {
            inner: ids
                .unwrap_or_default()
                .into_iter()
                .map(|id| id.inner.clone())
                .collect(),
        }
    }

    /// The value of the identifier of `type` the wire stated (`fix`), else
    /// the first another named source stated in key order, else the one
    /// stated under no source (`base`), else the derived one; `null` where
    /// none.
    #[napi]
    pub fn get(&self, kind: String) -> Result<Option<String>> {
        Ok(self.inner.get(&type_of(&kind)?).map(str::to_owned))
    }

    /// The identifier of `type` the wire stated (`fix`), else the first
    /// another named source stated in key order, else the one stated under
    /// no source (`base`), else the derived one; `null` where none.
    #[napi]
    pub fn get_identifier(&self, kind: String) -> Result<Option<JsIdentifier>> {
        Ok(self
            .inner
            .get_identifier(&type_of(&kind)?)
            .map(|inner| JsIdentifier {
                inner: inner.clone(),
            }))
    }

    /// The value of the identifier keyed `src:type`; `null` where none.
    #[napi]
    pub fn get_from(&self, src: String, kind: String) -> Result<Option<String>> {
        Ok(self
            .inner
            .get_from(&source_of(&src)?, &type_of(&kind)?)
            .map(str::to_owned))
    }

    /// Whether the set holds an identifier of `type`.
    #[napi]
    pub fn contains_kind(&self, kind: String) -> Result<bool> {
        Ok(self.inner.contains_kind(&type_of(&kind)?))
    }

    /// Every identifier of `type`, one per source.
    #[napi]
    pub fn of_kind(&self, kind: String) -> Result<Vec<JsIdentifier>> {
        let kind = type_of(&kind)?;
        Ok(self
            .inner
            .of_kind(&kind)
            .map(|inner| JsIdentifier {
                inner: inner.clone(),
            })
            .collect())
    }

    /// Every identifier, in key order.
    #[napi]
    pub fn to_array(&self) -> Vec<JsIdentifier> {
        self.inner
            .iter()
            .map(|inner| JsIdentifier {
                inner: inner.clone(),
            })
            .collect()
    }

    /// How many identifiers the set holds.
    #[napi(getter)]
    pub fn length(&self) -> u32 {
        u32::try_from(self.inner.len()).unwrap_or(u32::MAX)
    }

    /// Compare the complete native values.
    #[napi]
    pub fn equals(&self, other: &JsIdentifiers) -> bool {
        self.inner == other.inner
    }

    /// Render `[src:type=value, ...]`.
    #[napi(js_name = "toString")]
    pub fn js_string(&self) -> String {
        self.inner.to_string()
    }
}
