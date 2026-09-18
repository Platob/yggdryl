//! One way to name a time zone, everywhere in the project - and the datatype
//! a column of them declares.
//!
//! Registered names, aliases, fixed offsets, and the explicit zone-free marker
//! all resolve through this one value.
//!
//! [`Timezone::NAIVE`] gives every native temporal value and datatype a
//! non-optional zone while Arrow projects that marker as an absent timezone. A
//! zone is a process-lifetime interned handle; [`Timezone::offset_at`] applies
//! the registry rules bundled by this build.
//!
//! A zone is also a value in its own right. [`crate::DataType::Timezone`] is
//! the column that holds one: canonical text in Arrow's `Utf8`, kept a zone
//! across a round trip by its extension name, and read back through the same
//! `from_str` every other spelling crosses. It is neither a bounded ASCII code
//! nor a static vocabulary - an IANA name is as long as the registry says and
//! a fixed offset is generated, not enumerated - so it is its own datatype,
//! the way a URL and a version are.
//!
//! ```
//! use yggdryl::{DataType, Scalar, Timezone};
//!
//! # fn main() -> yggdryl::Result<()> {
//! // Aliases and case both canonicalize, so two spellings compare equal.
//! assert_eq!(Timezone::from_str("Asia/Calcutta")?, Timezone::from_str("Asia/Kolkata")?);
//! assert_eq!(Timezone::from_str("Z")?, Timezone::UTC);
//!
//! // A registered zone knows its own rules.
//! let new_york = Timezone::from_str("America/New_York")?;
//! assert_eq!(new_york.offset_at(1_700_000_000), Some(-5 * 3600));  // November: EST
//! assert_eq!(new_york.offset_at(1_688_000_000), Some(-4 * 3600));  // June: EDT
//! assert_eq!(new_york.abbreviation_at(1_688_000_000), Some("EDT"));
//!
//! // A fixed offset needs no registry at all.
//! assert_eq!(Timezone::from_str("+05:30")?.offset_at(0), Some(5 * 3600 + 1800));
//!
//! // And a column of zones declares the datatype, which canonicalizes on the
//! // way in exactly as the value does.
//! assert_eq!(DataType::from_str("timezone")?, DataType::Timezone);
//! assert_eq!(
//!     DataType::Timezone.scalar("Asia/Calcutta")?,
//!     Scalar::Timezone(Timezone::from_str("Asia/Kolkata")?),
//! );
//! # Ok(())
//! # }
//! ```

use std::collections::HashMap;
use std::num::NonZeroU32;
use std::sync::{OnceLock, RwLock};

use smol_str::SmolStr;
pub use value::Timezone;
pub(crate) use value::{civil_from_days, days_from_civil};

use crate::types::typed::define_field_types;
use crate::{Error, Result};

/// Arrow casts owned by the time zone datatype.
pub(crate) mod casts {
    use std::sync::Arc;

    use arrow_array::{Array, ArrayRef, StringArray};
    use arrow_buffer::BooleanBuffer;
    use arrow_schema::DataType as ArrowDataType;

    use crate::arrow::{Error, Result};
    use crate::types::budget::MaterializationBudget;
    use crate::types::cast::{arrow_cast_exposed, downcast};
    use crate::types::cast::columns::is_exposed;
    use crate::{DataType, Field, Timezone};

    /// Parse and canonicalize every exposed text cell into time zone Utf8 storage.
    pub(crate) fn ingest_timezone_array(
        array: &ArrayRef,
        field: &Field,
        exposure: Option<&BooleanBuffer>,
        budget: &mut MaterializationBudget,
    ) -> Result<ArrayRef> {
        let text = if array.data_type() == &ArrowDataType::Utf8 {
            Arc::clone(array)
        } else {
            arrow_cast_exposed(
                array,
                &ArrowDataType::Utf8,
                true,
                exposure,
                &Field::new(field.name(), DataType::utf8(), true),
                budget,
            )?
        };
        let source = downcast::<StringArray>(text.as_ref())?;
        budget.add_array(field.dtype(), source.len())?;
        let mut values = Vec::with_capacity(source.len());
        let mut payload = 0_usize;
        for index in 0..source.len() {
            if !is_exposed(exposure, index) || source.is_null(index) {
                values.push(None);
                continue;
            }
            let raw = source.value(index);
            let parsed = Timezone::from_str(raw).map_err(|error| {
                Error::IncompatibleSchema(format!(
                    "field {:?} row {index}: {raw:?} does not read as time zone: {error}",
                    field.name()
                ))
            })?;
            let canonical = parsed.to_string();
            payload = payload.saturating_add(canonical.len());
            values.push(Some(canonical));
        }
        budget.add_bytes(payload)?;
        Ok(Arc::new(StringArray::from(values)))
    }
}

// ------------------------------------------------------------------------
// Time zone field marker and typed aliases.
// ------------------------------------------------------------------------

define_field_types!(TimezoneType, Timezone);


// ------------------------------------------------------------------------
// The registry of time zones this build knows the rules for.
//
// This is a *rules* table, not a history. Each entry carries the standard
// offset and the daylight-saving rule **in force today**, which is what a
// schema, a partition value, or a freshly written batch actually needs. It is
// deliberately not a replacement for the IANA database: applying today's rule
// to a 1975 instant would answer confidently and wrongly.
//
// That is also why the table is short. A zone whose real rule is historical,
// irregular, or politically volatile - Israel, Iran, Egypt, Lord Howe - is
// *left out* rather than approximated, so it parses as an ordinary named zone
// and reports its offset as unknown. Refusing to answer is recoverable; a
// plausible wrong answer is not.
// ------------------------------------------------------------------------

const NAIVE_ID: u32 = 1;
const UTC_ID: u32 = 2;
const FIXED_BASE: u32 = 3;
const MIN_OFFSET_MINUTES: i32 = -(24 * 60 - 1);
const MAX_OFFSET_MINUTES: i32 = 24 * 60 - 1;
const FIXED_COUNT: u32 = (MAX_OFFSET_MINUTES - MIN_OFFSET_MINUTES + 1) as u32;
const FIXED_COUNT_USIZE: usize = FIXED_COUNT as usize;
const REGISTERED_BASE: u32 = FIXED_BASE + FIXED_COUNT;

const fn fixed_name_bytes() -> [[u8; 6]; FIXED_COUNT_USIZE] {
    let mut names = [[0; 6]; FIXED_COUNT_USIZE];
    let mut index = 0;
    while index < FIXED_COUNT_USIZE {
        let minutes = MIN_OFFSET_MINUTES + index as i32;
        let sign = if minutes < 0 { b'-' } else { b'+' };
        let absolute = minutes.abs();
        let hours = absolute / 60;
        let minute = absolute % 60;
        names[index] = [
            sign,
            b'0' + (hours / 10) as u8,
            b'0' + (hours % 10) as u8,
            b':',
            b'0' + (minute / 10) as u8,
            b'0' + (minute % 10) as u8,
        ];
        index += 1;
    }
    names
}

const fn nonzero(value: u32) -> NonZeroU32 {
    match NonZeroU32::new(value) {
        Some(value) => value,
        None => panic!("a time-zone handle must be nonzero"),
    }
}

pub(super) const NAIVE_HANDLE: NonZeroU32 = nonzero(NAIVE_ID);
pub(super) const UTC_HANDLE: NonZeroU32 = nonzero(UTC_ID);

/// Which clock a transition time is measured against.
///
/// The three bases are not interchangeable, and picking the wrong one moves a
/// transition by up to two hours: the United States writes its rule in local
/// standard time, the European Union writes it in UTC so that every member
/// state switches simultaneously, and a few zones write it in wall-clock time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Basis {
    /// The transition time is UTC, so every zone using it switches at once.
    Utc,
    /// The transition time is local standard time, ignoring any saving.
    Standard,
    /// The transition time is the wall clock reading at that moment.
    Wall,
}

/// One end of a daylight-saving period, as an nth-weekday-of-month rule.
#[derive(Clone, Copy, Debug)]
pub(super) struct Edge {
    /// Month of the year, 1 through 12.
    pub(super) month: u8,
    /// Which weekday of the month: 1 through 4, or 5 meaning the last one.
    pub(super) week: u8,
    /// Day of the week, 0 for Sunday through 6 for Saturday.
    pub(super) weekday: u8,
    /// Time of day the change happens, in seconds, read against `basis`.
    pub(super) seconds: i32,
    /// The clock `seconds` is measured against.
    pub(super) basis: Basis,
}

