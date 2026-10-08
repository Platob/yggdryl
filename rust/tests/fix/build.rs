//! `rust/src/fix/build.rs`: the one dispatch from a FIX value's text to its
//! field's own reader - the type's FIX door where FIX spells a value its own
//! way, the type's text reader otherwise, a code-set word by the set under
//! the crate's fold - and the typed value the row then holds, rendered in
//! the type's ISO spelling while the entries keep the wire.

use yggdryl::holder::Buffer;
use yggdryl::text::{TextOptions, read_text_lines};
use yggdryl::{
    DataType, DateTime64, FixCodec, FixMsg, Scalar, Time64, TimeUnit, Timezone, Url, Value,
    fix_schema, into_json_scalar,
};

use super::SoleMessage;

/// The committed dictionary under a codec that refuses no message type, on
/// one thread.
fn reader() -> FixCodec {
    super::fixed_codec(super::committed_registry()).with_exclude_msgtypes::<[&str; 0], &str>([])
}

/// One FIX 5.0 SP2 frame - FIXT.1.1 with `ApplVerID(1128)=9`, because
/// `LegMaturityTime(1212)` and `AggressorIndicator(1057)` are 5.0 fields and
/// a 4.4 frame does not know them - stating `tag=value` once.
fn latest(reader: &FixCodec, tag: i32, value: &str) -> FixMsg {
    reader
        .sole_line(format!("8=FIXT.1.1|1128=9|35=D|{tag}={value}|10=0|").as_bytes())
        .expect("a readable message")
}

/// The value one such frame types under `tag`.
fn typed(reader: &FixCodec, tag: i32, value: &str) -> Scalar {
    latest(reader, tag, value)
        .by_tag(tag)
        .expect("the tag is held")
}

fn instant(count: i64) -> Scalar {
    Scalar::datetime64(count, TimeUnit::Nanosecond, Timezone::UTC).expect("a nanosecond count")
}

/// The anomalies one message carries, as `(field, reason)` pairs.
fn anomalies(message: &FixMsg) -> Vec<(&str, &str)> {
    message
        .anomalies()
        .iter()
        .map(|anomaly| (anomaly.field(), anomaly.reason()))
        .collect()
}

/// 2026-08-14T14:52:55Z, as nanoseconds since the epoch.
const TRANSACT: i64 = 1_786_719_175_000_000_000;
/// 2026-09-30T00:00:00, a wall clock, as nanoseconds since the epoch day.
const MATURITY_DATE: i64 = 1_790_726_400_000_000_000;
/// 09:30:00 into a day, as nanoseconds.
const HALF_PAST_NINE: i64 = 34_200_000_000_000;

