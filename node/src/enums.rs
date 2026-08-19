//! The core's static enum vocabularies, listed for the `enums` export.

use std::collections::HashMap;

use napi_derive::napi;
use yggdryl::{Codec, DataTypeId, DataTypeKind, IOKind, Level, Scheme, TimeUnit, UnionMode};

/// Every static enum vocabulary of the core, as canonical spellings.
///
/// The single source `binding.js` freezes into the `enums` export, so the
/// listings can never drift from the Rust constants they mirror. Pure enums
/// cross the boundary as strings by convention; this is the enumeration of
/// what those strings can be.
#[napi(js_name = "_enumValuesNative", skip_typescript)]
pub fn enum_values_native() -> HashMap<String, Vec<String>> {
    let spell = |values: &[&str]| values.iter().map(|value| (*value).to_owned()).collect();
    HashMap::from([
        (
            "dataTypeIds".to_owned(),
            spell(&DataTypeId::ALL.map(DataTypeId::as_str)),
        ),
        (
            "dataTypeKinds".to_owned(),
            spell(&DataTypeKind::ALL.map(DataTypeKind::as_str)),
        ),
        (
            "timeUnits".to_owned(),
            spell(&TimeUnit::ALL.map(TimeUnit::as_str)),
        ),
        (
            "unionModes".to_owned(),
            spell(&UnionMode::ALL.map(UnionMode::as_str)),
        ),
        ("codecs".to_owned(), spell(&Codec::ALL.map(Codec::as_str))),
        ("ioKinds".to_owned(), spell(&IOKind::ALL.map(IOKind::as_str))),
        (
            "compatibilitySchemes".to_owned(),
            Scheme::COMPATIBILITY_TARGETS
                .iter()
                .map(|scheme| scheme.as_str().to_owned())
                .collect(),
        ),
    ])
}

/// The named points of the shared 0-to-9 compression scale.
#[napi(js_name = "_levelValuesNative", skip_typescript)]
pub fn level_values_native() -> HashMap<String, u32> {
    HashMap::from([
        ("none".to_owned(), u32::from(Level::NONE.get())),
        ("fast".to_owned(), u32::from(Level::FAST.get())),
        ("default".to_owned(), u32::from(Level::DEFAULT.get())),
        ("best".to_owned(), u32::from(Level::BEST.get())),
    ])
}
