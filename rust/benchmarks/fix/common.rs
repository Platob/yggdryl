use std::path::PathBuf;

use yggdryl::holder::local::Folder;
use yggdryl::{DataType, Field, FixBranch, FixRegistry};

/// Large-dictionary size: reportable in release, quick to smoke-test in debug.
pub(crate) const LARGE_FIELDS: usize = crate::bench_profile::corpus(400, 50);

/// Second-branch size used by resolution and storage measurements.
pub(crate) const BRANCH_FIELDS: usize = crate::bench_profile::corpus(100, 20);

/// The tracked seed dictionary, relative to the crate manifest.
pub(crate) fn seed_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("config")
        .join("fix")
}

/// The venue dictionary the branched measurements resolve against.
pub(crate) fn venue() -> FixBranch {
    FixBranch::from_str("cme").expect("a valid branch")
}

/// `count` generated fields in the venue branch, tags from 5000 up.
fn vendored(count: usize) -> Vec<Field> {
    let venue = venue();
    (0..count)
        .map(|index| {
            let mut field = DataType::Int64.nullable_field(format!("Vendor{index:05}"));
            let tag = i32::try_from(5_000 + index).expect("a small tag");
            field
                .as_fix_mut()
                .set_id(&venue, tag)
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
pub(crate) fn two_branches(count: usize) -> FixRegistry {
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

/// One in fifty generated fields is a repeating group.
///
/// A dictionary's components and repeating groups are a small minority of it,
/// and that ratio keeps the benchmark corpus representative while both shapes
/// resolve through the same indexes.
const NESTED_EVERY: usize = 50;

/// `count` generated fields with tags from 5000 up, one in
/// [`NESTED_EVERY`] of them a repeating group.
///
/// The nested tags are exactly `5000 + NESTED_EVERY * k`, so a benchmark
/// names a primitive hit and a nested hit by arithmetic.
pub(crate) fn mixed_nestedness(count: usize) -> Vec<Field> {
    (0..count)
        .map(|index| {
            let tag = i32::try_from(5_000 + index).expect("a small tag");
            let mut field = if index % NESTED_EVERY == 0 {
                let item = DataType::from_fields([DataType::Utf8.nullable_field("Member")])
                    .expect("a struct item")
                    .required_field("item");
                DataType::list(item).nullable_field(format!("NoGroup{index:05}"))
            } else {
                DataType::Int64.nullable_field(format!("Generated{index:05}"))
            };
            field.as_fix_mut().set_tag(tag).expect("a generated tag");
            field
                .as_fix_mut()
                .set_aliases([format!("MixedAlias{index:05}")])
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