/// The values the bridge capture states that the dispatch used to refuse,
/// each typed by its field's own reader: a `TZTimeOnly` stating no zone is
/// the clock on the epoch day in the column's zone, a `UTCTimeOnly` is the
/// time of day, a `LocalMktDate` is that day's midnight as a wall clock, a
/// `UTCTimestamp` the instant, and a bridge's `Aggressor` is the code the
/// dictionary aliases it to.
#[test]
fn every_capture_value_the_dispatch_refused_types_through_its_fields_own_reader() {
    let reader = reader();
    // `LegMaturityTime(1212)` is a `TZTimeOnly`: `093000` is the compact
    // clock a bridge writes, read on the epoch day as a wall clock in the
    // column's zone, UTC - a value, where the datetime door throws a
    // dateless zoneless clock away.
    assert_eq!(typed(&reader, 1212, "093000"), instant(HALF_PAST_NINE));
    assert_eq!(typed(&reader, 1212, "09:30"), instant(HALF_PAST_NINE));
    assert_eq!(
        typed(&reader, 1212, "09:30:00.123+05:30"),
        instant(HALF_PAST_NINE + 123_000_000 - 19_800_000_000_000)
    );
    // A dated value is no `TZTimeOnly`, and the anomaly says so by the byte.
    let dated = latest(&reader, 1212, "20060901-07:39Z");
    assert!(dated.by_tag(1212).unwrap().is_null());
    assert_eq!(anomalies(&dated).len(), 1, "{:?}", anomalies(&dated));
    assert_eq!(anomalies(&dated)[0].0, "legmaturitytime");

    // `AggressorIndicator(1057)`: the bridge's `Aggressor` and `Passive` are
    // the two members' one meaning each, written into the set as aliases.
    assert_eq!(typed(&reader, 1057, "Aggressor"), Scalar::from(true));
    assert_eq!(typed(&reader, 1057, "Passive"), Scalar::from(false));
    assert_eq!(typed(&reader, 1057, "Y"), Scalar::from(true));

    // `TransactTime(60)`, a `UTCTimestamp`, states no zone and is UTC by
    // saying nothing; `LegMaturityDate(611)`, a `LocalMktDate`, is a wall
    // clock with no zone at all.
    assert_eq!(typed(&reader, 60, "20260814-14:52:55"), instant(TRANSACT));
    assert_eq!(
        typed(&reader, 611, "20260930"),
        Scalar::datetime64(MATURITY_DATE, TimeUnit::Nanosecond, Timezone::NAIVE).unwrap()
    );
    // A `LocalMktDate` sent with a zone is no local market value: refused by
    // name, never read as the instant it is not.
    let zoned = latest(&reader, 611, "20260930+02:00");
    assert!(zoned.by_tag(611).unwrap().is_null());
    let [(field, reason)] = anomalies(&zoned)[..] else {
        panic!("one anomaly: {:?}", anomalies(&zoned));
    };
    assert_eq!(field, "legmaturitydate");
    assert!(
        reason.contains("stating no zone") && reason.ends_with(", got \"20260930+02:00\""),
        "{reason}"
    );

    // `MDEntryTime(273)`, a `UTCTimeOnly` on `time64(ns)`: the compact clock
    // and the one that stops at its minutes are the time of day, restated
    // at the column's unit.
    let half_past_nine =
        Scalar::time64(HALF_PAST_NINE, TimeUnit::Nanosecond, Timezone::NAIVE).unwrap();
    assert_eq!(typed(&reader, 273, "093000"), half_past_nine);
    assert_eq!(typed(&reader, 273, "09:30"), half_past_nine);
    assert_eq!(typed(&reader, 273, "093000123"), {
        Scalar::time64(
            HALF_PAST_NINE + 123_000_000,
            TimeUnit::Nanosecond,
            Timezone::NAIVE,
        )
        .unwrap()
    });
    // A zone on a time of day names the type that holds one.
    let zoned_clock = latest(&reader, 273, "093000Z");
    assert!(zoned_clock.by_tag(273).unwrap().is_null());
    assert!(
        anomalies(&zoned_clock)[0].1.contains("DateTime64"),
        "{:?}",
        anomalies(&zoned_clock)
    );

    // `PartySubIDType(803)` reads by a code set: a word the set does not
    // spell and the type does not read refuses naming the set, not the
    // integer parser.
    let worded = latest(&reader, 803, "TraderName");
    assert!(worded.by_tag(803).unwrap().is_null());
    let [(field, reason)] = anomalies(&worded)[..] else {
        panic!("one anomaly: {:?}", anomalies(&worded));
    };
    assert_eq!(field, "partysubidtype");
    assert!(
        reason.ends_with(
            "no code of partysubidtypecodeset is spelled \"TraderName\", got \"TraderName\""
        ),
        "{reason}"
    );
    assert!(!reason.contains("int32"), "{reason}");
    // A code the set spells, by its name under the crate's fold, is its value.
    assert_eq!(typed(&reader, 803, "contact_name"), Scalar::from(9_i32));
    assert_eq!(typed(&reader, 803, "9"), Scalar::from(9_i32));

    // `TransactTime(60)` is a `UTCTimestamp`, no `TZTimeOnly`: an event clock
    // with no date and no zone is still no instant.
    let unstated = reader
        .parse_fix_line(b"8=FIX.4.4|35=D|60=10:15:30.000|10=0|")
        .expect("a message");
    assert!(unstated.by_tag(60).map_or(true, |held| held.is_null()));
    assert_eq!(anomalies(&unstated)[0].0, "transacttime");
}

