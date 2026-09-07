//! What one repeating group's occurrence is called.
//!
//! A repeating group is a counter-named `List` whose child is a non-null
//! `Struct`. That child is one *occurrence* of the group, and this is the one
//! rule that names it: `NoPartyIDs` heads occurrences called `PartyID`,
//! `NoTrdRegTimestamps` heads `TrdRegTimestamp`.
//!
//! # Derived, never looked up
//!
//! There is no table, no per-tag override and no specification-name lookup,
//! because none is needed: the rule below names all 521 shipped groups with
//! 521 distinct results and no collision. A table would be a second source of
//! truth for a fact the counter's own spelling already carries.
//!
//! The FIX specification's own group names cannot do this job. Orchestra names
//! *collections* rather than occurrences - tag 453 is `Parties`, tag 454 is
//! `SecAltIDGrp` - 63 of 580 remain plural once `Grp` is removed, and 21
//! shipped counters have between two and twelve competing Orchestra names
//! because one counter tag heads several groups. The counter is the one thing
//! a group always has exactly one of.

use smol_str::SmolStr;

/// The Latin plurals, longest first so a longer suffix is never shadowed.
///
/// Matched case-insensitively at the end of the stem, which is why the
/// replacement has to restore the case it found rather than being copied in.
const LATIN: [(&str, &str); 4] = [
    ("appendices", "appendix"),
    ("matrices", "matrix"),
    ("vertices", "vertex"),
    ("indices", "index"),
];

/// Returns the component name headed by a repeating-group counter.
///
/// Reads the counter's `display` spelling and never its folded name: folding
/// destroys the uppercase run that both the `No` strip and the Latin arm need,
/// and it would turn `NoPartyIDs` into a stem no rule can shorten correctly.
///
/// Answers `None` when the spelling is not a counter's - when it does not
/// begin `No` followed by an ASCII uppercase letter - and the caller falls
/// back to the counter's own name. Call it only for a `List`: a name beginning
/// `No` is not a counter test, and applying it by name alone would catch the
/// ten shipped non-counters such as `NotifyBrokerOfCredit` and
/// `NonCashDividendTreatment`, and five tagged counters that head no shipped
/// group body.
///
/// # The singular arms
///
/// Ordered, first match wins:
///
/// | # | arm | rule |
/// | --- | --- | --- |
/// | 1 | Latin | `matrices`/`indices`/`appendices`/`vertices` at the end, case-insensitively |
/// | 2 | already singular | the stem does not end in a byte-exact lowercase `s` |
/// | 3 | `ss` | the stem ends in `ss` |
/// | 4 | `ies` | longer than four bytes: drop `ies`, append `y` |
/// | 5 | `sses` | drop `es` |
/// | 6 | sibilant `es` | the prefix before `es` ends in `x`, `ch`, `sh` or `zz` |
/// | 7 | default | drop exactly the final `s` |
///
/// No arm but 1, 4 and 5 removes more than the final `s`, which is what keeps
/// a trailing uppercase run intact: `IDs` matches no multi-letter arm, so
/// `NoPartyIDs` yields `PartyID` and the forty-four counters ending `IDs` need
/// no acronym pass.
///
/// Arm 2 tests a byte-exact lowercase `s` because one shipped counter,
/// `NoSideTrdRegTS`, ends in an uppercase one; case-folding that test would
/// answer `SideTrdRegT`.
///
/// # Two results that read oddly, and are right
///
/// `NoLinesOfText` yields `LinesOfText`, the one derived name still plural:
/// the rule cannot singularize a head noun buried inside the stem, and the
/// specification spells the same group `LinesOfTextGrp`. `NoOfSecSizes`
/// yields `OfSecSize`, the one result that is not a noun phrase, because
/// removing `No` exposes a preposition; the specification's `SecSizesGrp` is
/// plural and no better. Neither is worth a one-entry exception table, which
/// would be a second source of truth for a derivable name.
pub(crate) fn component_name(counter_display: &str) -> Option<SmolStr> {
    let stem = counter_display.strip_prefix("No")?;
    if !stem.starts_with(|first: char| first.is_ascii_uppercase()) {
        return None;
    }
    Some(singularize(stem))
}

/// The stripped stem as one occurrence, by the first arm that matches.
fn singularize(stem: &str) -> SmolStr {
    // Arm 1. The only arm that rewrites inside the stem, so the only one that
    // can lose case: the replacement takes the case of the byte it replaces.
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
    // Arm 2. Byte-exact: an uppercase `S` is not a plural marker here.
    if !stem.ends_with('s') {
        return SmolStr::new(stem);
    }
    // Arm 3.
    if stem.ends_with("ss") {
        return SmolStr::new(stem);
    }
    // Arm 4. The length guard keeps `Ties` from becoming `Ty`.
    if stem.len() > 4 && stem.ends_with("ies") {
        let mut held = String::with_capacity(stem.len() - 2);
        held.push_str(&stem[..stem.len() - 3]);
        held.push('y');
        return SmolStr::new(held);
    }
    // Arm 5.
    if stem.ends_with("sses") {
        return SmolStr::new(&stem[..stem.len() - 2]);
    }
    // Arm 6.
    if let Some(prefix) = stem.strip_suffix("es") {
        if prefix.ends_with('x')
            || prefix.ends_with("ch")
            || prefix.ends_with("sh")
            || prefix.ends_with("zz")
        {
            return SmolStr::new(prefix);
        }
    }
    // Arm 7.
    SmolStr::new(&stem[..stem.len() - 1])
}

/// The name one repeating group's occurrence takes under `counter`.
///
/// The component the counter heads, folded to ASCII lowercase like every other
/// field name, and the counter's own name where the display states no counter
/// spelling. One group has one occurrence name whatever the occurrence's type,
/// so the empty-group stand-in is named here too.
///
/// The occurrence carries only this name: no display, no `fix:tag`, no
/// metadata at all. A normal field's display is not derivable and has to be
/// stored; an occurrence's is - it is the counter's - so storing it again
/// would be a second copy of a fact the parent already holds. It is
/// descriptive and never identity: no digest, deduplication key, equality or
/// drift check may read it.
pub(crate) fn occurrence_name(counter: &crate::Field) -> SmolStr {
    counter.display().and_then(component_name).map_or_else(
        || SmolStr::new(counter.name()),
        |held| SmolStr::new(held.to_ascii_lowercase()),
    )
}
