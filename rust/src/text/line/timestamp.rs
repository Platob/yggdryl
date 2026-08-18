//! Reading an entry timestamp, and turning a naive one into a real instant.
//!
//! # Precedence
//!
//! **An offset present in the timestamp text always wins; the
//! [`timezone`](TextLineOptions::timezone) option applies only to timestamps
//! that carry none.** The alternative - the option overriding the text - would
//! silently rewrite data the log author was explicit about.
//!
//! # The limitation a log-ingesting reader has to know
//!
//! [`Timezone::offset_at`] applies the rules **in force today**. Parsing
//! archived logs from a year when a zone's rules differed will therefore
//! produce a wrong instant for the readings inside a changed interval. This
//! feature's main use is exactly historical log ingestion, so the limit is
//! stated here and on the `unix` column rather than left in a module nobody
//! opens. A fixed offset and UTC are exact, because neither has rules to
//! change.

use crate::generic::iso;
use crate::{Error, Result, TimeUnit, Timezone};

use super::options::TextLineOptions;

/// Read a timestamp off the front of `text` under `options`.
///
/// Returns the civil reading, its unit, and the offset **in seconds** that
/// turns it into an instant. `date` and `time` are built from the civil
/// reading, so only `unix` moves with the zone.
///
/// # Errors
///
/// Returns the parser's failure, or the zone policy's refusal for a local
/// reading that does not exist.
pub(crate) fn read(
    text: &str,
    options: &TextLineOptions,
    cache: &mut OffsetCache,
) -> Result<(i64, TimeUnit, i64)> {
    // Routed through `parse_timestamp` first so an offset in the text is not
    // thrown away. This works whether or not the option is set.
    if let Ok((instant, unit, zone)) = iso::parse_timestamp(text) {
        let per = iso::per_second(unit).unwrap_or(1);
        let offset = i64::from(zone.offset_at(instant / per.max(1)).unwrap_or(0));
        // `parse_timestamp` already applied the offset, so the civil reading is
        // recovered by putting it back.
        return Ok((instant + offset * per, unit, offset));
    }
    let (count, unit, end) = iso::parse_datetime_prefix(text)?;
    let _ = end;
    let Some(zone) = options.timezone() else {
        // Unset: today's behavior exactly - the civil reading counted from the
        // epoch, with no zone applied.
        return Ok((count, unit, 0));
    };
    let per = iso::per_second(unit).unwrap_or(1);
    let local_seconds = count.div_euclid(per);
    // Resolving through the registry per row would be pure waste: an offset is
    // stable for months, so a sorted log resolves once or twice per file.
    let offset = i64::from(cache.offset(zone, local_seconds)?);
    Ok((count, unit, offset))
}

