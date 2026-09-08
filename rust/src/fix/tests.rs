//! Focused edge cases of the FIX module, driven with explicit inputs.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use smol_str::SmolStr;

use super::global::autoload;
use super::registry::control_byte;
use super::store::shard_of;
use crate::holder::local::Folder;
use crate::types::MsgType;
use crate::{
    DataType, Error, Field, FixBranch, FixCode, FixEntry, FixId, FixKey, FixLineage,
    FixLineageEntry, FixMsg, FixPedigree, FixRegistry, MimeType, Scalar, Version,
};

/// The venue dictionary every branched case is written against.
fn cme() -> FixBranch {
    FixBranch::from_str("cme").unwrap()
}

/// A nullable text field carrying one canonical tag.
fn tagged(name: &str, tag: i32) -> Field {
    let mut field = DataType::Utf8.nullable_field(name);
    field.as_fix_mut().set_tag(tag).unwrap();
    field
}

/// A nullable text field carrying one canonical identifier.
fn identified(name: &str, branch: &FixBranch, tag: i32) -> Field {
    let mut field = DataType::Utf8.nullable_field(name);
    field.as_fix_mut().set_id(branch, tag).unwrap();
    field
}

/// A field carrying every `fix:` property.
fn full(name: &str, tag: i32, tags: &[i32], aliases: &[&str]) -> Field {
    let mut field = tagged(name, tag);
    field.as_fix_mut().set_tags(tags).unwrap();
    field.as_fix_mut().set_aliases(aliases).unwrap();
    field
        .as_fix_mut()
        .set_description(format!("{name} described"))
        .unwrap();
    field
}

/// A nullable Struct component - a FIX component - carrying one canonical tag.
fn component(name: &str, tag: i32) -> Field {
    let mut field = DataType::from_fields([tagged("Member", 9_001)])
        .unwrap()
        .nullable_field(name);
    field.as_fix_mut().set_tag(tag).unwrap();
    field
}

/// A nullable List of Struct - a FIX repeating group - carrying one canonical
/// tag.
fn group(name: &str, tag: i32) -> Field {
    let item = DataType::from_fields([tagged("Member", 9_002)])
        .unwrap()
        .required_field("item");
    let mut field = DataType::list(item).nullable_field(name);
    field.as_fix_mut().set_tag(tag).unwrap();
    field
}

/// A fresh directory of this test's own under the platform temporary root.
fn scratch(label: &str) -> PathBuf {
    let path = Folder::temporary()
        .unwrap()
        .path()
        .unwrap()
        .join(format!("yggdryl-fix-unit-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    path
}

/// The stored keys of one registry, probed through every index.
fn probe<'registry>(
    registry: &'registry FixRegistry,
    tag: i32,
    alternate: i32,
    name: &str,
    alias: &str,
) -> [Option<&'registry str>; 4] {
    let standard = FixBranch::STANDARD;
    [
        registry.get_field_by_tag(tag).map(Field::name),
        registry.get_field_by_tag(alternate).map(Field::name),
        registry
            .get_field_by_name(name, Some(&standard))
            .map(Field::name),
        registry
            .get_field_by_name(alias, Some(&standard))
            .map(Field::name),
    ]
}

#[test]
fn name_indexes_fold_ascii_without_crossing_branches() {
    let cme = cme();
    let standard = tagged("MsgType", 35);
    let vendor = identified("MsgType", &cme, 5_035);
    let registry = FixRegistry::from_fields([standard.clone(), vendor.clone()]).unwrap();
    assert_eq!(
        registry.get_field_by_name("MSGTYPE", Some(&FixBranch::STANDARD)),
        Some(&standard)
    );
    assert_eq!(
        registry.get_field_by_name("msgtype", Some(&cme)),
        Some(&vendor)
    );
    assert!(
        registry
            .get_field_by_name("msgtype", Some(&FixBranch::from_str("cm").unwrap()))
            .is_none()
    );
}

#[test]
fn protocol_and_msgtype_inference_are_shallow_borrowed_redirects() {
    type Case = (&'static [u8], MimeType, Option<&'static [u8]>);

    let cases: &[Case] = &[
        (
            b"2025-01-01 INFO 8=FIX.4.4|35=D|55=AAPL|",
            MimeType::FIX,
            Some(b"D"),
        ),
        (
            b"prefix MsgType=8 Symbol=AAPL suffix",
            MimeType::ULLINK,
            Some(b"8"),
        ),
        (
            b"35=F^ASymbol=MSFT^AMsgType=ignored",
            MimeType::FIXUL,
            Some(b"ignored"),
        ),
        (
            b"[8=FIX.4.2<SOH>35=A<SOH>55=IBM]",
            MimeType::FIX,
            Some(b"A"),
        ),
        (
            b"8=FIXT.1.1\\x0135=AE\\x0155=EUR/USD",
            MimeType::FIX,
            Some(b"AE"),
        ),
        (
            b"raw 8=FIX.4.4{SOH}9=224{SOH}35=8{SOH}10=118{SOH}",
            MimeType::FIX,
            Some(b"8"),
        ),
        (
            b"sending >> 8=FIX.4.2|35=UL|#SYMBOL=TTF|#SIDE=1|10=044|",
            MimeType::FIXUL,
            Some(b"UDF"),
        ),
        (
            b"8=FIX.4.4|35=D|11=ORDER-1|SYMBOL=AAPL|SIDE=1|10=000",
            MimeType::FIXUL,
            Some(b"D"),
        ),
        (
            b"8=FIX.4.4|35=D|11=ORDER-1|213=SYMBOL=AAPL|SIDE=1|10=000|",
            MimeType::FIXUL,
            Some(b"D"),
        ),
        (
            b"8=FIX.4.4|35=D|213=<Order/>|10=000|",
            MimeType::FIXML,
            Some(b"D"),
        ),
        (
            b"8=FIX.4.4|35=8|58=quoting #A=1 and #B=2|10=001|",
            MimeType::FIX,
            Some(b"8"),
        ),
        (
            b"toBridge #ISINCODE=XX|#SYMBOL=TTF|#SIDE=1",
            MimeType::ULLINK,
            None,
        ),
        (
            b"ACCOUNT=A1|MSGTYPE=D|CLORDID=ORDER-1|SYMBOL=AAPL|SIDE=1",
            MimeType::ULLINK,
            Some(b"D"),
        ),
        (
            b"After Enrichment -> ACCOUNT=ACCT-000117 CLIENTID=MCFP2 VENUE=XPAR",
            // No marker and no `MSGTYPE=`: with no dictionary to resolve
            // these names against, an attribute run is what it looks like.
            MimeType::KEYVALUE,
            None,
        ),
        (
            // A bridge configuration document: JSON, and the namespace that
            // says whose. The first ObjectName's `type=` is what the entry is,
            // and `plugin-type=` shares its last five bytes without being it.
            br#"{"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:name=ULMSG_BROKER_TO_DMZ,plugin-type=FIX,type=Plugin","type":"read"},"value":{"Category":"InterBridge"},"status":200}"#,
            MimeType::ULCONFIG,
            Some(b"Plugin"),
        ),
        (
            // A wildcard read names no type in its own MBean, so the entry it
            // answers with names the document's.
            br#"{"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:*","type":"read"},"value":{"com.ullink.ulbridge.sessioninterfaces.plugins:name=X,plugin-type=FIX,type=ConfigurationPlugin":{"Name":"X"}},"status":200}"#,
            MimeType::ULCONFIG,
            Some(b"ConfigurationPlugin"),
        ),
        (
            // Neither MBean names a type, so the operation is what is left.
            br#"{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:*","type":"read"}"#,
            MimeType::ULCONFIG,
            Some(b"read"),
        ),
        (
            // The namespace is the whole of what makes the reading: JSON
            // without it is JSON.
            br#"{"request":{"mbean":"java.lang:type=Memory","type":"read"},"status":200}"#,
            MimeType::JSON,
            None,
        ),
        (
            // A raw `MSGTYPE=` in front of a document still outranks it, the
            // same way it outranks a frame it is relaying.
            br#"MSGTYPE=8 {"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:*","type":"read"}"#,
            MimeType::ULLINK,
            Some(b"8"),
        ),
        (
            b"MsgType=prefix 8=FIX.4.4|35=D|55=AAPL|10=001| Symbol=suffix",
            MimeType::FIXUL,
            Some(b"prefix"),
        ),
        (
            b"<Order ClOrdID='XML-1'>body</Order>",
            // A document is what it opens as, before any pair rule runs: an
            // attribute inside a tag is not a field.
            MimeType::XML,
            None,
        ),
        (
            b"level=INFO timestamp=2025-01-01 message=random",
            MimeType::KEYVALUE,
            None,
        ),
        (b"35=", MimeType::OCTET_STREAM, None),
        (b"not a protocol line", MimeType::OCTET_STREAM, None),
    ];
    for (line, protocol, msgtype) in cases {
        assert_eq!(&MimeType::infer_bytes(line), protocol, "{line:?}");
        assert_eq!(MsgType::infer_bytes(line), *msgtype, "{line:?}");
        let text = std::str::from_utf8(line).unwrap();
        assert_eq!(&MimeType::infer_text(text), protocol, "{text}");
        assert_eq!(
            MsgType::infer_text(text),
            msgtype.and_then(|value| std::str::from_utf8(value).ok()),
            "{text}"
        );
    }

    // An overflowing numeric key is one bad candidate, not a reason to stop
    // before a later valid frame in the same log line.
    let overflow = b"999999999999999999999=x 8=FIX.4.4|35=D|";
    assert_eq!(MimeType::infer_bytes(overflow), MimeType::FIX);
    assert_eq!(MsgType::infer_bytes(overflow), Some(b"D".as_slice()));

    // Classification needs no dictionary at all: it is transport, and every
    // captured line has a shape whatever protocol it carried.
    assert_eq!(MimeType::infer_bytes(b"35=D|55=AAPL|"), MimeType::FIX);
    assert_eq!(MsgType::infer_bytes(b"35=D|55=AAPL|"), Some(&b"D"[..]));
    assert_eq!(
        MimeType::infer_text("ACCOUNT=A1|MSGTYPE=8|"),
        MimeType::ULLINK
    );
    assert_eq!(MsgType::infer_text("ACCOUNT=A1|MSGTYPE=8|"), Some("8"));

    assert_eq!(MimeType::ULLINK.as_str(), "text/ullink");
    assert_eq!(MimeType::FIX.as_str(), "text/fix");
    assert_eq!(MimeType::FIXUL.as_str(), "text/fixul");
    assert_eq!(MimeType::FIXML.as_str(), "text/fixml");

    for value in ["U1", "UABC", "UL"] {
        let line = format!("8=FIX.4.4|35={value}|10=000|");
        assert_eq!(MsgType::infer_text(&line), Some("UDF"));
    }
    assert_eq!(MsgType::infer_text("35=U|"), Some("U"));
}

#[test]
fn a_bridge_configuration_states_its_own_half_of_the_exchange() {
    use crate::types::MsgDirection;

    const ANSWERED: &[u8] = br#"{"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:*","type":"read"},"value":{"com.ullink.ulbridge.sessioninterfaces.plugins:name=X,plugin-type=FIX,type=Plugin":{"ExtendedActions":[{"name":"send-test-request","description":"Send a test request message.","parameters":[{"name":"test-request-id","description":"The outgoing test request ID to send"}]}]}},"status":200}"#;
    const ASKED: &[u8] =
        br#"{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:*","type":"read"}"#;

    // The document says which half it is, and the words inside it - `send`,
    // `outgoing`, `in` - are its own payload rather than a transport marker.
    // Without the bound they would answer, and answer wrongly.
    assert_eq!(
        MsgDirection::infer_bytes(ANSWERED),
        Some(MsgDirection::RECV)
    );
    assert_eq!(MsgDirection::infer_bytes(ASKED), Some(MsgDirection::SENT));
    // An error is an answer that came back, not a request that went out.
    let failed =
        br#"{"request":{"mbean":"com.ullink.ulbridge:*","type":"read"},"error":"no such MBean"}"#;
    assert_eq!(MsgDirection::infer_bytes(failed), Some(MsgDirection::RECV));
    // A write states a `value` of its own, and it is still what went out: the
    // echoed request is the reading, not the keys an answer happens to share.
    let write = br#"{"type":"write","mbean":"com.ullink.ulbridge:type=Bridge","attribute":"LogLevel","value":3}"#;
    assert_eq!(MsgDirection::infer_bytes(write), Some(MsgDirection::SENT));
    assert_eq!(MimeType::infer_bytes(write), MimeType::ULCONFIG);
    assert_eq!(MsgType::infer_bytes(write), Some(&b"Bridge"[..]));

    // A bulk read answers an array of these, and an array closes with `]`.
    let bulk = br#"[{"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:name=A,type=Plugin","type":"read"},"status":200},{"request":{"mbean":"com.ullink.ulbridge:type=Bridge","type":"read"},"status":200}]"#;
    assert_eq!(MimeType::infer_bytes(bulk), MimeType::ULCONFIG);
    assert_eq!(MsgType::infer_bytes(bulk), Some(&b"Plugin"[..]));
    assert_eq!(MsgDirection::infer_bytes(bulk), Some(MsgDirection::RECV));

    // The `type=` is read inside the ObjectName that states it, so a value
    // elsewhere spelling the same five bytes is that value's business.
    let quoting = br#"{"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:*","type":"read"},"value":{"Comment":"routed by ,type=Decoy"},"status":200}"#;
    assert_eq!(MsgType::infer_bytes(quoting), Some(&b"read"[..]));

    // Nothing is stated, and nothing is taken off a line with no marker on it.
    let (direction, body) = MsgDirection::split_bytes(ANSWERED);
    assert_eq!(direction, Some(MsgDirection::RECV));
    assert_eq!(body, ANSWERED);

    // A verb the transport wrote outranks what the document says of itself,
    // and reading it takes the marker off exactly as it does for a frame.
    let marked = [b"sending >> ".as_slice(), ANSWERED].concat();
    let (direction, body) = MsgDirection::split_bytes(&marked);
    assert_eq!(direction, Some(MsgDirection::SENT));
    assert_eq!(body, [b">> ".as_slice(), ANSWERED].concat());
    // The bound is the whole prefix, so a `[jolokia]` in the prose is prose.
    let stamped = [
        b"2026-08-14 09:12:03 INFO [jolokia] recv << ".as_slice(),
        ANSWERED,
    ]
    .concat();
    assert_eq!(
        MsgDirection::infer_bytes(&stamped),
        Some(MsgDirection::RECV)
    );
    assert_eq!(MimeType::infer_bytes(&stamped), MimeType::ULCONFIG);
    assert_eq!(MsgType::infer_bytes(&stamped), Some(&b"Plugin"[..]));

    // The default fills silence and never overrides the statement.
    assert_eq!(
        MsgDirection::at_payload(ANSWERED, 0, Some(MsgDirection::SENT)),
        Some(MsgDirection::RECV),
    );

    // A reading wider than the type is a type the capture carried: it is
    // coerced into the column rather than dropped, and stays put.
    let held = MsgType::coerce("ConfigurationPlugin");
    assert!(held.is_synthetic());
    assert_eq!(held, MsgType::coerce("ConfigurationPlugin"));
    assert_ne!(held, MsgType::coerce("Plugin"));
    assert_eq!(MsgType::coerce("Plugin").as_str(), "Plugin");

    // A class the bridge spells is not an MBean it names: every `$type`,
    // `className` and init file in these documents carries one, and a record
    // quoting one is an ordinary JSON record.
    let quoted = br#"{"level":"INFO","message":"reloading com.ullink.ulbridge2.plugins.ULMsg"}"#;
    assert_eq!(MimeType::infer_bytes(quoted), MimeType::JSON);
    assert_eq!(MsgDirection::infer_bytes(quoted), None);
    // The namespace also has to stand inside the document rather than in the
    // prose in front of it, or the prose is what named it.
    let prose = br#"reloading com.ullink.ulbridge.sessioninterfaces.plugins:* {"level":"INFO"}"#;
    assert_eq!(MimeType::infer_bytes(prose), MimeType::OCTET_STREAM);
    // An unterminated document is not one.
    let truncated = br#"{"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:*","#;
    assert_eq!(MimeType::infer_bytes(truncated), MimeType::OCTET_STREAM);

    // The whole reading is borrowed out of the caller's bytes.
    let read = MsgType::infer_bytes(ANSWERED).unwrap();
    assert!(ANSWERED.as_ptr_range().contains(&read.as_ptr()));
}

/// One Jolokia read of one session interface, trimmed to what is read.
const ULCONFIG_SINGLE: &[u8] = br#"{"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:name=ULMSG_BROKER_TO_POSTTRADE,plugin-type=FIX,type=Plugin","type":"read"},"value":{"SenderCompID":"ULB_BKRBDG","TargetCompID":"ULB_PTBDG","BeginString":"FIX.4.2","Category":"InterBridge","PrimaryHost":"localhost","CurrentPort":7061,"BackupHost":null,"BackupPort":-1,"OutgoingMsgSeqNum":129,"NeedReload":false,"Name":"ULMSG_BROKER_TO_POSTTRADE","ExtendedActions":[{"name":"hot-reset","enabled":true}]},"status":200}"#;

/// One wildcard read, answering for two MBeans of different types.
const ULCONFIG_WILDCARD: &[u8] = br#"{"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:*","type":"read"},"value":{"com.ullink.ulbridge.sessioninterfaces.plugins:name=A,plugin-type=FIX,type=ConfigurationPlugin":{"Name":"A","Category":"InterBridge"},"com.ullink.ulbridge.sessioninterfaces.plugins:name=B,plugin-type=FIX,type=Plugin":{"Name":"B","CurrentPort":9905,"SenderCompID":"PICTET_BPAG"}},"status":200}"#;

/// A codec over the shipped dictionary, pinned to ULBridge's own.
fn ulbridge_codec() -> crate::FixCodec {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = FixRegistry::from_handle(&Folder::new(root).unwrap())
        .unwrap()
        .with_ulbridge_fields()
        .unwrap();
    let branch = FixBranch::from_str(crate::ULBRIDGE_BRANCH).unwrap();
    crate::FixCodec::new(Arc::new(registry)).with_branch(&branch)
}

#[test]
fn a_bridge_configuration_reads_as_the_envelope_and_one_occurrence_per_mbean() {
    let codec = ulbridge_codec();
    let msg = codec.transform_line(ULCONFIG_SINGLE, false).unwrap();

    // The envelope is what the exchange was, and it types: a status is a
    // number rather than the text it arrived as.
    assert_eq!(
        msg.by_tag(crate::MBEAN_TAG).unwrap().as_str(),
        Some(
            "com.ullink.ulbridge.sessioninterfaces.plugins:name=ULMSG_BROKER_TO_POSTTRADE,plugin-type=FIX,type=Plugin"
        ),
    );
    assert_eq!(
        msg.by_tag(crate::OPERATION_TAG).unwrap(),
        &Scalar::from("read")
    );
    assert_eq!(
        msg.by_tag(crate::STATUS_TAG).unwrap(),
        &Scalar::from(200_i64)
    );

    // A field the specification publishes keeps the specification's tag, and
    // it resolves under a pinned venue branch because a name is looked for in
    // the message's own dictionary first and in the standard one after.
    assert_eq!(
        msg.by_path("SessionInterfaces.0.SenderCompID").unwrap(),
        &Scalar::from("ULB_BKRBDG"),
    );
    assert_eq!(
        msg.by_path("SessionInterfaces.0.TargetCompID").unwrap(),
        &Scalar::from("ULB_PTBDG"),
    );
    assert_eq!(
        msg.by_path("SessionInterfaces.0.BeginString").unwrap(),
        &Scalar::from("FIX.4.2"),
    );

    // The ObjectName's own properties are read where the classifier reads
    // them, so one spelling answers for the column and for the row.
    assert_eq!(
        msg.by_path("SessionInterfaces.0.MBeanType").unwrap(),
        &Scalar::from("Plugin"),
    );
    assert_eq!(
        msg.by_path("SessionInterfaces.0.PluginType").unwrap(),
        &Scalar::from("FIX"),
    );

    // Everything else types to what it is rather than to the text it was.
    assert_eq!(
        msg.by_path("SessionInterfaces.0.CurrentPort").unwrap(),
        &Scalar::from(7061_i64),
    );
    assert_eq!(
        msg.by_path("SessionInterfaces.0.BackupPort").unwrap(),
        &Scalar::from(-1_i64),
        "a sentinel is a number the bridge sent, not an absence",
    );
    assert_eq!(
        msg.by_path("SessionInterfaces.0.NeedReload").unwrap(),
        &Scalar::from(false),
    );
    // A stated null is an absence: no field, no entry.
    assert!(msg.get_by_path("SessionInterfaces.0.BackupHost").is_none());

    // A group inside a group is one level deeper than a key addresses, so it
    // is retained as the JSON it is rather than dropped.
    let actions = msg
        .by_path("SessionInterfaces.0.ExtendedActions")
        .unwrap()
        .as_str()
        .expect("the array, as text");
    assert!(actions.contains("hot-reset"), "{actions}");
}

#[test]
fn a_wildcard_read_is_one_occurrence_per_mbean_it_answered_for() {
    let codec = ulbridge_codec();
    let msg = codec.transform_line(ULCONFIG_WILDCARD, false).unwrap();

    // One entry or fifty is one shape: the request's own MBean is a pattern
    // and every MBean it matched is an occurrence, in canonical ObjectName
    // order because a JSON object has no order of its own.
    assert_eq!(
        msg.by_tag(crate::MBEAN_TAG).unwrap(),
        &Scalar::from("com.ullink.ulbridge.sessioninterfaces.plugins:*"),
    );
    assert_eq!(
        msg.by_path("SessionInterfaces.0.Name").unwrap(),
        &Scalar::from("A")
    );
    assert_eq!(
        msg.by_path("SessionInterfaces.1.Name").unwrap(),
        &Scalar::from("B")
    );
    // Each occurrence states its own type, whatever the document declared.
    assert_eq!(
        msg.by_path("SessionInterfaces.0.MBeanType").unwrap(),
        &Scalar::from("ConfigurationPlugin"),
    );
    assert_eq!(
        msg.by_path("SessionInterfaces.1.MBeanType").unwrap(),
        &Scalar::from("Plugin"),
    );
    assert_eq!(
        msg.by_path("SessionInterfaces.1.SenderCompID").unwrap(),
        &Scalar::from("PICTET_BPAG"),
    );

    // A dictionary without ULBridge's fields keeps every key rather than
    // dropping it: a venue sends fields no dictionary has.
    let bare = crate::FixCodec::new(Arc::new(FixRegistry::new()));
    let plain = bare.transform_line(ULCONFIG_WILDCARD, false).unwrap();
    assert!(
        plain.get_by_name("mbean").is_some(),
        "kept under its own folded spelling, untyped",
    );
    assert!(
        plain.get_by_tag(crate::MBEAN_TAG).is_none(),
        "and with no tag"
    );
}

#[test]
fn ulbridge_fields_are_a_dictionary_of_their_own() {
    let held = crate::fix_ulbridge_fields().unwrap();
    let branch = FixBranch::from_str(crate::ULBRIDGE_BRANCH).unwrap();
    assert_eq!(held[0].name(), "MBean");
    // The branch is what keeps a venue's own 20001 a different field.
    assert_eq!(
        held[0].as_fix().id().unwrap(),
        Some(FixId::from_parts(&branch, crate::MBEAN_TAG).unwrap()),
    );
    assert_ne!(
        held[0].as_fix().id().unwrap().unwrap(),
        FixId::standard(crate::MBEAN_TAG),
    );

    // Every tag this dictionary claims is inside FIX's user-defined range,
    // which is the one range a non-standard branch may claim at all.
    for field in held {
        let tag = field.as_fix().tag().unwrap().expect("a tag");
        assert!(tag >= crate::ULBRIDGE_TAG_MIN, "{}: {tag}", field.name());
        assert!(tag < FixId::USER_TAG_MAX, "{}: {tag}", field.name());
        assert!(field.description().is_some(), "{}", field.name());
    }

    // The names FIX already publishes are not among them: a field the
    // specification has is never given a second tag.
    for published in ["SenderCompID", "TargetCompID", "BeginString"] {
        assert!(
            !held.iter().any(|field| field.name() == published),
            "{published} is FIX's own",
        );
    }

    // Registering is idempotent in the sense that matters: the same fields
    // twice is a replacement, never a conflict.
    let registry = FixRegistry::new()
        .with_ulbridge_fields()
        .unwrap()
        .with_ulbridge_fields()
        .unwrap();
    assert_eq!(registry.len(), held.len());
}

#[test]
fn a_branch_folds_once_and_refuses_what_it_cannot_hold() {
    assert_eq!(FixBranch::from_str("CME").unwrap().name(), "cme");
    assert_eq!(FixBranch::from_str("cme").unwrap(), cme());
    assert_eq!(FixBranch::from_str("STD").unwrap().name(), "std");
    assert_eq!(FixBranch::from_str("standard").unwrap().name(), "standard");
    assert!(FixBranch::STANDARD.is_standard());
    assert!(!cme().is_standard());
    assert_eq!(FixBranch::default(), FixBranch::STANDARD);
    assert_eq!(FixBranch::STANDARD.to_string(), "");
    assert_eq!(FixBranch::from_str("a-b.c_9").unwrap().name(), "a-b.c_9");

    let complete = FixBranch::from_parts("CME", "4.4".parse::<Version>().unwrap()).unwrap();
    assert_eq!(complete.name(), "cme");
    assert_eq!(complete.digest(), cme().digest());
    assert_eq!(complete.version(), "4.4".parse::<Version>().unwrap());

    let cases = [
        (" cme", 0),
        ("2cme", 0),
        (".cme", 0),
        ("1:cme", 0),
        ("cm e", 2),
        ("cm,e", 2),
        ("cme/x", 3),
        ("aaaaaaaaaaaaaaaaaaaaaaaa", FixBranch::MAX_LENGTH),
    ];
    for (text, position) in cases {
        let error = FixBranch::from_str(text).unwrap_err();
        match &error {
            Error::Parse {
                target: "fix branch",
                position: at,
                ..
            } => assert_eq!(*at, position, "{text:?}"),
            other => panic!("{text:?}: {other}"),
        }
    }
    // The bound is exactly `smol_str`'s inline capacity, which is what the
    // allocation test holds it to.
    assert_eq!(FixBranch::MAX_LENGTH, 23);
    assert!(FixBranch::from_str(&"a".repeat(FixBranch::MAX_LENGTH)).is_ok());
}

#[test]
fn merge_with_folds_the_fields_and_the_dialects_beside_them() {
    let cme = cme().with_aliases(["globex"]).unwrap();
    let mut dictionary =
        FixRegistry::from_fields([tagged("symbol", 55), identified("VenueSym", &cme, 5_055)])
            .unwrap();
    dictionary.set_branch(cme.clone()).unwrap();

    let incoming = FixBranch::from_parts("cme", "4.4".parse::<Version>().unwrap())
        .unwrap()
        .with_aliases(["cmegroup"])
        .unwrap();
    let mut other = FixRegistry::from_fields([
        tagged("SYMBOL", 55),
        identified("VenueTime", &incoming, 5_060),
    ])
    .unwrap();
    other.set_branch(incoming.clone()).unwrap();

    let (added, merged) = dictionary.merge_with(&other).unwrap();
    assert_eq!((added, merged), (1, 1));
    assert_eq!(dictionary.len(), 3);
    // The incoming spelling wins the field, as it does in any fold.
    assert_eq!(dictionary.field_by_tag(55).unwrap().name(), "SYMBOL");

    // The dialect arrives beside the fields: the incoming record is taken
    // whole, and every spelling either side answered to is kept.
    let held = dictionary.branch_named("cme").expect("the venue dialect");
    assert_eq!(held.version(), "4.4".parse::<Version>().unwrap());
    assert_eq!(held.aliases(), ["globex", "cmegroup"]);
    for spelling in ["globex", "CMEGROUP", "cme"] {
        assert_eq!(
            dictionary.branch_named(spelling).map(FixBranch::name),
            Some("cme"),
            "{spelling}",
        );
    }

    // One mutation: a refusal leaves the dictionary as it was, branches too.
    let before = dictionary.clone();
    let mut refusing = FixRegistry::from_fields([tagged("symbol", 55)]).unwrap();
    refusing
        .insert({
            let mut widened = tagged("SYMBOL", 55);
            widened.set_dtype(DataType::LargeUtf8).unwrap();
            widened
        })
        .unwrap();
    let branch = FixBranch::from_str("blp").unwrap();
    refusing
        .insert(identified("BlpSym", &branch, 5_070))
        .unwrap();
    assert!(dictionary.merge_with(&refusing).is_err());
    assert_eq!(dictionary, before);
    assert!(dictionary.branch_named("blp").is_none());
}

#[test]
fn a_branch_answers_to_its_aliases_and_never_loses_its_own_name() {
    let bloomberg = FixBranch::from_str("bloomberg")
        .unwrap()
        .with_aliases(["BLP", "blpfix"])
        .unwrap();
    // Folded exactly as a name is, and the canonical name is not one of them.
    assert_eq!(bloomberg.aliases(), ["blp", "blpfix"]);
    assert!(bloomberg.has_alias("blp") && bloomberg.has_alias("BLPFIX"));
    assert!(!bloomberg.has_alias("bloomberg"));
    // An alias changes no identity, so nothing a digest keys moves.
    assert_eq!(
        bloomberg.digest(),
        FixBranch::from_str("BLOOMBERG").unwrap().digest()
    );
    assert!(bloomberg.has_identity(&FixBranch::from_str("bloomberg").unwrap()));

    let mut registry =
        FixRegistry::from_fields([identified("VenueSym", &bloomberg, 5_055)]).unwrap();
    registry.set_branch(bloomberg.clone()).unwrap();

    // Every spelling reaches the one dictionary, and the canonical answer is
    // what comes back however it was reached.
    for spelling in ["bloomberg", "BLOOMBERG", "blp", "BLP", "blpfix"] {
        assert_eq!(
            registry.branch_named(spelling).map(FixBranch::name),
            Some("bloomberg"),
            "{spelling}",
        );
    }
    assert!(registry.branch_named("nowhere").is_none());
    // An alias is a lookup spelling and never an identity. A field stores the
    // canonical *name* and nothing else of the dialect, so the branch parsed
    // back out of it declares no alias - the declaration lives in the
    // dictionary, which is where `branch_named` reads it from.
    let stored = registry
        .field_by_tag(5_055)
        .unwrap()
        .as_fix()
        .branch()
        .unwrap();
    assert!(stored.has_identity(&bloomberg));
    assert_eq!(stored.aliases(), [] as [&str; 0]);
    assert_ne!(stored, bloomberg, "a name is not the whole declaration");

    // A canonical name never loses to another dialect's alias for it.
    let shadowing = FixBranch::from_str("blp").unwrap();
    registry.set_branch(shadowing.clone()).unwrap();
    assert_eq!(
        registry.branch_named("blp").map(FixBranch::name),
        Some("blp")
    );
    assert_eq!(
        registry.branch_named("blpfix").map(FixBranch::name),
        Some("bloomberg"),
        "the alias nobody claims still reaches the dialect that declared it",
    );
}

#[test]
fn a_branch_alias_is_held_to_the_grammar_a_branch_name_is() {
    let branch = FixBranch::from_str("bloomberg").unwrap();

    // An alias is parsed as a branch, so one grammar answers for both.
    let error = branch.clone().with_aliases(["2blp"]).unwrap_err();
    assert!(
        matches!(
            &error,
            Error::Parse {
                target: "fix branch",
                ..
            }
        ),
        "{error}",
    );

    // A spelling that already reaches this dictionary is not a second way to.
    for clash in [vec!["BLOOMBERG"], vec!["blp", "BLP"]] {
        let error = branch.clone().with_aliases(clash).unwrap_err();
        assert!(matches!(&error, Error::InvalidRecord { .. }), "{error}");
        assert!(error.to_string().contains("twice"), "{error}");
    }

    // Declaring aliases replaces what was declared before, whole.
    let held = branch
        .clone()
        .with_aliases(["blp"])
        .unwrap()
        .with_aliases(["blpfix"])
        .unwrap();
    assert_eq!(held.aliases(), ["blpfix"]);
    assert_eq!(
        branch.aliases(),
        [] as [&str; 0],
        "the default declares none"
    );
}

#[test]
fn a_forced_branch_digest_collision_is_atomic_and_names_both_branches() {
    let mut registry = FixRegistry::from_fields([tagged("Symbol", 55)]).unwrap();
    let before = registry.clone();
    let collision = FixBranch {
        name: SmolStr::new_static("collision"),
        digest: FixBranch::STANDARD.digest(),
        version: Version::default(),
        aliases: Vec::new(),
    };
    let error = registry.set_branch(collision).unwrap_err();
    assert!(
        matches!(
            &error,
            Error::Conflict { path, .. }
                if path.contains("collision")
                    && path.contains("#00000000")
        ),
        "{error}"
    );
    assert_eq!(registry, before);
}

#[test]
fn an_identifier_is_packed_and_renders_without_inventing_branch_text() {
    const STANDARD: FixId = FixId::standard(35);
    assert_eq!(size_of::<FixId>(), 8);
    assert_eq!(STANDARD, FixId::standard(35));
    assert_eq!(FixBranch::STANDARD.digest(), 0);
    assert_eq!(FixId::standard(35).branch_digest().to_ne_bytes(), [0; 4]);
    assert_eq!(FixId::standard(35).to_string(), "35:");
    let cme = cme();
    let vendor = FixId::from_parts(&cme, 5001).unwrap();
    assert_eq!(vendor.to_string(), format!("5001:#{:08x}", cme.digest()));
    for text in ["35:", "5001:cme", "0:", "39999:cme"] {
        assert_eq!(
            text.parse::<FixId>().unwrap(),
            FixId::from_str(text).unwrap()
        );
    }
    // Case folds on the way in, so one dictionary has one spelling.
    assert_eq!(
        FixId::from_str("5001:CME").unwrap(),
        FixId::from_parts(&cme, 5001).unwrap()
    );
    let id = FixId::from_str("5001:cme").unwrap();
    assert_eq!(id.branch_digest(), cme.digest());
    assert_eq!(id.tag(), 5001);
    assert!(!id.is_standard());
    assert!(FixId::standard(35).is_standard());

    // Ordering is tag-major, then by the unsigned branch digest.
    let mut ids = [
        FixId::from_str("1:").unwrap(),
        FixId::from_str("9000:cme").unwrap(),
        FixId::from_str("0:").unwrap(),
        FixId::from_str("5000:cme").unwrap(),
    ];
    ids.sort();
    assert_eq!(ids.map(FixId::tag), [0, 1, 5000, 9000]);

    // A bare tag is not an identifier, and the tag half may not be empty or
    // signed.
    for text in ["35", "", "cme", ":cme", "+5:cme", "-5:cme"] {
        let error = FixId::from_str(text).unwrap_err();
        assert!(matches!(&error, Error::Parse { .. }), "{text:?}: {error}");
    }
    assert!(matches!(
        FixId::from_str("35:standard").unwrap_err(),
        Error::InvalidMetadataValue { .. }
    ));
    for (text, position) in [("35", 2_usize), ("abc:cme", 0)] {
        match FixId::from_str(text).unwrap_err() {
            Error::Parse {
                target: "fix identifier",
                position: at,
                ..
            } => assert_eq!(at, position, "{text:?}"),
            other => panic!("{text:?}: {other}"),
        }
    }
    assert!(matches!(
        FixId::from_str("5:cme 0").unwrap_err(),
        Error::Parse {
            target: "fix branch",
            position: 5,
            ..
        }
    ));
    // An over-long branch is refused as a branch, in the identifier's
    // own coordinates after the tag and separator.
    let long = format!("5001:{}", "a".repeat(24));
    match FixId::from_str(&long).unwrap_err() {
        Error::Parse {
            target: "fix branch",
            position,
            ..
        } => assert_eq!(position, 5 + FixBranch::MAX_LENGTH),
        other => panic!("{other}"),
    }
    // Parsed text is held to the same rule as constructed parts.
    assert!(
        FixId::from_str("35:cme")
            .unwrap_err()
            .to_string()
            .contains("fix:branch")
    );
}

#[test]
fn the_identifier_finalizer_spreads_the_committed_dictionary_control_bytes() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("config")
        .join("fix");
    let folder = Folder::new(root).unwrap();
    let registry = FixRegistry::from_handle(&folder).unwrap();
    let classes: HashSet<u8> = registry
        .iter()
        .map(|field| control_byte(field.as_fix().id().unwrap().unwrap()))
        .collect();
    assert!(
        classes.len() >= 16,
        "{} fields reached only {} control-byte classes",
        registry.len(),
        classes.len()
    );
}

