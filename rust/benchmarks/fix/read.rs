use std::hint::black_box;
use std::sync::Arc;

use criterion::{Criterion, Throughput};
use yggdryl::{DataType, FixBranch, FixCategory, FixCodec, FixRegistry, Version};

use super::seed;

/// The four dialects a capture line arrives in, one row each.
///
/// Each is a real shape rather than a synthetic one: a framed tag stream with
/// prose either side, a bare tag stream, a bridge line keyed by name, and a
/// wide order with a repeating group.
const TAGGED: &str =
    "sending >> 8=FIX.4.4|9=176|35=D|11=ORDER-1|55=AAPL|54=1|38=100|40=2|10=203| << queued";
const BARE: &str = "8=FIX.4.4|9=176|35=D|11=ORDER-1|55=AAPL|54=1|38=100|40=2|10=203|";
const NAMED: &str =
    "ACCOUNT=A1|MSGTYPE=D|CLORDID=ORDER-1|SYMBOL=AAPL|SIDE=1|ORDERQTY=100|ORDTYPE=2";
const GROUPED: &str = "MSGTYPE=D|#NOPARTYIDS=2|#NOPARTYIDS[0]=PARTYID=SYNTH-01\u{4}\u{3}PARTYIDSOURCE=D\u{4}\u{3}PARTYROLE=1|#NOPARTYIDS[1]=PARTYID=SYNTH-02\u{4}\u{3}PARTYIDSOURCE=D\u{4}\u{3}PARTYROLE=3";
const NUMERIC_GROUPED: &str = "35=D|453=2|448=SYNTH-01|447=D|452=1|802=1|523=DESK|803=1|448=SYNTH-02|447=D|452=3|55=AAPL|10=0|";
const PINNED_NUMERIC: &[u8] = b"6000=1|6001=42|6002=7|";

fn pinned_numeric_reader() -> FixCodec {
    let mut registry = FixRegistry::new();
    for (branch, name, dtype) in [
        ("alpha", "Alpha", DataType::Int32),
        ("beta", "Beta", DataType::Utf8),
    ] {
        let branch = FixBranch::from_str(branch).unwrap();
        let field = |name: String, tag, dtype: DataType| {
            let mut field = dtype.nullable_field(name);
            field.as_fix_mut().set_tag(tag).unwrap();
            field.as_fix_mut().set_branch(&branch).unwrap();
            field
        };
        let counter = field(format!("No{name}Rows"), 6000, DataType::Int32);
        let member = field(format!("{name}ID"), 6001, dtype.clone());
        let tail = field(format!("{name}Value"), 6002, dtype);
        registry
            .add_fields([counter, member.clone(), tail])
            .unwrap();
        let mut component = DataType::from_fields([member])
            .unwrap()
            .required_field(format!("{name}Entry"));
        component.as_fix_mut().set_branch(&branch).unwrap();
        registry
            .create_definition(FixCategory::Components, component.clone())
            .unwrap();
        let mut group = DataType::list(component.clone()).nullable_field(format!("{name}Rows"));
        group.as_fix_mut().set_branch(&branch).unwrap();
        group.as_fix_mut().set_counter(6000).unwrap();
        group.as_fix_mut().set_component(component.name()).unwrap();
        registry
            .create_definition(FixCategory::Groups, group)
            .unwrap();
    }
    FixCodec::new(Arc::new(registry)).with_branch(&FixBranch::from_str("beta").unwrap())
}
/// A bridge row keyed by `#` names, one of them twinned by its bare spelling.
///
/// The twin is what makes the shape its own benchmark: whether one `#` drops
/// is a fact about the whole row, so this measures the scan that decides it
/// on a row where it actually fires.
const HASHED: &str = "toBridge ORDERID=OD-1|MSGTYPE=D|#CLORDID=ORDER-1|#SYMBOL=AAPL|#SIDE=1\
|#ORDERQTY=100|#ORDTYPE=2|#ORDERID=OD-2";

/// One Jolokia read of one session interface, as a bridge answers it.
///
/// A fifth dialect, and the one that is a document rather than a run of pairs:
/// it is parsed as JSON, walked into the same pairs the others produce, and
/// built by the same builder - so what it costs over a frame is the parse and
/// the walk, which is what this measures.
const ULCONFIG: &str = concat!(
    r#"{"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:name=ULMSG_BROKER_TO_DMZ,"#,
    r#"plugin-type=FIX,type=Plugin","type":"read"},"value":{"SenderCompID":"ULB_BKRBDG","#,
    r#""TargetCompID":"ULB_PTBDG","BeginString":"FIX.4.2","Category":"InterBridge","#,
    r#""PrimaryHost":"localhost","CurrentPort":7061,"BackupHost":null,"BackupPort":-1,"#,
    r#""OutgoingMsgSeqNum":129,"IncomingMsgSeqNum":129,"LogLevel":-1,"PriorityLevel":5,"#,
    r#""Name":"ULMSG_BROKER_TO_DMZ","Version":"2.0.3","State":"logged","Type":"A","#,
    r#""NeedReload":false,"NotificationsStatus":false,"BinaryName":"ULMsg.jar","#,
    r#""ClassName":"ULMsg","MinimumBridgeRevision":"20050101000000"},"status":200}"#,
);