/// A daylight-saving rule: when it starts, when it ends, and how much is saved.
#[derive(Clone, Copy, Debug)]
pub(super) struct Saving {
    /// When the clocks go forward.
    pub(super) start: Edge,
    /// When the clocks go back.
    pub(super) end: Edge,
    /// Seconds added to the standard offset while the rule is in force.
    pub(super) save: i32,
}

/// One registered zone.
#[derive(Clone, Copy, Debug)]
pub(super) struct Zone {
    /// The canonical IANA name.
    pub(super) name: &'static str,
    /// Seconds east of UTC when no saving is in force.
    pub(super) standard: i32,
    /// The abbreviation used outside any saving period.
    pub(super) standard_abbreviation: &'static str,
    /// The daylight-saving rule, when the zone observes one.
    pub(super) saving: Option<Saving>,
    /// The abbreviation used inside a saving period.
    pub(super) daylight_abbreviation: Option<&'static str>,
}

/// One hour, the saving almost every zone that observes one applies.
const HOUR: i32 = 3_600;

/// The rule the United States and Canada have used since 2007: forward on the
/// second Sunday in March and back on the first Sunday in November, each at
/// "02:00 local time".
///
/// The two ends read that phrase against different clocks, and the difference
/// is a whole hour. In March saving has not begun, so 02:00 is standard time;
/// in November it is still in force, so 02:00 is the wall clock *including*
/// the saving. Encoding both as standard time would move the autumn
/// transition an hour late.
const AMERICAN: Saving = Saving {
    start: Edge {
        month: 3,
        week: 2,
        weekday: 0,
        seconds: 2 * HOUR,
        basis: Basis::Standard,
    },
    end: Edge {
        month: 11,
        week: 1,
        weekday: 0,
        seconds: 2 * HOUR,
        basis: Basis::Wall,
    },
    save: HOUR,
};

/// The rule the European Union has used since 1996: forward on the last Sunday
/// in March and back on the last Sunday in October, both at 01:00 **UTC**, so
/// that the whole union switches at the same instant rather than at the same
/// local reading.
const EUROPEAN: Saving = Saving {
    start: Edge {
        month: 3,
        week: 5,
        weekday: 0,
        seconds: HOUR,
        basis: Basis::Utc,
    },
    end: Edge {
        month: 10,
        week: 5,
        weekday: 0,
        seconds: HOUR,
        basis: Basis::Utc,
    },
    save: HOUR,
};

/// The southern-hemisphere rule most of eastern Australia uses: forward on the
/// first Sunday in October and back on the first Sunday in April, at 02:00
/// local standard time. Its period spans the new year, which is what the
/// wrapping comparison in the offset lookup exists for.
const AUSTRALIAN: Saving = Saving {
    start: Edge {
        month: 10,
        week: 1,
        weekday: 0,
        seconds: 2 * HOUR,
        basis: Basis::Standard,
    },
    end: Edge {
        month: 4,
        week: 1,
        weekday: 0,
        seconds: 2 * HOUR,
        basis: Basis::Standard,
    },
    save: HOUR,
};

/// New Zealand's rule: forward on the last Sunday in September, back on the
/// first Sunday in April, at 02:00 local standard time.
const NEW_ZEALAND: Saving = Saving {
    start: Edge {
        month: 9,
        week: 5,
        weekday: 0,
        seconds: 2 * HOUR,
        basis: Basis::Standard,
    },
    end: Edge {
        month: 4,
        week: 1,
        weekday: 0,
        seconds: 2 * HOUR,
        basis: Basis::Standard,
    },
    save: HOUR,
};

/// Build a zone that observes no daylight saving.
const fn fixed(name: &'static str, standard: i32, abbreviation: &'static str) -> Zone {
    Zone {
        name,
        standard,
        standard_abbreviation: abbreviation,
        saving: None,
        daylight_abbreviation: None,
    }
}

/// Build a zone that observes `saving`.
const fn saving(
    name: &'static str,
    standard: i32,
    standard_abbreviation: &'static str,
    rule: Saving,
    daylight_abbreviation: &'static str,
) -> Zone {
    Zone {
        name,
        standard,
        standard_abbreviation,
        saving: Some(rule),
        daylight_abbreviation: Some(daylight_abbreviation),
    }
}

/// Every zone this build knows the current rules for, sorted by name.
///
/// The sort is load-bearing: lookup is a binary search, and the test beside
/// this module asserts the ordering so an unsorted insertion cannot ship.
pub(super) static ZONES: &[Zone] = &[
    fixed("Africa/Abidjan", 0, "GMT"),
    fixed("Africa/Accra", 0, "GMT"),
    fixed("Africa/Johannesburg", 2 * HOUR, "SAST"),
    fixed("Africa/Lagos", HOUR, "WAT"),
    fixed("Africa/Nairobi", 3 * HOUR, "EAT"),
    fixed("America/Anchorage", -9 * HOUR, "AKST"),
    fixed("America/Argentina/Buenos_Aires", -3 * HOUR, "-03"),
    fixed("America/Bogota", -5 * HOUR, "-05"),
    saving("America/Chicago", -6 * HOUR, "CST", AMERICAN, "CDT"),
    saving("America/Denver", -7 * HOUR, "MST", AMERICAN, "MDT"),
    saving("America/Edmonton", -7 * HOUR, "MST", AMERICAN, "MDT"),
    saving("America/Halifax", -4 * HOUR, "AST", AMERICAN, "ADT"),
    fixed("America/Lima", -5 * HOUR, "-05"),
    saving("America/Los_Angeles", -8 * HOUR, "PST", AMERICAN, "PDT"),
    fixed("America/Mexico_City", -6 * HOUR, "CST"),
    saving("America/New_York", -5 * HOUR, "EST", AMERICAN, "EDT"),
    fixed("America/Panama", -5 * HOUR, "EST"),
    fixed("America/Phoenix", -7 * HOUR, "MST"),
    fixed("America/Sao_Paulo", -3 * HOUR, "-03"),
    saving("America/Toronto", -5 * HOUR, "EST", AMERICAN, "EDT"),
    saving("America/Vancouver", -8 * HOUR, "PST", AMERICAN, "PDT"),
    saving("America/Winnipeg", -6 * HOUR, "CST", AMERICAN, "CDT"),
    fixed("Asia/Baghdad", 3 * HOUR, "+03"),
    fixed("Asia/Bangkok", 7 * HOUR, "+07"),
    fixed("Asia/Dhaka", 6 * HOUR, "+06"),
    fixed("Asia/Dubai", 4 * HOUR, "+04"),
    fixed("Asia/Hong_Kong", 8 * HOUR, "HKT"),
    fixed("Asia/Jakarta", 7 * HOUR, "WIB"),
    fixed("Asia/Karachi", 5 * HOUR, "PKT"),
    fixed("Asia/Kathmandu", 5 * HOUR + 45 * 60, "+0545"),
    fixed("Asia/Kolkata", 5 * HOUR + 30 * 60, "IST"),
    fixed("Asia/Kuala_Lumpur", 8 * HOUR, "+08"),
    fixed("Asia/Manila", 8 * HOUR, "PST"),
    fixed("Asia/Riyadh", 3 * HOUR, "+03"),
    fixed("Asia/Seoul", 9 * HOUR, "KST"),
    fixed("Asia/Shanghai", 8 * HOUR, "CST"),
    fixed("Asia/Singapore", 8 * HOUR, "+08"),
    fixed("Asia/Taipei", 8 * HOUR, "CST"),
    fixed("Asia/Tokyo", 9 * HOUR, "JST"),
    fixed("Atlantic/Reykjavik", 0, "GMT"),
    saving(
        "Australia/Adelaide",
        9 * HOUR + 30 * 60,
        "ACST",
        AUSTRALIAN,
        "ACDT",
    ),
    fixed("Australia/Brisbane", 10 * HOUR, "AEST"),
    fixed("Australia/Darwin", 9 * HOUR + 30 * 60, "ACST"),
    saving("Australia/Melbourne", 10 * HOUR, "AEST", AUSTRALIAN, "AEDT"),
    fixed("Australia/Perth", 8 * HOUR, "AWST"),
    saving("Australia/Sydney", 10 * HOUR, "AEST", AUSTRALIAN, "AEDT"),
    saving("Europe/Amsterdam", HOUR, "CET", EUROPEAN, "CEST"),
    saving("Europe/Athens", 2 * HOUR, "EET", EUROPEAN, "EEST"),
    saving("Europe/Berlin", HOUR, "CET", EUROPEAN, "CEST"),
    saving("Europe/Brussels", HOUR, "CET", EUROPEAN, "CEST"),
    saving("Europe/Bucharest", 2 * HOUR, "EET", EUROPEAN, "EEST"),
    saving("Europe/Budapest", HOUR, "CET", EUROPEAN, "CEST"),
    saving("Europe/Copenhagen", HOUR, "CET", EUROPEAN, "CEST"),
    saving("Europe/Dublin", 0, "GMT", EUROPEAN, "IST"),
    saving("Europe/Helsinki", 2 * HOUR, "EET", EUROPEAN, "EEST"),
    fixed("Europe/Istanbul", 3 * HOUR, "+03"),
    saving("Europe/Kyiv", 2 * HOUR, "EET", EUROPEAN, "EEST"),
    saving("Europe/Lisbon", 0, "WET", EUROPEAN, "WEST"),
    saving("Europe/London", 0, "GMT", EUROPEAN, "BST"),
    saving("Europe/Madrid", HOUR, "CET", EUROPEAN, "CEST"),
    fixed("Europe/Minsk", 3 * HOUR, "+03"),
    fixed("Europe/Moscow", 3 * HOUR, "MSK"),
    saving("Europe/Oslo", HOUR, "CET", EUROPEAN, "CEST"),
    saving("Europe/Paris", HOUR, "CET", EUROPEAN, "CEST"),
    saving("Europe/Prague", HOUR, "CET", EUROPEAN, "CEST"),
    saving("Europe/Rome", HOUR, "CET", EUROPEAN, "CEST"),
    saving("Europe/Stockholm", HOUR, "CET", EUROPEAN, "CEST"),
    saving("Europe/Vienna", HOUR, "CET", EUROPEAN, "CEST"),
    saving("Europe/Warsaw", HOUR, "CET", EUROPEAN, "CEST"),
    saving("Europe/Zurich", HOUR, "CET", EUROPEAN, "CEST"),
    saving("Pacific/Auckland", 12 * HOUR, "NZST", NEW_ZEALAND, "NZDT"),
    fixed("Pacific/Honolulu", -10 * HOUR, "HST"),
    fixed("UTC", 0, "UTC"),
];