#[test]
fn properties_round_trip_including_empty_and_single_element_lists() {
    let mut field = DataType::Int64.required_field("OrderQty");
    let view = field.as_fix();
    assert_eq!(view.tag().unwrap(), None);
    assert_eq!(view.tags().unwrap(), Vec::<i32>::new());
    assert_eq!(view.aliases().count(), 0);
    assert_eq!(view.description(), None);

    field.as_fix_mut().set_tag(38).unwrap();
    field.as_fix_mut().set_tags(&[152]).unwrap();
    field.as_fix_mut().set_aliases(["Qty"]).unwrap();
    field
        .as_fix_mut()
        .set_description("Quantity ordered.")
        .unwrap();
    assert_eq!(field.as_fix().tag().unwrap(), Some(38));
    assert_eq!(field.as_fix().tags().unwrap(), [152]);
    assert_eq!(field.as_fix().aliases().collect::<Vec<_>>(), ["Qty"]);
    assert_eq!(field.as_fix().description(), Some("Quantity ordered."));
    assert_eq!(field.get_metadata("fix:tag"), Some("38"));
    assert_eq!(field.get_metadata("fix:tags"), Some("152"));
    assert_eq!(field.get_metadata("fix:aliases"), Some("Qty"));

    // Order is priority and is kept; the aliases walk both ways.
    field.as_fix_mut().set_tags(&[3, 1, 2]).unwrap();
    field
        .as_fix_mut()
        .set_aliases(["Quantity", "Qty", "OrderQuantity"])
        .unwrap();
    assert_eq!(field.as_fix().tags().unwrap(), [3, 1, 2]);
    assert_eq!(field.get_metadata("fix:tags"), Some("3,1,2"));
    assert_eq!(
        field.as_fix().aliases().collect::<Vec<_>>(),
        ["Quantity", "Qty", "OrderQuantity"]
    );
    assert_eq!(
        field.as_fix().aliases().rev().collect::<Vec<_>>(),
        ["OrderQuantity", "Qty", "Quantity"]
    );

    // An empty list removes the property rather than storing "".
    field.as_fix_mut().set_tags(&[]).unwrap();
    field.as_fix_mut().set_aliases(Vec::<&str>::new()).unwrap();
    assert!(!field.has_metadata("fix:tags"));
    assert!(!field.has_metadata("fix:aliases"));
    assert_eq!(field.as_fix().tags().unwrap(), Vec::<i32>::new());
    assert_eq!(field.as_fix().aliases().count(), 0);

    // The value outlives the view it was read through.
    let description = field.as_fix().description();
    assert_eq!(description, Some("Quantity ordered."));
}

#[test]
fn a_property_write_rejects_bad_elements_and_leaves_the_field_unchanged() {
    let mut field = tagged("Symbol", 55);
    field.as_fix_mut().set_aliases(["Ticker"]).unwrap();
    let before = field.clone();

    let refusals = [
        field.as_fix_mut().set_tag(-1).unwrap_err(),
        field.as_fix_mut().set_tags(&[1, -2]).unwrap_err(),
        field.as_fix_mut().set_tags(&[1, 2, 1]).unwrap_err(),
        field.as_fix_mut().set_aliases(["Sym", ""]).unwrap_err(),
        field.as_fix_mut().set_aliases(["Sym,bol"]).unwrap_err(),
        field.as_fix_mut().set_aliases(["Sym", "SYM"]).unwrap_err(),
    ];
    for (index, error) in refusals.iter().enumerate() {
        assert!(
            matches!(error, Error::InvalidMetadataValue { key, .. } if key.starts_with("fix:")),
            "refusal {index}: {error}"
        );
        assert!(error.to_string().contains("expected"), "{error}");
    }
    assert_eq!(field, before, "a refusal changes nothing");
}

#[test]
fn the_branch_round_trips_and_the_standard_one_is_never_stored() {
    let cme = cme();
    let mut field = DataType::Utf8.nullable_field("TradeID");

    // Absent means standard, and no identity without a tag.
    assert_eq!(field.as_fix().branch().unwrap(), FixBranch::STANDARD);
    assert_eq!(field.as_fix().id().unwrap(), None);
    assert!(!field.has_metadata("fix:branch"));

    field.as_fix_mut().set_branch(&cme).unwrap();
    assert_eq!(field.as_fix().branch().unwrap(), cme);
    assert_eq!(field.get_metadata("fix:branch"), Some("cme"));
    assert_eq!(field.as_fix().id().unwrap(), None, "still no tag");

    field.as_fix_mut().set_tag(5001).unwrap();
    let id = field.as_fix().id().unwrap().unwrap();
    assert_eq!(id.to_string(), format!("5001:#{:08x}", cme.digest()));
    assert_eq!(id, FixId::from_str("5001:cme").unwrap());

    // The canonical answer is the folded spelling, whatever was written.
    let mut shouted = DataType::Utf8.nullable_field("TradeID");
    shouted
        .as_fix_mut()
        .set_branch(&FixBranch::from_str("CME").unwrap())
        .unwrap();
    assert_eq!(shouted.get_metadata("fix:branch"), Some("cme"));

    // Setting the standard branch removes the property rather than
    // storing "standard", so one declaration has one stored form.
    field.as_fix_mut().set_branch(&FixBranch::STANDARD).unwrap();
    assert!(!field.has_metadata("fix:branch"));
    assert_eq!(field.as_fix().branch().unwrap(), FixBranch::STANDARD);
    assert_eq!(
        field.as_fix().id().unwrap(),
        Some(FixId::standard(5001)),
        "the identity follows both halves"
    );
}