pub fn benchmarks(criterion: &mut Criterion) {
    let reader = FixCodec::new(Arc::new(seed()));
    let mut group = criterion.benchmark_group("fix/read");
    let message = reader
        .transform_fix_line(NUMERIC_GROUPED.as_bytes(), false)
        .unwrap();
    assert_eq!(message.by_tag(453).unwrap(), &yggdryl::Scalar::from(2_i32));
    assert_eq!(message.by_name("Parties").unwrap().len(), 2);
    assert_eq!(
        message
            .by_path("Parties.0.PtysSubGrp.0.PartySubID")
            .unwrap()
            .as_str(),
        Some("DESK")
    );
    assert_eq!(message.into_bytes(b'|'), NUMERIC_GROUPED.as_bytes());
    let bridge = reader
        .transform_ullink_line(GROUPED.as_bytes(), false)
        .unwrap();
    assert_eq!(bridge.by_tag(453).unwrap(), &yggdryl::Scalar::from(2_i32));
    assert_eq!(bridge.by_name("Parties").unwrap().len(), 2);
    group.bench_function("message_stable_hash_one_state_allocation", |bencher| {
        bencher.iter(|| black_box(message.stable_hash()));
    });

    for (label, row) in [
        ("tagged", TAGGED),
        ("bare", BARE),
        ("named", NAMED),
        ("grouped", GROUPED),
        ("numeric_grouped", NUMERIC_GROUPED),
        ("hashed", HASHED),
    ] {
        group.throughput(Throughput::Bytes(row.len() as u64));
        group.bench_function(label, |bencher| {
            bencher.iter(|| {
                black_box(&reader)
                    .transform_line(black_box(row).as_bytes(), false)
                    .expect("a readable row")
                    .next()
                    .expect("one message")
                    .expect("a typed message")
            });
        });
    }

    let pinned = pinned_numeric_reader();
    let decoded = pinned.transform_fix_line(PINNED_NUMERIC, false).unwrap();
    assert_eq!(
        decoded.by_path("BetaRows.0.BetaID").unwrap().as_str(),
        Some("42")
    );
    assert_eq!(decoded.by_name("BetaValue").unwrap().as_str(), Some("7"));
    assert!(decoded.get_by_name("AlphaRows").is_none());
    assert_eq!(decoded.into_bytes(b'|'), PINNED_NUMERIC);
    group.throughput(Throughput::Bytes(PINNED_NUMERIC.len() as u64));
    group.bench_function("numeric_grouped_pinned_branch", |bencher| {
        bencher.iter(|| {
            black_box(&pinned)
                .transform_fix_line(black_box(PINNED_NUMERIC), false)
                .expect("a pinned numeric group")
        });
    });

    // The twin scan at width: hundreds of `#` keys around one bare twin, so
    // how the scan scales shows here rather than hiding inside the short rows
    // above.
    let mut wide = String::from("MSGTYPE=D|ORDERID=OD-1");
    for at in 0..300 {
        use std::fmt::Write;
        write!(wide, "|#TAG{at}=V{at}").expect("writing to a String cannot fail");
    }
    wide.push_str("|#ORDERID=OD-2");
    group.throughput(Throughput::Bytes(wide.len() as u64));
    group.bench_function("hashed_wide", |bencher| {
        bencher.iter(|| {
            black_box(&reader)
                .transform_line(black_box(&wide).as_bytes(), false)
                .expect("a readable row")
                .next()
                .expect("one message")
                .expect("a typed message")
        });
    });

    // A row read at an older version pays one dated code lookup per value and
    // nothing else: the column it lands in is the dictionary's at every
    // version, so a dated read is a read.
    let dated = reader
        .clone()
        .with_version("4.2".parse::<Version>().expect("a version"));
    group.bench_function("tagged_at_version", |bencher| {
        bencher.iter(|| {
            black_box(&dated)
                .transform_line(black_box(BARE).as_bytes(), false)
                .expect("a readable row")
                .next()
                .expect("one message")
                .expect("a typed message")
        });
    });

    // A bridge configuration, against a dictionary that types it and one that
    // does not, so what the dictionary is worth on this shape is a number.
    let branch = FixBranch::from_str(yggdryl::ULBRIDGE_BRANCH).expect("a branch");
    let typed = FixCodec::new(Arc::new(
        seed()
            .with_ulbridge_fields()
            .expect("ULBridge's own fields"),
    ))
    .with_branch(&branch);
    group.throughput(Throughput::Bytes(ULCONFIG.len() as u64));
    for (label, reader) in [("ulconfig", &typed), ("ulconfig_untyped", &reader)] {
        group.bench_function(label, |bencher| {
            bencher.iter(|| {
                black_box(reader)
                    .transform_line(black_box(ULCONFIG).as_bytes(), false)
                    .expect("a readable document")
                    .try_fold(0_usize, |count, message| message.map(|_| count + 1))
                    .expect("typed messages")
            });
        });
    }

    // The emit that closes the round trip, from the entries rather than the row.
    let message = reader
        .transform_line(BARE.as_bytes(), false)
        .expect("a readable row")
        .next()
        .expect("one message")
        .expect("a typed message");
    group.bench_function("emit", |bencher| {
        bencher.iter(|| black_box(&message).into_bytes(black_box(b'|')));
    });
    group.finish();
}