/// Names that resolve to a registered zone under a different spelling.
///
/// Three kinds live here and they are not the same thing: the IANA database's
/// own renamings (`Asia/Calcutta` became `Asia/Kolkata`), the deprecated
/// country-prefixed aliases every operating system still ships (`US/Eastern`),
/// and the wire spellings of UTC that arrive from other systems (`Z`, `GMT`,
/// `Etc/UTC`). Canonicalizing all three on the way in is what keeps two
/// schemas naming the same zone comparable.
///
/// Sorted by alias, for the same reason [`ZONES`] is.
pub(super) static ALIASES: &[(&str, &str)] = &[
    ("Asia/Calcutta", "Asia/Kolkata"),
    ("Asia/Chongqing", "Asia/Shanghai"),
    ("Asia/Katmandu", "Asia/Kathmandu"),
    ("Asia/Saigon", "Asia/Bangkok"),
    ("Australia/Canberra", "Australia/Sydney"),
    ("Australia/NSW", "Australia/Sydney"),
    ("Australia/Queensland", "Australia/Brisbane"),
    ("Australia/Victoria", "Australia/Melbourne"),
    ("Brazil/East", "America/Sao_Paulo"),
    ("Canada/Atlantic", "America/Halifax"),
    ("Canada/Central", "America/Winnipeg"),
    ("Canada/Eastern", "America/Toronto"),
    ("Canada/Mountain", "America/Edmonton"),
    ("Canada/Pacific", "America/Vancouver"),
    ("Etc/GMT", "UTC"),
    ("Etc/UCT", "UTC"),
    ("Etc/UTC", "UTC"),
    ("Etc/Universal", "UTC"),
    ("Etc/Zulu", "UTC"),
    ("Europe/Kiev", "Europe/Kyiv"),
    ("Europe/Nicosia", "Europe/Athens"),
    ("GMT", "UTC"),
    ("GMT0", "UTC"),
    ("Greenwich", "UTC"),
    ("Hongkong", "Asia/Hong_Kong"),
    ("Iceland", "Atlantic/Reykjavik"),
    ("Israel", "Asia/Jerusalem"),
    ("Japan", "Asia/Tokyo"),
    ("Mexico/General", "America/Mexico_City"),
    ("NZ", "Pacific/Auckland"),
    ("Navajo", "America/Denver"),
    ("PRC", "Asia/Shanghai"),
    ("Poland", "Europe/Warsaw"),
    ("Portugal", "Europe/Lisbon"),
    ("ROC", "Asia/Taipei"),
    ("ROK", "Asia/Seoul"),
    ("Singapore", "Asia/Singapore"),
    ("Turkey", "Europe/Istanbul"),
    ("UCT", "UTC"),
    ("US/Alaska", "America/Anchorage"),
    ("US/Arizona", "America/Phoenix"),
    ("US/Central", "America/Chicago"),
    ("US/Eastern", "America/New_York"),
    ("US/Hawaii", "Pacific/Honolulu"),
    ("US/Mountain", "America/Denver"),
    ("US/Pacific", "America/Los_Angeles"),
    ("Universal", "UTC"),
    ("Z", "UTC"),
    ("Zulu", "UTC"),
];

static NAIVE_NAME: SmolStr = SmolStr::new_inline("NAIVE");
static UTC_NAME: SmolStr = SmolStr::new_inline("UTC");
static FIXED_NAME_BYTES: [[u8; 6]; FIXED_COUNT_USIZE] = fixed_name_bytes();
static FIXED_NAMES: [OnceLock<SmolStr>; FIXED_COUNT_USIZE] =
    [const { OnceLock::new() }; FIXED_COUNT_USIZE];
static REGISTERED_NAMES: OnceLock<Box<[OnceLock<SmolStr>]>> = OnceLock::new();
static INTERNED_NAMES: OnceLock<RwLock<InternedNames>> = OnceLock::new();

/// How many unregistered zone names one process retains.
///
/// A handle is a `NonZeroU32` that reads back as a `&'static str`, so an
/// interned name is retained for the life of the process and can never be
/// reclaimed. Names arrive from file content - an Arrow timestamp column, a
/// parsed schema - so without a ceiling a stream of distinct spellings grows
/// the table without bound. The IANA database this build carries names some
/// hundreds of zones; a schema needing more than sixty-four thousand
/// *unregistered* ones is a malformed source, and it is refused by name here
/// rather than consuming the process.
const MAX_INTERNED_NAMES: usize = 64 * 1024;

#[derive(Default)]
struct InternedNames {
    by_name: HashMap<&'static str, NonZeroU32>,
    names: Vec<&'static SmolStr>,
}

fn dynamic_base() -> u32 {
    REGISTERED_BASE
        + u32::try_from(ZONES.len()).expect("the built-in time-zone table fits in a u32")
}

fn registered_names() -> &'static [OnceLock<SmolStr>] {
    REGISTERED_NAMES.get_or_init(|| {
        (0..ZONES.len())
            .map(|_| OnceLock::new())
            .collect::<Box<[_]>>()
    })
}

fn interner() -> &'static RwLock<InternedNames> {
    INTERNED_NAMES.get_or_init(|| RwLock::new(InternedNames::default()))
}

fn capacity_error() -> Error {
    Error::Parse {
        target: "timezone",
        position: 0,
        reason: SmolStr::new_static("the process time-zone registry is full"),
    }
}

fn zone_index(name: &str) -> Option<usize> {
    ZONES.binary_search_by(|entry| entry.name.cmp(name)).ok()
}

pub(super) fn registered_handle(index: usize) -> NonZeroU32 {
    if ZONES[index].name == "UTC" {
        return UTC_HANDLE;
    }
    let index = u32::try_from(index).expect("the built-in time-zone table fits in a u32");
    nonzero(REGISTERED_BASE + index)
}