#[test]
fn a_specification_tag_belongs_to_the_standard_branch_at_every_door() {
    let cme = cme();
    let refusal = |error: &Error| {
        let Error::InvalidMetadataValue { key, reason } = error else {
            panic!("{error}");
        };
        assert_eq!(key, "fix:branch");
        assert!(reason.contains("5000"), "{reason}");
        assert!(reason.contains("\"cme\""), "{reason}");
    };

    // The constructor is the one implementation, and both boundaries matter.
    refusal(&FixId::from_parts(&cme, 35).unwrap_err());
    refusal(&FixId::from_parts(&cme, 4_999).unwrap_err());
    refusal(&FixId::from_parts(&cme, 40_000).unwrap_err());
    assert!(FixId::from_parts(&cme, 5_000).is_ok());
    assert!(FixId::from_parts(&cme, 39_999).is_ok());
    assert_eq!(FixId::USER_TAG_MIN, 5_000);
    assert_eq!(FixId::USER_TAG_MAX, 40_000);
    // The rule is one-way: the standard branch holds any tag.
    assert!(FixId::from_parts(&FixBranch::STANDARD, 40_000).is_ok());
    assert!(FixId::from_parts(&FixBranch::STANDARD, i32::MAX).is_ok());
    assert!(FixId::from_parts(&FixBranch::STANDARD, 0).is_ok());

    // `set_branch` on a field whose canonical tag is a specification one.
    let mut field = tagged("Symbol", 55);
    let before = field.clone();
    refusal(&field.as_fix_mut().set_branch(&cme).unwrap_err());
    assert_eq!(field, before, "a refusal changes nothing");

    // The same for a field whose *alternate* tag is one: an alternate
    // resolves as strongly as a canonical tag.
    let mut alternate = identified("TradeID", &cme, 5001);
    alternate.as_fix_mut().set_tags(&[5002]).unwrap();
    let mut standard = DataType::Utf8.nullable_field("TradeID");
    standard.as_fix_mut().set_tag(5001).unwrap();
    standard.as_fix_mut().set_tags(&[35, 5002]).unwrap();
    let before = standard.clone();
    refusal(&standard.as_fix_mut().set_branch(&cme).unwrap_err());
    assert_eq!(standard, before);

    // `set_tag` in a vendor branch: refused, never a silent renamespacing.
    let mut vendor = identified("TradeID", &cme, 5001);
    let before = vendor.clone();
    refusal(&vendor.as_fix_mut().set_tag(35).unwrap_err());
    assert_eq!(vendor, before);
    // `set_tags` likewise.
    refusal(&vendor.as_fix_mut().set_tags(&[5002, 35]).unwrap_err());
    assert_eq!(vendor, before);
    assert!(vendor.as_fix_mut().set_tags(&[5002]).is_ok());

    // Read back from raw metadata a hand edit could have written: refused at
    // the door, so nothing corrupt is ever indexed.
    let mut edited = tagged("MsgType", 35);
    edited.insert_metadata("fix:branch", "cme").unwrap();
    refusal(&edited.as_fix().id().unwrap_err());
    let error = FixRegistry::new().insert(edited).unwrap_err();
    refusal(&error);
}

#[test]
fn set_id_moves_both_halves_in_either_direction_and_atomically() {
    let cme = cme();
    let vendor = FixId::from_parts(&cme, 5001).unwrap();

    // Standard to vendor: the order `set_tag` then `set_branch` refuses.
    let mut field = tagged("Symbol", 55);
    assert!(field.as_fix_mut().set_branch(&cme).is_err());
    field.as_fix_mut().set_id(&cme, 5001).unwrap();
    assert_eq!(field.as_fix().id().unwrap(), Some(vendor));
    assert_eq!(field.get_metadata("fix:branch"), Some("cme"));
    assert_eq!(field.get_metadata("fix:tag"), Some("5001"));

    // Vendor back to standard, which the other single setter refuses too.
    assert!(field.as_fix_mut().set_tag(35).is_err());
    field.as_fix_mut().set_id(&FixBranch::STANDARD, 35).unwrap();
    assert_eq!(field.as_fix().id().unwrap(), Some(FixId::standard(35)));
    assert!(!field.has_metadata("fix:branch"));

    // A refused tag restores the branch entry it had already written.
    let mut vendored = identified("TradeID", &cme, 5001);
    let before = vendored.clone();
    let error = vendored
        .as_fix_mut()
        .set_id(&FixBranch::STANDARD, -1)
        .unwrap_err();
    assert!(error.to_string().contains("fix:tag"), "{error}");
    assert_eq!(vendored, before, "the branch came back");
    assert_eq!(vendored.get_metadata("fix:branch"), Some("cme"));

    // The same unwinding from a field that declared no branch: the
    // removal is undone by leaving the property absent. The tag's own shape
    // is the only half of a legal `FixId` that can still be refused, because
    // the identifier carries the branch rule already.
    let mut plain = tagged("Symbol", 55);
    let before = plain.clone();
    assert!(plain.as_fix_mut().set_id(&FixBranch::STANDARD, -1).is_err());
    assert_eq!(plain, before);
    assert!(!plain.has_metadata("fix:branch"));
    assert!(
        FixId::from_parts(&cme, -1).is_err(),
        "and never in a vendor one"
    );
}

#[test]
fn two_branches_may_hold_the_same_tag_and_the_same_name() {
    let cme = cme();
    let standard = FixBranch::STANDARD;
    let mut venue = identified("Symbol", &cme, 5055);
    venue.as_fix_mut().set_aliases(["Ticker"]).unwrap();
    venue.as_fix_mut().set_tags(&[9055]).unwrap();
    let mut spec = tagged("Symbol", 5055);
    spec.as_fix_mut().set_aliases(["Ticker"]).unwrap();
    spec.as_fix_mut().set_tags(&[9055]).unwrap();

    let registry = FixRegistry::from_fields([venue.clone(), spec.clone()]).unwrap();
    assert_eq!(registry.len(), 2);

    // Each identifier answers its own field, and each name in its own
    // dictionary.
    assert_eq!(registry.field_by_id(FixId::standard(5055)).unwrap(), &spec);
    assert_eq!(
        registry
            .field_by_id(FixId::from_parts(&cme, 5055).unwrap())
            .unwrap(),
        &venue
    );
    assert_eq!(
        registry
            .field_by_id(FixId::from_parts(&cme, 9055).unwrap())
            .unwrap(),
        &venue,
        "an alternate identifier resolves in its branch too"
    );
    assert_eq!(
        registry.field_by_name("SYMBOL", Some(&cme)).unwrap(),
        &venue
    );
    assert_eq!(
        registry.field_by_name("symbol", Some(&standard)).unwrap(),
        &spec
    );
    assert_eq!(
        registry.field_by_name("ticker", Some(&cme)).unwrap(),
        &venue
    );
    assert_eq!(
        registry.field_by_name("ticker", Some(&standard)).unwrap(),
        &spec
    );

    // The standard canonical definitions win the omitted-branch lookup.
    assert_eq!(registry.get_field_by_tag(5055), Some(&spec));
    assert_eq!(registry.get_field_by_tag(9055), Some(&spec));
    assert_eq!(registry.get_field("Symbol"), Some(&spec));
    assert_eq!(registry.get_field("Ticker"), Some(&spec));
    // A colon-bearing string is a name, never an identifier.
    assert!(registry.get_field("5055:cme").is_none());
    assert!(registry.get_field_by_name("absent", Some(&cme)).is_none());
    assert!(
        registry
            .get_field_by_id(FixId::from_parts(&cme, 6000).unwrap())
            .is_none()
    );

    // A conflict inside one branch is still a conflict, and it names that
    // branch; the same key in the other branch is not.
    let mut twice = identified("VenueSym", &cme, 5099);
    twice.as_fix_mut().set_aliases(["TICKER"]).unwrap();
    let mut probed = registry.clone();
    let error = probed.insert(twice.clone()).unwrap_err();
    assert!(
        matches!(&error, Error::Conflict { path, .. }
            if path == "alias \"TICKER\" in branch \"cme\" of VenueSym, held by Symbol"),
        "{error}"
    );
    assert_eq!(probed, registry);
    let mut moved = twice;
    moved
        .as_fix_mut()
        .set_id(&FixBranch::STANDARD, 5099)
        .unwrap();
    let error = probed.insert(moved).unwrap_err();
    assert!(
        matches!(&error, Error::Conflict { path, .. }
            if path.contains("in branch \"\"")),
        "{error}"
    );
    assert_eq!(probed, registry);

    // The failing halves name the key the way it was asked.
    let by_id = registry
        .field_by_id(FixId::from_parts(&cme, 6000).unwrap())
        .unwrap_err();
    assert!(
        matches!(&by_id, Error::Absent { expected: "fix field", path } if path.starts_with("identifier 6000:#")),
        "{by_id}"
    );
    // The specialized and generic pairs answer alike for an identifier.
    let id = FixId::standard(5055);
    assert_eq!(registry.get_field(id), registry.get_field_by_id(id));
    assert_eq!(registry.get_field(FixKey::Id(id)), Some(&spec));
    assert_eq!(
        registry.field(id).map(Field::name).ok(),
        registry.field_by_id(id).map(Field::name).ok()
    );
    assert!(registry.contains(id));
}

#[test]
fn an_omitted_branch_infers_one_deterministic_best_name_match() {
    let alpha = FixBranch::from_str("alpha").unwrap();
    let cme = cme();

    let standard_shared = tagged("Shared", 6_000);
    let cme_shared = identified("Shared", &cme, 6_001);

    let mut standard_alias = tagged("StandardAliasHolder", 6_002);
    standard_alias
        .as_fix_mut()
        .set_aliases(["VenueCanonical"])
        .unwrap();
    let venue_canonical = identified("VenueCanonical", &cme, 6_003);

    let alpha_tie = identified("VendorTie", &alpha, 6_004);
    let cme_tie = identified("VendorTie", &cme, 6_005);

    let registry = FixRegistry::from_fields([
        cme_tie.clone(),
        venue_canonical.clone(),
        cme_shared,
        standard_alias,
        standard_shared.clone(),
        alpha_tie.clone(),
    ])
    .unwrap();

    assert_eq!(
        registry.get_field_by_name("shared", None),
        Some(&standard_shared)
    );
    assert_eq!(
        registry.get_field_by_name("VenueCanonical", None),
        Some(&venue_canonical),
        "a canonical vendor name beats a standard alias"
    );
    assert_eq!(
        registry.get_field_by_name("vendortie", None),
        Some(&alpha_tie),
        "named branches tie-break by canonical spelling, not insertion order"
    );
    assert_eq!(registry.get_field("vendortie"), Some(&alpha_tie));
    assert_eq!(
        registry.get_field_by_name("VendorTie", Some(&cme)),
        Some(&cme_tie)
    );
}

#[test]
fn a_corrupt_stored_property_is_reported_under_its_full_key() {
    let cases = [
        ("fix:branch", "2cme"),
        ("fix:branch", "c me"),
        ("fix:branch", "1:cme"),
        ("fix:branch", "aaaaaaaaaaaaaaaaaaaaaaaa"),
        ("fix:tag", "3x"),
        ("fix:tag", "+35"),
        ("fix:tag", "-35"),
        ("fix:tag", ""),
        ("fix:tags", "1,,2"),
        ("fix:tags", "1,1"),
        ("fix:tags", "1, 2"),
        ("fix:tags", "1,-2"),
    ];
    for (key, stored) in cases {
        let mut field = tagged("Symbol", 55);
        field.insert_metadata(key, stored).unwrap();
        let error = match key {
            "fix:branch" => field.as_fix().branch().unwrap_err(),
            "fix:tag" => field.as_fix().tag().unwrap_err(),
            _ => field.as_fix().tags().unwrap_err(),
        };
        match &error {
            Error::InvalidMetadataValue { key: named, reason } => {
                assert_eq!(named, key);
                assert!(reason.contains(&format!("{stored:?}")), "{reason}");
            }
            other => panic!("{key}={stored:?}: {other}"),
        }
        // A field whose tag is corrupt never enters a registry.
        let error = FixRegistry::new().insert(field).unwrap_err();
        assert!(
            matches!(error, Error::InvalidMetadataValue { .. }),
            "{error}"
        );
    }

    // A stored empty alias element is skipped on read, never reported.
    let mut field = tagged("Symbol", 55);
    field
        .insert_metadata("fix:aliases", ",Ticker,,Sym,")
        .unwrap();
    assert_eq!(
        field.as_fix().aliases().collect::<Vec<_>>(),
        ["Ticker", "Sym"]
    );
}

#[test]
fn a_field_without_a_tag_never_enters() {
    let error = FixRegistry::new()
        .insert(DataType::Utf8.nullable_field("Symbol"))
        .unwrap_err();
    assert!(error.is_absent(), "{error}");
    let message = error.to_string();
    assert!(message.contains("fix:tag"), "{message}");
    assert!(message.contains("Symbol"), "{message}");

    let error = FixRegistry::new()
        .update(DataType::Utf8.nullable_field("Symbol"))
        .unwrap_err();
    assert!(error.is_absent(), "{error}");
}

#[test]
fn a_name_or_alias_resolves_in_any_case_to_the_canonical_spelling() {
    let registry = FixRegistry::from_fields([
        full("Symbol", 55, &[], &["Ticker", "SecuritySymbol"]),
        full("ClOrdID", 11, &[], &["ClientOrderID"]),
    ])
    .unwrap();

    for query in [
        "Symbol",
        "SYMBOL",
        "symbol",
        "Ticker",
        "TICKER",
        "securitysymbol",
    ] {
        assert_eq!(
            registry
                .field_by_name(query, Some(&FixBranch::STANDARD))
                .unwrap()
                .name(),
            "Symbol",
            "{query}"
        );
        assert_eq!(registry.field(query).unwrap().name(), "Symbol", "{query}");
        assert!(registry.contains(query), "{query}");
    }
    assert_eq!(
        registry
            .field_by_name("clientorderid", Some(&FixBranch::STANDARD))
            .unwrap()
            .name(),
        "ClOrdID"
    );
    assert!(
        registry
            .get_field_by_name("Symbols", Some(&FixBranch::STANDARD))
            .is_none()
    );
    assert!(!registry.contains("Symbols"));
}

#[test]
fn tier_order_never_lets_an_alternate_key_shadow_a_canonical_one() {
    // `Px` is Price's canonical name and also an alias LastPx declares; the
    // canonical claim wins whatever order the fields entered in. The same
    // holds for a tag: 31 is LastPx's own tag and Price lists it as an
    // alternate.
    let price = full("Px", 44, &[31], &["Price"]);
    let last = full("LastPx", 31, &[], &["Px", "LastPrice"]);
    for order in [[price.clone(), last.clone()], [last, price]] {
        let registry = FixRegistry::from_fields(order).unwrap();
        assert_eq!(
            registry
                .field_by_name("px", Some(&FixBranch::STANDARD))
                .unwrap()
                .name(),
            "Px"
        );
        assert_eq!(
            registry
                .field_by_name("Price", Some(&FixBranch::STANDARD))
                .unwrap()
                .name(),
            "Px"
        );
        assert_eq!(
            registry
                .field_by_name("LastPrice", Some(&FixBranch::STANDARD))
                .unwrap()
                .name(),
            "LastPx"
        );
        assert_eq!(registry.field_by_tag(31).unwrap().name(), "LastPx");
        assert_eq!(registry.field_by_tag(44).unwrap().name(), "Px");
    }
}

#[test]
fn a_tag_query_never_consults_names_and_a_name_query_never_consults_tags() {
    let registry = FixRegistry::from_fields([tagged("35", 1), tagged("MsgType", 35)]).unwrap();
    assert_eq!(registry.field_by_tag(35).unwrap().name(), "MsgType");
    assert_eq!(registry.field_by_tag(1).unwrap().name(), "35");
    assert_eq!(
        registry
            .field_by_name("35", Some(&FixBranch::STANDARD))
            .unwrap()
            .name(),
        "35"
    );
    assert!(
        registry
            .get_field_by_name("1", Some(&FixBranch::STANDARD))
            .is_none()
    );
    assert!(registry.get_field_by_tag(2).is_none());
}

#[test]
fn an_insert_conflict_names_both_fields_for_each_key_kind() {
    let stored = full("Symbol", 55, &[65], &["Ticker"]);
    let registry = FixRegistry::from_fields([stored]).unwrap();
    let cases = [
        (full("SymbolSfx", 55, &[], &[]), "identifier 55:"),
        (
            full("symbol", 56, &[], &[]),
            "name \"symbol\" in branch \"\"",
        ),
        (
            full("SymbolSfx", 56, &[65], &[]),
            "alternate identifier 65:",
        ),
        (
            full("SymbolSfx", 56, &[], &["TICKER"]),
            "alias \"TICKER\" in branch \"\"",
        ),
    ];
    for (incoming, key) in cases {
        let mut probed = registry.clone();
        let error = probed.insert(incoming).unwrap_err();
        let Error::Conflict { path, .. } = &error else {
            panic!("{key}: {error}");
        };
        assert!(path.contains(key), "{path}");
        assert!(
            path.contains("SymbolSfx") || path.contains("symbol"),
            "{path}"
        );
        assert!(path.ends_with(", held by Symbol"), "{path}");
        assert_eq!(probed, registry, "{key}: a refusal changes nothing");
        assert_eq!(format!("{probed:?}"), format!("{registry:?}"));
        assert_eq!(probed.len(), 1);
    }

    // Overlap across tiers is not a conflict.
    let mut registry = registry;
    assert_eq!(
        registry
            .insert(full("Ticker", 56, &[55], &["Symbol"]))
            .unwrap(),
        None
    );
    assert_eq!(
        registry
            .field_by_name("Ticker", Some(&FixBranch::STANDARD))
            .unwrap()
            .name(),
        "Ticker"
    );
    assert_eq!(
        registry
            .field_by_name("Symbol", Some(&FixBranch::STANDARD))
            .unwrap()
            .name(),
        "Symbol"
    );
    assert_eq!(registry.field_by_tag(55).unwrap().name(), "Symbol");
}

#[test]
fn reinserting_the_same_identity_replaces_wholesale() {
    let mut registry = FixRegistry::from_fields([
        full("Symbol", 55, &[65], &["Ticker"]),
        full("Price", 44, &[], &["Px"]),
    ])
    .unwrap();

    // Same tag, same folded name: the prior definition comes back whole and
    // its old keys are gone.
    let replacement = full("SYMBOL", 55, &[66], &["Sym"]);
    let prior = registry.insert(replacement.clone()).unwrap().unwrap();
    assert_eq!(prior.name(), "Symbol");
    assert_eq!(prior.as_fix().tags().unwrap(), [65]);
    assert_eq!(registry.field_by_tag(55).unwrap(), &replacement);
    assert_eq!(
        registry
            .field_by_name("symbol", Some(&FixBranch::STANDARD))
            .unwrap()
            .name(),
        "SYMBOL"
    );
    assert_eq!(registry.field_by_tag(66).unwrap().name(), "SYMBOL");
    assert_eq!(
        registry
            .field_by_name("Sym", Some(&FixBranch::STANDARD))
            .unwrap()
            .name(),
        "SYMBOL"
    );
    assert!(registry.get_field_by_tag(65).is_none());
    assert!(
        registry
            .get_field_by_name("Ticker", Some(&FixBranch::STANDARD))
            .is_none()
    );
    assert_eq!(registry.len(), 2);

    // A tag matching one field and a name matching another is never a
    // replacement, and a new key another field holds refuses the whole thing.
    let before = registry.clone();
    let error = registry.insert(full("Price", 55, &[], &[])).unwrap_err();
    assert!(error.is_conflict(), "{error}");
    let error = registry
        .insert(full("Symbol", 55, &[], &["px"]))
        .unwrap_err();
    assert!(
        matches!(&error, Error::Conflict { path, .. } if path == "alias \"px\" in branch \"\" of Symbol, held by Price"),
        "{error}"
    );
    assert_eq!(registry, before);
    assert_eq!(
        probe(&registry, 55, 66, "SYMBOL", "Sym"),
        [Some("SYMBOL"); 4]
    );
}

#[test]
fn a_merge_follows_the_truth_table() {
    let mut stored = full("Symbol", 55, &[65, 66], &["Ticker", "Sym"]);
    stored.insert_metadata("display", "Symbol").unwrap();
    stored.insert_metadata("owner", "stored").unwrap();
    let mut registry = FixRegistry::from_fields([stored]).unwrap();

    let mut incoming = DataType::Utf8.required_field("SYMBOL");
    incoming.as_fix_mut().set_tag(55).unwrap();
    incoming.as_fix_mut().set_tags(&[67, 66]).unwrap();
    incoming
        .as_fix_mut()
        .set_aliases(["Instrument", "sym"])
        .unwrap();
    incoming
        .insert_metadata("display", "Ticker symbol")
        .unwrap();
    incoming.insert_metadata("source", "incoming").unwrap();
    registry.update(incoming).unwrap();

    let merged = registry.field_by_tag(55).unwrap();
    // The incoming field wins the spelling, nullability and every shared key.
    assert_eq!(merged.name(), "SYMBOL");
    assert!(!merged.is_nullable());
    assert_eq!(merged.display(), Some("Ticker symbol"));
    // The stored field keeps what only it declared.
    assert_eq!(merged.get_metadata("owner"), Some("stored"));
    assert_eq!(merged.get_metadata("source"), Some("incoming"));
    assert_eq!(merged.as_fix().description(), Some("Symbol described"));
    // Lists concatenate, incoming first, deduplicated with case folded.
    assert_eq!(merged.as_fix().tags().unwrap(), [67, 66, 65]);
    assert_eq!(
        merged.as_fix().aliases().collect::<Vec<_>>(),
        ["Instrument", "sym", "Ticker"]
    );
    // Every key, old and new, resolves to the merged field.
    for tag in [55, 65, 66, 67] {
        assert_eq!(
            registry.field_by_tag(tag).unwrap().name(),
            "SYMBOL",
            "{tag}"
        );
    }
    for name in ["symbol", "ticker", "SYM", "instrument"] {
        assert_eq!(
            registry
                .field_by_name(name, Some(&FixBranch::STANDARD))
                .unwrap()
                .name(),
            "SYMBOL",
            "{name}"
        );
    }
    assert_eq!(registry.len(), 1);

    // A merge that adds nothing is a no-op.
    let before = registry.clone();
    registry.update(tagged("symbol", 55)).unwrap();
    assert_eq!(
        registry.field_by_tag(55).unwrap().as_fix().tags().unwrap(),
        [67, 66, 65]
    );
    assert_eq!(registry.field_by_tag(55).unwrap().name(), "symbol");
    assert_eq!(registry.len(), before.len());
}

