//! The registry of time zones this build knows the rules for.
//!
//! This is a *rules* table, not a history. Each entry carries the standard
//! offset and the daylight-saving rule **in force today**, which is what a
//! schema, a partition value, or a freshly written batch actually needs. It is
//! deliberately not a replacement for the IANA database: applying today's rule
//! to a 1975 instant would answer confidently and wrongly.
//!
//! That is also why the table is short. A zone whose real rule is historical,
//! irregular, or politically volatile - Israel, Iran, Egypt, Lord Howe - is
//! *left out* rather than approximated, so it parses as an ordinary named zone
//! and reports its offset as unknown. Refusing to answer is recoverable; a
//! plausible wrong answer is not.

use std::collections::HashMap;
use std::num::NonZeroU32;
use std::sync::{OnceLock, RwLock};

use smol_str::SmolStr;

use crate::{Error, Result};

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