pub(super) fn registered(name: &str) -> Option<NonZeroU32> {
    zone_index(name).map(registered_handle)
}

pub(super) fn registered_ignoring_case(name: &str) -> Option<NonZeroU32> {
    zone_index(name)
        .or_else(|| {
            ZONES
                .iter()
                .position(|entry| entry.name.eq_ignore_ascii_case(name))
        })
        .map(registered_handle)
}

pub(super) fn fixed_handle(seconds: i32) -> NonZeroU32 {
    let minutes = seconds / 60;
    debug_assert!((MIN_OFFSET_MINUTES..=MAX_OFFSET_MINUTES).contains(&minutes));
    let index = u32::try_from(minutes - MIN_OFFSET_MINUTES)
        .expect("a validated fixed offset has a non-negative index");
    nonzero(FIXED_BASE + index)
}

pub(super) fn fixed_offset(handle: NonZeroU32) -> Option<i32> {
    let value = handle.get();
    if value == UTC_ID {
        return Some(0);
    }
    if !(FIXED_BASE..REGISTERED_BASE).contains(&value) {
        return None;
    }
    let index = i32::try_from(value - FIXED_BASE).expect("the fixed-offset range fits in i32");
    Some((MIN_OFFSET_MINUTES + index) * 60)
}

fn fixed_str(handle: NonZeroU32) -> &'static str {
    let index =
        usize::try_from(handle.get() - FIXED_BASE).expect("the fixed-offset range fits in usize");
    std::str::from_utf8(&FIXED_NAME_BYTES[index]).expect("the generated fixed-offset name is ASCII")
}

fn fixed_name(handle: NonZeroU32) -> &'static SmolStr {
    let index =
        usize::try_from(handle.get() - FIXED_BASE).expect("the fixed-offset range fits in usize");
    FIXED_NAMES[index].get_or_init(|| SmolStr::new(fixed_str(handle)))
}

fn registered_name(index: usize) -> &'static SmolStr {
    registered_names()[index].get_or_init(|| SmolStr::new_static(ZONES[index].name))
}

fn dynamic_name(handle: NonZeroU32) -> &'static SmolStr {
    let index = usize::try_from(handle.get() - dynamic_base())
        .expect("a dynamic time-zone handle index fits in usize");
    let names = interner().read().unwrap_or_else(|error| error.into_inner());
    names
        .names
        .get(index)
        .copied()
        .expect("time-zone handles are minted only after their names are retained")
}

pub(super) fn smol_str(handle: NonZeroU32) -> &'static SmolStr {
    let value = handle.get();
    match value {
        NAIVE_ID => &NAIVE_NAME,
        UTC_ID => &UTC_NAME,
        value if (FIXED_BASE..REGISTERED_BASE).contains(&value) => fixed_name(handle),
        value if value < dynamic_base() => {
            let index = usize::try_from(value - REGISTERED_BASE)
                .expect("a registered time-zone handle index fits in usize");
            registered_name(index)
        }
        _ => dynamic_name(handle),
    }
}

pub(super) fn name(handle: NonZeroU32) -> &'static str {
    let value = handle.get();
    match value {
        NAIVE_ID => "NAIVE",
        UTC_ID => "UTC",
        value if (FIXED_BASE..REGISTERED_BASE).contains(&value) => fixed_str(handle),
        value if value < dynamic_base() => {
            let index = usize::try_from(value - REGISTERED_BASE)
                .expect("a registered time-zone handle index fits in usize");
            ZONES[index].name
        }
        _ => dynamic_name(handle).as_str(),
    }
}

pub(super) fn intern(name: &str) -> Result<NonZeroU32> {
    if let Some(handle) = registered(name) {
        return Ok(handle);
    }

    {
        let names = interner().read().unwrap_or_else(|error| error.into_inner());
        if let Some(handle) = names.by_name.get(name) {
            return Ok(*handle);
        }
    }

    let mut names = interner()
        .write()
        .unwrap_or_else(|error| error.into_inner());
    if let Some(handle) = names.by_name.get(name) {
        return Ok(*handle);
    }

    if names.names.len() >= MAX_INTERNED_NAMES {
        return Err(capacity_error());
    }
    let index = u32::try_from(names.names.len()).map_err(|_| capacity_error())?;
    let value = dynamic_base()
        .checked_add(index)
        .ok_or_else(capacity_error)?;
    let handle = NonZeroU32::new(value).ok_or_else(capacity_error)?;
    let stored: &'static SmolStr = Box::leak(Box::new(SmolStr::new(name)));
    names.by_name.insert(stored.as_str(), handle);
    names.names.push(stored);
    Ok(handle)
}

pub(super) fn zone_for_handle(handle: NonZeroU32) -> Option<&'static Zone> {
    let value = handle.get();
    if value == UTC_ID {
        return zone("UTC");
    }
    if !(REGISTERED_BASE..dynamic_base()).contains(&value) {
        return None;
    }
    let index = usize::try_from(value - REGISTERED_BASE)
        .expect("a registered time-zone handle index fits in usize");
    Some(&ZONES[index])
}

/// Find a registered zone by its exact canonical name.
pub(super) fn zone(name: &str) -> Option<&'static Zone> {
    zone_index(name).map(|index| &ZONES[index])
}

/// Resolve an alias to the canonical name it stands for.
///
/// The match is case-insensitive on the ASCII letters, because zone names
/// arrive from wire formats and command lines that do not agree on case, and
/// `utc` naming a different zone from `UTC` would be a trap rather than a
/// distinction.
pub(super) fn alias(name: &str) -> Option<&'static str> {
    ALIASES
        .binary_search_by(|(from, _)| (*from).cmp(name))
        .ok()
        .map(|index| ALIASES[index].1)
        .or_else(|| {
            ALIASES
                .iter()
                .find(|(from, _)| from.eq_ignore_ascii_case(name))
                .map(|(_, to)| *to)
        })
}
/// The time zone value: canonical names, fixed offsets, and the registry
/// rules bundled by this build.
mod value {
    use std::cmp::Ordering;
    use std::fmt;
    use std::num::NonZeroU32;
    use std::str::FromStr;

    use serde::de::Error as _;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use smol_str::SmolStr;

    use crate::types::scalar::{Value, text_scalar_value};
    use crate::{DataType, Error, Result, Scalar, hashing::stable_hash_display};

    use super::{Basis, Edge, Zone};

    /// Seconds in one day, the modulus every civil-time calculation turns on.
    const DAY: i64 = 86_400;

    /// A validated, canonical time zone name.
    ///
    /// The name is canonical on arrival: an alias resolves to what it stands for,
    /// a fixed offset normalizes to `+HH:MM`, and a registered name normalizes its
    /// case. The four-byte handle is process-local and never serialized; names are
    /// retained for the process lifetime, so copying a zone never allocates.
    #[repr(transparent)]
    #[derive(Clone, Copy, Eq, Hash, PartialEq)]
    pub struct Timezone(NonZeroU32);

    const _: () = assert!(std::mem::size_of::<Timezone>() == 4);

    impl Timezone {
        /// A wall-clock value with no time-zone interpretation.
        pub const NAIVE: Self = Self(crate::types::timezone::NAIVE_HANDLE);

        /// Coordinated Universal Time, the zero point every other zone offsets
        /// from and the one zone that is always registered.
        pub const UTC: Self = Self(crate::types::timezone::UTC_HANDLE);

        /// Parse and canonicalize a time zone name.
        ///
        /// Four spellings are accepted, in this order: a registered IANA name, an
        /// alias for one, a fixed offset (`+05:30`, `-0800`, `Z`), and any other
        /// syntactically plausible IANA name, which is kept as written so a zone
        /// this build has no rules for still round-trips through a schema.
        ///
        /// # Errors
        ///
        /// Returns an error for an empty name, a name holding a control character,
        /// or an offset whose hour or minute is out of range.
        #[allow(clippy::should_implement_trait)]
        pub fn from_str(value: &str) -> Result<Self> {
            <Self as FromStr>::from_str(value)
        }