#[test]
fn a_rejected_merge_leaves_the_registry_untouched() {
    let mut registry = FixRegistry::from_fields([
        full("Symbol", 55, &[65], &["Ticker"]),
        full("Price", 44, &[31], &["Px"]),
    ])
    .unwrap();
    let before = registry.clone();
    let snapshot = format!("{registry:?}");

    // A datatype disagreement names both datatypes and never widens.
    let mut widened = tagged("Symbol", 55);
    widened.set_dtype(DataType::LargeUtf8).unwrap();
    let error = registry.update(widened).unwrap_err();
    assert!(matches!(error, Error::InvalidRecord { .. }), "{error}");
    let message = error.to_string();
    assert!(
        message.contains("utf8") && message.contains("large_utf8"),
        "{message}"
    );

    // A name disagreement names both spellings.
    let error = registry.update(tagged("Sym", 55)).unwrap_err();
    assert!(matches!(error, Error::InvalidRecord { .. }), "{error}");
    let message = error.to_string();
    assert!(
        message.contains("\"Symbol\"") && message.contains("\"Sym\""),
        "{message}"
    );

    // A merged alternate key another field holds is a conflict naming both.
    let error = registry.update(full("Symbol", 55, &[31], &[])).unwrap_err();
    assert!(
        matches!(&error, Error::Conflict { path, .. } if path.ends_with(", held by Price")),
        "{error}"
    );
    let error = registry
        .update(full("Symbol", 55, &[], &["PX"]))
        .unwrap_err();
    assert!(error.is_conflict(), "{error}");

    // An unknown identifier is an absence, not a silent insert.
    let error = registry.update(tagged("Text", 58)).unwrap_err();
    assert!(
        matches!(&error, Error::Absent { path, .. } if path == "identifier 58:"),
        "{error}"
    );

    // A branch disagreement is that same absence: the branch is half
    // of the identity, so the incoming field names no stored one.
    let error = registry
        .update(identified("Symbol", &cme(), 5055))
        .unwrap_err();
    assert!(
        matches!(&error, Error::Absent { path, .. } if path.starts_with("identifier 5055:#")),
        "{error}"
    );

    assert_eq!(registry, before);
    assert_eq!(format!("{registry:?}"), snapshot);
    assert_eq!(
        probe(&registry, 55, 65, "symbol", "ticker"),
        [Some("Symbol"); 4]
    );
    assert_eq!(probe(&registry, 44, 31, "price", "px"), [Some("Price"); 4]);
    assert_eq!(registry.len(), 2);
}

#[test]
fn add_fields_adds_what_is_absent_and_merges_what_is_present() {
    let mut registry =
        FixRegistry::from_fields([full("Symbol", 55, &[65], &["Ticker"]), tagged("Price", 44)])
            .unwrap();

    // Tag 55 is stored and folds; tag 44 is stored under another spelling of
    // the same name and folds too; tag 60 is new. The venue's own 5055 shares
    // the tag of nothing, and its branch is half of the identity.
    let mut priced = tagged("PRICE", 44);
    priced.as_fix_mut().set_aliases(["Px"]).unwrap();
    let (added, merged) = registry
        .add_fields([
            full("Symbol", 55, &[66], &["Sym"]),
            priced,
            tagged("TransactTime", 60),
            identified("Symbol", &cme(), 5_055),
        ])
        .unwrap();
    assert_eq!((added, merged), (2, 2));
    assert_eq!(registry.len(), 4);

    // The merge kept what only the stored field declared and added the rest.
    let symbol = registry.field_by_tag(55).unwrap();
    assert_eq!(symbol.as_fix().tags().unwrap(), [66, 65]);
    assert_eq!(
        symbol.as_fix().aliases().collect::<Vec<_>>(),
        ["Sym", "Ticker"]
    );
    // The incoming spelling wins, which is what makes the caller's order the
    // precedence.
    assert_eq!(registry.field_by_tag(44).unwrap().name(), "PRICE");
    assert_eq!(registry.field_by_tag(60).unwrap().name(), "TransactTime");
    assert_eq!(
        registry
            .field_by_id(FixId::from_parts(&cme(), 5_055).unwrap())
            .unwrap()
            .as_fix()
            .branch()
            .unwrap(),
        cme()
    );

    // The identity is the whole probe: the same tag in another branch is
    // another field, added rather than folded into the specification's.
    let (added, merged) = registry
        .add_fields([identified("TransactTime", &cme(), 5_060)])
        .unwrap();
    assert_eq!((added, merged), (1, 0));
    assert_eq!(registry.len(), 5);
}

#[test]
fn add_fields_refuses_the_way_the_one_field_writes_refuse() {
    let mut registry = FixRegistry::from_fields([tagged("Symbol", 55)]).unwrap();

    // No `fix:tag` is no identity, so there is nothing to add or fold under.
    let error = registry
        .add_fields([DataType::Utf8.nullable_field("Nameless")])
        .unwrap_err();
    assert!(error.is_absent(), "{error}");
    assert!(error.to_string().contains("fix:tag"), "{error}");

    // A datatype that disagrees with the stored definition is refused, never
    // widened - the shape a CBlock's generic `float` takes against a stored
    // `float64`.
    let mut widened = tagged("Symbol", 55);
    widened.set_dtype(DataType::LargeUtf8).unwrap();
    let error = registry.add_fields([widened]).unwrap_err();
    assert!(matches!(error, Error::InvalidRecord { .. }), "{error}");

    // The whole fold is one mutation, so a refusal in the middle of it leaves
    // the dictionary exactly as it was - neither the field before nor the one
    // after arrives.
    let before = registry.clone();
    let mut clash = tagged("Symbol", 55);
    clash.set_dtype(DataType::LargeUtf8).unwrap();
    let error = registry
        .add_fields([tagged("Price", 44), clash, tagged("TransactTime", 60)])
        .unwrap_err();
    assert!(matches!(error, Error::InvalidRecord { .. }), "{error}");
    assert_eq!(registry, before);
    assert_eq!(registry.len(), 1);
}

#[test]
fn removal_keeps_every_position_consistent() {
    let mut registry = FixRegistry::from_fields([
        full("Symbol", 55, &[65], &["Ticker"]),
        full("Price", 44, &[45], &["Px"]),
        full("Text", 58, &[59], &["FreeText"]),
    ])
    .unwrap();

    // Removing the first field moves the last into its slot.
    let removed = registry.remove(55).unwrap();
    assert_eq!(removed.name(), "Symbol");
    assert_eq!(registry.len(), 2);
    assert_eq!(probe(&registry, 55, 65, "symbol", "ticker"), [None; 4]);
    assert_eq!(probe(&registry, 44, 45, "price", "px"), [Some("Price"); 4]);
    assert_eq!(
        probe(&registry, 58, 59, "text", "freetext"),
        [Some("Text"); 4]
    );
    assert_eq!(
        registry.iter().map(Field::name).collect::<Vec<_>>(),
        ["Price", "Text"]
    );

    // A name key removes through the alias tier too; a path never does.
    assert!(registry.remove("Symbol").is_none());
    assert_eq!(registry.remove("PX").unwrap().name(), "Price");
    assert_eq!(probe(&registry, 44, 45, "price", "px"), [None; 4]);
    assert_eq!(
        probe(&registry, 58, 59, "text", "freetext"),
        [Some("Text"); 4]
    );
    assert_eq!(registry.len(), 1);
    assert_eq!(registry.remove("FreeText").unwrap().name(), "Text");
    assert!(registry.is_empty());
    assert!(registry.remove(58).is_none());

    // A removed key can be claimed again.
    registry
        .insert(full("Text", 58, &[59], &["FreeText"]))
        .unwrap();
    assert_eq!(
        probe(&registry, 58, 59, "text", "freetext"),
        [Some("Text"); 4]
    );
}

#[test]
fn specialized_and_generic_accessors_answer_alike_for_every_key() {
    let registry = FixRegistry::from_fields([
        full("Symbol", 55, &[65], &["Ticker"]),
        full("Price", 44, &[], &[]),
    ])
    .unwrap();

    // Tag hit, alternate-tag hit, and a tag miss.
    for tag in [55, 65, 44, 1] {
        assert_eq!(
            registry.get_field(tag),
            registry.get_field_by_tag(tag),
            "{tag}"
        );
        assert_eq!(
            registry.get_field(FixKey::Tag(tag)),
            registry.get_field_by_tag(tag)
        );
        assert_eq!(
            registry.field(tag).map(Field::name).ok(),
            registry.field_by_tag(tag).map(Field::name).ok()
        );
        assert_eq!(
            registry.contains(tag),
            registry.get_field_by_tag(tag).is_some()
        );
    }
    // Name hit, alias hit, and a name miss.
    for name in ["symbol", "TICKER", "price", "absent"] {
        assert_eq!(
            registry.get_field(name),
            registry.get_field_by_name(name, Some(&FixBranch::STANDARD)),
            "{name}"
        );
        assert_eq!(
            registry.get_field(&name.to_owned()),
            registry.get_field_by_name(name, Some(&FixBranch::STANDARD))
        );
        assert_eq!(
            registry.field(name).map(Field::name).ok(),
            registry
                .field_by_name(name, Some(&FixBranch::STANDARD))
                .map(Field::name)
                .ok()
        );
        assert_eq!(
            registry.contains(name),
            registry
                .get_field_by_name(name, Some(&FixBranch::STANDARD))
                .is_some()
        );
    }

    // The failing halves name the key the way it was asked.
    let by_tag = registry.field_by_tag(1).unwrap_err();
    assert!(matches!(&by_tag, Error::Absent { expected: "fix field", path } if path == "tag 1"));
    let by_name = registry
        .field_by_name("absent", Some(&FixBranch::STANDARD))
        .unwrap_err();
    assert!(matches!(&by_name, Error::Absent { path, .. } if path == "name \"absent\""));
    let by_path = registry
        .field_by_path("Symbol.absent", Some(&FixBranch::STANDARD))
        .unwrap_err();
    assert!(matches!(&by_path, Error::Absent { path, .. } if path == "path \"Symbol.absent\""));
    assert_eq!(
        registry.field(1).unwrap_err().to_string(),
        by_tag.to_string()
    );
    assert_eq!(
        registry.field("Symbol.absent").unwrap_err().to_string(),
        by_path.to_string()
    );
}

#[test]
fn a_path_reaches_a_component_member_and_a_repeating_group_member() {
    let mut party_id = DataType::Utf8.nullable_field("PartyID");
    party_id.as_fix_mut().set_tag(448).unwrap();
    let mut role = DataType::Int32.nullable_field("PartyRole");
    role.as_fix_mut().set_tag(452).unwrap();
    let mut group = DataType::list(
        DataType::from_fields([party_id.clone(), role])
            .unwrap()
            .required_field("item"),
    )
    .nullable_field("NoPartyIDs");
    group.as_fix_mut().set_tag(453).unwrap();
    let mut instrument = DataType::from_fields([tagged("Symbol", 55), tagged("SecurityID", 48)])
        .unwrap()
        .nullable_field("Instrument");
    instrument.as_fix_mut().set_tag(1000).unwrap();
    instrument.as_fix_mut().set_aliases(["Instr"]).unwrap();

    let registry = FixRegistry::from_fields([group, instrument]).unwrap();
    assert_eq!(
        registry
            .field_by_path("NoPartyIDs", Some(&FixBranch::STANDARD))
            .unwrap()
            .as_fix()
            .tag()
            .unwrap(),
        Some(453)
    );
    assert_eq!(
        registry
            .field_by_path("NoPartyIDs.PartyID", Some(&FixBranch::STANDARD))
            .unwrap(),
        &party_id
    );
    assert_eq!(
        registry
            .field_by_path("nopartyids.PartyRole", Some(&FixBranch::STANDARD))
            .unwrap()
            .name(),
        "PartyRole"
    );
    assert_eq!(
        registry
            .field_by_path("Instrument.Symbol", Some(&FixBranch::STANDARD))
            .unwrap()
            .name(),
        "Symbol"
    );
    assert_eq!(
        registry
            .field_by_path("instr.SecurityID", Some(&FixBranch::STANDARD))
            .unwrap()
            .name(),
        "SecurityID"
    );
    assert_eq!(
        registry.get_field("Instrument.Symbol"),
        registry.get_field_by_path("Instrument.Symbol", Some(&FixBranch::STANDARD))
    );
    assert!(registry.contains("NoPartyIDs.PartyID"));
    // A member is reached through its parent only: the registry does not
    // index it.
    assert!(
        registry
            .get_field_by_name("PartyID", Some(&FixBranch::STANDARD))
            .is_none()
    );
    // The remainder of a path folds like the head does. One function that
    // folded its first segment and matched the rest exactly would refuse
    // `NoPartyIDs.PartyID` on a dictionary that stores its members folded,
    // which is every dictionary this crate writes.
    assert_eq!(
        registry.get_field_by_path("NoPartyIDs.partyid", Some(&FixBranch::STANDARD)),
        registry.get_field_by_path("NoPartyIDs.PartyID", Some(&FixBranch::STANDARD)),
    );
    assert_eq!(
        registry
            .get_field_by_path("NoPartyIDs.PARTY_ID", Some(&FixBranch::STANDARD))
            .map(Field::name),
        Some("PartyID"),
    );
    assert!(
        registry
            .get_field_by_path("Instrument.Absent", Some(&FixBranch::STANDARD))
            .is_none()
    );
    assert!(
        registry
            .get_field_by_path("Absent.Symbol", Some(&FixBranch::STANDARD))
            .is_none()
    );
}

#[test]
fn iteration_follows_the_canonical_tag_and_equality_ignores_order() {
    let fields = [
        tagged("Text", 58),
        tagged("Symbol", 55),
        tagged("Account", 1),
    ];
    let registry = FixRegistry::from_fields(fields.clone()).unwrap();
    let mut reversed = fields.clone();
    reversed.reverse();
    let other = FixRegistry::from_fields(reversed).unwrap();

    let mut iter = registry.iter();
    assert_eq!(iter.len(), 3);
    assert_eq!(iter.next().map(Field::name), Some("Account"));
    assert_eq!(iter.next_back().map(Field::name), Some("Text"));
    assert_eq!(iter.len(), 1);
    assert_eq!(iter.next().map(Field::name), Some("Symbol"));
    assert!(iter.next().is_none());
    assert_eq!(
        (&registry).into_iter().map(Field::name).collect::<Vec<_>>(),
        ["Account", "Symbol", "Text"]
    );
    // The cursor form walks the same order, and a binding advancing it with
    // only the last identifier it saw sees every field exactly once.
    let mut walked = Vec::new();
    let mut cursor = None;
    while let Some(field) = registry.next_field_after(cursor) {
        walked.push(field.name());
        cursor = field.as_fix().id().unwrap();
    }
    assert_eq!(walked, ["Account", "Symbol", "Text"]);
    assert!(
        registry
            .next_field_after(Some(FixId::standard(i32::MAX)))
            .is_none()
    );
    assert!(FixRegistry::new().next_field_after(None).is_none());
    // An alternate tag is an index entry, never a cursor stop.
    let mut aliased = tagged("MsgType", 35);
    aliased.as_fix_mut().set_tags(&[2]).unwrap();
    let with_alternate = FixRegistry::from_fields([aliased, tagged("Account", 1)]).unwrap();
    assert_eq!(
        with_alternate
            .next_field_after(Some(FixId::standard(1)))
            .map(Field::name)
            .unwrap(),
        "MsgType"
    );

    assert_eq!(registry, other);
    assert_ne!(registry, FixRegistry::new());
    assert_eq!(FixRegistry::new(), FixRegistry::default());
    assert!(FixRegistry::new().is_empty());
    assert_eq!(FixRegistry::new().iter().len(), 0);

    // A conflict anywhere fails the whole build.
    let error = FixRegistry::from_fields([tagged("Text", 58), tagged("Symbol", 58)]).unwrap_err();
    assert!(error.is_conflict(), "{error}");

    // Debug renders the fields under their identifiers, in order.
    let rendered = format!("{registry:?}");
    assert!(rendered.starts_with("{\"1:\": "), "{rendered}");
    assert!(rendered.find("\"55:\"").unwrap() < rendered.find("\"58:\"").unwrap());
}

#[test]
fn iteration_and_the_cursor_are_tag_major() {
    let cme = cme();
    let registry = FixRegistry::from_fields([
        tagged("Account", 1),
        identified("TradeID", &cme, 5001),
        tagged("MsgType", 35),
        identified("Venue", &cme, 9000),
    ])
    .unwrap();
    // Tags lead across branches; the digest only orders an equal tag.
    assert_eq!(
        registry.iter().map(Field::name).collect::<Vec<_>>(),
        ["Account", "MsgType", "TradeID", "Venue"]
    );
    let mut walked = Vec::new();
    let mut cursor = None;
    while let Some(field) = registry.next_field_after(cursor) {
        walked.push(field.name());
        cursor = field.as_fix().id().unwrap();
    }
    assert_eq!(walked, ["Account", "MsgType", "TradeID", "Venue"]);
    let rendered = format!("{registry:?}");
    assert!(rendered.starts_with("{\"1:\": "), "{rendered}");
}

#[test]
fn nestedness_routes_a_field_by_the_core_predicate_alone() {
    // `DataType::is_nested` is the whole rule, and it already unwraps a
    // dictionary to its value type: a dictionary-encoded Struct is nested and
    // a dictionary-encoded Utf8 is not.
    let encoded_text = {
        let mut field = DataType::dictionary(DataType::Int32, DataType::Utf8)
            .unwrap()
            .nullable_field("Coded");
        field.as_fix_mut().set_tag(60).unwrap();
        field
    };
    let encoded_struct = {
        let inner = DataType::from_fields([tagged("Member", 9_003)]).unwrap();
        let mut field = DataType::dictionary(DataType::Int32, inner)
            .unwrap()
            .nullable_field("Boxed");
        field.as_fix_mut().set_tag(61).unwrap();
        field
    };
    let fields = [
        (tagged("Symbol", 55), false),
        (encoded_text, false),
        (component("Instrument", 1_000), true),
        (group("NoPartyIDs", 453), true),
        (encoded_struct, true),
    ];
    let registry = FixRegistry::from_fields(fields.iter().map(|(field, _)| field.clone())).unwrap();
    for (field, nested) in &fields {
        let id = field.as_fix().id().unwrap().unwrap();
        assert_eq!(
            field.dtype().is_nested(),
            *nested,
            "{} is the core predicate's answer",
            field.name()
        );
        assert_eq!(
            registry.get_field_by_id(id).map(Field::name),
            Some(field.name()),
            "{} did not retain its indexed identity",
            field.name()
        );
    }
    assert_eq!(registry.len(), 5);
}

#[test]
fn datatype_shape_does_not_reorder_the_resolution_tiers() {
    // Scalar and nested fields share one identity space. A canonical key
    // therefore beats an alternate key regardless of either field's shape.
    let mut shadowing = tagged("Shadow", 5);
    shadowing.as_fix_mut().set_tags(&[453]).unwrap();
    shadowing.as_fix_mut().set_aliases(["Parties"]).unwrap();
    let mut parties = group("Parties", 453);
    parties.as_fix_mut().set_aliases(["Group"]).unwrap();
    let registry = FixRegistry::from_fields([shadowing.clone(), parties.clone()]).unwrap();
    // The nested field holds 453 and the name "Parties" canonically; the
    // primitive one holds them as an alternate tag and an alias.
    assert_eq!(
        registry.get_field_by_tag(453).map(Field::name),
        Some("Parties")
    );
    assert_eq!(
        registry
            .get_field_by_name("parties", Some(&FixBranch::STANDARD))
            .map(Field::name),
        Some("Parties")
    );
    assert_eq!(
        registry.get_field_by_tag(5).map(Field::name),
        Some("Shadow")
    );

    // The mirror: the primitive field holds them canonically and the nested
    // one as an alternate tag and an alias.
    let mut price = tagged("Price", 44);
    price.as_fix_mut().set_aliases(["Rate"]).unwrap();
    let mut legs = group("NoLegs", 555);
    legs.as_fix_mut().set_tags(&[44]).unwrap();
    legs.as_fix_mut().set_aliases(["Price"]).unwrap();
    let registry = FixRegistry::from_fields([legs.clone(), price.clone()]).unwrap();
    assert_eq!(
        registry.get_field_by_tag(44).map(Field::name),
        Some("Price")
    );
    assert_eq!(
        registry
            .get_field_by_name("PRICE", Some(&FixBranch::STANDARD))
            .map(Field::name),
        Some("Price")
    );
    assert_eq!(
        registry
            .get_field_by_name("rate", Some(&FixBranch::STANDARD))
            .map(Field::name),
        Some("Price"),
        "an alias held by another field still resolves"
    );
    assert_eq!(
        registry.get_field_by_tag(555).map(Field::name),
        Some("NoLegs")
    );
}