/// The dictionary states every field's FIX datatype as the specification
/// spells it, and the crate's own fields state the family's.
#[test]
fn every_field_states_the_fix_datatype_it_was_declared_under() {
    let registry = super::committed_registry();
    let declared = |tag: i32| {
        registry
            .field_by_tag(tag)
            .expect("a dictionary field")
            .as_fix()
            .datatype()
            .map(str::to_owned)
    };
    assert_eq!(declared(1212).as_deref(), Some("TZTimeOnly"));
    assert_eq!(declared(1079).as_deref(), Some("TZTimeOnly"));
    assert_eq!(declared(60).as_deref(), Some("UTCTimestamp"));
    assert_eq!(declared(273).as_deref(), Some("UTCTimeOnly"));
    assert_eq!(declared(611).as_deref(), Some("LocalMktDate"));
    assert_eq!(declared(38).as_deref(), Some("Qty"));
    // A field typed by a code set states the set's base type.
    assert_eq!(declared(54).as_deref(), Some("char"));
    assert_eq!(declared(1057).as_deref(), Some("Boolean"));
    assert_eq!(declared(803).as_deref(), Some("int"));
    // The crate's own registered fields state their family's: a clock is
    // an instant, a count an int, a code its FIX datatype where FIX has
    // one and a string otherwise.
    assert_eq!(
        declared(yggdryl::CURRUNIX_TAG_NAME.0).as_deref(),
        Some("UTCTimestamp")
    );
    assert_eq!(declared(yggdryl::SEQNUM_TAG_NAME.0).as_deref(), Some("int"));
    assert_eq!(
        declared(yggdryl::MICCODE_TAG_NAME.0).as_deref(),
        Some("Exchange")
    );
    assert_eq!(
        declared(yggdryl::ORIGCCY_TAG_NAME.0).as_deref(),
        Some("Currency")
    );
    assert_eq!(
        declared(yggdryl::CROSSCODE_TAG_NAME.0).as_deref(),
        Some("String")
    );
    // Every field the generator wrote states one.
    let unstated: Vec<&str> = registry
        .iter()
        .filter(|field| {
            field
                .as_fix()
                .tag()
                .ok()
                .flatten()
                .is_some_and(|tag| tag < 65_000)
                && !matches!(
                    field.dtype(),
                    DataType::Struct(_) | DataType::Serie(_) | DataType::LargeSerie(_)
                )
                && field.as_fix().datatype().is_none()
        })
        .map(|field| field.name())
        .collect();
    assert!(unstated.is_empty(), "{unstated:?}");
    // The property is one word of the specification's, set and read on any
    // field; an empty name removes it.
    let mut field = DataType::utf8().nullable_field("tenor");
    field.as_fix_mut().set_datatype("Tenor").unwrap();
    assert_eq!(field.as_fix().datatype(), Some("Tenor"));
    assert!(field.as_fix_mut().set_datatype("Month Year").is_err());
    assert_eq!(field.as_fix().datatype(), Some("Tenor"));
    field.as_fix_mut().set_datatype("").unwrap();
    assert_eq!(field.as_fix().datatype(), None);
}