        /// Build a zone from a fixed offset east of UTC, in seconds.
        ///
        /// # Errors
        ///
        /// Returns an error when the offset is beyond ±24 hours or is not a whole
        /// number of minutes, neither of which any real zone uses.
        pub fn from_offset(seconds: i32) -> Result<Self> {
            if seconds.abs() >= 24 * 3_600 {
                return Err(parse_error(
                    0,
                    "a fixed time zone offset must be within 24 hours of UTC",
                ));
            }
            if seconds % 60 != 0 {
                return Err(parse_error(
                    0,
                    "a fixed time zone offset must be a whole number of minutes",
                ));
            }
            if seconds == 0 {
                return Ok(Self::UTC);
            }
            Ok(Self(crate::types::timezone::fixed_handle(seconds)))
        }

        /// Return the canonical name without allocating.
        pub fn as_str(&self) -> &str {
            crate::types::timezone::name(self.0)
        }

        /// Borrow the canonical name as a shared string.
        ///
        /// The interner retains this value for the process lifetime. Arrow
        /// projection can therefore clone a long name's shared allocation rather
        /// than copy its bytes.
        pub fn as_smol_str(&self) -> &SmolStr {
            crate::types::timezone::smol_str(self.0)
        }

        /// Consume this zone and return the shared name.
        pub fn into_smol_str(self) -> SmolStr {
            self.as_smol_str().clone()
        }

        /// Return whether this zone is UTC itself.
        pub fn is_utc(&self) -> bool {
            *self == Self::UTC
        }

        /// Return whether this is the explicit zone-free marker.
        pub fn is_naive(&self) -> bool {
            *self == Self::NAIVE
        }

        /// Return whether this build knows the rules for this zone.
        ///
        /// A zone that is not known still parses, compares, and round-trips; it
        /// simply answers `None` to every offset question rather than guessing.
        pub fn is_known(&self) -> bool {
            self.entry().is_some() || self.fixed_offset().is_some()
        }

        /// Return whether the name is a fixed offset rather than a place.
        pub fn is_fixed(&self) -> bool {
            self.fixed_offset().is_some()
        }

        /// Return whether this zone ever observes daylight saving.
        ///
        /// An unknown zone answers `false`, because nothing is known to observe.
        pub fn observes_saving(&self) -> bool {
            self.entry().is_some_and(|zone| zone.saving.is_some())
        }

        /// Return the offset east of UTC, in seconds, at an instant.
        ///
        /// `epoch` is seconds since the Unix epoch, UTC. The answer accounts for
        /// daylight saving under the rules in force today; a zone this build has
        /// no rules for answers `None`.
        ///
        /// ```
        /// use yggdryl::Timezone;
        ///
        /// # fn main() -> yggdryl::Result<()> {
        /// let sydney = Timezone::from_str("Australia/Sydney")?;
        ///
        /// // Sydney's saving period spans the new year, so January is +11.
        /// assert_eq!(sydney.offset_at(1_704_067_200), Some(11 * 3600));
        /// // ... and July is +10.
        /// assert_eq!(sydney.offset_at(1_720_000_000), Some(10 * 3600));
        /// # Ok(())
        /// # }
        /// ```
        pub fn offset_at(&self, epoch: i64) -> Option<i32> {
            if let Some(offset) = self.fixed_offset() {
                return Some(offset);
            }
            let zone = self.entry()?;
            Some(zone.standard + self.saving_at(zone, epoch))
        }

        /// Return the standard offset east of UTC, ignoring daylight saving.
        pub fn standard_offset(&self) -> Option<i32> {
            self.fixed_offset()
                .or_else(|| self.entry().map(|zone| zone.standard))
        }

        /// Return whether daylight saving is in force at an instant.
        pub fn is_saving_at(&self, epoch: i64) -> Option<bool> {
            if self.is_fixed() {
                return Some(false);
            }
            let zone = self.entry()?;
            Some(self.saving_at(zone, epoch) != 0)
        }

        /// Return the abbreviation in use at an instant, such as `EST` or `CEST`.
        pub fn abbreviation_at(&self, epoch: i64) -> Option<&'static str> {
            let zone = self.entry()?;
            if self.saving_at(zone, epoch) == 0 {
                return Some(zone.standard_abbreviation);
            }
            zone.daylight_abbreviation
                .or(Some(zone.standard_abbreviation))
        }

        /// Convert a UTC instant to the local reading in this zone.
        ///
        /// Both values are seconds since the Unix epoch; the local reading is the
        /// wall clock expressed as if it were UTC, which is the convention every
        /// naive timestamp in the project already uses.
        ///
        /// # Errors
        ///
        /// Returns an error when this build has no rules for the zone, because a
        /// silently unconverted instant is worse than a refusal.
        pub fn into_local(self, epoch: i64) -> Result<i64> {
            let offset = self.offset_at(epoch).ok_or_else(|| self.unknown_error())?;
            Ok(epoch + i64::from(offset))
        }

        /// Convert a local reading in this zone to the UTC instant it names.
        ///
        /// A local reading is ambiguous for one hour each year when clocks go back
        /// and non-existent for one hour when they go forward. This resolves both
        /// the way every mainstream library does: the *earlier* interpretation of
        /// an ambiguous reading, and the post-transition offset for a reading that
        /// never happened.
        ///
        /// # Errors
        ///
        /// Returns an error when this build has no rules for the zone.
        pub fn into_utc(self, local: i64) -> Result<i64> {
            // The offset depends on the instant, and the instant is what is being
            // solved for, so guess with the standard offset and correct once.
            let standard = self.standard_offset().ok_or_else(|| self.unknown_error())?;
            let guess = local - i64::from(standard);
            let offset = self.offset_at(guess).ok_or_else(|| self.unknown_error())?;
            let corrected = local - i64::from(offset);
            // Re-reading the offset at the corrected instant catches the case where
            // the first guess landed on the far side of a transition.
            let settled = self
                .offset_at(corrected)
                .ok_or_else(|| self.unknown_error())?;
            Ok(local - i64::from(settled))
        }

        /// Return every zone this build knows the rules for, sorted by name.
        ///
        /// This is the "installed" set: it needs no files, no environment, and no
        /// network, so it answers the same way on every machine the project runs
        /// on, which is the property a schema needs.
        pub fn registered() -> impl ExactSizeIterator<Item = Self> {
            crate::types::timezone::ZONES
                .iter()
                .enumerate()
                .map(|(index, _)| Self(crate::types::timezone::registered_handle(index)))
        }

        /// Return every alias and the canonical name it resolves to.
        pub fn aliases() -> impl ExactSizeIterator<Item = (&'static str, &'static str)> {
            crate::types::timezone::ALIASES.iter().copied()
        }

        /// Return a deterministic cross-language hash of the canonical name.
        pub fn stable_hash(&self) -> u64 {
            stable_hash_display(self)
        }