#[test]
fn a_nested_field_can_never_claim_a_primitive_field_key() {
    // The identity space is not split, so every conflict a pair of primitives
    // would raise, a primitive and a nested field raise too - naming both.
    let stored = full("Symbol", 55, &[65], &["Ticker"]);
    let registry = FixRegistry::from_fields([stored]).unwrap();
    let mut same_alternate = component("Legs", 5_556);
    same_alternate.as_fix_mut().set_tags(&[65]).unwrap();
    let mut same_alias = component("Legs", 5_557);
    same_alias.as_fix_mut().set_aliases(["ticker"]).unwrap();
    let cases = [
        (group("Parties", 55), "identifier 55:"),
        (group("symbol", 5_555), "name \"symbol\" in branch \"\""),
        (same_alternate, "alternate identifier 65:"),
        (same_alias, "alias \"ticker\" in branch \"\""),
    ];
    for (claimant, key) in cases {
        let claiming = claimant.name().to_owned();
        let mut probed = registry.clone();
        let error = probed.insert(claimant).unwrap_err();
        let Error::Conflict { path, .. } = &error else {
            panic!("{key}: {error}");
        };
        assert!(path.contains(key), "{path}");
        assert!(path.contains(&claiming), "{path}");
        assert!(path.ends_with(", held by Symbol"), "{path}");
        // Nothing was written: the registry still holds the one field.
        assert_eq!(probed, registry, "{key}: a refusal changes nothing");
        assert_eq!(probed.len(), 1);
    }

    // And the other direction: a primitive claiming a nested field's key.
    let mut registry = FixRegistry::from_fields([group("NoPartyIDs", 453)]).unwrap();
    let error = registry.insert(tagged("Parties", 453)).unwrap_err();
    let Error::Conflict { path, .. } = &error else {
        panic!("{error}");
    };
    assert!(path.contains("identifier 453:"), "{path}");
    assert!(path.ends_with(", held by NoPartyIDs"), "{path}");
    assert_eq!(registry.len(), 1);
}

#[test]
fn iteration_and_the_cursor_walk_every_shape_in_identifier_order() {
    let registry = FixRegistry::from_fields([
        tagged("Account", 1),
        group("NoPartyIDs", 453),
        tagged("Symbol", 55),
        component("Instrument", 1_000),
        tagged("Text", 58),
    ])
    .unwrap();
    let expected = ["Account", "Symbol", "Text", "NoPartyIDs", "Instrument"];
    assert_eq!(
        registry.iter().map(Field::name).collect::<Vec<_>>(),
        expected
    );
    assert_eq!(registry.iter().len(), 5);
    assert_eq!(registry.len(), 5);
    assert!(!registry.is_empty());

    // The same order from the back, and the two ends meet without yielding an
    // entry twice or losing one between them.
    let mut backwards: Vec<&str> = registry.iter().rev().map(Field::name).collect();
    backwards.reverse();
    assert_eq!(backwards, expected);
    let mut iter = registry.iter();
    let mut ends = Vec::new();
    while let Some(front) = iter.next() {
        ends.push(front.name());
        if let Some(back) = iter.next_back() {
            ends.push(back.name());
        }
    }
    ends.sort_unstable();
    let mut sorted = expected;
    sorted.sort_unstable();
    assert_eq!(ends, sorted);

    // The cursor a binding advances walks the merge too.
    let mut walked = Vec::new();
    let mut cursor = None;
    while let Some(field) = registry.next_field_after(cursor) {
        walked.push(field.name());
        cursor = field.as_fix().id().unwrap();
    }
    assert_eq!(walked, expected);

    // Equality and Debug span every field shape.
    let mut reversed: Vec<Field> = registry.iter().cloned().collect();
    reversed.reverse();
    assert_eq!(registry, FixRegistry::from_fields(reversed).unwrap());
    let rendered = format!("{registry:?}");
    assert!(rendered.starts_with("{\"1:\": "), "{rendered}");
    assert!(
        rendered.find("\"453:\"").unwrap() < rendered.find("\"1000:\"").unwrap(),
        "{rendered}"
    );

    // A nested-only registry still iterates whole.
    let nested_only =
        FixRegistry::from_fields([group("NoPartyIDs", 453), component("Instrument", 1_000)])
            .unwrap();
    assert_eq!(
        nested_only.iter().map(Field::name).collect::<Vec<_>>(),
        ["NoPartyIDs", "Instrument"]
    );
    assert_eq!(
        nested_only
            .iter()
            .rev()
            .map(Field::name)
            .collect::<Vec<_>>(),
        ["Instrument", "NoPartyIDs"]
    );
}

#[test]
fn shard_arithmetic_picks_the_one_shard_that_holds_a_tag() {
    assert_eq!(shard_of(0), 0);
    assert_eq!(shard_of(99), 0);
    assert_eq!(shard_of(100), 1);
    assert_eq!(shard_of(101), 1);
    assert_eq!(shard_of(10_000), 100);
    assert_eq!(shard_of(i32::MAX), 21_474_836);
}

#[test]
fn the_default_resolves_in_the_documented_order_from_explicit_inputs() {
    let root = scratch("autoload");
    let location = root.join("location");
    let home = root.join("home");
    let config = Folder::new(home.join(".config")).unwrap();

    // Nothing configured, or no home at all: the empty registry.
    assert!(autoload(None, Some(config.clone())).unwrap().is_empty());
    assert!(autoload(None, None).unwrap().is_empty());

    // A present dictionary under the configuration directory loads.
    let mut configured = Folder::new(home.join(".config").join("fix")).unwrap();
    FixRegistry::from_fields([tagged("Symbol", 55)])
        .unwrap()
        .write_into(&mut configured)
        .unwrap();
    let loaded = autoload(None, Some(config.clone())).unwrap();
    assert_eq!(loaded.field_by_tag(55).unwrap().name(), "Symbol");

    // The explicit location beats it, spelled as a path or as a URL.
    let mut located = Folder::new(&location).unwrap();
    FixRegistry::from_fields([tagged("Price", 44)])
        .unwrap()
        .write_into(&mut located)
        .unwrap();
    let as_path = location.to_string_lossy().into_owned();
    let as_url = located.url().to_string();
    for spelling in [as_path, as_url] {
        let loaded = autoload(Some(&spelling), Some(config.clone())).unwrap();
        assert_eq!(loaded.len(), 1, "{spelling}");
        assert_eq!(
            loaded.field_by_tag(44).unwrap().name(),
            "Price",
            "{spelling}"
        );
        assert!(loaded.get_field_by_tag(55).is_none(), "{spelling}");
    }

    // A location that is set but names nothing, or a scheme this crate has no
    // backend for, is an error - never the empty registry.
    let missing = root.join("missing").to_string_lossy().into_owned();
    let error = autoload(Some(&missing), Some(config.clone())).unwrap_err();
    assert!(error.is_absent(), "{error}");
    let error = autoload(Some("mem://1/fix"), Some(config.clone())).unwrap_err();
    assert!(error.to_string().contains("scheme mem"), "{error}");

    // A malformed shard anywhere is an error naming the shard.
    std::fs::write(location.join("primitive").join("0.json"), b"not json").unwrap();
    let error = autoload(Some(&location.to_string_lossy()), None).unwrap_err();
    let message = error.to_string();
    assert!(message.contains("0.json"), "{message}");
    std::fs::write(home.join(".config/fix/primitive/0.json"), b"[1]").unwrap();
    let error = autoload(None, Some(config)).unwrap_err();
    let message = error.to_string();
    assert!(message.contains("0.json"), "{message}");
    assert!(message.contains("field mapping"), "{message}");

    let _ = std::fs::remove_dir_all(&root);
}

/// A registry, a root and a value the message tests share.
fn order() -> (Arc<FixRegistry>, Field, Scalar) {
    let mut party_id = DataType::Utf8.nullable_field("PartyID");
    party_id.as_fix_mut().set_tag(448).unwrap();
    let mut role = DataType::Int32.nullable_field("PartyRole");
    role.as_fix_mut().set_tag(452).unwrap();
    let item = DataType::from_fields([party_id, role])
        .unwrap()
        .required_field("item");
    let mut group = DataType::list(item).nullable_field("NoPartyIDs");
    group.as_fix_mut().set_tag(453).unwrap();
    let mut instrument = DataType::from_fields([tagged("Symbol", 55)])
        .unwrap()
        .nullable_field("Instrument");
    instrument.as_fix_mut().set_tag(1000).unwrap();
    let mut qty = DataType::Int64.required_field("OrderQty");
    qty.as_fix_mut().set_tag(38).unwrap();
    qty.as_fix_mut().set_aliases(["Qty"]).unwrap();
    let registry = Arc::new(
        FixRegistry::from_fields([
            group.clone(),
            instrument.clone(),
            qty.clone(),
            tagged("Symbol", 55),
        ])
        .unwrap(),
    );
    let root = DataType::from_fields([
        qty,
        instrument,
        group,
        DataType::Utf8.nullable_field("9999"),
    ])
    .unwrap()
    .required_field("NewOrderSingle");
    let value = Scalar::from_record([
        ("OrderQty", Scalar::from(100)),
        (
            "Instrument",
            Scalar::from_record([("Symbol", Scalar::from("AAPL"))]).unwrap(),
        ),
        (
            "NoPartyIDs",
            Scalar::from_sequence([
                Scalar::from_record([
                    ("PartyID", Scalar::from("BROKER")),
                    ("PartyRole", Scalar::from(1)),
                ])
                .unwrap(),
                Scalar::from_record([
                    ("PartyID", Scalar::from("CLIENT")),
                    ("PartyRole", Scalar::from(3)),
                ])
                .unwrap(),
            ]),
        ),
        ("9999", Scalar::from("custom")),
    ])
    .unwrap();
    (registry, root, value)
}

#[test]
fn a_message_resolves_values_through_its_registry() {
    let (registry, root, value) = order();
    let msg = FixMsg::with_registry(Arc::clone(&registry), root.clone(), value).unwrap();
    assert!(Arc::ptr_eq(msg.registry(), &registry));
    assert_eq!(msg.as_field(), &root);

    // A record input canonicalizes to the ordered sequence the root declares.
    let row = msg.as_value().as_sequence().unwrap();
    assert_eq!(row.len(), 4);
    assert_eq!(row[0], Scalar::from(100));
    assert_eq!(row[3], Scalar::from("custom"));

    // By tag, through the registry's canonical name.
    assert_eq!(msg.by_tag(38).unwrap(), &Scalar::from(100));
    // By name, folded through the registry, and by alias.
    assert_eq!(msg.by_name("orderqty").unwrap(), &Scalar::from(100));
    assert_eq!(msg.by_name("QTY").unwrap(), &Scalar::from(100));
    // An unknown tag is kept under its rendered name.
    assert_eq!(msg.by_tag(9999).unwrap(), &Scalar::from("custom"));
    assert_eq!(msg.by_name("9999").unwrap(), &Scalar::from("custom"));
    // A path descends a component by name and a group by index.
    assert_eq!(
        msg.by_path("Instrument.symbol").unwrap(),
        &Scalar::from("AAPL")
    );
    assert_eq!(
        msg.by_path("NoPartyIDs.1.PartyID").unwrap(),
        &Scalar::from("CLIENT")
    );
    assert_eq!(
        msg.by_path("nopartyids.0.PartyRole").unwrap(),
        &Scalar::from(1)
    );
    assert_eq!(msg.by_path("NoPartyIDs").unwrap().len(), 2);
    assert!(
        msg.get_by_path("NoPartyIDs.PartyID").is_none(),
        "a group member needs its index"
    );
    assert!(msg.get_by_path("NoPartyIDs.2.PartyID").is_none());
    assert!(msg.get_by_path("OrderQty.deeper").is_none());
    assert!(
        msg.get_by_tag(55).is_none(),
        "Symbol is nested, not a root child"
    );
    // A member the registry does not know still matches its exact spelling.
    assert_eq!(
        msg.by_path("NoPartyIDs.0.PartyID").unwrap(),
        &Scalar::from("BROKER")
    );
    assert!(msg.get_by_path("NoPartyIDs.0.partyid").is_none());
    assert!(msg.get_by_tag(-1).is_none());

    // The generic pair matches the specialized one for every key.
    for tag in [38, 9999, 55, 453] {
        assert_eq!(msg.get(tag), msg.get_by_tag(tag), "{tag}");
        assert_eq!(msg.value(tag).ok(), msg.by_tag(tag).ok(), "{tag}");
    }
    for name in [
        "OrderQty",
        "qty",
        "Instrument.Symbol",
        "NoPartyIDs.1.PartyID",
        "absent",
    ] {
        assert_eq!(msg.get(name), msg.get_by_path(name), "{name}");
        assert_eq!(msg.value(name).ok(), msg.by_path(name).ok(), "{name}");
    }
    let error = msg.by_tag(55).unwrap_err();
    assert!(matches!(&error, Error::Absent { expected: "fix value", path } if path == "tag 55"));
    let error = msg.by_path("absent.x").unwrap_err();
    assert!(matches!(&error, Error::Absent { path, .. } if path == "path \"absent.x\""));

    // Equality and hashing follow the schema and the value.
    let same = FixMsg::with_registry(Arc::clone(&registry), root, msg.as_value().clone()).unwrap();
    assert_eq!(msg, same);
    assert_eq!(crate::stable_hash_of(&msg), crate::stable_hash_of(&same));
}

#[test]
fn a_message_rejects_a_value_its_field_refuses() {
    let (registry, root, _) = order();
    let error = FixMsg::with_registry(
        Arc::clone(&registry),
        root.clone(),
        Scalar::from_record([("OrderQty", Scalar::from("many"))]).unwrap(),
    )
    .unwrap_err();
    assert!(matches!(error, Error::InvalidRecord { .. }), "{error}");
    assert!(error.to_string().contains("OrderQty"), "{error}");

    let error = FixMsg::with_registry(
        registry,
        DataType::Int64.required_field("scalar"),
        Scalar::from(1),
    )
    .unwrap_err();
    assert!(error.to_string().contains("struct root"), "{error}");
}

#[test]
fn a_message_resolves_a_bare_tag_in_its_own_branch_then_the_standard_one() {
    let cme = cme();
    // Tag 5001 is defined in both dictionaries; 35 only in the standard one.
    let mut venue_trade = identified("TradeID", &cme, 5001);
    venue_trade.as_fix_mut().set_aliases(["TID"]).unwrap();
    let mut spec_trade = tagged("SecondaryTradeID", 5001);
    spec_trade.as_fix_mut().set_aliases(["STID"]).unwrap();
    let msg_type = tagged("MsgType", 35);
    let registry = Arc::new(
        FixRegistry::from_fields([venue_trade.clone(), spec_trade.clone(), msg_type.clone()])
            .unwrap(),
    );

    // The root declares the venue's branch, so the message speaks it.
    let mut root = DataType::from_fields([venue_trade.clone(), msg_type.clone()])
        .unwrap()
        .required_field("VenueExecutionReport");
    root.as_fix_mut().set_branch(&cme).unwrap();
    let value = Scalar::from_record([
        ("TradeID", Scalar::from("T-1")),
        ("MsgType", Scalar::from("8")),
    ])
    .unwrap();
    let msg = FixMsg::with_registry(Arc::clone(&registry), root, value).unwrap();
    assert_eq!(msg.branch(), &cme);

    // Step one: the message's own branch.
    assert_eq!(msg.by_tag(5001).unwrap(), &Scalar::from("T-1"));
    assert_eq!(msg.by_name("tid").unwrap(), &Scalar::from("T-1"));
    // Step two: the standard branch, so MsgType stays reachable.
    assert_eq!(msg.by_tag(35).unwrap(), &Scalar::from("8"));
    assert_eq!(msg.by_name("msgtype").unwrap(), &Scalar::from("8"));
    // The standard field 5001 names a root child this message does not hold,
    // so its alias misses rather than answering the venue's value.
    assert!(msg.get_by_name("stid").is_none());

    // An identifier is exact and never tiers.
    let venue_id = FixId::from_parts(&cme, 5001).unwrap();
    assert_eq!(msg.by_id(venue_id).unwrap(), &Scalar::from("T-1"));
    assert!(
        msg.get_by_id(FixId::standard(5001)).is_none(),
        "a foreign branch misses"
    );
    assert_eq!(msg.get(venue_id), msg.get_by_id(venue_id));
    assert_eq!(msg.value(venue_id).unwrap(), msg.by_id(venue_id).unwrap());
    let error = msg.by_id(FixId::standard(5001)).unwrap_err();
    assert!(
        matches!(&error, Error::Absent { expected: "fix value", path } if path == "identifier 5001:"),
        "{error}"
    );

    // A standard message resolves only in the standard branch.
    let plain_root = DataType::from_fields([spec_trade, msg_type])
        .unwrap()
        .required_field("ExecutionReport");
    let plain = FixMsg::with_registry(
        registry,
        plain_root,
        Scalar::from_record([
            ("SecondaryTradeID", Scalar::from("S-1")),
            ("MsgType", Scalar::from("8")),
        ])
        .unwrap(),
    )
    .unwrap();
    assert_eq!(plain.branch(), &FixBranch::STANDARD);
    assert_eq!(plain.by_tag(5001).unwrap(), &Scalar::from("S-1"));
    assert_eq!(plain.by_name("stid").unwrap(), &Scalar::from("S-1"));
    assert!(plain.get_by_name("tid").is_none());
    assert!(plain.get_by_id(venue_id).is_none());
}

#[test]
fn a_message_rejects_a_root_whose_branch_is_corrupt() {
    let mut root = DataType::from_fields([tagged("MsgType", 35)])
        .unwrap()
        .required_field("row");
    root.insert_metadata("fix:branch", "2cme").unwrap();
    let error = FixMsg::with_registry(
        Arc::new(FixRegistry::new()),
        root,
        Scalar::from_record([("MsgType", Scalar::from("8"))]).unwrap(),
    )
    .unwrap_err();
    assert!(
        matches!(&error, Error::InvalidMetadataValue { key, .. } if key == "fix:branch"),
        "{error}"
    );
}

/// The worked case: tag 32 is `LastShares` typed `int` in 4.0, `LastShares`
/// typed `Qty` from 4.2, and `LastQty` from 4.3 on.
fn last_qty() -> Field {
    let mut field = DataType::Float64.nullable_field("LastQty");
    field.as_fix_mut().set_tag(32).unwrap();
    field
        .as_fix_mut()
        .set_lineage(&[
            FixLineageEntry::new(FixPedigree::new(version("2.7"), None))
                .with_name("LastShares")
                .with_dtype("int"),
            FixLineageEntry::new(FixPedigree::new(version("4.2"), None))
                .with_name("LastShares")
                .with_dtype("Qty"),
            FixLineageEntry::new(FixPedigree::new(version("4.3"), None))
                .with_name("LastQty")
                .with_dtype("Qty"),
        ])
        .unwrap();
    field
}

fn version(text: &str) -> Version {
    text.parse().unwrap()
}

#[test]
fn a_lineage_answers_the_name_and_datatype_of_every_version_it_holds() {
    let field = last_qty();
    let view = field.as_fix();

    assert_eq!(view.since(), Some(version("2.7")));
    assert_eq!(view.until(), None);
    assert_eq!(view.name_at(version("4.2")), Some("LastShares"));
    assert_eq!(view.name_at(version("4.3")), Some("LastQty"));
    assert_eq!(view.name_at(version("5.0SP2")), Some("LastQty"));
    // A version older than the first entry states nothing, so the caller
    // falls back to the field's own name.
    assert_eq!(view.name_at(version("2.6")), None);

    assert_eq!(
        view.dtype_at(version("4.0")).unwrap(),
        Some(DataType::Int32)
    );
    // `Qty` is a FIX `float`, and the logical-name table says so.
    assert_eq!(
        view.dtype_at(version("4.4")).unwrap(),
        Some(DataType::Float64)
    );
}

#[test]
fn two_entries_at_one_version_order_by_extension_pack() {
    let mut field = DataType::Utf8.nullable_field("BasisPoints");
    field.as_fix_mut().set_tag(9999).unwrap();
    field
        .as_fix_mut()
        .set_lineage(&[
            FixLineageEntry::new(FixPedigree::new(version("5.0SP2"), Some(309)))
                .with_name("BasisPoints"),
            FixLineageEntry::new(FixPedigree::new(version("5.0SP2"), Some(204)))
                .with_name("Superseded"),
            FixLineageEntry::new(FixPedigree::new(version("5.0SP2"), None)).with_name("Base"),
        ])
        .unwrap();

    let dated: Vec<_> = field
        .as_fix()
        .lineage()
        .map(|entry| {
            let entry = entry.unwrap();
            (entry.ep(), entry.name().unwrap())
        })
        .collect();
    assert_eq!(
        dated,
        [
            (None, "Base"),
            (Some(204), "Superseded"),
            (Some(309), "BasisPoints"),
        ]
    );
    // The newest reading is the last written, so resolution is a scan that
    // stops rather than a sort.
    assert_eq!(
        field.as_fix().name_at(version("5.0SP2")),
        Some("BasisPoints")
    );
}

#[test]
fn a_lineage_rewrites_the_aliases_it_implies_and_a_query_by_either_answers() {
    let registry = FixRegistry::from_fields([last_qty()]).unwrap();

    let aliases: Vec<_> = registry
        .field_by_tag(32)
        .unwrap()
        .as_fix()
        .aliases()
        .collect();
    assert_eq!(aliases, ["LastShares"]);
    assert_eq!(registry.field("LastShares").unwrap().name(), "LastQty");
    assert_eq!(registry.field("LastQty").unwrap().name(), "LastQty");
}

#[test]
fn a_version_filters_the_read_and_the_registry_stays_version_agnostic() {
    let registry = FixRegistry::from_fields([last_qty()]).unwrap();

    // The dictionary holds the tag whatever version is asked for; only the
    // read is filtered.
    assert_eq!(
        registry.field_at(version("4.2"), 32).unwrap().name(),
        "LastQty"
    );
    let refused = registry.field_at(version("2.6"), 32).unwrap_err();
    assert!(refused.to_string().contains("2.6"), "{refused}");
    assert!(registry.get_field_at(version("2.6"), 32).is_none());

    assert_eq!(
        registry.versions(),
        [version("2.7"), version("4.2"), version("4.3")]
    );
    assert_eq!(
        registry.newest(),
        Some(FixPedigree::new(version("4.3"), None))
    );
    // "FIX Latest" is the real pedigree the dictionary carries, never a
    // sentinel at the top of the value space.
    assert_ne!(registry.newest().unwrap().version(), Version::MAX);
}

