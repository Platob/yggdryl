//! One order's life, code identity and the settled nanosecond clocks it carries.

use super::SoleMessage;

use std::sync::Arc;

use yggdryl::types::Uuid;
use yggdryl::{
    CODE_TAG_NAME, DataType, Error, FixLifecycle, FixMsg, FixRegistry, INSTUUID_TAG_NAME,
    PREVTIMESTAMP_TAG_NAME, PREVUUID_TAG_NAME, PUUID_TAG_NAME, SNAPSHOTAT_TAG_NAME, Scalar,
    TimeUnit, Timezone, UPDATEDAT_TAG_NAME, UUID_TAG_NAME,
};

fn registry() -> Arc<FixRegistry> {
    super::committed_registry()
}

/// The bytes one identity column holds.
fn bytes(message: &FixMsg, tag: i32) -> Option<[u8; 16]> {
    let held = message.get_by_tag(tag).filter(|held| !held.is_null())?;
    let Scalar::Uuid(uuid) = held else {
        panic!("tag {tag} must hold a native UUID, got {held:?}");
    };
    let bytes = uuid.into_bytes();
    assert_eq!(bytes[6] >> 4, 8);
    assert_eq!(bytes[8] >> 6, 2);
    Some(bytes)
}

/// The messages of one order's life, as a venue and its client tell it.
const LIFE: [&[u8]; 6] = [
    // The order, sent under the client's own identifier.
    b"8=FIX.4.4|35=D|11=A1|55=AAPL|207=XNAS|15=USD|54=1|38=100|44=12.5|60=20260102-10:15:30.000|10=0|",
    // Acknowledged under the venue's, which now names the same chain.
    b"8=FIX.4.4|35=8|11=A1|37=O1|17=E1|150=0|39=0|55=AAPL|207=XNAS|15=USD|38=100|14=0|151=100|60=20260102-10:15:30.250|10=0|",
    // Half of it done.
    b"8=FIX.4.4|35=8|37=O1|17=E2|150=F|39=1|55=AAPL|207=XNAS|15=USD|38=100|14=50|151=50|32=50|31=12.5|60=20260102-10:15:31.000|10=0|",
    // Replaced: the new client identifier names the old one, and joins.
    b"8=FIX.4.4|35=G|41=A1|11=A2|55=AAPL|207=XNAS|15=USD|54=1|38=120|44=12.6|60=20260102-10:15:32.000|10=0|",
    b"8=FIX.4.4|35=8|41=A1|11=A2|37=O1|17=E3|150=5|39=5|55=AAPL|207=XNAS|15=USD|38=120|14=50|151=70|60=20260102-10:15:32.100|10=0|",
    // Filled under the new identifier alone: the chain ends here.
    b"8=FIX.4.4|35=8|11=A2|17=E4|150=F|39=2|55=AAPL|207=XNAS|15=USD|38=120|14=120|151=0|32=70|31=12.6|60=20260102-10:15:33.000|10=0|",
];