        /// Look this zone up in the registry.
        fn entry(&self) -> Option<&'static Zone> {
            crate::types::timezone::zone_for_handle(self.0)
        }

        /// Read a fixed offset out of the name, when the name is one.
        fn fixed_offset(&self) -> Option<i32> {
            crate::types::timezone::fixed_offset(self.0)
        }

        /// Return the saving in force at an instant, in seconds.
        fn saving_at(&self, zone: &'static Zone, epoch: i64) -> i32 {
            let Some(rule) = zone.saving else {
                return 0;
            };
            let year = year_of(epoch);
            let start = transition(rule.start, year, zone.standard, rule.save);
            let end = transition(rule.end, year, zone.standard, rule.save);

            let inside = if start <= end {
                // Northern hemisphere: the period sits inside one calendar year.
                epoch >= start && epoch < end
            } else {
                // Southern hemisphere: the period wraps the new year, so a moment
                // is inside it when it is after the spring start or before the
                // autumn end of the same year.
                epoch >= start || epoch < end
            };
            if inside { rule.save } else { 0 }
        }

        /// The error an offset question raises for a zone with no known rules.
        fn unknown_error(&self) -> Error {
            Error::Parse {
                target: "timezone",
                position: 0,
                reason: SmolStr::new(format!(
                    "this build has no rules for the time zone {}",
                    self.as_str()
                )),
            }
        }
    }

    /// Build the parse error shape this module reports.
    fn parse_error(position: usize, reason: &'static str) -> Error {
        Error::Parse {
            target: "timezone",
            position,
            reason: SmolStr::new_static(reason),
        }
    }

    /// Read a fixed-offset spelling, returning `None` when it is not one.
    ///
    /// Accepts `+HH:MM`, `+HHMM`, `+HH`, and the same with `-`, plus the bare
    /// `UTC±HH:MM` form some systems emit.
    ///
    /// # Errors
    ///
    /// Returns an error when the text looks like an offset but its hour or minute
    /// is out of range, which is a typo worth reporting rather than a name.
    fn parse_offset(value: &str) -> Result<Option<i32>> {
        let text = value
            .strip_prefix("UTC")
            .or_else(|| value.strip_prefix("utc"))
            .unwrap_or(value);
        let (sign, rest) = match text.as_bytes().first() {
            Some(b'+') => (1, &text[1..]),
            Some(b'-') => (-1, &text[1..]),
            _ => return Ok(None),
        };

        let (hours, minutes) = match rest.split_once(':') {
            Some((hours, minutes)) => (hours, minutes),
            None => match rest.len() {
                4 => rest.split_at(2),
                2 | 1 => (rest, "0"),
                _ => {
                    return Err(parse_error(
                        1,
                        "a time zone offset must be HH, HHMM, or HH:MM",
                    ));
                }
            },
        };

        let hours: i32 = hours
            .parse()
            .map_err(|_| parse_error(1, "a time zone offset hour must be a number"))?;
        let minutes: i32 = minutes
            .parse()
            .map_err(|_| parse_error(1, "a time zone offset minute must be a number"))?;
        if !(0..24).contains(&hours) {
            return Err(parse_error(1, "a time zone offset hour must be under 24"));
        }
        if !(0..60).contains(&minutes) {
            return Err(parse_error(1, "a time zone offset minute must be under 60"));
        }
        Ok(Some(sign * (hours * 3_600 + minutes * 60)))
    }

    /// Return the number of days from the Unix epoch to a civil date.
    ///
    /// This is Howard Hinnant's `days_from_civil`, which is exact for every year
    /// in range and needs no table.
    pub(crate) const fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
        let year = if month <= 2 { year - 1 } else { year } as i64;
        let era = if year >= 0 { year } else { year - 399 } / 400;
        let year_of_era = year - era * 400;
        let month = month as i64;
        let day_of_year =
            (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day as i64 - 1;
        let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
        era * 146_097 + day_of_era - 719_468
    }

    /// Return the civil date a day number falls on, as `(year, month, day)`.
    ///
    /// The exact inverse of [`days_from_civil`], from the same paper.
    pub(crate) const fn civil_from_days(days: i64) -> (i32, u32, u32) {
        let days = days + 719_468;
        let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
        let day_of_era = days - era * 146_097;
        let year_of_era =
            (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
        let year = year_of_era + era * 400;
        let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
        let month_index = (5 * day_of_year + 2) / 153;
        let day = day_of_year - (153 * month_index + 2) / 5 + 1;
        let month = if month_index < 10 {
            month_index + 3
        } else {
            month_index - 9
        };
        (
            (if month <= 2 { year + 1 } else { year }) as i32,
            month as u32,
            day as u32,
        )
    }

    /// Return the civil year a day number falls in.
    const fn year_from_days(days: i64) -> i32 {
        let days = days + 719_468;
        let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
        let day_of_era = days - era * 146_097;
        let year_of_era =
            (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
        let year = year_of_era + era * 400;
        let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
        let month = (5 * day_of_year + 2) / 153;
        (if month >= 10 { year + 1 } else { year }) as i32
    }

    /// Return the civil year an instant falls in, in UTC.
    const fn year_of(epoch: i64) -> i32 {
        year_from_days(epoch.div_euclid(DAY))
    }

    /// Return the weekday of a day number, 0 for Sunday.
    const fn weekday_from_days(days: i64) -> u32 {
        // 1970-01-01 was a Thursday, which is weekday 4.
        (days + 4).rem_euclid(7) as u32
    }

    /// Return the day number of the nth given weekday in a month.
    ///
    /// `week` counts from 1; 5 means the last such weekday, which is what every
    /// zone that says "last Sunday" needs.
    fn nth_weekday(year: i32, month: u8, week: u8, weekday: u8) -> i64 {
        let first = days_from_civil(year, u32::from(month), 1);
        let first_weekday = weekday_from_days(first);
        let shift = (u32::from(weekday) + 7 - first_weekday) % 7;
        if week >= 5 {
            // Step forward in weeks while the day is still inside this month.
            let mut day = first + i64::from(shift);
            let next_month = if month == 12 {
                days_from_civil(year + 1, 1, 1)
            } else {
                days_from_civil(year, u32::from(month) + 1, 1)
            };
            while day + 7 < next_month {
                day += 7;
            }
            return day;
        }
        first + i64::from(shift) + i64::from(week - 1) * 7
    }

    /// Return the UTC instant one edge of a saving rule happens at.
    fn transition(edge: Edge, year: i32, standard: i32, save: i32) -> i64 {
        let day = nth_weekday(year, edge.month, edge.week, edge.weekday);
        let local = day * DAY + i64::from(edge.seconds);
        match edge.basis {
            // The rule is written in UTC, so the reading is already the instant.
            Basis::Utc => local,
            // The rule is written in local standard time.
            Basis::Standard => local - i64::from(standard),
            // The rule is written in wall-clock time, which at the end of a saving
            // period still includes the saving being removed.
            Basis::Wall => local - i64::from(standard) - i64::from(save),
        }
    }

    impl FromStr for Timezone {
        type Err = Error;

        fn from_str(value: &str) -> Result<Self> {
            if value.is_empty() {
                return Err(parse_error(0, "a time zone name must not be empty"));
            }
            if let Some(position) = value.chars().position(char::is_control) {
                return Err(parse_error(
                    position,
                    "a time zone name must not hold a control character",
                ));
            }

            if value.eq_ignore_ascii_case("naive") {
                return Ok(Self::NAIVE);
            }

            // A fixed offset is canonical as `+HH:MM`, and `+00:00` is UTC itself.
            if let Some(offset) = parse_offset(value)? {
                return Self::from_offset(offset);
            }

            // A registered name wins without consulting the dynamic interner.
            if let Some(handle) = crate::types::timezone::registered(value) {
                return Ok(Self(handle));
            }
            if let Some(canonical) = crate::types::timezone::alias(value) {
                return Ok(Self(crate::types::timezone::intern(canonical)?));
            }
            if let Some(handle) = crate::types::timezone::registered_ignoring_case(value) {
                return Ok(Self(handle));
            }

            // An unregistered name is kept as written: this build has no rules for
            // it, but a schema that names it must still round-trip unchanged.
            Ok(Self(crate::types::timezone::intern(value)?))
        }
    }

    impl Timezone {
        /// Build a zone from a shared name.
        ///
        /// This is the Arrow import path. Interning canonicalizes it through the
        /// same path as every other spelling; the process registry then owns the
        /// one retained copy.
        ///
        /// # Errors
        ///
        /// Returns an error for an empty name or one holding a control character.
        pub fn from_smol_str(value: SmolStr) -> Result<Self> {
            Self::from_str(value.as_str())
        }
    }

    impl fmt::Debug for Timezone {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter
                .debug_tuple("Timezone")
                .field(&self.as_str())
                .finish()
        }
    }

    impl PartialOrd for Timezone {
        fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
            Some(self.cmp(other))
        }
    }

    impl Ord for Timezone {
        fn cmp(&self, other: &Self) -> Ordering {
            self.as_str().cmp(other.as_str())
        }
    }

    impl AsRef<str> for Timezone {
        fn as_ref(&self) -> &str {
            self.as_str()
        }
    }

    impl fmt::Display for Timezone {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str(self.as_str())
        }
    }

    impl Serialize for Timezone {
        fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            serializer.serialize_str(self.as_str())
        }
    }

    impl<'de> Deserialize<'de> for Timezone {
        fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
        where
            D: Deserializer<'de>,
        {
            let value = SmolStr::deserialize(deserializer)?;
            Self::from_smol_str(value).map_err(D::Error::custom)
        }
    }

    impl From<Timezone> for SmolStr {
        fn from(value: Timezone) -> Self {
            value.into_smol_str()
        }
    }

    text_scalar_value!(Timezone, Timezone, DataTypeId::Timezone, DataType::Timezone);
}

