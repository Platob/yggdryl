use std::path::PathBuf;

use yggdryl::local::Folder;
use yggdryl::{DataType, Field, FixId, FixNamespace, FixRegistry};

/// The tracked seed dictionary, relative to the crate manifest.
pub(crate) fn seed_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("config")
        .join("fix")
}

/// The venue dictionary the namespaced measurements resolve against.
pub(crate) fn venue() -> FixNamespace {
    FixNamespace::from_str("cme").expect("a valid namespace")
}

/// `count` generated fields in the venue namespace, tags from 5000 up.
fn vendored(count: usize) -> Vec<Field> {
    let venue = venue();
    (0..count)
        .map(|index| {
            let mut field = DataType::Int64.nullable_field(format!("Vendor{index:05}"));
            let tag = i32::try_from(5_000 + index).expect("a small tag");
            let id = FixId::from_parts(venue.clone(), tag).expect("a vendor identifier");
            field
                .as_fix_mut()
                .set_id(&id)
                .expect("a generated identity");
            field
                .as_fix_mut()
                .set_aliases([format!("VendorAlias{index:05}")])
                .expect("a generated alias");
            field
        })
        .collect()
}

/// The tracked seed beside a venue dictionary of `count` fields.
pub(crate) fn two_namespaces(count: usize) -> FixRegistry {
    FixRegistry::from_fields(seed().iter().cloned().chain(vendored(count)))
        .expect("the generated dictionary has no conflict")
}

/// The tracked seed dictionary, loaded.
pub(crate) fn seed() -> FixRegistry {
    let folder = Folder::new(seed_root()).expect("the seed folder is a local path");
    FixRegistry::from_handle(&folder).expect("the tracked seed loads")
}

/// `count` generated fields with tags from 5000 up, each carrying an alias.
pub(crate) fn generated(count: usize) -> Vec<Field> {
    (0..count)
        .map(|index| {
            let mut field = DataType::Int64.nullable_field(format!("Generated{index:05}"));
            let tag = i32::try_from(5_000 + index).expect("a small tag");
            field.as_fix_mut().set_tag(tag).expect("a generated tag");
            field
                .as_fix_mut()
                .set_aliases([format!("GeneratedAlias{index:05}")])
                .expect("a generated alias");
            field
        })
        .collect()
}

/// A fresh directory of this benchmark's own under the platform temporary root.
pub(crate) fn scratch(label: &str) -> PathBuf {
    let path = Folder::temporary()
        .expect("the temporary directory")
        .path()
        .expect("a platform path")
        .join(format!("yggdryl-fix-bench-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    path
}