#[test]
fn every_message_of_one_order_carries_the_chains_identity_until_it_ends() {
    let registry = registry();
    let reader = super::fixed_codec(Arc::clone(&registry));
    let mut life = FixLifecycle::new(Arc::clone(&registry));
    let mut stamped = Vec::with_capacity(LIFE.len());
    for line in LIFE {
        let message = reader.sole_line(line, false).expect("the line reads");
        stamped.push(life.fill(message).expect("the stamp lands"));
        // Alive from the first message to the fill that ends it.
        assert_eq!(life.alive(), usize::from(stamped.len() < LIFE.len()));
    }

    // One instrument, one chain, six messages.
    let instruments: Vec<_> = stamped
        .iter()
        .map(|held| bytes(held, INSTUUID_TAG_NAME.0).expect("an instrument"))
        .collect();
    assert!(instruments.iter().all(|held| *held == instruments[0]));
    assert_eq!(instruments[0].len(), 16);
    let chains: Vec<_> = stamped
        .iter()
        .map(|held| bytes(held, PUUID_TAG_NAME.0).expect("a chain"))
        .collect();
    assert!(
        chains.iter().all(|held| *held == chains[0]),
        "the replace's new identifier joined the chain the old one opened"
    );
    let ids: Vec<_> = stamped
        .iter()
        .map(|held| bytes(held, UUID_TAG_NAME.0).expect("an id"))
        .collect();
    for (pair, messages) in ids.windows(2).zip(stamped.windows(2)) {
        assert_ne!(pair[0], pair[1], "different finalized message content");
        if messages[0].updatedat() < messages[1].updatedat() {
            assert!(pair[0] < pair[1], "UUIDs sort by the full grid instant");
        }
    }
    for tag in [PREVTIMESTAMP_TAG_NAME.0, PREVUUID_TAG_NAME.0] {
        assert_eq!(stamped[0].by_tag(tag).unwrap(), &Scalar::Null);
    }
    for pair in stamped.windows(2) {
        assert_eq!(
            pair[1].by_tag(PREVTIMESTAMP_TAG_NAME.0).unwrap(),
            pair[0].by_tag(UPDATEDAT_TAG_NAME.0).unwrap(),
        );
        assert_eq!(
            pair[1].by_tag(PREVUUID_TAG_NAME.0).unwrap(),
            pair[0].by_tag(UUID_TAG_NAME.0).unwrap(),
        );
    }
    let code = stamped[0]
        .by_tag(CODE_TAG_NAME.0)
        .unwrap()
        .as_str()
        .unwrap();
    assert_eq!(
        chains[0],
        Uuid::from_v8(yggdryl::hashing::xxhash::xxh128(code.as_bytes())).into_bytes()
    );
    assert_eq!(
        yggdryl::hashing::txhash::unix_from_scalar(
            stamped[0].by_tag(60).unwrap(),
            yggdryl::TimeUnit::Microsecond,
        )
        .unwrap(),
        1_767_348_930_000_000,
    );

    // Nothing here is an entry: the wire re-emits byte for byte.
    for (line, message) in LIFE.iter().zip(&stamped) {
        assert_eq!(message.into_bytes(b'|'), *line);
    }

    // Reusing the code has the same identity but a fresh live incarnation.
    let tomorrow = String::from_utf8(LIFE[0].to_vec())
        .unwrap()
        .replace("20260102", "20260103");
    let again = life
        .fill(reader.sole_line(tomorrow.as_bytes(), false).unwrap())
        .unwrap();
    assert_eq!(bytes(&again, PUUID_TAG_NAME.0).unwrap(), chains[0]);
    assert!(again.by_tag(PREVUUID_TAG_NAME.0).unwrap().is_null());
    assert_eq!(life.alive(), 1);
    life.clear();
    assert_eq!(life.alive(), 0);
    // The same line at the same instant is the same chain identity, which is
    // what makes two reads of one capture agree.
    let replayed = life
        .fill(reader.sole_line(LIFE[0], false).unwrap())
        .unwrap();
    assert_eq!(bytes(&replayed, PUUID_TAG_NAME.0).unwrap(), chains[0]);
    assert_eq!(bytes(&replayed, UUID_TAG_NAME.0), Some(ids[0]));
}

#[test]
fn a_message_naming_no_order_has_an_id_and_no_chain() {
    let registry = registry();
    let reader = super::fixed_codec(Arc::clone(&registry));
    let heartbeat = reader
        .sole_line(b"8=FIX.4.4|35=0|34=7|52=20260102-10:15:30.000|10=0|", false)
        .unwrap();
    let mut stamped = reader.lifecycle([heartbeat]);
    let held = stamped.next().unwrap().unwrap();
    assert!(stamped.next().is_none());
    assert!(
        bytes(&held, UUID_TAG_NAME.0).is_some(),
        "every message has an id"
    );
    assert_eq!(held.by_tag(CODE_TAG_NAME.0).unwrap().as_str(), Some(""));
    assert_eq!(
        bytes(&held, PUUID_TAG_NAME.0),
        Some(Uuid::from_v8(yggdryl::hashing::xxhash::xxh128(b"")).into_bytes())
    );
    assert!(
        bytes(&held, INSTUUID_TAG_NAME.0).is_none(),
        "no instrument, no identity"
    );
    for tag in [PREVTIMESTAMP_TAG_NAME.0, PREVUUID_TAG_NAME.0] {
        assert_eq!(held.by_tag(tag).unwrap(), &Scalar::Null);
    }
    assert_eq!(held.updatedat(), held.by_tag(52).unwrap());
    let undated = reader
        .lifecycle([reader.sole_line(b"8=FIX.4.4|35=0|10=0|", false).unwrap()])
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(undated.updatedat(), undated.by_tag(52).unwrap());
    assert!(
        !undated.updatedat().is_null(),
        "the configured intake clock is settled once"
    );
}