#[test]
fn a_removed_entry_ends_the_field_and_a_field_with_no_lineage_answers_everywhere() {
    let mut retired = DataType::Utf8.nullable_field("Retired");
    retired.as_fix_mut().set_tag(9998).unwrap();
    retired
        .as_fix_mut()
        .set_lineage(&[
            FixLineageEntry::new(FixPedigree::new(version("4.0"), None)).with_name("Retired"),
            FixLineageEntry::new(FixPedigree::new(version("4.4"), None)).remove(),
        ])
        .unwrap();
    let view = retired.as_fix();
    assert_eq!(view.until(), Some(version("4.4")));
    assert!(view.defined_at(version("4.3")));
    assert!(!view.defined_at(version("4.4")));
    assert!(!view.defined_at(version("5.0SP2")));

    let undated = tagged("Symbol", 55);
    let view = undated.as_fix();
    assert_eq!(view.since(), None);
    assert_eq!(view.until(), None);
    assert_eq!(view.name_at(version("4.2")), None);
    assert_eq!(view.dtype_at(version("4.2")).unwrap(), None);
    // No history is no filter, so an undated dictionary resolves as before.
    assert!(view.defined_at(version("4.2")));
    assert!(view.defined_at(Version::MIN));
}

#[test]
fn a_lineage_disagreeing_with_its_own_field_is_refused_naming_both_sides() {
    let mut field = DataType::Utf8.nullable_field("LastQty");
    field.as_fix_mut().set_tag(32).unwrap();

    let error = field
        .as_fix_mut()
        .set_lineage(&[
            FixLineageEntry::new(FixPedigree::new(version("4.3"), None)).with_name("LastShares")
        ])
        .unwrap_err();
    let rendered = error.to_string();
    assert!(rendered.contains("LastShares"), "{rendered}");
    assert!(rendered.contains("LastQty"), "{rendered}");
    // A refusal leaves the field exactly as it was.
    assert_eq!(field.as_fix().lineage().count(), 0);
    assert_eq!(field.as_fix().aliases().count(), 0);

    let error = field
        .as_fix_mut()
        .set_lineage(&[
            FixLineageEntry::new(FixPedigree::new(version("4.3"), None)).with_dtype("int")
        ])
        .unwrap_err();
    let rendered = error.to_string();
    assert!(rendered.contains("int32"), "{rendered}");
    assert!(rendered.contains("utf8"), "{rendered}");
}

#[test]
fn two_entries_sharing_one_pedigree_are_refused() {
    let mut field = DataType::Utf8.nullable_field("Twice");
    field.as_fix_mut().set_tag(9997).unwrap();

    let error = field
        .as_fix_mut()
        .set_lineage(&[
            FixLineageEntry::new(FixPedigree::new(version("4.2"), Some(1))),
            FixLineageEntry::new(FixPedigree::new(version("4.2"), Some(1))),
        ])
        .unwrap_err();
    assert!(
        matches!(&error, Error::Parse { target, .. } if *target == "fix lineage"),
        "{error}"
    );
    assert!(error.to_string().contains("EP1"), "{error}");
}

