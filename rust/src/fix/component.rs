//! Names of repeating groups and their occurrence components.
//!
//! Catalog definitions carry resolved names. CBlock input without a declared
//! group name uses the published Parties family, then the counter's noun stem.

use smol_str::SmolStr;

const LATIN: [(&str, &str); 4] = [
    ("appendices", "appendix"),
    ("matrices", "matrix"),
    ("vertices", "vertex"),
    ("indices", "index"),
];

fn singularize(stem: &str) -> SmolStr {
    for (plural, singular) in LATIN {
        if stem.len() >= plural.len() {
            let (head, tail) = stem.split_at(stem.len() - plural.len());
            if tail.eq_ignore_ascii_case(plural) {
                let mut held = String::with_capacity(head.len() + singular.len());
                held.push_str(head);
                if tail.starts_with(|first: char| first.is_ascii_uppercase()) {
                    held.push(singular.as_bytes()[0].to_ascii_uppercase() as char);
                    held.push_str(&singular[1..]);
                } else {
                    held.push_str(singular);
                }
                return SmolStr::new(held);
            }
        }
    }
    let folded = stem.to_ascii_lowercase();
    if !stem.ends_with('s')
        || folded.ends_with("ss")
        || folded.ends_with("us")
        || folded.ends_with("is")
        || folded.ends_with("news")
        || folded.ends_with("series")
        || folded.ends_with("species")
    {
        return SmolStr::new(stem);
    }
    if stem.len() > 4 && stem.ends_with("ies") {
        return SmolStr::new(format!("{}y", &stem[..stem.len() - 3]));
    }
    if stem.ends_with("sses") {
        return SmolStr::new(&stem[..stem.len() - 2]);
    }
    if let Some(prefix) = stem.strip_suffix("es") {
        if prefix.ends_with('x')
            || prefix.ends_with("ch")
            || prefix.ends_with("sh")
            || prefix.ends_with("zz")
        {
            return SmolStr::new(prefix);
        }
    }
    SmolStr::new(&stem[..stem.len() - 1])
}

fn is_collection_plural(name: &str) -> bool {
    let end = name.trim_end_matches(|ch: char| ch.is_ascii_digit()).len();
    let stem = &name[..end];
    singularize(stem).as_str() != stem
        || stem.as_bytes().windows(4).any(|held| {
            held[0] == b's' && held[1] == b'O' && held[2] == b'f' && held[3].is_ascii_uppercase()
        })
}

fn counter_collection(counter: &crate::Field) -> SmolStr {
    let spelling = counter.display().unwrap_or_else(|| counter.name());
    let stem = spelling.strip_prefix("No").unwrap_or(spelling);
    let end = stem.trim_end_matches(|ch: char| ch.is_ascii_digit()).len();
    let (stem, digits) = stem.split_at(end);
    if let Some(prefix) = stem.strip_suffix("PartyIDs") {
        return SmolStr::new(format!("{prefix}Parties{digits}"));
    }
    SmolStr::new(format!("{stem}{digits}"))
}

fn with_group_suffix(display: &str) -> SmolStr {
    if display.ends_with("Grp") || display.ends_with("grp") {
        SmolStr::new(display)
    } else {
        SmolStr::new(format!("{display}Grp"))
    }
}

/// The one canonical name a repeating-group spelling resolves to.
pub(crate) fn canonical_group_name(name: &str) -> SmolStr {
    let folded = name.to_ascii_lowercase();
    if let Some((canonical, _, _, _)) = super::constants::shipped_group_names(&folded) {
        return SmolStr::new_static(canonical);
    }
    match folded.as_str() {
        "secaltidgrp" | "securityaltid" | "securityaltidgrp" => SmolStr::new_static("secaltids"),
        _ => SmolStr::new(folded),
    }
}