#[test]
fn the_instrument_identity_is_the_same_across_spellings_and_venues() {
    let registry = registry();
    let reader = super::fixed_codec(Arc::clone(&registry));
    let mut life = FixLifecycle::new(Arc::clone(&registry));
    let mut identity = |line: &[u8]| {
        bytes(
            &life.fill(reader.sole_line(line, false).unwrap()).unwrap(),
            INSTUUID_TAG_NAME.0,
        )
    };
    // An ISIN outranks a symbol, so the same security under two symbols is
    // one instrument, and case is not a difference.
    let by_isin =
        identity(b"8=FIX.4.4|35=D|11=B1|48=US0378331005|22=4|55=AAPL|207=XNAS|15=USD|10=0|");
    let by_isin_again =
        identity(b"8=FIX.4.4|35=D|11=B2|48=us0378331005|22=4|55=APPLE|207=xnas|15=usd|10=0|");
    assert_eq!(by_isin, by_isin_again);
    // Another market is another instrument identity.
    let elsewhere =
        identity(b"8=FIX.4.4|35=D|11=B3|48=US0378331005|22=4|55=AAPL|207=XLON|15=USD|10=0|");
    assert_ne!(by_isin, elsewhere);
    // Without an ISIN the symbol stands in, and a stated one wins over a
    // symbol that would say otherwise.
    let by_symbol = identity(b"8=FIX.4.4|35=D|11=B4|55=AAPL|207=XNAS|15=USD|10=0|");
    assert!(by_symbol.is_some());
    assert_ne!(by_symbol, by_isin);
    // A bridge row names the same facts under its own keys.
    let bridged = identity(b"#ISINCODE=US0378331005|#LASTMKT=XNAS|#CURRENCY=USD|CLORDID=B5|");
    assert_eq!(bridged, by_isin);
}

#[test]
fn a_stamped_stream_read_again_keeps_what_it_carries() {
    let registry = registry();
    let reader = super::fixed_codec(Arc::clone(&registry));
    let once: Vec<FixMsg> = reader
        .lifecycle(
            LIFE.iter()
                .map(|line| reader.sole_line(line, false).unwrap()),
        )
        .map(|held| held.unwrap())
        .collect();
    assert_eq!(once.len(), LIFE.len());
    let twice: Vec<FixMsg> = reader
        .lifecycle(once.clone())
        .map(|held| held.unwrap())
        .collect();
    assert_eq!(twice.len(), LIFE.len());
    for (first, second) in once.iter().zip(&twice) {
        for tag in [INSTUUID_TAG_NAME.0, UUID_TAG_NAME.0, PUUID_TAG_NAME.0] {
            assert_eq!(bytes(first, tag), bytes(second, tag), "tag {tag}");
        }
        assert_eq!(first.entries().len(), second.entries().len());
        assert_eq!(first, second);
    }
}

/// Schema-less test cells state their native shapes; the intake boundary refuses
/// mandatory layouts before a message can reach lifecycle.
fn row_message(
    registry: Arc<FixRegistry>,
    cells: impl IntoIterator<Item = (i32, Scalar)>,
) -> yggdryl::Result<FixMsg> {
    let mut cells: Vec<_> = cells.into_iter().collect();
    if !cells.iter().any(|(tag, _)| *tag == 52) {
        cells.push((52, clock(0)));
    }
    let mut fields = Vec::new();
    let mut values = Vec::new();
    for (tag, value) in std::iter::once((35, Scalar::from("D"))).chain(cells) {
        let known = registry.get_field_by_tag(tag).unwrap();
        let mut field = if value.is_null() {
            known.clone()
        } else {
            value.dtype().unwrap().nullable_field(known.name())
        };
        field.as_fix_mut().set_tag(tag).unwrap();
        fields.push(field);
        values.push(value);
    }
    let field = DataType::from_fields(fields).unwrap().required_field("D");
    FixMsg::with_registry(registry, field, Scalar::from_sequence(values))
}

fn clock(instant: i64) -> Scalar {
    Scalar::datetime64(instant, TimeUnit::Nanosecond, Timezone::UTC).unwrap()
}