/// The Arrow extension name preserving [`crate::DataType::Timezone`] over its
/// Utf8 storage.
pub(crate) const TIMEZONE_EXTENSION_NAME: &str = "yggdryl.timezone";

#[cfg(test)]
/// Time zone canonicalization, offsets, and daylight-saving transitions.
///
/// The offset expectations here are real answers checked against the IANA
/// database, not against this implementation - a test that only agrees with
/// the code it tests would pass on a wrong table.
mod tests {
    use super::{Timezone};

    /// Parse a zone or fail the test with the reason.
    fn zone(value: &str) -> Timezone {
        Timezone::from_str(value).expect("a valid time zone")
    }

    #[test]
    fn a_timezone_is_one_copyable_four_byte_handle() {
        fn assert_copy<T: Copy>() {}

        assert_copy::<Timezone>();
        assert_eq!(std::mem::size_of::<Timezone>(), 4);
    }

    #[test]
    fn naive_is_a_canonical_non_zoned_marker() {
        assert_eq!(zone("naive"), Timezone::NAIVE);
        assert!(Timezone::NAIVE.is_naive());
        assert_eq!(Timezone::NAIVE.as_str(), "NAIVE");
        assert_eq!(Timezone::NAIVE.offset_at(0), None);
    }

    /// Seconds since the Unix epoch for a UTC civil date and time.
    fn utc(year: i32, month: u32, day: u32, hour: i64, minute: i64) -> i64 {
        super::days_from_civil(year, month, day) * 86_400 + hour * 3_600 + minute * 60
    }

    mod the_registry {

        #[test]
        fn zones_are_sorted_so_the_binary_search_is_valid() {
            let names: Vec<&str> = crate::types::timezone::ZONES.iter().map(|zone| zone.name).collect();
            let mut sorted = names.clone();
            sorted.sort_unstable();

            assert_eq!(names, sorted, "the zone table must be sorted by name");
        }

        #[test]
        fn aliases_are_sorted_and_never_shadow_a_real_zone() {
            let names: Vec<&str> = crate::types::timezone::ALIASES.iter().map(|(from, _)| *from).collect();
            let mut sorted = names.clone();
            sorted.sort_unstable();
            assert_eq!(names, sorted, "the alias table must be sorted by alias");

            for (from, _) in crate::types::timezone::ALIASES {
                assert!(
                    crate::types::timezone::zone(from).is_none(),
                    "{from} is both a zone and an alias"
                );
            }
        }

        #[test]
        fn every_offset_is_a_whole_number_of_minutes_within_a_day() {
            for zone in crate::types::timezone::ZONES {
                assert_eq!(
                    zone.standard % 60,
                    0,
                    "{} has a sub-minute offset",
                    zone.name
                );
                assert!(
                    zone.standard.abs() < 24 * 3_600,
                    "{} is more than a day from UTC",
                    zone.name
                );
                if let Some(saving) = zone.saving {
                    assert!(
                        saving.save > 0 && saving.save <= 2 * 3_600,
                        "{} saves an implausible amount",
                        zone.name
                    );
                    assert!(
                        zone.daylight_abbreviation.is_some(),
                        "{} observes saving but has no daylight abbreviation",
                        zone.name
                    );
                }
            }
        }

        #[test]
        fn a_zone_that_observes_no_saving_declares_no_daylight_abbreviation() {
            for zone in crate::types::timezone::ZONES {
                if zone.saving.is_none() {
                    assert!(
                        zone.daylight_abbreviation.is_none(),
                        "{} has a daylight abbreviation but no rule",
                        zone.name
                    );
                }
            }
        }
    }

    mod canonicalization {
        use std::hash::{DefaultHasher, Hash, Hasher};

        use super::{Timezone, zone};

        fn hash(value: Timezone) -> u64 {
            let mut state = DefaultHasher::new();
            value.hash(&mut state);
            state.finish()
        }

        #[test]
        fn an_alias_resolves_to_what_it_stands_for() {
            assert_eq!(zone("Asia/Calcutta"), zone("Asia/Kolkata"));
            assert_eq!(zone("US/Eastern"), zone("America/New_York"));
            assert_eq!(zone("Europe/Kiev"), zone("Europe/Kyiv"));
            assert_eq!(zone("Japan").as_str(), "Asia/Tokyo");
        }

        #[test]
        fn every_alias_interns_to_its_canonical_handle() {
            for &(alias, canonical) in crate::types::timezone::ALIASES {
                let alias = zone(alias);
                let canonical = zone(canonical);

                assert_eq!(alias, canonical);
                assert_eq!(hash(alias), hash(canonical));
                assert!(std::ptr::eq(alias.as_smol_str(), canonical.as_smol_str()));
            }
        }

        #[test]
        fn every_spelling_of_utc_is_utc() {
            for value in [
                "UTC",
                "utc",
                "Z",
                "Zulu",
                "GMT",
                "Etc/UTC",
                "Universal",
                "+00:00",
                "-00:00",
            ] {
                assert_eq!(zone(value), Timezone::UTC, "{value} should be UTC");
                assert!(zone(value).is_utc());
            }
        }

        #[test]
        fn case_is_normalized_for_a_registered_name() {
            assert_eq!(zone("america/new_york").as_str(), "America/New_York");
            assert_eq!(zone("EUROPE/PARIS").as_str(), "Europe/Paris");
        }

        #[test]
        fn a_fixed_offset_normalizes_to_one_spelling() {
            for value in ["+05:30", "+0530", "UTC+05:30"] {
                assert_eq!(zone(value).as_str(), "+05:30", "{value} should normalize");
            }
            assert_eq!(zone("-08").as_str(), "-08:00");
            assert_eq!(zone("-0800").as_str(), "-08:00");
        }

        #[test]
        fn every_fixed_offset_round_trips_through_its_reserved_handle() {
            for minutes in -(24 * 60 - 1)..=(24 * 60 - 1) {
                let from_count = Timezone::from_offset(minutes * 60).unwrap();
                let from_name = zone(from_count.as_str());

                assert_eq!(from_count, from_name, "{minutes}");
            }
        }

        #[test]
        fn an_unregistered_name_is_kept_exactly_as_written() {
            // A schema naming a zone this build has no rules for must still round
            // trip unchanged, or importing a foreign schema would corrupt it.
            let custom = zone("Custom/Accepted");

            assert_eq!(custom.as_str(), "Custom/Accepted");
            assert!(!custom.is_known());
            assert_eq!(custom.offset_at(0), None);
        }

        #[test]
        fn an_unregistered_name_is_retained_once_for_the_process() {
            let first = zone("Custom/Interned");
            let second = zone("Custom/Interned");

            assert_eq!(first, second);
            assert!(std::ptr::eq(first.as_smol_str(), second.as_smol_str()));
        }

        #[test]
        fn an_impossible_name_is_refused() {
            assert!(Timezone::from_str("").is_err());
            assert!(Timezone::from_str("Europe/\u{7}Paris").is_err());
            // A spelling that looks like an offset but is not one is a typo.
            assert!(Timezone::from_str("+25:00").is_err());
            assert!(Timezone::from_str("+05:75").is_err());
        }

        #[test]
        fn a_fixed_offset_can_be_built_from_seconds() {
            assert_eq!(Timezone::from_offset(0).unwrap(), Timezone::UTC);
            assert_eq!(Timezone::from_offset(19_800).unwrap().as_str(), "+05:30");
            assert_eq!(Timezone::from_offset(-28_800).unwrap().as_str(), "-08:00");

            assert!(Timezone::from_offset(24 * 3_600).is_err());
            assert!(Timezone::from_offset(90).is_err());
        }

        #[test]
        fn the_canonical_name_survives_serde() {
            let value = zone("US/Pacific");
            let text = serde_json::to_string(&value).unwrap();

            assert_eq!(text, "\"America/Los_Angeles\"");
            assert_eq!(
                serde_json::from_str::<Timezone>("\"Asia/Calcutta\"").unwrap(),
                zone("Asia/Kolkata")
            );
        }
    }

    mod offsets {
        use super::{utc, zone};