#[test]
fn a_lineage_round_trips_canonically_and_a_hand_edit_names_its_byte_position() {
    let field = last_qty();
    let stored = field
        .as_metadata()
        .get("fix:lineage")
        .expect("the lineage is stored")
        .to_owned();
    assert_eq!(
        stored,
        concat!(
            r#"{"entries":["#,
            r#"{"since":"2.7","name":"LastShares","type":{"type":"int32"}},"#,
            r#"{"since":"4.2","name":"LastShares","type":{"type":"float64"}},"#,
            r#"{"since":"4.3","name":"LastQty","type":{"type":"float64"}}]}"#,
        )
    );

    // Rewriting what was read back produces the same text.
    let entries: Vec<_> = field
        .as_fix()
        .lineage()
        .map(|entry| entry.unwrap())
        .collect();
    let mut rebuilt = DataType::Float64.nullable_field("LastQty");
    rebuilt.as_fix_mut().set_tag(32).unwrap();
    rebuilt.as_fix_mut().set_lineage(&entries).unwrap();
    assert_eq!(
        rebuilt.as_metadata().get("fix:lineage"),
        Some(stored.as_str())
    );

    // Keys follow the document's declared order, so a reordered one is
    // refused rather than mis-scanned.
    let reordered = r#"{"entries":[{"name":"LastShares","since":"2.7"}]}"#;
    let mut edited = DataType::Utf8.nullable_field("LastShares");
    edited.set_metadata([("fix:lineage", reordered)]).unwrap();
    let error = edited.as_fix().dtype_at(version("4.2")).unwrap_err();
    assert!(
        matches!(&error, Error::Parse { target, position, .. }
            if *target == "fix lineage" && *position == reordered.find(r#""since""#).unwrap()),
        "{error}"
    );
    // A read that cannot parse answers nothing rather than a wrong answer.
    assert_eq!(edited.as_fix().name_at(version("4.2")), None);
    assert_eq!(edited.as_fix().since(), None);
}

/// The committed dictionary, as the codec every enrichment case reads with.
fn enriching() -> super::FixCodec {
    super::FixCodec::new(Arc::new(committed()))
}

#[test]
fn a_report_states_what_is_left_once_it_has_stated_the_rest() {
    let codec = enriching();
    // Appendix D: a part-filled working order. What is left is what was
    // ordered minus what was done, and the fill's worth is its quantity at
    // its price.
    let held = codec
        .transform_fix_line(
            b"8=FIX.4.4|35=8|39=1|150=F|38=100|14=40|32=40|31=10.5|54=1|10=0|",
            true,
        )
        .expect("a readable report");
    assert_eq!(held.by_tag(151).unwrap(), &Scalar::from(60.0_f64));
    assert_eq!(held.by_tag(381).unwrap(), &Scalar::from(420.0_f64));
    // One fill, so the average is that fill's price.
    assert_eq!(held.by_tag(6).unwrap(), &Scalar::from(10.5_f64));

    // A closed order leaves nothing, whatever the arithmetic of the other two
    // would say: Appendix D shows zero on every terminal row.
    let closed = codec
        .transform_fix_line(b"8=FIX.4.4|35=8|39=4|150=4|38=100|14=40|10=0|", true)
        .expect("a readable report");
    assert_eq!(closed.by_tag(151).unwrap(), &Scalar::from(0.0_f64));

    // The same identity read backwards: what was ordered is what is left plus
    // what was done.
    let ordered = codec
        .transform_fix_line(b"8=FIX.4.4|35=8|39=1|14=40|151=60|10=0|", true)
        .expect("a readable report");
    assert_eq!(ordered.by_tag(38).unwrap(), &Scalar::from(100.0_f64));
}

#[test]
fn a_stated_value_is_never_replaced_and_filling_twice_changes_nothing() {
    let codec = enriching();
    // The venue's own arithmetic wins even where it disagrees with the
    // specification's: the row says what was sent.
    let held = codec
        .transform_fix_line(b"8=FIX.4.4|35=8|39=1|38=100|14=40|151=999|10=0|", true)
        .expect("a readable report");
    assert_eq!(held.by_tag(151).unwrap(), &Scalar::from(999.0_f64));

    // Idempotent: a value derived once is a stated value the second time, so
    // a second pass derives it to itself.
    let once = codec
        .transform_fix_line(b"8=FIX.4.4|35=8|39=1|38=100|14=40|10=0|", true)
        .expect("a readable report");
    let twice = codec.enrich_fixmsg(once.clone()).expect("a second pass");
    assert_eq!(once, twice);
}

#[test]
fn filling_leaves_the_wire_exactly_as_it_arrived() {
    let codec = enriching();
    const LINE: &[u8] = b"8=FIX.4.4|35=8|39=1|38=100|14=40|32=40|31=10.5|54=1|10=0|";
    let bare = codec
        .transform_fix_line(LINE, false)
        .expect("a readable report");
    let filled = codec
        .transform_fix_line(LINE, true)
        .expect("a readable report");

    // The row gained columns.
    assert_eq!(bare.get_by_tag(151), None);
    assert_eq!(filled.by_tag(151).unwrap(), &Scalar::from(60.0_f64));
    // The entries did not, so the two re-emit the same bytes: the entries are
    // what arrived and the row is the reading of them.
    assert_eq!(bare.entries(), filled.entries());
    assert_eq!(bare.into_bytes(b'|'), filled.into_bytes(b'|'));
    assert_eq!(filled.into_bytes(b'|'), LINE);
}

#[test]
fn a_rule_answers_nothing_rather_than_a_guess() {
    let codec = enriching();
    // An input the message never stated: nothing is derived from an absence.
    let held = codec
        .transform_fix_line(b"8=FIX.4.4|35=8|39=1|38=100|10=0|", true)
        .expect("a readable report");
    assert_eq!(held.get_by_tag(151), None, "no CumQty to subtract");

    // A negative remainder means the two inputs were never about one order,
    // so the rule declines rather than stating a quantity that cannot exist.
    let crossed = codec
        .transform_fix_line(b"8=FIX.4.4|35=8|39=1|38=40|14=100|10=0|", true)
        .expect("a readable report");
    assert_eq!(crossed.get_by_tag(151), None);

    // A status the matrices do not place answers nothing either.
    let unknown = codec
        .transform_fix_line(b"8=FIX.4.4|35=8|39=Z|38=100|14=40|10=0|", true)
        .expect("a readable report");
    assert_eq!(unknown.get_by_tag(151), None);

    // A message type the rule does not speak for is left alone: an order has
    // no remainder to state until something reports on it.
    let order = codec
        .transform_fix_line(b"8=FIX.4.4|35=D|38=100|14=40|10=0|", true)
        .expect("a readable order");
    assert_eq!(order.get_by_tag(151), None);
}

#[test]
fn a_foreign_exchange_trade_settles_in_the_currency_it_was_dealt_in() {
    let codec = enriching();
    // Appendix O: the settlement currency defaults to the dealt one, and the
    // settled amount is the traded amount at the stated rate.
    let held = codec
        .transform_fix_line(
            b"8=FIX.4.4|35=8|39=2|150=F|38=100|14=100|32=100|31=1.25|15=EUR|155=1.1|10=0|",
            true,
        )
        .expect("a readable report");
    assert_eq!(held.by_tag(381).unwrap(), &Scalar::from(125.0_f64));
    assert_eq!(
        held.by_tag(119).unwrap(),
        &Scalar::from(137.5_f64),
        "the traded amount at the stated rate",
    );
    let settled = held.by_tag(120).unwrap();
    assert_eq!(settled.as_str(), Some("EUR"));

    // A trade that states its own settlement currency keeps it.
    let stated = codec
        .transform_fix_line(b"8=FIX.4.4|35=8|39=2|15=EUR|120=USD|10=0|", true)
        .expect("a readable report");
    assert_eq!(stated.by_tag(120).unwrap().as_str(), Some("USD"));
}

#[test]
fn a_field_states_the_spellings_that_mean_nothing_was_sent() {
    let mut field = DataType::Float64.nullable_field("StopPx");
    field.as_fix_mut().set_tag(99).unwrap();
    field.as_fix_mut().set_nulls(["N/A", "NONE", ""]).unwrap();
    assert_eq!(field.get_metadata("fix:nulls"), Some("N/A,NONE,"));
    assert_eq!(
        field.as_fix().nulls().collect::<Vec<_>>(),
        ["N/A", "NONE"],
        "an empty spelling is stored and walked past, as every list here does",
    );

    // Matched case-insensitively against the trimmed text, exactly as the
    // capture-wide list matches.
    for held in ["N/A", "n/a", " none ", "NONE"] {
        assert!(field.as_fix().is_null_value(held), "{held}");
    }
    for held in ["12.5", "NA", "N/A/"] {
        assert!(!field.as_fix().is_null_value(held), "{held}");
    }

    // The row types it as null; the entry keeps what arrived, because the
    // entries are the wire and the row is the reading of it.
    let registry = Arc::new(FixRegistry::from_fields([field]).unwrap());
    let message = super::FixCodec::new(Arc::clone(&registry))
        .transform_fix_line(b"99=N/A|", false)
        .expect("a readable frame");
    assert_eq!(message.get_by_tag(99), Some(&Scalar::Null));
    let entry = message
        .entries()
        .iter()
        .find(|held| held.tag() == 99)
        .expect("the pair still arrived");
    assert_eq!(entry.value(), "N/A");

    // A value the list does not name is read as the price it is.
    let message = super::FixCodec::new(registry)
        .transform_fix_line(b"99=12.5|", false)
        .expect("a readable frame");
    assert_eq!(message.by_tag(99).unwrap(), &Scalar::from(12.5_f64));
}

#[test]
fn a_null_spelling_is_refused_when_it_carries_the_separator_or_repeats() {
    let mut field = DataType::Utf8.nullable_field("Account");
    field.as_fix_mut().set_tag(1).unwrap();

    let error = field.as_fix_mut().set_nulls(["a,b"]).unwrap_err();
    assert!(error.to_string().contains("without ','"), "{error}");
    let error = field.as_fix_mut().set_nulls(["NONE", "none"]).unwrap_err();
    assert!(error.to_string().contains("twice"), "{error}");
    // A refusal leaves the field exactly as it was.
    assert_eq!(field.get_metadata("fix:nulls"), None);

    // Empty input removes the property.
    field.as_fix_mut().set_nulls(["NONE"]).unwrap();
    field.as_fix_mut().set_nulls::<[&str; 0], &str>([]).unwrap();
    assert_eq!(field.get_metadata("fix:nulls"), None);
}

#[test]
fn a_lineage_stores_the_type_a_spelling_resolves_to_and_drops_a_rename_of_it() {
    let mut field = DataType::Utf8.nullable_field("account");
    field.as_fix_mut().set_tag(1).unwrap();
    // The specification renamed the type without changing it: `char` and
    // `String` are one `utf8`.
    field
        .as_fix_mut()
        .set_lineage(&[
            FixLineageEntry::new(FixPedigree::new(version("2.7"), None))
                .with_name("account")
                .with_dtype("char"),
            FixLineageEntry::new(FixPedigree::new(version("4.2"), None))
                .with_name("account")
                .with_dtype("String"),
        ])
        .unwrap();
    assert_eq!(
        field.as_metadata().get("fix:lineage"),
        Some(r#"{"entries":[{"since":"2.7","name":"account","type":{"type":"utf8"}}]}"#)
    );
    // The oldest entry survives, so `since` still dates the field.
    assert_eq!(field.as_fix().since(), Some(version("2.7")));
    // And the type is answered at both versions, from the one entry left.
    assert_eq!(
        field.as_fix().dtype_at(version("4.4")).unwrap(),
        Some(DataType::Utf8)
    );
}

#[test]
fn only_a_type_equivalent_entry_collapses() {
    // Each of these differs from its predecessor in exactly one stated fact,
    // so none of them collapses.
    for entries in [
        vec![
            FixLineageEntry::new(FixPedigree::new(version("2.7"), None))
                .with_name("kept")
                .with_dtype("char"),
            FixLineageEntry::new(FixPedigree::new(version("4.2"), None))
                .with_name("renamed")
                .with_dtype("String"),
        ],
        vec![
            FixLineageEntry::new(FixPedigree::new(version("2.7"), None)).with_dtype("char"),
            FixLineageEntry::new(FixPedigree::new(version("4.2"), None))
                .with_dtype("String")
                .deprecate(),
        ],
        vec![
            FixLineageEntry::new(FixPedigree::new(version("2.7"), None)).with_dtype("char"),
            FixLineageEntry::new(FixPedigree::new(version("4.2"), None))
                .with_dtype("String")
                .remove(),
        ],
        vec![
            FixLineageEntry::new(FixPedigree::new(version("2.7"), None)).with_dtype("char"),
            FixLineageEntry::new(FixPedigree::new(version("4.2"), None))
                .with_dtype("String")
                .with_doc("said differently"),
        ],
        // A genuine retype is a change, whatever the spellings look like.
        vec![
            FixLineageEntry::new(FixPedigree::new(version("2.7"), None)).with_dtype("int"),
            FixLineageEntry::new(FixPedigree::new(version("4.2"), None)).with_dtype("Qty"),
        ],
    ] {
        let mut field = DataType::Utf8.nullable_field("kept");
        // The last two cases retype to something the field is not, so the
        // lineage is rendered directly rather than through the agreement
        // check `set_lineage` makes.
        let rendered = FixLineage::render(&entries).expect("the entries render");
        let held: Vec<_> = FixLineage::over(Some(&rendered))
            .map(|entry| entry.expect("a readable entry"))
            .collect();
        assert_eq!(held.len(), 2, "{rendered}");
        field.as_fix_mut().set_tag(9993).unwrap();
    }

    // An extension pack that stated nothing new is exactly what collapses:
    // `ep` dates the statement rather than being one.
    let rendered = FixLineage::render(&[
        FixLineageEntry::new(FixPedigree::new(version("5.0SP2"), None)).with_dtype("char"),
        FixLineageEntry::new(FixPedigree::new(version("5.0SP2"), Some(309))).with_dtype("String"),
    ])
    .expect("the entries render");
    assert_eq!(
        rendered,
        r#"{"entries":[{"since":"5.0SP2","type":{"type":"utf8"}}]}"#
    );
}

#[test]
fn an_empty_lineage_removes_the_document_and_the_aliases_it_derived() {
    let mut field = last_qty();
    assert_eq!(field.as_fix().aliases().count(), 1);

    field.as_fix_mut().set_lineage(&[]).unwrap();
    assert_eq!(field.as_metadata().get("fix:lineage"), None);
    assert_eq!(field.as_metadata().get("fix:aliases"), None);
    assert_eq!(field.as_fix().since(), None);
}

/// Fixture A: the standard `SideCodeSet`, dated as the specification dates it.
fn side() -> Field {
    let mut field = DataType::Utf8.nullable_field("Side");
    field.as_fix_mut().set_tag(54).unwrap();
    field
        .as_fix_mut()
        .set_codes(&[
            FixCode::new("Buy", "1").with_since(version("2.7"), Some(254)),
            FixCode::new("Sell", "2").with_since(version("2.7"), Some(254)),
            FixCode::new("Undisclosed", "7").with_since(version("4.1"), None),
            FixCode::new("CrossShort", "9").with_since(version("4.2"), None),
            FixCode::new("CrossShortExempt", "A").with_since(version("4.3"), None),
        ])
        .unwrap();
    field
}

/// Fixture B: `CommTypeCodeSet`, whose long names are what tier 2 folds.
fn comm_type() -> Field {
    let mut field = DataType::Utf8.nullable_field("CommType");
    field.as_fix_mut().set_tag(13).unwrap();
    field
        .as_fix_mut()
        .set_codes(&[
            FixCode::new("PerUnit", "1"),
            FixCode::new("Percent", "2"),
            FixCode::new("Absolute", "3"),
            FixCode::new("PercentageWaivedCashDiscount", "4"),
            FixCode::new("PercentageWaivedEnhancedUnits", "5"),
            FixCode::new("PointsPerBondOrContract", "6")
                .with_description("Good Till Date (GTD) points per bond"),
            FixCode::new("BasisPoints", "7").with_since(version("5.0SP2"), Some(208)),
            FixCode::new("AmountPerContract", "8"),
        ])
        .unwrap();
    field
}

#[test]
fn a_code_set_resolves_by_value_by_name_and_by_every_folding_of_a_name() {
    let field = comm_type();
    let view = field.as_fix();

    // Tier 1: a spelling that is already a legal code is never reinterpreted.
    assert_eq!(view.code_value("4"), Some("4"));
    assert_eq!(
        view.code("4").unwrap().name(),
        "PercentageWaivedCashDiscount"
    );
    assert_eq!(view.code_name("4"), Some("PercentageWaivedCashDiscount"));

    // Tier 2: the crate's one fold, so four spellings are one.
    for spelling in [
        "PercentageWaivedCashDiscount",
        "percentage_waived_cash_discount",
        "PERCENTAGE WAIVED CASH DISCOUNT",
        "percentage-waived-cash-discount",
    ] {
        assert_eq!(view.code_value(spelling), Some("4"), "{spelling}");
    }
    // A shared prefix does not collide.
    assert_eq!(view.code_value("PercentageWaivedEnhancedUnits"), Some("5"));
    assert_eq!(view.code_by_name("basispoints").unwrap().value(), "7");
}

#[test]
fn an_alias_shares_a_value_and_an_unknown_spelling_falls_through() {
    let mut field = DataType::Utf8.nullable_field("Side");
    field.as_fix_mut().set_tag(54).unwrap();
    field
        .as_fix_mut()
        .set_codes(&[
            FixCode::new("Buy", "1").with_aliases(["Bought", "BUYSIDE"]),
            FixCode::new("Sell", "2"),
        ])
        .unwrap();
    let view = field.as_fix();

    assert_eq!(view.code_value("Buy"), Some("1"));
    assert_eq!(view.code_value("bought"), Some("1"));
    assert_eq!(view.code_value("buy_side"), Some("1"));
    let buy = view.code("1").unwrap();
    assert_eq!(buy.aliases().collect::<Vec<_>>(), ["Bought", "BUYSIDE"]);

    // A venue sends codes no dictionary lists, so an unresolved spelling is
    // answered as nothing and the caller keeps its own text.
    assert_eq!(view.code_value("VenueOwnSide"), None);
    assert_eq!(view.code_name("Z"), None);
}

#[test]
fn a_reader_meeting_one_spelling_at_a_time_grows_a_code_rather_than_dropping_it() {
    // A source naming one wire value twice is declaring an alias, and a
    // reader building the set meets the second name on its own. `is_spelled`
    // is what it asks before adding: the fold, so the answer is the same one
    // the stored set will give, and over the whole set, because whether a
    // spelling is free is a question about the set and not about one code.
    let mut buy = FixCode::new("Buy", "1");
    assert!(buy.is_spelled("b_uy"));
    assert!(!buy.is_spelled("Bought"));
    buy.push_alias("Bought");
    assert!(buy.is_spelled("bought"));
    assert_eq!(buy.aliases(), ["Bought"]);

    let mut field = DataType::Utf8.nullable_field("Side");
    field.as_fix_mut().set_tag(54).unwrap();
    field
        .as_fix_mut()
        .set_codes(&[buy, FixCode::new("Sell", "2")])
        .unwrap();
    let view = field.as_fix();

    // The grown code is one code with two spellings, not two codes.
    assert_eq!(view.codes().count(), 2);
    assert_eq!(view.code_value("Bought"), Some("1"));
    assert_eq!(view.code_name("1"), Some("Buy"));
}

#[test]
fn an_ambiguous_spelling_resolves_to_nothing_rather_than_the_first_match() {
    let mut field = DataType::Utf8.nullable_field("Side");
    field.as_fix_mut().set_tag(54).unwrap();
    field
        .as_fix_mut()
        .set_codes(&[
            FixCode::new("Cross", "8"),
            FixCode::new("CrossOther", "9").with_aliases(["cross"]),
        ])
        .unwrap();
    let view = field.as_fix();

    // Two codes reach one spelling, so picking either would be a guess.
    assert_eq!(view.code_value("Cross"), None);
    assert_eq!(view.code_by_name("cross"), None);
    // Tier 1 still answers, because a legal wire value is never a spelling.
    assert_eq!(view.code_value("8"), Some("8"));
    // Two names sharing one value are an alias, not an ambiguity.
    let mut aliased = DataType::Utf8.nullable_field("Side");
    aliased.as_fix_mut().set_tag(54).unwrap();
    aliased
        .as_fix_mut()
        .set_codes(&[
            FixCode::new("Cross", "8"),
            FixCode::new("CrossSame", "8").with_aliases(["cross"]),
        ])
        .unwrap();
    assert_eq!(aliased.as_fix().code_value("cross"), Some("8"));
}

#[test]
fn tier_three_reads_a_leading_abbreviation_and_leaves_both_traps_alone() {
    let mut field = DataType::Utf8.nullable_field("TimeInForce");
    field.as_fix_mut().set_tag(59).unwrap();
    field
        .as_fix_mut()
        .set_codes(&[
            FixCode::new("GoodTillDate", "6").with_description("Good Till Date (GTD)"),
            FixCode::new("BrokenDate", "7")
                .with_description("Broken date; SettlDate (64) is required"),
            FixCode::new("SwapValueFactor", "8")
                .with_description("Swap Value Factor (SVP) through a central counterparty (CCP)"),
        ])
        .unwrap();
    let view = field.as_fix();

    assert_eq!(view.code_value("gtd"), Some("6"));
    assert_eq!(view.code_value("GTD"), Some("6"));
    // A numeric parenthesization is a tag cross-reference, never a spelling.
    assert_eq!(view.code_value("64"), None);
    // Only the abbreviation on the leading phrase counts.
    assert_eq!(view.code_value("svp"), Some("8"));
    assert_eq!(view.code_value("ccp"), None);
}

#[test]
fn a_version_prefers_the_codes_it_knows_and_still_reads_the_rest() {
    let field = side();
    let view = field.as_fix();

    assert_eq!(view.code_value_at(version("4.2"), "CrossShort"), Some("9"));
    // A capture whose frame says 4.1 routinely carries values added in 4.2 -
    // a venue upgrades one side, a bridge relabels a session - and refusing
    // them drops exactly the traffic someone is trying to explain. The
    // version prefers, it does not gate.
    assert_eq!(view.code_value_at(version("4.1"), "CrossShort"), Some("9"));
    assert_eq!(view.code_name_at(version("4.1"), "9"), Some("CrossShort"));
    assert_eq!(view.code_name_at(version("4.2"), "9"), Some("CrossShort"));

    // A code dated by extension pack alone reads the same way, and its
    // pedigree stays readable beside the value it resolved to.
    let comm = comm_type();
    let comm = comm.as_fix();
    assert_eq!(comm.code_value_at(version("4.4"), "BasisPoints"), Some("7"));
    assert_eq!(
        comm.code_value_at(version("5.0SP2"), "BasisPoints"),
        Some("7")
    );
    assert_eq!(comm.code("7").unwrap().ep(), Some(208));

    let mut retired = DataType::Utf8.nullable_field("OldFlag");
    retired.as_fix_mut().set_tag(9996).unwrap();
    retired
        .as_fix_mut()
        .set_codes(&[FixCode::new("Retired", "R")
            .with_since(version("4.0"), None)
            .with_deprecated(version("4.4"))])
        .unwrap();
    let retired = retired.as_fix();
    // Deprecation reads the same way: a venue that never stopped sending a
    // retired code is a venue whose traffic still has to be read, and the
    // code's own `deprecated` pedigree is what says it should not be sent.
    assert_eq!(retired.code_value_at(version("4.3"), "Retired"), Some("R"));
    assert_eq!(retired.code_value_at(version("4.4"), "Retired"), Some("R"));
    assert_eq!(retired.code_value("Retired"), Some("R"));
    assert_eq!(
        retired.code("R").and_then(super::FixCodeValue::deprecated),
        Some(version("4.4")),
        "what the version knows is still readable beside the value",
    );
}

#[test]
fn a_code_set_round_trips_canonically_and_a_hand_edit_names_its_byte_position() {
    let field = side();
    let stored = field
        .as_metadata()
        .get("fix:codes")
        .expect("the code set is stored")
        .to_owned();
    assert_eq!(
        stored,
        concat!(
            r#"{"codes":["#,
            r#"{"value":"1","name":"Buy","since":"2.7","ep":254},"#,
            r#"{"value":"2","name":"Sell","since":"2.7","ep":254},"#,
            r#"{"value":"7","name":"Undisclosed","since":"4.1"},"#,
            r#"{"value":"9","name":"CrossShort","since":"4.2"},"#,
            r#"{"value":"A","name":"CrossShortExempt","since":"4.3"}]}"#,
        )
    );

    // Taking the set away and putting it back produces the same text.
    let mut rebuilt = field.clone();
    let taken = rebuilt.as_fix_mut().remove_codes().unwrap().unwrap();
    assert_eq!(rebuilt.as_metadata().get("fix:codes"), None);
    rebuilt.as_fix_mut().set_codes(&taken).unwrap();
    assert_eq!(
        rebuilt.as_metadata().get("fix:codes"),
        Some(stored.as_str())
    );

    // Keys follow the document's declared order, so a reordered one is
    // refused rather than mis-scanned.
    let reordered = r#"{"codes":[{"name":"Buy","value":"1"}]}"#;
    let mut edited = DataType::Utf8.nullable_field("Side");
    edited.set_metadata([("fix:codes", reordered)]).unwrap();
    let error = edited.as_fix().codes().next().unwrap().unwrap_err();
    assert!(
        matches!(&error, Error::Parse { target, position, .. }
            if *target == "fix codes" && *position == reordered.find(r#""value""#).unwrap()),
        "{error}"
    );
    // A read that cannot parse answers nothing rather than a wrong answer.
    assert_eq!(edited.as_fix().code_value("Buy"), None);
    assert_eq!(edited.as_fix().code("1"), None);
}

#[test]
fn two_codes_may_share_a_value_but_never_a_name_and_neither_may_be_empty() {
    let mut field = DataType::Utf8.nullable_field("Side");
    field.as_fix_mut().set_tag(54).unwrap();

    let error = field
        .as_fix_mut()
        .set_codes(&[FixCode::new("Buy", "1"), FixCode::new("BUY", "2")])
        .unwrap_err();
    assert!(error.to_string().contains("BUY"), "{error}");
    assert_eq!(field.as_metadata().get("fix:codes"), None, "atomic");

    let error = field
        .as_fix_mut()
        .set_codes(&[FixCode::new("Buy", "")])
        .unwrap_err();
    assert!(error.to_string().contains("value"), "{error}");

    // Two names on one value is an alias, which is legal.
    field
        .as_fix_mut()
        .set_codes(&[FixCode::new("Buy", "1"), FixCode::new("Bought", "1")])
        .unwrap();
    assert_eq!(field.as_fix().codes().count(), 2);
    assert_eq!(field.as_fix().code_value("Bought"), Some("1"));
}

#[test]
fn a_code_set_carries_every_fact_the_specification_states_about_a_member() {
    let mut field = DataType::Utf8.nullable_field("Side");
    field.as_fix_mut().set_tag(54).unwrap();
    field
        .as_fix_mut()
        .set_codes(&[FixCode::new("Buy", "1")
            .with_description(r#"Buy; the "long" side"#)
            .with_aliases(["Bought"])
            .with_since(version("2.7"), Some(254))
            .with_deprecated(version("5.0SP2"))
            .with_sort(10)
            .with_group("Directional")])
        .unwrap();

    let view = field.as_fix();
    let code = view.code("1").unwrap();
    assert_eq!(code.name(), "Buy");
    assert_eq!(code.since(), Some(version("2.7")));
    assert_eq!(code.ep(), Some(254));
    assert_eq!(code.deprecated(), Some(version("5.0SP2")));
    assert_eq!(code.sort(), Some(10));
    assert_eq!(code.group(), Some("Directional"));
    // A description holding a quote survives the round trip through the one
    // codec that escaped it.
    assert_eq!(
        code.parse_doc().unwrap().as_deref(),
        Some(r#"Buy; the "long" side"#)
    );
    assert_eq!(code.aliases().collect::<Vec<_>>(), ["Bought"]);
    // An empty set removes the property rather than storing an empty one.
    let mut cleared = field.clone();
    cleared.as_fix_mut().set_codes(&[]).unwrap();
    assert_eq!(cleared.as_metadata().get("fix:codes"), None);
    assert_eq!(cleared.as_fix().codes().count(), 0);
}

#[test]
fn a_field_merge_folds_every_key_by_its_own_rule() {
    // Stored: the older, lower-priority source.
    let mut stored = DataType::Utf8.nullable_field("LastQty");
    stored.as_fix_mut().set_tag(32).unwrap();
    stored.as_fix_mut().set_tags(&[65, 66]).unwrap();
    stored
        .as_fix_mut()
        .set_description("the stored wording")
        .unwrap();
    stored
        .as_fix_mut()
        .set_lineage(&[
            FixLineageEntry::new(FixPedigree::new(version("2.7"), None)).with_name("LastShares"),
            FixLineageEntry::new(FixPedigree::new(version("4.3"), None)).with_name("LastQty"),
        ])
        .unwrap();
    stored
        .as_fix_mut()
        .set_codes(&[
            FixCode::new("StoredOnly", "9"),
            FixCode::new("Shared", "1").with_description("the stored reading"),
        ])
        .unwrap();

    // Incoming: the newer, higher-priority source.
    let mut incoming = DataType::Utf8.nullable_field("LastQty");
    incoming.as_fix_mut().set_tag(32).unwrap();
    incoming.as_fix_mut().set_tags(&[67, 66]).unwrap();
    incoming
        .as_fix_mut()
        .set_description("the incoming wording")
        .unwrap();
    incoming
        .as_fix_mut()
        .set_lineage(&[
            FixLineageEntry::new(FixPedigree::new(version("4.3"), None)).with_name("LastQty"),
            // This entry states a type the 4.3 one does not, so the merge is
            // read on three pedigrees rather than on the collapse of two
            // that say the same thing.
            FixLineageEntry::new(FixPedigree::new(version("5.0SP2"), None))
                .with_name("LastQty")
                .with_dtype("String"),
        ])
        .unwrap();
    incoming
        .as_fix_mut()
        .set_codes(&[
            FixCode::new("IncomingOnly", "5"),
            FixCode::new("Shared", "1").with_description("the incoming reading"),
        ])
        .unwrap();

    incoming.as_fix_mut().merge_with(&stored.as_fix()).unwrap();
    let merged = incoming.as_fix();

    // Identity is not merged, it is agreed.
    assert_eq!(merged.tag().unwrap(), Some(32));
    // Lists union, incoming first, deduplicated.
    assert_eq!(merged.tags().unwrap(), [67, 66, 65]);
    // The description is never compared: incoming has one, so it wins.
    assert_eq!(merged.description(), Some("the incoming wording"));
    // Lineages merge by pedigree and re-sort oldest first.
    let dated: Vec<_> = merged
        .lineage()
        .map(|entry| entry.unwrap().since())
        .collect();
    assert_eq!(dated, [version("2.7"), version("4.3"), version("5.0SP2")]);
    // Codes merge by wire value; the incoming wins a shared one and the
    // stored keeps a value only it has.
    assert_eq!(merged.code_name("1"), Some("Shared"));
    assert_eq!(
        merged.code("1").unwrap().parse_doc().unwrap().as_deref(),
        Some("the incoming reading")
    );
    assert_eq!(merged.code_name("5"), Some("IncomingOnly"));
    assert_eq!(merged.code_name("9"), Some("StoredOnly"));
    // The aliases are rewritten from the merged lineage, never left as the
    // union, so the derivation stays the writer's.
    assert_eq!(merged.aliases().collect::<Vec<_>>(), ["LastShares"]);
}

#[test]
fn a_merge_keeps_a_stored_description_the_incoming_does_not_state() {
    let mut stored = DataType::Utf8.nullable_field("Symbol");
    stored.as_fix_mut().set_tag(55).unwrap();
    stored
        .as_fix_mut()
        .set_description("a very long stored wording nobody wants compared")
        .unwrap();

    let mut incoming = DataType::Utf8.nullable_field("Symbol");
    incoming.as_fix_mut().set_tag(55).unwrap();
    incoming.as_fix_mut().set_aliases(["Ticker"]).unwrap();

    // The FIX half folds the `fix:` keys and nothing else, so on its own it
    // leaves a description alone in both directions: it is not FIX's key.
    incoming.as_fix_mut().merge_with(&stored.as_fix()).unwrap();
    assert_eq!(incoming.as_fix().description(), None);
    assert_eq!(incoming.as_fix().aliases().collect::<Vec<_>>(), ["Ticker"]);

    // The whole merge is two halves, and the generic one carries it: a
    // description the incoming definition does not state is kept from what
    // was stored, because a field's meaning does not disappear when a
    // dictionary that never wrote it down is folded in.
    let mut registry = FixRegistry::from_fields([stored.clone()]).unwrap();
    registry.update(incoming.clone()).unwrap();
    let held = registry.field_by_tag(55).unwrap();
    assert_eq!(
        held.description(),
        Some("a very long stored wording nobody wants compared")
    );
    assert_eq!(held.as_fix().aliases().collect::<Vec<_>>(), ["Ticker"]);

    // And one the incoming definition does state wins, because the caller's
    // ordering is the precedence.
    let mut restated = incoming;
    restated
        .set_description("the wording this source publishes")
        .unwrap();
    registry.update(restated).unwrap();
    assert_eq!(
        registry.field_by_tag(55).unwrap().description(),
        Some("the wording this source publishes")
    );
}

#[test]
fn a_merge_of_disagreeing_identities_is_refused_and_changes_nothing() {
    let mut incoming = DataType::Utf8.nullable_field("Symbol");
    incoming.as_fix_mut().set_tag(55).unwrap();
    incoming.as_fix_mut().set_aliases(["Ticker"]).unwrap();
    let before = incoming.clone();

    let mut other = DataType::Utf8.nullable_field("Symbol");
    other.as_fix_mut().set_tag(56).unwrap();
    let error = incoming
        .as_fix_mut()
        .merge_with(&other.as_fix())
        .unwrap_err();
    assert!(error.is_conflict(), "{error}");
    let message = error.to_string();
    assert!(
        message.contains("55") && message.contains("56"),
        "{message}"
    );
    assert_eq!(incoming, before, "a refusal leaves the field as it was");

    let mut vendor = DataType::Utf8.nullable_field("Symbol");
    vendor.as_fix_mut().set_id(&cme(), 5055).unwrap();
    let mut mine = DataType::Utf8.nullable_field("Symbol");
    mine.as_fix_mut().set_tag(5055).unwrap();
    let error = mine.as_fix_mut().merge_with(&vendor.as_fix()).unwrap_err();
    assert!(error.is_conflict(), "{error}");
    assert!(error.to_string().contains("cme"), "{error}");
}

#[test]
fn a_merge_adding_nothing_leaves_the_field_byte_identical() {
    let mut field = DataType::Utf8.nullable_field("LastQty");
    field.as_fix_mut().set_tag(32).unwrap();
    field.as_fix_mut().set_tags(&[65]).unwrap();
    field.as_fix_mut().set_description("wording").unwrap();
    field
        .as_fix_mut()
        .set_lineage(&[
            FixLineageEntry::new(FixPedigree::new(version("2.7"), None)).with_name("LastShares"),
            FixLineageEntry::new(FixPedigree::new(version("4.3"), None)).with_name("LastQty"),
        ])
        .unwrap();
    field
        .as_fix_mut()
        .set_codes(&[FixCode::new("Shared", "1")])
        .unwrap();

    let before = field.clone();
    let other = field.clone();
    field.as_fix_mut().merge_with(&other.as_fix()).unwrap();
    assert_eq!(field, before);
    // Merging a bare field into a full one is also a no-op.
    let mut bare = DataType::Utf8.nullable_field("LastQty");
    bare.as_fix_mut().set_tag(32).unwrap();
    field.as_fix_mut().merge_with(&bare.as_fix()).unwrap();
    assert_eq!(field, before);
}

/// The committed dictionary, loaded through the store the way every seed case
/// loads it.
fn committed() -> FixRegistry {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("config")
        .join("fix");
    FixRegistry::from_handle(&Folder::new(root).unwrap()).unwrap()
}

#[test]
fn the_component_rule_has_one_case_for_every_arm() {
    use super::component::component_name;

    // Arm 1, Latin, and the only arm that can lose case inside the stem.
    assert_eq!(
        component_name("NoContractualMatrices").as_deref(),
        Some("ContractualMatrix")
    );
    assert_eq!(
        component_name("NoLegContractualMatrices").as_deref(),
        Some("LegContractualMatrix")
    );
    // Matched case-insensitively, and the replacement takes the case it found.
    assert_eq!(
        component_name("Noindices").as_deref(),
        None,
        "No + lowercase is not a counter"
    );
    assert_eq!(component_name("NoXindices").as_deref(), Some("Xindex"));
    // R5 preserves the case of the *first* replaced character, not the run.
    assert_eq!(component_name("NoXINDICES").as_deref(), Some("XIndex"));
    assert_eq!(component_name("NoAppendices").as_deref(), Some("Appendix"));
    assert_eq!(component_name("NoVertices").as_deref(), Some("Vertex"));

    // Arm 2, already singular, tested byte-exact so an uppercase S survives.
    assert_eq!(
        component_name("NoSideTrdRegTS").as_deref(),
        Some("SideTrdRegTS")
    );
    assert_eq!(
        component_name("NoInstrumentParty").as_deref(),
        Some("InstrumentParty")
    );

    // Arm 3, `ss`, which no shipped counter reaches.
    assert_eq!(component_name("NoAddress").as_deref(), Some("Address"));
    assert_eq!(component_name("NoBusiness").as_deref(), Some("Business"));

    // Arm 4, `ies`, and the length guard that keeps `Ties` off it.
    assert_eq!(component_name("NoParties").as_deref(), Some("Party"));
    assert_eq!(
        component_name("NoTies").as_deref(),
        Some("Tie"),
        "four bytes falls to arm 7"
    );

    // Arm 5, `sses`.
    assert_eq!(component_name("NoClasses").as_deref(), Some("Class"));
    assert_eq!(component_name("NoAddresses").as_deref(), Some("Address"));

    // Arm 6, sibilant `es`, which no shipped counter reaches either.
    assert_eq!(component_name("NoBoxes").as_deref(), Some("Box"));
    assert_eq!(component_name("NoBranches").as_deref(), Some("Branch"));
    assert_eq!(component_name("NoBrushes").as_deref(), Some("Brush"));
    assert_eq!(component_name("NoBuzzes").as_deref(), Some("Buzz"));

    // Arm 7, the default, which never removes more than the final `s` - so a
    // trailing uppercase run survives and no acronym pass is needed.
    assert_eq!(component_name("NoPartyIDs").as_deref(), Some("PartyID"));
    assert_eq!(component_name("NoLegs").as_deref(), Some("Leg"));

    // The two results that read oddly and are still right.
    assert_eq!(
        component_name("NoLinesOfText").as_deref(),
        Some("LinesOfText")
    );
    assert_eq!(component_name("NoOfSecSizes").as_deref(), Some("OfSecSize"));

    // Not a counter spelling: the caller keeps the counter's own name. These
    // are the shipped `No...` non-counters the rule must never claim.
    for held in [
        "NotifyBrokerOfCredit",
        "NonCashDividendTreatment",
        "Notification",
        "No",
        "N",
        "",
        "Nothing",
    ] {
        assert_eq!(component_name(held), None, "{held}");
    }
}

#[test]
fn the_component_rule_names_every_shipped_group_distinctly() {
    use super::component::component_name;

    let registry = committed();
    let mut derived: Vec<(SmolStr, SmolStr)> = Vec::new();
    for field in registry.iter() {
        if !matches!(field.dtype(), DataType::List(_) | DataType::LargeList(_)) {
            continue;
        }
        let display = field
            .display()
            .unwrap_or_else(|| panic!("{} states no display", field.name()));
        let name = component_name(display)
            .unwrap_or_else(|| panic!("{display} is a counter the rule does not name"));
        derived.push((SmolStr::new(display), name));
    }

    assert_eq!(derived.len(), 521, "shipped repeating groups");
    let distinct: HashSet<&SmolStr> = derived.iter().map(|(_, name)| name).collect();
    assert_eq!(
        distinct.len(),
        521,
        "one component name per group, none shared"
    );
    // Nothing derives to an empty name, and nothing keeps its counter spelling.
    for (display, name) in &derived {
        assert!(!name.is_empty(), "{display}");
        assert_ne!(name, display, "{display}");
    }
}

#[test]
fn the_fix_walk_reaches_through_a_group_and_never_matches_its_occurrence() {
    use super::component::component_name;

    let registry = committed();

    // The decisive case: `PartyID` under `NoPartyIDs` is tag 448, the member,
    // and never the occurrence struct that will carry the same spelling.
    let held = registry
        .field_by_path("NoPartyIDs.PartyID", None)
        .expect("the member, by its component-shaped spelling");
    assert_eq!(held.as_fix().tag().unwrap(), Some(448));
    assert_eq!(held.name(), "partyid");

    // Every group whose derived name collides with a member of its own struct
    // must resolve to that member. This is the assertion the item rename would
    // otherwise break silently, so it is pinned before the rename and again
    // after it.
    let mut shadowed = 0_usize;
    for field in registry.iter() {
        let (DataType::List(item) | DataType::LargeList(item)) = field.dtype() else {
            continue;
        };
        let display = field.display().expect("a counter states a display");
        let derived = component_name(display).expect("a counter the rule names");
        let Some(member) = item
            .fields()
            .iter()
            .find(|held| crate::types::folds_equal(held.name(), &derived))
        else {
            continue;
        };
        shadowed += 1;
        let path = format!("{}.{derived}", field.name());
        let reached = registry
            .field_by_path(&path, None)
            .unwrap_or_else(|error| panic!("{path}: {error}"));
        assert_eq!(
            reached.name(),
            member.name(),
            "{path} reached the occurrence"
        );
        assert_eq!(
            reached.as_fix().tag().unwrap(),
            member.as_fix().tag().unwrap(),
            "{path}",
        );
    }
    assert_eq!(
        shadowed, 269,
        "groups whose component name a member already carries"
    );

    // An occurrence is not a path segment, so nobody may spell one.
    assert!(
        registry
            .get_field_by_path("NoPartyIDs.item.PartyRole", None)
            .is_none()
    );
    assert!(
        registry
            .get_field_by_path("NoPartyIDs.item", None)
            .is_none()
    );
}

#[test]
fn a_temporal_type_is_adopted_backward_and_no_other_family_is() {
    // A field FIX carried as text before it declared it temporal: the
    // earlier entry adopts the later type and the two then state one thing.
    let rendered = FixLineage::render(&[
        FixLineageEntry::new(FixPedigree::new(version("4.2"), None)).with_dtype("String"),
        FixLineageEntry::new(FixPedigree::new(version("4.4"), None)).with_dtype("UTCTimestamp"),
    ])
    .expect("the entries render");
    assert_eq!(
        rendered,
        concat!(
            r#"{"entries":[{"since":"4.2","type":"#,
            r#"{"type":"datetime64","unit":"nanosecond","timezone":"UTC"}}]}"#,
        )
    );

    // Two temporal eras each take their own preceding run of strings.
    let rendered = FixLineage::render(&[
        FixLineageEntry::new(FixPedigree::new(version("4.0"), None)).with_dtype("String"),
        FixLineageEntry::new(FixPedigree::new(version("4.2"), None)).with_dtype("LocalMktDate"),
        FixLineageEntry::new(FixPedigree::new(version("4.3"), None)).with_dtype("String"),
        FixLineageEntry::new(FixPedigree::new(version("4.4"), None)).with_dtype("UTCTimeOnly"),
    ])
    .expect("the entries render");
    assert_eq!(
        rendered,
        concat!(
            r#"{"entries":[{"since":"4","type":{"type":"datetime64","unit":"nanosecond"}},"#,
            r#"{"since":"4.3","type":{"type":"time64","unit":"nanosecond"}}]}"#,
        )
    );

    // Every other later type is a constraint the earlier version did not
    // carry, so nothing is adopted backward and both entries stand.
    for (earlier, later) in [
        ("String", "Boolean"),
        ("String", "Currency"),
        ("String", "Exchange"),
        ("String", "Country"),
        ("String", "int"),
        ("int", "Qty"),
        ("int", "SeqNum"),
        ("int", "char"),
        ("PriceOffset", "char"),
        // A temporal never yields either: the earlier type already held an
        // instant, and widening it backward would restate a real retype. The
        // zone is what differs here - `UTCDateOnly` and `UTCTimestamp` are one
        // type now, both being an instant in UTC.
        ("LocalMktDate", "UTCTimestamp"),
    ] {
        let rendered = FixLineage::render(&[
            FixLineageEntry::new(FixPedigree::new(version("4.2"), None)).with_dtype(earlier),
            FixLineageEntry::new(FixPedigree::new(version("4.4"), None)).with_dtype(later),
        ])
        .expect("the entries render");
        let held: Vec<_> = FixLineage::over(Some(&rendered))
            .map(|entry| entry.expect("a readable entry"))
            .collect();
        assert_eq!(held.len(), 2, "{earlier} -> {later}: {rendered}");
    }
}

#[test]
fn every_type_adopted_backward_parses_the_wire_spelling_of_its_era() {
    // G8-R7: an earlier entry may only adopt a later temporal type where the
    // text FIX actually transmitted at the earlier version still parses as
    // that type through the reader the crate already has. These are the three
    // types the committed corpus adopts backward, each against the spelling
    // its FIX datatype states on the wire.
    for (tag, spelling, wire, dtype) in [
        (
            9_001,
            "UTCTimeOnly",
            "10:15:30.123",
            DataType::Time64(crate::TimeUnit::Nanosecond),
        ),
        (
            9_002,
            "LocalMktTime",
            "10:15:30",
            DataType::Time64(crate::TimeUnit::Nanosecond),
        ),
        (
            9_003,
            "UTCTimestamp",
            "20240102-10:15:30.123",
            DataType::DateTime64 {
                unit: crate::TimeUnit::Nanosecond,
                timezone: crate::Timezone::UTC,
            },
        ),
        (
            9_004,
            "TZTimeOnly",
            "10:15:30-05:00",
            DataType::DateTime64 {
                unit: crate::TimeUnit::Nanosecond,
                timezone: crate::Timezone::UTC,
            },
        ),
        (
            9_005,
            "LocalMktDate",
            "20240102",
            DataType::DateTime64 {
                unit: crate::TimeUnit::Nanosecond,
                timezone: crate::Timezone::NAIVE,
            },
        ),
    ] {
        let held: DataType = spelling.parse().expect("a resolvable FIX datatype");
        assert_eq!(held, dtype, "{spelling}");

        let mut field = held.nullable_field("dated");
        field.as_fix_mut().set_tag(tag).unwrap();
        let registry = Arc::new(FixRegistry::from_fields([field]).unwrap());
        let message = super::FixCodec::new(registry)
            .transform_pairs([(tag.to_string().as_bytes(), wire.as_bytes())], false)
            .expect("the row builds");
        assert_ne!(
            message.by_tag(tag).unwrap(),
            &Scalar::Null,
            "{spelling} reads {wire}",
        );
    }
}

/// One stored document with every `since` value spelled as `Version` spells it.
///
/// The generator writes the source file's spelling, the crate writes its own,
/// and the two parse to one version. Nothing else in the document is touched.
fn canonical_versions(document: &str) -> String {
    let mut out = String::with_capacity(document.len());
    let mut rest = document;
    while let Some(at) = rest.find(r#""since":""#) {
        let (head, tail) = rest.split_at(at + r#""since":""#.len());
        out.push_str(head);
        let end = tail.find('"').expect("a closed version");
        let (spelling, tail) = tail.split_at(end);
        out.push_str(
            &spelling
                .parse::<Version>()
                .expect("a readable version")
                .to_string(),
        );
        rest = tail;
    }
    out.push_str(rest);
    out
}

/// Every field in the committed dictionary, occurrences and members included.
fn every_committed_field(registry: &FixRegistry) -> Vec<Field> {
    fn walk(field: &Field, out: &mut Vec<Field>) {
        out.push(field.clone());
        match field.dtype() {
            DataType::List(item) | DataType::LargeList(item) => walk(item, out),
            DataType::Struct(fields) => {
                for held in fields.iter() {
                    walk(held, out);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    for field in registry.iter() {
        walk(field, &mut out);
    }
    out
}

#[test]
fn every_committed_lineage_is_the_document_the_rust_writer_renders() {
    let registry = committed();
    let mut lineages = 0_usize;
    let mut entries = 0_usize;
    for field in every_committed_field(&registry) {
        let view = field.as_fix();
        let Some(stored) = field.as_metadata().get("fix:lineage") else {
            continue;
        };
        lineages += 1;
        let held: Vec<_> = view
            .lineage()
            .map(|entry| entry.expect("a readable entry"))
            .collect();
        assert!(!held.is_empty(), "{} keeps its oldest entry", field.name());
        entries += held.len();

        // Every stored type resolves: the generator writes the crate's own
        // serialized datatype, so nothing here is an unresolvable spelling.
        for entry in &held {
            assert!(
                entry.parse_dtype().expect("a resolvable type").is_some(),
                "{} at {}",
                field.name(),
                entry.since()
            );
        }

        // The cross-host assertion: the dictionary generator wrote this
        // document in Python, and re-rendering the entries it holds through
        // the Rust writer must reproduce it byte for byte, or the two hosts
        // have forked on either normalization or collapse.
        //
        // A version is canonicalized first because the two hosts spell one
        // version two ways and always have: the generator carries the source
        // file's own `4.0` while `Version` displays the same value as `4`.
        // Both parse to one version, so this is a spelling the assertion
        // must not be sensitive to; every other byte it is.
        assert_eq!(
            FixLineage::render(&held).expect("the entries render"),
            canonical_versions(stored),
            "{}",
            field.name()
        );

        // Nothing collapsible survives: two adjacent entries always differ in
        // something one of them states.
        for pair in held.windows(2) {
            let (older, newer) = (pair[0], pair[1]);
            assert!(
                older.parse_dtype().unwrap() != newer.parse_dtype().unwrap()
                    || older.name() != newer.name()
                    || older.is_deprecated() != newer.is_deprecated()
                    || older.is_removed() != newer.is_removed()
                    || older.doc() != newer.doc(),
                "{} states nothing new at {}",
                field.name(),
                newer.since()
            );
        }
    }
    assert_eq!(lineages, 1_564, "fields carrying a lineage");
    // 1,926 before these two phases: 268 entries stated nothing their
    // predecessor did not once types were resolved and the temporal ones
    // adopted backward, and one more was a second statement about one dated
    // point.
    assert_eq!(entries, 1_669, "lineage entries");
}

#[test]
fn every_shipped_occurrence_carries_the_name_the_rust_rule_derives() {
    let registry = committed();
    let mut named = 0_usize;
    for field in registry.iter() {
        let (DataType::List(item) | DataType::LargeList(item)) = field.dtype() else {
            continue;
        };
        named += 1;
        // The cross-host assertion: the dictionary generator wrote these names
        // in Python, and the Rust rule must answer the same thing for every
        // one of them or the two hosts have forked.
        assert_eq!(
            item.name(),
            super::occurrence_name(field),
            "{} occurrence",
            field.name(),
        );
        // The occurrence carries only its name: no display, no `fix:tag`, no
        // metadata at all, because its display is the counter's and storing a
        // derivable fact twice is what N4 forbids.
        assert!(item.as_metadata().is_empty(), "{} occurrence", field.name());
        assert_eq!(item.display(), None, "{} occurrence", field.name());
        assert!(
            item.as_fix().tag().unwrap().is_none(),
            "{} occurrence",
            field.name()
        );
        assert!(!item.is_nullable(), "{} occurrence", field.name());
        assert_ne!(item.name(), "item", "{} occurrence", field.name());
    }
    assert_eq!(named, 521, "shipped repeating groups");
}

#[test]
fn an_entry_carries_its_dialect_as_a_fixed_width_digest() {
    // The measured footprint, so the report states the tree's number rather
    // than an estimate. Four members: `i32`, `i64`, and two `SmolStr`.
    //
    // The shape this replaced, laid out by the same rules, is declared beside
    // it so the saving is measured rather than reasoned about: a name is a
    // whole `SmolStr` where a digest is four bytes that pack beside the tag.
    struct Was {
        _tag: i32,
        _branch: Option<SmolStr>,
        _key: SmolStr,
        _value: SmolStr,
    }
    assert_eq!(std::mem::size_of::<Was>(), 80, "the branch-name entry");
    // 80 with the branch as a name, and 80 again with it as an XXH32 beside
    // the tag and a children vector added: the digest paid for the recursion.
    // An entry that never nests allocates nothing for that vector.
    assert_eq!(
        std::mem::size_of::<FixEntry>(),
        80,
        "one entry, as this tree lays it out",
    );

    let cme = cme();
    let mut registry =
        FixRegistry::from_fields([tagged("Symbol", 55), identified("VenueSym", &cme, 5_055)])
            .unwrap();
    registry.set_branch(cme.clone()).unwrap();
    let registry = Arc::new(registry);

    // The reverse resolution the digest exists for.
    assert_eq!(
        registry.branch_by_digest(cme.digest() as i32).unwrap(),
        &cme
    );
    // 0 is the standard branch and resolves to it, so a reader joining the
    // column never meets a row it cannot explain.
    assert_eq!(registry.get_branch_by_digest(0), Some(&FixBranch::STANDARD),);
    // A digest no branch carries names none rather than panicking. The
    // argument is the entry's own signed reading, so this is an ordinary
    // absence rather than a range refusal: every `i32` is a legal digest.
    assert!(registry.get_branch_by_digest(-1).is_none());
    let refused = registry.branch_by_digest(-1).unwrap_err();
    assert!(refused.is_absent(), "{refused}");

    // Every entry states a dialect, the standard one included, and a standard
    // row states 0 rather than nothing.
    let codec = super::FixCodec::new(Arc::clone(&registry)).with_branch(&cme);
    let msg = codec
        .transform_fix_line(b"55=AAPL|5055=XYZ|VenueOwnThing=?|", false)
        .expect("a readable frame");
    let entries = msg.entries();
    assert!(!entries.is_empty());
    let venue = entries.iter().find(|held| held.tag() == 5_055).unwrap();
    assert_eq!(venue.branch(), cme.digest() as i32);
    assert_eq!(
        registry.branch_by_digest(venue.branch()).unwrap(),
        &cme,
        "the digest resolves to the dialect that answered the pair",
    );
    // A key that named no field at all resolved in no dialect, which is 0.
    let unknown = entries
        .iter()
        .find(|held| held.tag() == 0)
        .expect("the key no dictionary explained");
    assert_eq!(unknown.branch(), FixBranch::STANDARD.digest() as i32);
    assert_eq!(unknown.branch(), 0);
    assert_eq!(unknown.id(), None, "no tag is no identity");

    // The identity an entry names is packed from the digest it stores.
    assert_eq!(venue.id().unwrap(), FixId::from_parts(&cme, 5_055).unwrap());
}

#[test]
fn the_entry_column_states_a_non_null_branch() {
    let root = super::fix_schema(&FixRegistry::new(), "row").unwrap();
    for column in [super::ENTRIES_COLUMN, super::UNMAPPED_COLUMN] {
        let held = root
            .fields()
            .iter()
            .find(|field| field.name() == column)
            .unwrap_or_else(|| panic!("a {column} column"));
        let DataType::List(item) = held.dtype() else {
            panic!("a list, got {}", held.dtype());
        };
        // Exactly three fixentry levels on every root-to-leaf path, each with
        // the same five members, the fifth a non-null nofixentries that is a
        // deeper list twice and the binary leaf at the bottom.
        let mut held = item;
        for level in 1..=3 {
            assert_eq!(held.name(), "fixentry", "{column} level {level}");
            assert!(!held.is_nullable(), "{column} level {level}");
            let members = held.dtype().as_fields().expect("an occurrence struct");
            let names: Vec<&str> = members.iter().map(Field::name).collect();
            assert_eq!(
                names,
                ["tag", "branch", "key", "value", "nofixentries"],
                "{column} level {level}",
            );
            let branch = &members[1];
            assert_eq!(branch.dtype(), &DataType::Int32, "{column} level {level}");
            assert!(
                !branch.is_nullable(),
                "{column} branch carries no validity bitmap"
            );
            let tail = &members[4];
            assert!(!tail.is_nullable(), "{column} level {level} tail");
            match tail.dtype() {
                DataType::List(deeper) if level < 3 => held = deeper,
                DataType::Binary if level == 3 => break,
                other => panic!("{column} level {level}: {other}"),
            }
        }
    }
}

/// A chain of entries `depth` long, each child the sole passenger of the one
/// above, ending in a leaf pair carrying `value`.
fn nested_entries(depth: usize, value: &str) -> Vec<FixEntry> {
    let mut held = FixEntry::new(523, "523", value);
    for level in (1..depth).rev() {
        let mut parent = FixEntry::new(453, "453", level.to_string());
        parent.push(held);
        held = parent;
    }
    vec![FixEntry::new(35, "35", "D"), held]
}

#[test]
fn a_deep_arrival_materializes_three_levels_and_folds_the_rest() {
    let registry = Arc::new(committed());
    let root = DataType::from_fields([DataType::Utf8.nullable_field("35")])
        .unwrap()
        .required_field("D");
    let message = |value: &str| {
        FixMsg::from_parts(
            Arc::clone(&registry),
            root.clone(),
            Scalar::from_sequence([Scalar::from("D")]),
            nested_entries(5, value),
        )
        .unwrap()
    };
    let deep = message("x");

    // The Rust tree is never truncated: all five levels are held whole.
    let mut held = &deep.entries()[1];
    let mut levels = 1;
    while let Some(next) = held.children().first() {
        held = next;
        levels += 1;
    }
    assert_eq!(levels, 5, "the record holds what the wire nested");

    // The Arrow value materializes exactly three fixentry levels; the fourth
    // and fifth fold into a non-empty leaf.
    let schema = super::fix_schema(&registry, "row").unwrap();
    let row = deep.to_row(&schema).unwrap();
    let columns = row.as_sequence().expect("a row").to_vec();
    let entries = columns[columns.len() - 2]
        .as_sequence()
        .expect("the arrival column");
    let level1 = entries[1].as_sequence().expect("the counter entry");
    let level2 = level1[4].as_sequence().expect("one child list")[0]
        .as_sequence()
        .expect("the level-2 entry")
        .to_vec();
    let level3 = level2[4].as_sequence().expect("one child list")[0]
        .as_sequence()
        .expect("the level-3 entry")
        .to_vec();
    let leaf = level3[4].as_bytes().expect("the binary leaf");
    assert!(!leaf.is_empty(), "two levels folded into it");

    // The leaf recovers exactly the folded entries through the one JSON
    // parser this crate has: level 4 carrying level 5.
    let decoded = crate::from_json_scalar(leaf).expect("a decodable leaf");
    let folded = decoded.as_sequence().expect("the folded children");
    assert_eq!(folded.len(), 1);
    let level4 = folded[0].as_sequence().expect("the level-4 entry").to_vec();
    assert_eq!(level4[0].as_i64(), Some(453));
    let level5 = level4[4].as_sequence().expect("its children")[0]
        .as_sequence()
        .expect("the level-5 entry")
        .to_vec();
    assert_eq!(level5[0].as_i64(), Some(523));
    assert_eq!(level5[3].as_str(), Some("x"));

    // A flat sibling's child list is empty: nothing arrived under it and
    // nothing was folded for it.
    let flat = entries[0].as_sequence().expect("the msgtype entry");
    assert_eq!(flat[4].as_sequence().map(<[Scalar]>::len), Some(0));

    // Wire emission walks the whole tree pre-order, so what comes back is
    // what went in.
    let bytes = deep.into_bytes(b'|');
    assert_eq!(
        std::str::from_utf8(&bytes).unwrap(),
        "35=D|453=1|453=2|453=3|453=4|523=x|",
    );

    // Two messages that differ only below the materialization depth still
    // hash apart, because the digest walks the untruncated tree.
    assert_ne!(message("x").digest(), message("y").digest());
    // And a parent of one is not a flat pair beside one: the hashed child
    // count is what separates them.
    let nested = FixMsg::from_parts(
        Arc::clone(&registry),
        root.clone(),
        Scalar::from_sequence([Scalar::from("D")]),
        nested_entries(2, "x"),
    )
    .unwrap();
    let flattened = FixMsg::from_parts(
        Arc::clone(&registry),
        root.clone(),
        Scalar::from_sequence([Scalar::from("D")]),
        vec![
            FixEntry::new(35, "35", "D"),
            FixEntry::new(453, "453", "1"),
            FixEntry::new(523, "523", "x"),
        ],
    )
    .unwrap();
    assert_ne!(nested.digest(), flattened.digest());

    // Depth is a materialization concern, never a refusal: thirty levels
    // read, fold and type without a complaint.
    let towering = FixMsg::from_parts(
        Arc::clone(&registry),
        root.clone(),
        Scalar::from_sequence([Scalar::from("D")]),
        nested_entries(30, "deep"),
    )
    .unwrap();
    let row = towering.to_row(&schema).expect("no depth refusal");
    assert!(row.as_sequence().is_some());
}