#[test]
fn instrument_payload_keeps_its_recipe_and_chain_payload_is_only_the_code() {
    let registry = registry();
    let codec = super::fixed_codec(Arc::clone(&registry));
    let original = codec.sole_line(LIFE[0], false).unwrap();
    let arrival_digest = original.digest();
    let entries = original.entries().to_vec();
    let message = FixLifecycle::new(registry).fill(original).unwrap();
    let raw_instrument = yggdryl::hashing::xxhash::xxh128(b"XNAS\x1f\x1fAAPL\x1fUSD\x1f");
    let instuuid = Uuid::from_v8(raw_instrument);
    assert_ne!(
        raw_instrument,
        instuuid.get(),
        "the version/variant replace payload bits"
    );
    assert_eq!(
        message.by_tag(INSTUUID_TAG_NAME.0).unwrap(),
        &Scalar::Uuid(instuuid)
    );
    let code = format!("{instuuid}/A1");
    assert_eq!(
        message.by_tag(CODE_TAG_NAME.0).unwrap().as_str(),
        Some(code.as_str())
    );
    assert_eq!(
        message.puuid(),
        &Scalar::Uuid(Uuid::from_v8(yggdryl::hashing::xxhash::xxh128(
            code.as_bytes()
        )))
    );
    assert_eq!(message.entries(), entries);
    assert_eq!(message.digest(), arrival_digest);
    assert_eq!(message.into_bytes(b'|'), LIFE[0]);
}

#[test]
fn event_clock_precedence_is_transaction_then_sending_and_explicit_update_is_independent() {
    let registry = registry();
    for (clocks, event, updated) in [
        (
            vec![(60, 1_001), (52, 2_002), (UPDATEDAT_TAG_NAME.0, 3_003)],
            1_001,
            3_003,
        ),
        (
            vec![(52, 2_002), (UPDATEDAT_TAG_NAME.0, 3_003)],
            2_002,
            3_003,
        ),
        (vec![(UPDATEDAT_TAG_NAME.0, 3_003)], 0, 3_003),
        (Vec::new(), 0, 0),
    ] {
        let row = row_message(
            Arc::clone(&registry),
            clocks.into_iter().map(|(tag, time)| (tag, clock(time))),
        )
        .unwrap();
        let message = FixLifecycle::new(Arc::clone(&registry))
            .try_with_interval_ns(1)
            .unwrap()
            .fill(row)
            .unwrap();
        assert_eq!(
            message.by_tag(SNAPSHOTAT_TAG_NAME.0).unwrap(),
            &clock(event)
        );
        assert_eq!(message.updatedat(), &clock(updated));
    }
}

#[test]
fn lifecycle_uuid_order_keeps_nanoseconds_across_negative_and_bit_boundaries() {
    let registry = registry();
    let mut life = FixLifecycle::new(Arc::clone(&registry))
        .try_with_interval_ns(1)
        .unwrap();
    let mut previous = None;
    for time in [
        i64::MIN,
        -65_537,
        -65_536,
        -17,
        -16,
        -1,
        0,
        1,
        15,
        16,
        65_535,
        65_536,
        i64::MAX,
    ] {
        let row = row_message(Arc::clone(&registry), [(60, clock(time))]).unwrap();
        let message = life.fill(row).unwrap();
        let current = bytes(&message, UUID_TAG_NAME.0).unwrap();
        if let Some(previous) = previous {
            assert!(previous < current, "nanosecond {time}");
        }
        previous = Some(current);
    }
    assert_eq!(life.alive(), 0);
}

#[test]
fn non_native_mandatory_clocks_refuse_at_intake_without_touching_lifecycle() {
    let registry = registry();
    for unit in [TimeUnit::Microsecond, TimeUnit::Millisecond] {
        for invalid in [i64::MIN, -1, 0, i64::MAX] {
            for tag in [52, 60, UPDATEDAT_TAG_NAME.0] {
                let error = row_message(
                    Arc::clone(&registry),
                    [(
                        tag,
                        Scalar::datetime64(invalid, unit, Timezone::UTC).unwrap(),
                    )],
                )
                .unwrap_err();
                assert!(matches!(error, Error::InvalidRecord { .. }));
            }
        }
    }
}

