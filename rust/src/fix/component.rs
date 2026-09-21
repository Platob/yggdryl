//! Names of repeating groups and their occurrence components.
//!
//! Catalog definitions carry resolved names. CBlock input without a declared
//! group name uses the published Parties family, then the counter's noun stem.

use crate::sequence::SequenceType;
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
    if !stem.ends_with('s') || stem.ends_with("ss") {
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

/// The one canonical name a repeating-group spelling resolves to.
pub(crate) fn canonical_group_name(name: &str) -> SmolStr {
    let folded = name.to_ascii_lowercase();
    match folded.as_str() {
        "secaltidgrp" | "securityaltid" => SmolStr::new_static("secaltids"),
        "regulatorytradeidgrp" => SmolStr::new_static("regulatorytradeids"),
        _ => SmolStr::new(folded),
    }
}

pub(crate) fn group_name(counter: &crate::Field) -> SmolStr {
    let spelling = counter.display().unwrap_or_else(|| counter.name());
    let stem = spelling.strip_prefix("No").unwrap_or(spelling);
    let name = if let Some(prefix) = stem.strip_suffix("PartyIDs") {
        let end = prefix
            .trim_end_matches(|ch: char| ch.is_ascii_digit())
            .len();
        let (prefix, digits) = prefix.split_at(end);
        format!("{prefix}Parties{digits}")
    } else if let Some(prefix) = stem.strip_suffix("PartyIDs2") {
        format!("{prefix}Parties2")
    } else if let Some(prefix) = stem.strip_suffix("PartyIDs3") {
        format!("{prefix}Parties3")
    } else if let Some(prefix) = stem.strip_suffix("PartyIDs4") {
        format!("{prefix}Parties4")
    } else {
        stem.to_owned()
    };
    canonical_group_name(&name)
}

pub(crate) fn entry_name(group: &str) -> SmolStr {
    let group = group
        .strip_suffix("Grp")
        .or_else(|| group.strip_suffix("grp"))
        .unwrap_or(group);
    let end = group.trim_end_matches(|ch: char| ch.is_ascii_digit()).len();
    let (stem, digits) = group.split_at(end);
    SmolStr::new(format!("{}{digits}", singularize(stem)).to_ascii_lowercase())
}

pub(crate) fn occurrence_name(group: &crate::Field) -> SmolStr {
    if let crate::DataType::Sequence(SequenceType::List(item))
    | crate::DataType::Sequence(SequenceType::LargeList(item)) = group.dtype()
    {
        return SmolStr::new(item.name());
    }
    entry_name(&group_name(group))
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
}