/// The offset in effect for a *local* reading, in seconds.
///
/// # The circularity, and why the loop is here
///
/// [`Timezone::offset_at`] takes an **instant**, and a naive reading is not one
/// yet: to know the offset we would already need the answer. A single-pass
/// "treat local as UTC, look up the offset, subtract" is wrong near a
/// transition, because the guessed instant can land on the *other* side of it.
///
/// So: guess, look up, re-apply, and **verify the result maps back to the same
/// local reading**. If it does, the offset is right by construction. If it does
/// not, the reading is inside a gap or an overlap and the stated policy decides.
/// Do not simplify this into the single pass; that is the bug.
///
/// # Gaps and overlaps
///
/// - **Overlap** (the fall-back hour, a local time that occurs twice): the
///   **earliest** offset wins - the first occurrence, which is what a log
///   written during that hour most likely meant, and it keeps a sorted log
///   sorted.
/// - **Gap** (spring-forward, a local time that never occurs): the reading is
///   **shifted forward** by the transition's size, landing on the first instant
///   that does exist. Raising instead would fail a whole file for one line a
///   clock skew produced; shifting is recoverable and is what every mainstream
///   library does.
fn resolve(zone: &Timezone, local_seconds: i64) -> Result<i32> {
    /// How far either side of the naive reading a candidate offset is taken
    /// from. Wide enough to bracket any single transition - the largest in use
    /// is two hours - and narrow enough that two transitions cannot fall inside
    /// it.
    const PROBE: i64 = 6 * 3_600;

    // UTC and a fixed offset have no rules to consult, so they never touch the
    // registry - which is most of what makes a fixed-offset read as cheap as an
    // unzoned one.
    if zone.is_utc() {
        return Ok(0);
    }
    if let Some(offset) = fixed_offset(zone) {
        return Ok(offset);
    }

    // The circularity: `offset_at` wants an *instant*, and a naive reading is
    // not one yet - to know the offset we would already need the answer. A
    // single-pass "treat local as UTC, look up, subtract" is wrong near a
    // transition, because the guessed instant can land on the other side of it.
    //
    // So the candidates are the offsets in effect a little before, at, and a
    // little after the naive instant, which brackets any one transition. An
    // offset is *real* for this reading when applying it lands on an instant
    // whose own offset is that same offset - which is the verification the
    // single pass skips. Do not simplify this away; that is the bug.
    let mut candidates = [
        zone.offset_at(local_seconds - PROBE),
        zone.offset_at(local_seconds),
        zone.offset_at(local_seconds + PROBE),
    ];
    if candidates.iter().any(Option::is_none) {
        return Err(unknown_zone(zone));
    }
    candidates.sort_unstable();

    let mut valid: Option<i32> = None;
    let mut smallest: Option<i32> = None;
    for offset in candidates.into_iter().flatten() {
        smallest = Some(smallest.map_or(offset, |held: i32| held.min(offset)));
        if zone.offset_at(local_seconds - i64::from(offset)) == Some(offset) {
            // A *larger* offset yields an *earlier* instant, so keeping the
            // largest valid one is the overlap policy: the earliest occurrence
            // of a local time that happens twice. Where the reading is not
            // ambiguous there is exactly one valid offset and this picks it.
            valid = Some(valid.map_or(offset, |held: i32| held.max(offset)));
        }
    }
    match valid {
        Some(offset) => Ok(offset),
        // No offset maps back, so the reading is inside a gap and never
        // occurred. The policy shifts *forward* onto the first instant that
        // does, which is what the smallest candidate yields.
        None => smallest.ok_or_else(|| unknown_zone(zone)),
    }
}

/// A zone's offset when it is fixed, so the registry is never consulted.
fn fixed_offset(zone: &Timezone) -> Option<i32> {
    zone.is_fixed().then(|| zone.offset_at(0)).flatten()
}

/// Report a zone the registry does not know.
fn unknown_zone(zone: &Timezone) -> Error {
    Error::InvalidRecord {
        path: smol_str::SmolStr::new_static("$.timezone"),
        reason: crate::text::expected_got(
            "a time zone the registry knows",
            smol_str::format_smolstr!("{zone}"),
        ),
    }
}

/// One resolved offset and the interval it is valid over.
///
/// An offset is stable for months at a time, so resolving one per row through
/// the registry would be pure waste: a sorted log resolves once or twice per
/// file instead. The interval is deliberately conservative - one day either
/// side of the reading, re-checked at the edges - because a transition can
/// only move an offset at a boundary, and a day is far smaller than any real
/// rule interval while being cheap to verify.
#[derive(Clone, Debug)]
pub(crate) struct Resolved {
    offset: i32,
    /// The half-open local-second interval the offset is trusted over.
    valid: std::ops::Range<i64>,
}

/// A cache of the last resolved offset, consulted before the registry.
#[derive(Clone, Debug, Default)]
pub(crate) struct OffsetCache(Option<Resolved>);

impl OffsetCache {
    /// The offset for a local reading, from the cache when it still applies.
    ///
    /// # Errors
    ///
    /// Returns the zone policy's refusal for an unknown zone.
    pub(crate) fn offset(&mut self, zone: &Timezone, local_seconds: i64) -> Result<i32> {
        if let Some(held) = &self.0 {
            if held.valid.contains(&local_seconds) {
                return Ok(held.offset);
            }
        }
        let offset = resolve(zone, local_seconds)?;
        // Trust the offset only over the window either side that resolves the
        // same way; a transition inside it would make the cache lie.
        // One day either side: a transition can only move an offset at a
        // boundary, and a day is far smaller than any real rule interval while
        // staying cheap to verify.
        const DAY: i64 = 86_400;
        let low = local_seconds - DAY;
        let high = local_seconds + DAY;
        let valid = if zone.is_utc() || zone.is_fixed() {
            i64::MIN..i64::MAX
        } else if resolve(zone, low).ok() == Some(offset)
            && resolve(zone, high).ok() == Some(offset)
        {
            low..high
        } else {
            local_seconds..local_seconds + 1
        };
        self.0 = Some(Resolved { offset, valid });
        Ok(offset)
    }
}