/// A FIX value the type's reader took is the typed scalar in the row, and
/// every text rendering of it is the type's ISO 8601 spelling through the one
/// renderer - short, no fraction where the fraction is zero - while the
/// message's own entries, and the residual record built from them, spell
/// FIX's own wire for the value: `YYYYMMDD-HH:MM:SS` for an instant,
/// `HH:MM:SS` for a time of day, and for a `TZTimeOnly` the clock it is -
/// its UTC time of day closed by `Z` - which its door reads back.
#[test]
fn a_typed_fix_value_renders_in_its_types_iso_spelling_while_the_entries_keep_the_wire() {
    let reader = reader();
    let message = reader
        .sole_line(
            b"8=FIXT.1.1|1128=9|35=D|60=20260814-14:52:55|611=20260930|1212=093000|273=093000|10=0|",
        )
        .expect("a readable message");
    let text = |value: &Scalar| {
        DataType::utf8()
            .scalar(value.clone())
            .expect("a temporal casts to text")
            .as_str()
            .expect("text")
            .to_owned()
    };

    let clock = message.by_tag(1212).unwrap();
    assert_eq!(clock, instant(HALF_PAST_NINE));
    assert_eq!(text(&clock), "1970-01-01T09:30:00Z");
    assert_eq!(
        into_json_scalar(&clock).unwrap(),
        "\"1970-01-01T09:30:00Z\""
    );
    assert_eq!(
        DateTime64::from_scalar(&clock).unwrap().to_string(),
        "1970-01-01T09:30:00Z"
    );

    let time = message.by_tag(273).unwrap();
    assert_eq!(text(&time), "09:30:00");
    assert_eq!(Time64::from_scalar(&time).unwrap().to_string(), "09:30:00");
    assert_eq!(into_json_scalar(&time).unwrap(), "\"09:30:00\"");

    let day = message.by_tag(611).unwrap();
    assert_eq!(text(&day), "2026-09-30T00:00:00");
    assert_eq!(
        DateTime64::from_scalar(&day).unwrap().to_string(),
        "2026-09-30T00:00:00"
    );

    let transact = message.by_tag(60).unwrap();
    assert_eq!(text(&transact), "2026-08-14T14:52:55Z");
    assert_eq!(
        into_json_scalar(&transact).unwrap(),
        "\"2026-08-14T14:52:55Z\""
    );

    // The wire the message re-emits is FIX's own spelling of each typed
    // value - the instant with its date and the shortest exact fraction, the
    // time of day as its clock - never the ISO text the row renders.
    let wire = message.into_text('|').unwrap();
    for pair in ["60=20260814-14:52:55", "273=09:30:00", "1212=09:30:00Z"] {
        assert!(wire.contains(pair), "{pair} in {wire}");
    }
    assert!(!wire.contains("2026-08-14T"), "{wire}");
    // A `TZTimeOnly` goes out undated, as FIX spells one, so the clock door
    // that refuses a dated value reads it back to the same instant.
    let again = reader
        .sole_line(wire.as_bytes())
        .expect("the wire it wrote reads");
    assert_eq!(again.by_tag(1212).unwrap(), instant(HALF_PAST_NINE));
    // The residual record files the entries' wire text, not the row's.
    let schema = fix_schema(reader.registry(), "fix").unwrap();
    let at = schema.index_of("fixentries").unwrap();
    let row = message.into_row(&schema).unwrap();
    let residual = &row.as_sequence().unwrap()[at];
    assert_eq!(
        residual
            .get_key_str("273:mdentrytime")
            .and_then(Scalar::as_str),
        Some("09:30:00")
    );
}

/// The capture through the committed dictionary: every value whose field's
/// type refuses it is one of the bridge's own words under a code set, and
/// the refusal names the set. `legmaturitytime` and `aggressorindicator`,
/// the two the dispatch refused by shape and by vocabulary, type.
#[test]
fn the_captures_only_refused_values_are_code_set_words_the_bridge_alone_spells() {
    use std::collections::BTreeMap;

    let source = Buffer::from_bytes(include_bytes!("ulbridge.log").to_vec()).with_media_type(
        Url::from_str("file:///ulbridge.log")
            .expect("a URL")
            .media_type(),
    );
    let mut options = TextOptions::new()
        .try_with_rowheader(yggdryl::ULBRIDGE_ROWHEADER)
        .expect("the bridge's row header compiles")
        .with_timezone(Timezone::UTC);
    options.start_rownum = Some(1);
    let codec = reader().with_capture_names(options.capture_names());
    let lines: Vec<_> = read_text_lines(&source, &options)
        .expect("a line reader")
        .map(|line| line.expect("a line"))
        .collect();
    let messages: Vec<FixMsg> = codec
        .parse_text_lines(lines.iter())
        .filter_map(Result::ok)
        .collect();
    assert!(messages.len() > 100, "{} messages", messages.len());

    // A type refusal's reason ends with what arrived, `, got "<text>"`; an
    // identifier restatement's names what it replaced.
    let mut refused: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for message in &messages {
        for anomaly in message.anomalies() {
            if anomaly.reason().contains(", got \"") {
                refused
                    .entry(anomaly.field())
                    .or_default()
                    .push(anomaly.reason());
            }
        }
    }
    let fields: Vec<&str> = refused.keys().copied().collect();
    assert_eq!(
        fields,
        [
            "partyrole",
            "partyrolequalifier",
            "partysubidtype",
            "pricetype",
            "trdregtimestamptype",
        ],
        "{refused:#?}"
    );
    for (field, reasons) in &refused {
        for reason in reasons {
            assert!(
                reason.contains(&format!(": no code of {field}"))
                    || reason.contains(": no code of partydetailrolequalifiercodeset"),
                "{field}: {reason}"
            );
            assert!(reason.contains("codeset is spelled "), "{field}: {reason}");
        }
    }
    assert!(
        refused["partysubidtype"]
            .iter()
            .any(|reason| reason.contains("\"TraderName\"")),
        "{:?}",
        refused["partysubidtype"]
    );
}
