//! Native identifiers: a value under a key - the source that gave it and the
//! type of name it is - and the sorted map an element states them in. A key
//! crosses as its text, `src:type`, a key from the base source spelled as
//! its type alone (`isin`), read exactly by the core's [`IdKey`]; a type
//! crosses as the word it folds to, read by [`IdType`].

use std::collections::{BTreeMap, HashMap};

use napi::bindgen_prelude::{ClassInstance, Result};
use napi_derive::napi;
use yggdryl::{IdKey, IdType, Identifier, Identifiers, Scalar};

use crate::{napi_error, ordering_value};

/// The key `text` spells, read exactly.
fn key_of(text: &str) -> Result<IdKey> {
    text.parse().map_err(napi_error)
}

/// The type `text` folds to.
fn type_of(text: &str) -> Result<IdType> {
    text.parse().map_err(napi_error)
}

/// One identifier: a value under a key, `src:type`, a key from the base
/// source spelled as its type alone.
#[napi(js_name = "Identifier")]
#[derive(Clone)]
pub struct JsIdentifier {
    pub(crate) inner: Identifier,
}

#[napi]
impl JsIdentifier {
    /// `key` is read exactly - `src:type`, or a type alone for the base
    /// source, each word folded to lower case without its breaks - and
    /// `value` is trimmed text that states something, held as the key's type
    /// stores it.
    #[napi(constructor)]
    pub fn new(key: String, value: String) -> Result<Self> {
        Identifier::new(key_of(&key)?, &value)
            .map(|inner| Self { inner })
            .map_err(napi_error)
    }

    /// The identifier a name no key spells names, or `null` where it names
    /// none, the value states nothing or its type refuses the value.
    ///
    /// An explicit `src:type` keeps its source. Otherwise a whole name a
    /// security type is spelled by - `ISINCode`, `security_cusip` - is that
    /// type from the base source, and a security type is never read off a
    /// name that names another instrument's (`underlyingisin`, `legisin`).
    /// Otherwise the name folds - lower case, no `_`, `-`, space or `#` -
    /// and the longest identifier name it ends with is the type: a type the
    /// crate names whose spelling ends with `id`, `account`, `isin`,
    /// `cusip`, `sedol` or `figi`, a parentage word (`parent`, `orig`,
    /// `origin`, `original`) right before it kept inside the type. The
    /// source is the rest of the folded name with its dots trimmed at both
    /// ends and kept inside, the base source where nothing is left or where
    /// it folds to a source the crate reserves - `base`, `fix`, `derived` -
    /// which names no namespace: `Derived_ISIN` is `isin`.
    ///
    /// `firm.x.ParentOrderID` is `firm.x:parentorderid`, `OMS_InstrumentID`
    /// `oms:instrumentid`, `marketorderid` `market:orderid`, `ISINCode`
    /// `isin`; `underlyingisin` and `transversalkey` name none.
    #[napi]
    pub fn from_key(key: String, value: String) -> Option<JsIdentifier> {
        Identifier::from_key(&key, &value).map(|inner| Self { inner })
    }

    /// Who gave the value: `oms`, `proprietary`, `derived`, `base` where no
    /// source is named.
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

    /// The key as an `Identifiers` map spells it: `src:type`, the type
    /// alone for the base source.
    #[napi(getter)]
    pub fn key(&self) -> String {
        self.inner.key().to_string()
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

    /// Render `key=value`.
    #[napi(js_name = "toString")]
    pub fn js_string(&self) -> String {
        self.inner.to_string()
    }
}

/// A sorted map from a key to its value, iterated in key order, whose base
/// key of a type is the type's answer: a named source fills it where it is
/// empty, so `ullink:isin` alone is also `isin`.
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
    /// The map `ids` fill, the first identifier of a key standing and each
    /// named source filling the base key of its type where it is empty.
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

    /// The map a plain object from each key's text to its value states -
    /// each key read exactly, `isin` the base key and `ullink:isin` a named
    /// one - closed so every type held has its base key: a type stating
    /// none takes its first named source's value, else its derivation's.
    ///
    /// A key that reads as no key, a value that states nothing or that its
    /// type refuses, and two spellings of one key with two values are each
    /// refused naming the key.
    #[napi(factory)]
    pub fn from_object(
        #[napi(ts_arg_type = "Record<string, string>")] entries: HashMap<String, String>,
    ) -> Result<Self> {
        let entries = Scalar::from_mapping(
            entries
                .into_iter()
                .map(|(key, value)| (Scalar::from(key), Scalar::from(value))),
        )
        .map_err(napi_error)?;
        Identifiers::from_scalar(&entries)
            .map(|inner| Self { inner })
            .map_err(napi_error)
    }

    /// The map as a plain object from each key's text to its value, in key
    /// order; `fromObject` reads it back unchanged.
    #[napi(ts_return_type = "Record<string, string>")]
    #[allow(clippy::wrong_self_convention)]
    pub fn into_object(&self) -> BTreeMap<String, String> {
        self.inner
            .iter()
            .map(|id| (id.key().to_string(), id.value().to_owned()))
            .collect()
    }

    /// The value of `type`'s base key: the type's answer, whichever source
    /// stated it; `null` where the map holds nothing of the type.
    #[napi]
    pub fn get(&self, kind: String) -> Result<Option<String>> {
        Ok(self.inner.get(&type_of(&kind)?).map(str::to_owned))
    }

    /// The value held under exactly `key` - `isin`, `ullink:isin`; `null`
    /// where none.
    #[napi]
    pub fn get_from(&self, key: String) -> Result<Option<String>> {
        Ok(self.inner.get_from(&key_of(&key)?).map(str::to_owned))
    }

    /// Whether `type`'s base key holds only a derivation - the value of
    /// `derived:<type>`, which no named source states.
    #[napi]
    pub fn is_derived(&self, kind: String) -> Result<bool> {
        Ok(self.inner.is_derived(&type_of(&kind)?))
    }

    /// Whether the map holds anything of `type`.
    #[napi]
    pub fn contains_kind(&self, kind: String) -> Result<bool> {
        Ok(self.inner.contains_kind(&type_of(&kind)?))
    }

    /// Every identifier of `type`, its base key included, in key order.
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

    /// Render `[key=value, ...]`.
    #[napi(js_name = "toString")]
    pub fn js_string(&self) -> String {
        self.inner.to_string()
    }
}