        #[test]
        fn a_northern_zone_switches_on_its_own_rule() {
            let new_york = zone("America/New_York");

            // The United States switches on the second Sunday in March at 02:00
            // local standard, which in 2024 is the 10th, 07:00 UTC.
            assert_eq!(
                new_york.offset_at(utc(2024, 3, 10, 6, 59)),
                Some(-5 * 3_600)
            );
            assert_eq!(new_york.offset_at(utc(2024, 3, 10, 7, 0)), Some(-4 * 3_600));

            // ... and back on the first Sunday in November, the 3rd, 06:00 UTC.
            assert_eq!(
                new_york.offset_at(utc(2024, 11, 3, 5, 59)),
                Some(-4 * 3_600)
            );
            assert_eq!(new_york.offset_at(utc(2024, 11, 3, 6, 0)), Some(-5 * 3_600));
        }

        #[test]
        fn the_european_rule_switches_the_whole_union_at_one_instant() {
            let paris = zone("Europe/Paris");
            let helsinki = zone("Europe/Helsinki");
            let london = zone("Europe/London");

            // Last Sunday in March 2024 is the 31st, at 01:00 UTC exactly.
            let before = utc(2024, 3, 31, 0, 59);
            let after = utc(2024, 3, 31, 1, 0);

            assert_eq!(paris.offset_at(before), Some(3_600));
            assert_eq!(paris.offset_at(after), Some(2 * 3_600));
            assert_eq!(helsinki.offset_at(before), Some(2 * 3_600));
            assert_eq!(helsinki.offset_at(after), Some(3 * 3_600));
            assert_eq!(london.offset_at(before), Some(0));
            assert_eq!(london.offset_at(after), Some(3_600));
        }

        #[test]
        fn a_southern_zone_is_saving_across_the_new_year() {
            let sydney = zone("Australia/Sydney");

            // January is inside the saving period that began the previous October.
            assert_eq!(sydney.offset_at(utc(2024, 1, 15, 0, 0)), Some(11 * 3_600));
            // July is outside it.
            assert_eq!(sydney.offset_at(utc(2024, 7, 15, 0, 0)), Some(10 * 3_600));

            // First Sunday in April 2024 is the 7th, 02:00 local standard = 16:00
            // UTC on the 6th.
            assert_eq!(sydney.offset_at(utc(2024, 4, 6, 15, 59)), Some(11 * 3_600));
            assert_eq!(sydney.offset_at(utc(2024, 4, 6, 16, 0)), Some(10 * 3_600));
        }

        #[test]
        fn new_zealand_starts_on_the_last_sunday_in_september() {
            let auckland = zone("Pacific/Auckland");

            // 2024's last Sunday in September is the 29th, 02:00 local standard
            // (+12) = 14:00 UTC on the 28th.
            assert_eq!(
                auckland.offset_at(utc(2024, 9, 28, 13, 59)),
                Some(12 * 3_600)
            );
            assert_eq!(
                auckland.offset_at(utc(2024, 9, 28, 14, 0)),
                Some(13 * 3_600)
            );
        }

        #[test]
        fn a_zone_without_saving_answers_the_same_all_year() {
            for name in ["Asia/Tokyo", "Asia/Kolkata", "America/Phoenix", "UTC"] {
                let value = zone(name);
                let winter = value.offset_at(utc(2024, 1, 15, 12, 0));
                let summer = value.offset_at(utc(2024, 7, 15, 12, 0));

                assert_eq!(winter, summer, "{name} should not move");
                assert_eq!(value.is_saving_at(utc(2024, 7, 15, 12, 0)), Some(false));
                assert!(!value.observes_saving());
            }
            assert_eq!(zone("Asia/Kolkata").offset_at(0), Some(5 * 3_600 + 1_800));
            assert_eq!(zone("Asia/Kathmandu").offset_at(0), Some(5 * 3_600 + 2_700));
        }

        #[test]
        fn abbreviations_follow_the_saving_state() {
            let new_york = zone("America/New_York");
            assert_eq!(
                new_york.abbreviation_at(utc(2024, 1, 15, 12, 0)),
                Some("EST")
            );
            assert_eq!(
                new_york.abbreviation_at(utc(2024, 7, 15, 12, 0)),
                Some("EDT")
            );

            let berlin = zone("Europe/Berlin");
            assert_eq!(berlin.abbreviation_at(utc(2024, 1, 15, 12, 0)), Some("CET"));
            assert_eq!(
                berlin.abbreviation_at(utc(2024, 7, 15, 12, 0)),
                Some("CEST")
            );

            assert_eq!(zone("Custom/Unknown").abbreviation_at(0), None);
        }

        #[test]
        fn a_fixed_offset_never_observes_saving() {
            let value = zone("+05:30");

            assert_eq!(value.offset_at(utc(2024, 1, 1, 0, 0)), Some(19_800));
            assert_eq!(value.offset_at(utc(2024, 7, 1, 0, 0)), Some(19_800));
            assert_eq!(value.is_saving_at(0), Some(false));
            assert!(value.is_fixed());
            assert!(value.is_known());
        }

        #[test]
        fn the_standard_offset_ignores_saving_entirely() {
            assert_eq!(zone("America/New_York").standard_offset(), Some(-5 * 3_600));
            assert_eq!(zone("Europe/Paris").standard_offset(), Some(3_600));
            assert_eq!(zone("Custom/Unknown").standard_offset(), None);
        }

        #[test]
        fn transitions_land_on_a_sunday_for_every_rule_and_year() {
            // The nth-weekday arithmetic is the part most likely to be subtly
            // wrong, so check it holds across a span of years including leap ones.
            for year in 2020..2036 {
                for name in ["America/New_York", "Europe/Paris", "Australia/Sydney"] {
                    let value = zone(name);
                    let january = value.offset_at(utc(year, 1, 15, 12, 0)).unwrap();
                    let july = value.offset_at(utc(year, 7, 15, 12, 0)).unwrap();

                    assert_ne!(january, july, "{name} in {year} should move once a year");
                }
            }
        }
    }

    mod conversions {
        use super::{utc, zone};

        #[test]
        fn a_utc_instant_becomes_a_local_reading() {
            let new_york = zone("America/New_York");
            let instant = utc(2024, 7, 4, 16, 0);

            // 16:00 UTC in July is 12:00 in New York, which is EDT.
            assert_eq!(
                new_york.into_local(instant).unwrap(),
                utc(2024, 7, 4, 12, 0)
            );
            // In January the same reading is one hour further back.
            let winter = utc(2024, 1, 4, 16, 0);
            assert_eq!(new_york.into_local(winter).unwrap(), utc(2024, 1, 4, 11, 0));
        }

        #[test]
        fn a_local_reading_becomes_the_instant_it_names() {
            let paris = zone("Europe/Paris");
            let local = utc(2024, 7, 4, 14, 0);

            // 14:00 in Paris in July is 12:00 UTC.
            assert_eq!(paris.into_utc(local).unwrap(), utc(2024, 7, 4, 12, 0));
            // A round trip through both directions is the identity.
            let instant = utc(2024, 7, 4, 12, 0);
            assert_eq!(
                paris.into_utc(paris.into_local(instant).unwrap()).unwrap(),
                instant
            );
        }

        #[test]
        fn a_conversion_refuses_a_zone_it_has_no_rules_for() {
            let unknown = zone("Custom/Unknown");

            let message = unknown.into_local(0).unwrap_err().to_string();
            assert!(message.contains("Custom/Unknown"), "{message}");
            assert!(unknown.into_utc(0).is_err());
        }

        #[test]
        fn the_registry_is_installed_without_any_environment() {
            let registered: Vec<String> = super::Timezone::registered()
                .map(|zone| zone.as_str().to_owned())
                .collect();

            assert!(registered.len() > 60, "{}", registered.len());
            assert!(registered.iter().any(|name| name == "UTC"));
            assert!(registered.iter().any(|name| name == "Europe/Paris"));
            // Every registered name must itself parse back to the same zone.
            for name in &registered {
                assert_eq!(zone(name).as_str(), name);
            }
            assert!(super::Timezone::aliases().len() > 20);
        }
    }
}

// ------------------------------------------------------------------------
// Arrow projection: the canonical text, under this family's extension name.
// ------------------------------------------------------------------------

impl TimezoneType {
    /// The Arrow storage a time zone column lays out: its canonical text.
    pub(crate) const fn arrow_storage() -> arrow_schema::DataType {
        arrow_schema::DataType::Utf8
    }
}