#[test]
fn negative_clock_keeps_stated_instrument_but_mismatching_message_identities_refuse() {
    let registry = registry();
    let mut message = row_message(
        Arc::clone(&registry),
        [
            (60, clock(-1)),
            (CODE_TAG_NAME.0, Scalar::from("A")),
            (INSTUUID_TAG_NAME.0, Scalar::Uuid(Uuid::from_v8(123))),
        ],
    )
    .unwrap();
    for tag in [UUID_TAG_NAME.0, PUUID_TAG_NAME.0] {
        let before = message.clone();
        assert!(message.set(tag, Scalar::Uuid(Uuid::new(7))).is_err());
        assert_eq!(message, before);
    }
    let instrument = message.by_tag(INSTUUID_TAG_NAME.0).unwrap().clone();
    let mut life = FixLifecycle::new(registry).try_with_interval_ns(1).unwrap();
    let message = life.fill(message).unwrap();
    assert_eq!(message.by_tag(INSTUUID_TAG_NAME.0).unwrap(), &instrument);
    assert_eq!(message.updatedat(), &clock(-1));
    assert_eq!(life.alive(), 1);
}

#[test]
fn a_refused_mandatory_column_never_becomes_a_message_or_a_planned_chain() {
    let registry = registry();
    let mut field = DataType::Int32.nullable_field(UUID_TAG_NAME.1);
    field.as_fix_mut().set_tag(UUID_TAG_NAME.0).unwrap();
    let key = registry.get_field_by_tag(11).unwrap().clone();
    let msgtype = registry.get_field_by_tag(35).unwrap().clone();
    let row = DataType::from_fields([field, key, msgtype])
        .unwrap()
        .required_field("D");
    let error = FixMsg::with_registry(
        Arc::clone(&registry),
        row,
        Scalar::from_sequence([Scalar::Null, Scalar::from("NEW"), Scalar::from("D")]),
    )
    .unwrap_err();
    assert!(matches!(error, Error::InvalidRecord { ref path, .. } if path == "$.uuid"));
    assert_eq!(FixLifecycle::new(registry).alive(), 0);
}

#[test]
fn the_committed_capture_keeps_its_direct_count_and_each_doors_full_replay() {
    let codec = super::dataset::codec();
    let lines = super::dataset::text_lines();
    assert_eq!(lines.len(), 129);
    let mut direct_life = FixLifecycle::new(Arc::clone(codec.registry()));
    let mut enriched_life = FixLifecycle::new(Arc::clone(codec.registry()));
    let mut direct_trace = Vec::new();
    let mut enriched_trace = Vec::new();
    let mut messages = 0;
    for (index, line) in lines.iter().enumerate() {
        for message in codec.parse_text_line(line).expect("a capture line reads") {
            let message = message.expect("a capture message reads");
            let enriched = codec
                .enrich_message(message.clone())
                .expect("the capture message enriches");
            for (life, message, trace) in [
                (&mut direct_life, message, &mut direct_trace),
                (&mut enriched_life, enriched, &mut enriched_trace),
            ] {
                let entries = message.entries().to_vec();
                let digest = message.digest();
                let wire = message.into_bytes(b'|');
                let stamped = life.fill(message).expect("the capture message stamps");
                if [101, 102].contains(&(index + 1)) {
                    assert!(stamped.get_by_tag(35).is_none());
                    assert_eq!(stamped.by_tag(CODE_TAG_NAME.0).unwrap().as_str(), Some(""));
                }
                assert_eq!(stamped.entries(), entries);
                assert_eq!(stamped.digest(), digest);
                assert_eq!(stamped.into_bytes(b'|'), wire);
                trace.push((stamped, life.alive()));
            }
            messages += 1;
        }
    }
    assert_eq!(messages, 83);
    assert_eq!(direct_life.alive(), 4);
    // Line 73's derived terminal state closes ABBN.S. The untyped FIXML
    // at line 101 no longer reopens it through a hard-tag fallback.
    assert_eq!(enriched_life.alive(), 3);
    // Each door replays its own projection: enrichment may end a chain at a
    // different message and the untyped FIXML supplies no identifiers.
    for (trace, expected) in [(direct_trace, 4), (enriched_trace, 3)] {
        assert_eq!(trace.len(), 83);
        let mut replay = FixLifecycle::new(Arc::clone(codec.registry()));
        for (message, alive) in trace {
            let stamped = replay
                .fill(message.clone())
                .expect("the stamped capture message replays");
            assert_eq!(stamped, message);
            assert_eq!(stamped.entries(), message.entries());
            assert_eq!(stamped.digest(), message.digest());
            assert_eq!(stamped.into_bytes(b'|'), message.into_bytes(b'|'));
            assert_eq!(replay.alive(), alive);
        }
        assert_eq!(replay.alive(), expected);
    }
}