// Only `rust/tests/fix/mod_.rs` names these two, through the `internals`
// door below; `group_names` is what the crate itself reads.
#[cfg(feature = "internals")]
pub(crate) fn group_name(counter: &crate::Field) -> SmolStr {
    group_names(counter, None).0
}

pub(crate) fn entry_display(group: &str) -> SmolStr {
    let group = group
        .strip_suffix("Grp")
        .or_else(|| group.strip_suffix("grp"))
        .unwrap_or(group);
    let end = group.trim_end_matches(|ch: char| ch.is_ascii_digit()).len();
    let (stem, digits) = group.split_at(end);
    SmolStr::new(format!("{}{digits}", singularize(stem)))
}

#[cfg(feature = "internals")]
pub(crate) fn entry_name(group: &str) -> SmolStr {
    SmolStr::new(entry_display(group).to_ascii_lowercase())
}

/// Canonical/display names for a group and one occurrence of it.
pub(crate) fn group_names(
    counter: &crate::Field,
    declared: Option<&str>,
) -> (SmolStr, SmolStr, SmolStr, SmolStr) {
    if let Some(declared) = declared.filter(|name| !name.is_empty()) {
        let folded = declared.to_ascii_lowercase();
        if let Some((group, display, entry, entry_display)) =
            super::constants::shipped_group_names(&folded)
        {
            return (
                SmolStr::new_static(group),
                SmolStr::new_static(display),
                SmolStr::new_static(entry),
                SmolStr::new_static(entry_display),
            );
        }
    }

    let collection = counter_collection(counter);
    let display = match declared.filter(|name| !name.is_empty()) {
        Some("SecurityAltID") | Some("SecAltIDGrp") => SmolStr::new_static("SecAltIDs"),
        Some(declared) => match declared.strip_suffix("Grp") {
            Some(stem) if is_collection_plural(stem) => SmolStr::new(stem),
            Some(_) if is_collection_plural(&collection) => collection,
            Some(_) => SmolStr::new(declared),
            None => SmolStr::new(declared),
        },
        None if collection.eq_ignore_ascii_case("SecurityAltID") => {
            SmolStr::new_static("SecAltIDs")
        }
        None if is_collection_plural(&collection) => collection,
        None => with_group_suffix(&collection),
    };
    let group = canonical_group_name(&display);
    let occurrence_source = declared.unwrap_or(display.as_str());
    let occurrence_display = entry_display(occurrence_source);
    let occurrence = SmolStr::new(occurrence_display.to_ascii_lowercase());
    (group, display, occurrence, occurrence_display)
}

pub(crate) fn occurrence_name(group: &crate::Field) -> SmolStr {
    if let crate::DataType::List(item) | crate::DataType::LargeList(item) = group.dtype() {
        return SmolStr::new(item.name());
    }
    group_names(group, None).2
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/fix/mod_.rs` pins and a caller cannot reach.
    //!
    //! A published collection is named once, and its occurrence's singular
    //! with it; both spellings reach a caller only as the names on a row.

    /// The group name a counter field's own name implies.
    #[must_use]
    pub fn group_name(counter: &crate::Field) -> String {
        super::group_name(counter).to_string()
    }

    /// The singular an occurrence of `group` is named by.
    #[must_use]
    pub fn entry_name(group: &str) -> String {
        super::entry_name(group).to_string()
    }

    /// The one canonical name a repeating-group spelling resolves to.
    #[must_use]
    pub fn canonical_group_name(name: &str) -> String {
        super::canonical_group_name(name).to_string()
    }

    /// The collection's name and display beside its occurrence's, in that
    /// order, for a counter field and the spelling its schema declared.
    #[must_use]
    pub fn group_names(
        counter: &crate::Field,
        declared: Option<&str>,
    ) -> (String, String, String, String) {
        let (group, display, entry, entry_display) = super::group_names(counter, declared);
        (
            group.to_string(),
            display.to_string(),
            entry.to_string(),
            entry_display.to_string(),
        )
    }
}
