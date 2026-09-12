//! Focused edge cases of the FIX module, driven with explicit inputs.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use smol_str::SmolStr;

use super::global::autoload;
use super::registry::control_byte;
use super::store::shard_of;
use crate::fix::{
    FixCodes, FixFill, FixFillEntry, FixFillSource, FixFillValue, FixReplacement, FixReplacements,
};
use crate::holder::local::Folder;
use crate::types::BytesParameters;
use crate::{
    DataType, Error, Field, FixCategory, FixCode, FixCodec, FixEntry, FixId, FixKey, FixLineage,
    FixLineageEntry, FixMsg, FixPedigree, FixRegistry, MimeType, Scalar, Version,
};

/// One path, resolved once, as every FIX navigator now takes it.
fn fpath(spelling: &str) -> crate::FieldPath {
    crate::FieldPath::from_str(spelling).unwrap_or_else(|error| panic!("{spelling}: {error}"))
}

/// A nullable text field carrying one canonical tag.
fn tagged(name: &str, tag: i32) -> Field {
    let mut field = DataType::utf8().nullable_field(name);
    field.as_fix_mut().set_tag(tag).unwrap();
    field
}

/// A nullable text field one venue dictionary contributed: a tag, and the
/// membership that says whose it is.
fn member(name: &str, dialect: &str, tag: i32) -> Field {
    let mut field = tagged(name, tag);
    field.as_fix_mut().set_branches([dialect]).unwrap();
    field
}

/// The identity a tag and a name make, for a fixture that states both.
fn id_of(tag: i32, name: &str) -> FixId {
    FixId::of(tag, name).unwrap()
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

fn counter(name: &str, tag: i32) -> Field {
    let mut field = DataType::Int32.nullable_field(name);
    field.as_fix_mut().set_tag(tag).unwrap();
    field
}

fn named_group(name: &str, tag: i32) -> Field {
    let item = DataType::from_fields([tagged("Member", 9_002)])
        .unwrap()
        .required_field("MemberComponent");
    let mut field = DataType::list(item).nullable_field(name);
    field.as_fix_mut().set_counter(tag).unwrap();
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

/// How many fields every registry holds before a test inserts one: the
/// crate's own, which `FixRegistry::new` seeds.
fn crated() -> usize {
    crate::fix_crate_fields().unwrap().len()
}

/// The crate's own field names, in the order every registry iterates them:
/// last, because their tags are above every tag a test claims.
fn crate_names() -> Vec<&'static str> {
    crate::fix_crate_fields()
        .unwrap()
        .iter()
        .map(Field::name)
        .collect()
}

/// `names`, then the crate's own: the order a registry holding `names`
/// iterates.
fn then_crated<'a>(names: &[&'a str]) -> Vec<&'a str> {
    names.iter().copied().chain(crate_names()).collect()
}

/// The stored keys of one registry, probed through every index.
fn probe<'registry>(
    registry: &'registry FixRegistry,
    tag: i32,
    alternate: i32,
    name: &str,
    alias: &str,
) -> [Option<&'registry str>; 4] {
    [
        registry.get_field_by_tag(tag).map(Field::name),
        registry.get_field_by_tag(alternate).map(Field::name),
        registry.get_field_by_name(name).map(Field::name),
        registry.get_field_by_name(alias).map(Field::name),
    ]
}

#[test]
fn name_indexes_fold_ascii_and_membership_never_resolves() {
    let standard = tagged("MsgType", 35);
    let vendor = member("VenueSym", "cme", 5_035);
    let registry = FixRegistry::from_fields([standard.clone(), vendor.clone()]).unwrap();
    for spelling in ["MSGTYPE", "msgtype", "Msg_Type", "msg-type"] {
        assert_eq!(
            registry.get_field_by_name(spelling),
            Some(&standard),
            "{spelling}"
        );
    }
    // One namespace: a venue's field is reached by its name exactly as the
    // specification's is, and what the membership says is who spoke it.
    assert_eq!(registry.get_field_by_name("venuesym"), Some(&vendor));
    assert!(vendor.as_fix().has_branch("cme"));
    assert!(!standard.as_fix().has_branch("cme"));
    assert!(registry.get_field_by_name("cme").is_none());
    assert_eq!(registry.dialects(), ["cme"]);
}

#[test]
fn three_spellings_of_one_name_under_one_tag_are_one_identity() {
    let registry = FixRegistry::from_fields([tagged("MsgType", 35)]).unwrap();
    let held = registry
        .field_by_tag(35)
        .unwrap()
        .as_fix()
        .id()
        .unwrap()
        .expect("a tagged field has an identity");
    for spelling in ["Msg_Type", "msgtype", "MsgType", "MSG-TYPE", "Msg Type"] {
        let id = id_of(35, spelling);
        assert_eq!(id, held, "{spelling}");
        assert_eq!(
            registry.get_field_by_id(id).map(Field::name),
            Some("MsgType")
        );
    }
    // Both halves reach the digest: another tag or another name is another
    // identity.
    assert_ne!(id_of(36, "MsgType"), held);
    assert_ne!(id_of(35, "MsgSeqNum"), held);
    assert!(registry.get_field_by_id(id_of(36, "MsgType")).is_none());
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
            Some(b"UL"),
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
        assert_eq!(FixCodec::infer_msgtype_bytes(line), *msgtype, "{line:?}");
        let text = std::str::from_utf8(line).unwrap();
        assert_eq!(&MimeType::infer_text(text), protocol, "{text}");
        assert_eq!(
            FixCodec::infer_msgtype_text(text),
            msgtype.and_then(|value| std::str::from_utf8(value).ok()),
            "{text}"
        );
    }

    // An overflowing numeric key is one bad candidate, not a reason to stop
    // before a later valid frame in the same log line.
    let overflow = b"999999999999999999999=x 8=FIX.4.4|35=D|";
    assert_eq!(MimeType::infer_bytes(overflow), MimeType::FIX);
    assert_eq!(
        FixCodec::infer_msgtype_bytes(overflow),
        Some(b"D".as_slice())
    );

    // Classification needs no dictionary at all: it is transport, and every
    // captured line has a shape whatever protocol it carried.
    assert_eq!(MimeType::infer_bytes(b"35=D|55=AAPL|"), MimeType::FIX);
    assert_eq!(
        FixCodec::infer_msgtype_bytes(b"35=D|55=AAPL|"),
        Some(&b"D"[..])
    );
    assert_eq!(
        MimeType::infer_text("ACCOUNT=A1|MSGTYPE=8|"),
        MimeType::ULLINK
    );
    assert_eq!(
        FixCodec::infer_msgtype_text("ACCOUNT=A1|MSGTYPE=8|"),
        Some("8")
    );

    assert_eq!(MimeType::ULLINK.as_str(), "text/ullink");
    assert_eq!(MimeType::FIX.as_str(), "text/fix");
    assert_eq!(MimeType::FIXUL.as_str(), "text/fixul");
    assert_eq!(MimeType::FIXML.as_str(), "text/fixml");

    for value in ["U1", "UABC", "UL"] {
        let line = format!("8=FIX.4.4|35={value}|10=000|");
        assert_eq!(FixCodec::infer_msgtype_text(&line), Some(value));
    }
    assert_eq!(FixCodec::infer_msgtype_text("35=U|"), Some("U"));
}

#[test]
fn the_prose_in_front_of_a_configuration_document_names_its_half_and_the_document_nothing() {
    let reading = FixRegistry::new().msgdirection();

    const ANSWERED: &[u8] = br#"{"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:*","type":"read"},"value":{"com.ullink.ulbridge.sessioninterfaces.plugins:name=X,plugin-type=FIX,type=Plugin":{"ExtendedActions":[{"name":"send-test-request","description":"Send a test request message.","parameters":[{"name":"test-request-id","description":"The outgoing test request ID to send"}]}]}},"status":200}"#;
    const ASKED: &[u8] =
        br#"{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:*","type":"read"}"#;

    // A document states nothing of which way it moved (decision 15): the
    // words inside it - `send`, `outgoing`, `in` - are its own payload rather
    // than a transport marker, and the echoed `request` key is the answer's
    // shape, not a direction. Without the bound the words would answer, and
    // answer wrongly.
    assert_eq!(reading.read_bytes(ANSWERED), None);
    assert_eq!(reading.read_bytes(ASKED), None);
    // The prose Jolokia writes in front of the document does say: an answer
    // came back, a request went out, as codes of tag 385's set.
    let answered = [b"Response: ".as_slice(), ANSWERED].concat();
    assert_eq!(reading.read_bytes(&answered), Some("R"));
    let asked = [b"Request: ".as_slice(), ASKED].concat();
    assert_eq!(reading.read_bytes(&asked), Some("S"));
    // An error is an answer that came back, and a bare one still says so
    // only through its prose.
    let failed =
        br#"{"request":{"mbean":"com.ullink.ulbridge:*","type":"read"},"error":"no such MBean"}"#;
    assert_eq!(reading.read_bytes(failed), None);
    let failed = [b"[Jolokia] (DEBUG) Response: ".as_slice(), failed].concat();
    assert_eq!(reading.read_bytes(&failed), Some("R"));
    // A write states a `value` of its own; the keys a document carries never
    // were the reading, and the prefix bound still holds them out of it.
    let write = br#"{"type":"write","mbean":"com.ullink.ulbridge:type=Bridge","attribute":"LogLevel","value":3}"#;
    assert_eq!(reading.read_bytes(write), None);
    assert_eq!(MimeType::infer_bytes(write), MimeType::ULCONFIG);
    assert_eq!(FixCodec::infer_msgtype_bytes(write), Some(&b"Bridge"[..]));

    // A bulk read answers an array of these, and an array closes with `]`.
    let bulk = br#"[{"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:name=A,type=Plugin","type":"read"},"status":200},{"request":{"mbean":"com.ullink.ulbridge:type=Bridge","type":"read"},"status":200}]"#;
    assert_eq!(MimeType::infer_bytes(bulk), MimeType::ULCONFIG);
    assert_eq!(FixCodec::infer_msgtype_bytes(bulk), Some(&b"Plugin"[..]));
    assert_eq!(reading.read_bytes(bulk), None);

    // The `type=` is read inside the ObjectName that states it, so a value
    // elsewhere spelling the same five bytes is that value's business.
    let quoting = br#"{"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:*","type":"read"},"value":{"Comment":"routed by ,type=Decoy"},"status":200}"#;
    assert_eq!(FixCodec::infer_msgtype_bytes(quoting), Some(&b"read"[..]));

    // A verb the transport wrote is read like any other prose.
    let marked = [b"sending >> ".as_slice(), ANSWERED].concat();
    assert_eq!(reading.read_bytes(&marked), Some("S"));
    // The bound is the whole prefix, so a `[jolokia]` in the prose is prose.
    let stamped = [
        b"2026-08-14 09:12:03 INFO [jolokia] recv << ".as_slice(),
        ANSWERED,
    ]
    .concat();
    assert_eq!(reading.read_bytes(&stamped), Some("R"));
    assert_eq!(MimeType::infer_bytes(&stamped), MimeType::ULCONFIG);
    assert_eq!(
        FixCodec::infer_msgtype_bytes(&stamped),
        Some(&b"Plugin"[..])
    );

    // The prose's reading is filled as tag 385 on the line door, which takes
    // no pin; a bare document fills nothing there.
    let codec = FixCodec::new(Arc::new(FixRegistry::new()));
    let message = codec
        .parse_line(&answered)
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(message.by_tag(385).unwrap().as_str(), Some("R"));
    let message = codec.parse_line(ANSWERED).unwrap().next().unwrap().unwrap();
    assert_eq!(message.get_by_tag(385), None);

    let named = br#"{"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:name=A,type=ConfigurationPlugin","type":"read"},"status":200}"#;
    assert_eq!(
        FixCodec::infer_msgtype_bytes(named),
        Some(b"ConfigurationPlugin".as_slice()),
    );
    let stored = DataType::utf8()
        .scalar(Scalar::from("ConfigurationPlugin"))
        .expect("message names remain complete text");
    assert_eq!(stored.as_str(), Some("ConfigurationPlugin"));

    // A class the bridge spells is not an MBean it names: every `$type`,
    // `className` and init file in these documents carries one, and a record
    // quoting one is an ordinary JSON record.
    let quoted = br#"{"level":"INFO","message":"reloading com.ullink.ulbridge2.plugins.ULMsg"}"#;
    assert_eq!(MimeType::infer_bytes(quoted), MimeType::JSON);
    assert_eq!(reading.read_bytes(quoted), None);
    // The namespace also has to stand inside the document rather than in the
    // prose in front of it, or the prose is what named it.
    let prose = br#"reloading com.ullink.ulbridge.sessioninterfaces.plugins:* {"level":"INFO"}"#;
    assert_eq!(MimeType::infer_bytes(prose), MimeType::OCTET_STREAM);
    // An unterminated document is not one.
    let truncated = br#"{"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:*","#;
    assert_eq!(MimeType::infer_bytes(truncated), MimeType::OCTET_STREAM);

    // The whole reading is borrowed out of the caller's bytes.
    let read = FixCodec::infer_msgtype_bytes(ANSWERED).unwrap();
    assert!(ANSWERED.as_ptr_range().contains(&read.as_ptr()));
}

/// One Jolokia read of one session interface, trimmed to what is read.
const ULCONFIG_SINGLE: &[u8] = br#"{"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:name=ULMSG_BROKER_TO_POSTTRADE,plugin-type=FIX,type=Plugin","type":"read"},"value":{"SenderCompID":"ULB_BKRBDG","TargetCompID":"ULB_PTBDG","BeginString":"FIX.4.2","Category":"InterBridge","PrimaryHost":"localhost","CurrentPort":7061,"BackupHost":null,"BackupPort":-1,"OutgoingMsgSeqNum":129,"NeedReload":false,"Name":"ULMSG_BROKER_TO_POSTTRADE","ExtendedActions":[{"name":"hot-reset","enabled":true}]},"status":200}"#;

/// One wildcard read, answering for two MBeans of different types.
const ULCONFIG_WILDCARD: &[u8] = br#"{"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:*","type":"read"},"value":{"com.ullink.ulbridge.sessioninterfaces.plugins:name=A,plugin-type=FIX,type=ConfigurationPlugin":{"Name":"A","Category":"InterBridge"},"com.ullink.ulbridge.sessioninterfaces.plugins:name=B,plugin-type=FIX,type=Plugin":{"Name":"B","CurrentPort":9905,"SenderCompID":"CLIENT_BPAG"}},"status":200}"#;

/// A codec over the shipped dictionary, holding ULBridge's own fields too.
fn ulbridge_codec() -> crate::FixCodec {
    static REGISTRY: std::sync::OnceLock<Arc<FixRegistry>> = std::sync::OnceLock::new();
    let registry =
        Arc::clone(REGISTRY.get_or_init(|| {
            Arc::new(committed().as_ref().clone().with_ulbridge_fields().unwrap())
        }));
    crate::FixCodec::new(registry)
}

#[test]
fn a_bridge_configuration_reads_as_one_flat_message() {
    let codec = ulbridge_codec();
    let mut messages = codec.parse_line(ULCONFIG_SINGLE).unwrap();
    let msg = messages.next().unwrap().unwrap();
    assert!(messages.next().is_none());
    assert!(msg.get_by_name("SessionInterfaces").is_none());

    // The envelope is what the exchange was, and it types: a status is a
    // number rather than the text it arrived as.
    assert_eq!(
        msg.by_tag(crate::MBEAN_TAG_NAME.0).unwrap().as_str(),
        Some(
            "com.ullink.ulbridge.sessioninterfaces.plugins:name=ULMSG_BROKER_TO_POSTTRADE,plugin-type=FIX,type=Plugin"
        ),
    );
    assert_eq!(
        msg.by_tag(crate::OPERATION_TAG_NAME.0).unwrap(),
        &Scalar::from("read")
    );
    assert_eq!(
        msg.by_tag(crate::STATUS_TAG_NAME.0).unwrap(),
        &Scalar::from(200_i64)
    );

    // A field the specification publishes keeps the specification's tag, and
    // it resolves beside the bridge's own because the two share one
    // namespace: membership says who spoke a field and never resolves it.
    assert_eq!(
        msg.by_name("SenderCompID").unwrap(),
        &Scalar::from("ULB_BKRBDG"),
    );
    assert_eq!(
        msg.by_name("TargetCompID").unwrap(),
        &Scalar::from("ULB_PTBDG"),
    );
    assert_eq!(
        msg.by_name("BeginString").unwrap(),
        &Scalar::from("FIX.4.2"),
    );

    // The ObjectName's own properties are read where the classifier reads
    // them, so one spelling answers for the column and for the row.
    assert_eq!(msg.by_name("MBeanType").unwrap(), &Scalar::from("Plugin"),);
    assert_eq!(msg.by_name("PluginType").unwrap(), &Scalar::from("FIX"),);

    // Everything else types to what it is rather than to the text it was.
    assert_eq!(msg.by_name("CurrentPort").unwrap(), &Scalar::from(7061_i64),);
    assert_eq!(
        msg.by_name("BackupPort").unwrap(),
        &Scalar::from(-1_i64),
        "a sentinel is a number the bridge sent, not an absence",
    );
    assert_eq!(msg.by_name("NeedReload").unwrap(), &Scalar::from(false),);
    // A stated null is an absence: no field, no entry.
    assert!(msg.get_by_name("BackupHost").is_none());

    // Nested attributes retain their canonical JSON text in a scalar field.
    let actions = msg
        .by_name("ExtendedActions")
        .unwrap()
        .as_str()
        .expect("the array, as text");
    assert!(actions.contains("hot-reset"), "{actions}");
}

#[test]
fn a_wildcard_read_is_one_flat_message_per_mbean() {
    let codec = ulbridge_codec();
    let messages = codec
        .parse_line(ULCONFIG_WILDCARD)
        .unwrap()
        .collect::<crate::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(messages.len(), 2);
    for message in &messages {
        assert_eq!(
            message.by_tag(crate::MBEAN_TAG_NAME.0).unwrap(),
            &Scalar::from("com.ullink.ulbridge.sessioninterfaces.plugins:*")
        );
        assert!(message.get_by_name("SessionInterfaces").is_none());
    }
    assert_eq!(messages[0].by_name("Name").unwrap(), &Scalar::from("A"));
    assert_eq!(messages[1].by_name("Name").unwrap(), &Scalar::from("B"));
    assert_eq!(
        messages[0].by_name("MBeanType").unwrap(),
        &Scalar::from("ConfigurationPlugin")
    );
    assert_eq!(
        messages[1].by_name("MBeanType").unwrap(),
        &Scalar::from("Plugin")
    );
    assert_eq!(
        messages[1].by_name("SenderCompID").unwrap(),
        &Scalar::from("CLIENT_BPAG")
    );
    let recovered = crate::UlPlugin::from_fixmsg(&messages[1]).unwrap();
    assert_eq!(recovered.name(), Some("B"));
    assert_eq!(recovered.get("CurrentPort"), Some(&Scalar::from(9905_i64)));
    let rebuilt = recovered.into_fixmsg(&codec).unwrap();
    assert_eq!(
        rebuilt.by_name("MBean").unwrap(),
        messages[1].by_name("MBean").unwrap()
    );
    assert_eq!(
        rebuilt.by_name("SessionInterface").unwrap(),
        messages[1].by_name("SessionInterface").unwrap()
    );
    // A dictionary without ULBridge's fields keeps every key rather than
    // dropping it: a venue sends fields no dictionary has.
    let bare = crate::FixCodec::new(Arc::new(FixRegistry::new()));
    let plain = bare
        .parse_line(ULCONFIG_WILDCARD)
        .unwrap()
        .collect::<crate::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(plain.len(), 2);
    assert!(plain[0].get_by_name("mbean").is_some());
    assert!(plain[0].get_by_tag(crate::MBEAN_TAG_NAME.0).is_none());
}

#[test]
fn ulconfig_bulk_iteration_keeps_request_and_error_envelopes_and_fuses() {
    let document = crate::from_json_scalar(br#"[{"request":{"mbean":"com.ullink.ulbridge:type=Bridge","type":"read"},"status":404,"error":"missing"},{"request":{"mbean":"com.ullink.ulbridge:type=Bridge","type":"read"},"value":{"Name":"Bridge","CurrentPort":9905}}]"#).unwrap();
    let mut configurations = crate::UlPlugin::from_json_scalar(&document).unwrap();
    assert_eq!(configurations.size_hint(), (0, None));
    let error = configurations.next().unwrap();
    let value = configurations.next().unwrap();
    assert_eq!(value.name(), Some("Bridge"));
    assert!(configurations.next().is_none());
    assert!(configurations.next().is_none());
    assert_eq!(configurations.size_hint(), (0, Some(0)));
    let codec = ulbridge_codec();
    let message = error.into_fixmsg(&codec).unwrap();
    assert_eq!(message.by_name("Status").unwrap(), &Scalar::from(404_i64));
    assert_eq!(message.by_name("Error").unwrap(), &Scalar::from("missing"));
    assert!(message.get_by_name("SessionInterface").is_none());
    let empty = br#"{"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:*","type":"read"},"value":{},"status":200}"#;
    let mut values = crate::UlPlugin::from_json_bytes(empty).unwrap();
    assert!(values.next().is_none());
    assert!(values.next().is_none());
    let request = br#"{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:*","type":"read"}"#;
    let request = crate::UlPlugin::from_json_bytes(request)
        .unwrap()
        .next()
        .unwrap();
    assert!(request.mbean().is_none());
    let message = request.into_fixmsg(&codec).unwrap();
    assert_eq!(message.by_name("Operation").unwrap(), &Scalar::from("read"));
    assert!(message.get_by_name("SessionInterface").is_none());
}

#[test]
fn a_selected_ulconfig_identity_excludes_other_mbeans() {
    use std::hash::{Hash, Hasher};
    let document = String::from_utf8(ULCONFIG_WILDCARD.to_vec()).unwrap();
    let changed_sibling = document.replace("9905", "9906");
    let first = crate::UlPlugin::from_json_bytes(document.as_bytes())
        .unwrap()
        .next()
        .unwrap();
    let same = crate::UlPlugin::from_json_bytes(changed_sibling.as_bytes())
        .unwrap()
        .next()
        .unwrap();
    assert_eq!(first, same);
    let rebuilt = crate::UlPlugin::new(
        first.mbean(),
        first.as_attributes().clone(),
        first.as_envelope().clone(),
    );
    assert_eq!(first, rebuilt);
    let digest = |value: &crate::UlPlugin| {
        let mut hasher = std::hash::DefaultHasher::new();
        value.hash(&mut hasher);
        hasher.finish()
    };
    assert_eq!(digest(&first), digest(&same));
    assert_eq!(first.stable_hash(), same.stable_hash());
    assert_eq!(first.stable_hash(), rebuilt.stable_hash());
    let changed_exchange = document.replace("\"status\":200", "\"status\":503");
    let changed = crate::UlPlugin::from_json_bytes(changed_exchange.as_bytes())
        .unwrap()
        .next()
        .unwrap();
    assert_ne!(first, changed);
    assert_ne!(first.stable_hash(), changed.stable_hash());
}

#[test]
fn ulconfig_conversion_reports_an_unrepresentable_attribute() {
    let attributes = Scalar::from_record([("CurrentPort", Scalar::from(f64::NAN))]).unwrap();
    let value = crate::UlPlugin::new(None, attributes, Scalar::Null);
    let codec = crate::FixCodec::new(Arc::new(FixRegistry::new()));
    let error = value.into_fixmsg(&codec).unwrap_err();
    assert!(error.to_string().contains("non-finite"), "{error}");
}

#[test]
fn ulconfig_intake_refuses_malformed_bulk_responses_before_yielding() {
    for body in [b"null".as_slice(), b"true", b"1", br#""text""#] {
        let error = crate::UlPlugin::from_json_bytes(body).unwrap_err();
        assert!(error.to_string().contains("ulconfig"), "{error}");
    }
    for body in [
        br#"[{"value":{"Name":"valid"}},42]"#.as_slice(),
        br#"[{"value":{"Name":"valid"}},[]]"#,
    ] {
        let error = crate::UlPlugin::from_json_bytes(body).unwrap_err();
        assert!(error.to_string().contains("ulconfig[1]"), "{error}");
    }
    let mut empty = crate::UlPlugin::from_json_bytes(b"[]").unwrap();
    assert!(empty.next().is_none());
    assert!(empty.next().is_none());
}

#[test]
fn ulbridge_fields_are_a_dictionary_of_their_own() {
    let held = crate::fix_ulbridge_fields().unwrap();
    assert_eq!(held[0].name(), "MBean");
    // The name is what keeps a venue's own 20001 a different field: the
    // identity is the tag and the name together.
    let (tag, name) = crate::MBEAN_TAG_NAME;
    assert_eq!(held[0].as_fix().id().unwrap(), Some(id_of(tag, name)));
    assert_ne!(held[0].as_fix().id().unwrap().unwrap(), id_of(tag, "Venue"));

    // Every tag this dictionary claims is from its own block, and every
    // field says whose dictionary it is.
    for field in held {
        let tag = field.as_fix().tag().unwrap().expect("a tag");
        assert!(tag >= crate::ULBRIDGE_TAG_MIN, "{}: {tag}", field.name());
        assert!(
            field.as_fix().has_branch(crate::ULBRIDGE_DIALECT),
            "{}",
            field.name()
        );
        assert_eq!(
            field.as_fix().branches().collect::<Vec<_>>(),
            [crate::ULBRIDGE_DIALECT],
            "{}",
            field.name()
        );
        assert!(field.description().is_some(), "{}", field.name());
        assert!(!field.dtype().is_nested(), "{}", field.name());
        assert_ne!(tag, 20_005);
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
    assert_eq!(registry.len(), held.len() + crated());
    assert_eq!(registry.dialects(), [crate::ULBRIDGE_DIALECT]);
    assert!(
        registry
            .get_definition(crate::FixCategory::Groups, "SessionInterfaces")
            .is_none()
    );
}

#[test]
fn membership_folds_once_sorts_and_refuses_what_it_cannot_hold() {
    let mut field = tagged("TradeID", 5001);
    // Folded by ASCII case, deduplicated under the fold, and sorted, so two
    // registries built from the same dictionaries in any order hash alike.
    field
        .as_fix_mut()
        .set_branches(["Globex", "CME", "cme", "globex"])
        .unwrap();
    assert_eq!(field.get_metadata("fix:branches"), Some("cme,globex"));
    assert_eq!(
        field.as_fix().branches().collect::<Vec<_>>(),
        ["cme", "globex"]
    );
    assert!(field.as_fix().has_branch("CME") && field.as_fix().has_branch("globex"));
    assert!(!field.as_fix().has_branch("cm"));

    // Adding is idempotent under the fold, and keeps the list sorted.
    field.as_fix_mut().add_branch("GLOBEX").unwrap();
    assert_eq!(field.get_metadata("fix:branches"), Some("cme,globex"));
    field.as_fix_mut().add_branch("Blp").unwrap();
    assert_eq!(field.get_metadata("fix:branches"), Some("blp,cme,globex"));

    // Held to the alias grammar: non-empty, no separator. A refusal leaves
    // the field exactly as it was.
    let before = field.clone();
    for refused in [vec!["cme", ""], vec!["cm,e"]] {
        let error = field
            .as_fix_mut()
            .set_branches(refused.clone())
            .unwrap_err();
        assert!(
            matches!(&error, Error::InvalidMetadataValue { key, .. } if key == "fix:branches"),
            "{refused:?}: {error}"
        );
        assert_eq!(field, before, "{refused:?}");
    }
    assert!(field.as_fix_mut().add_branch("").is_err());
    assert_eq!(field, before);

    // Empty input removes the property rather than storing "".
    field
        .as_fix_mut()
        .set_branches::<[&str; 0], &str>([])
        .unwrap();
    assert!(!field.has_metadata("fix:branches"));
    assert_eq!(field.as_fix().branches().count(), 0);
}

#[test]
fn merge_with_folds_the_fields_and_the_dialects_beside_them() {
    let mut dictionary =
        FixRegistry::from_fields([tagged("symbol", 55), member("VenueSym", "cme", 5_055)]).unwrap();

    let other = FixRegistry::from_fields([
        member("SYMBOL", "globex", 55),
        member("VenueTime", "globex", 5_060),
    ])
    .unwrap();

    // The other dictionary holds the crate's own fields as every registry
    // does, and they are neither added nor merged: a fold never counts them.
    let (added, merged) = dictionary.merge_with(&other).unwrap();
    assert_eq!((added, merged), (1, 1));
    assert_eq!(dictionary.len(), 3 + crated());
    // A folded input name retains the canonical identity's stored spelling.
    assert_eq!(dictionary.field_by_tag(55).unwrap().name(), "symbol");

    // The dialects arrive beside the fields: membership unions onto the
    // stored field, and a field only one side held keeps saying whose it is.
    assert_eq!(
        dictionary
            .field_by_tag(55)
            .unwrap()
            .as_fix()
            .branches()
            .collect::<Vec<_>>(),
        ["globex"]
    );
    assert!(
        dictionary
            .field_by_tag(5_055)
            .unwrap()
            .as_fix()
            .has_branch("cme")
    );
    assert!(
        dictionary
            .field_by_tag(5_060)
            .unwrap()
            .as_fix()
            .has_branch("globex")
    );
    assert_eq!(dictionary.dialects(), ["cme", "globex"]);

    // One mutation: a refusal leaves the dictionary as it was, memberships
    // too.
    let before = dictionary.clone();
    let mut refusing = FixRegistry::from_fields([tagged("symbol", 55)]).unwrap();
    refusing
        .insert({
            let mut widened = tagged("SYMBOL", 55);
            widened.set_dtype(DataType::large_utf8()).unwrap();
            widened
        })
        .unwrap();
    refusing.insert(member("BlpSym", "blp", 5_070)).unwrap();
    assert!(dictionary.merge_with(&refusing).is_err());
    assert_eq!(dictionary, before);
    assert_eq!(dictionary.dialects(), ["cme", "globex"]);
}

#[test]
fn an_identifier_is_one_integer_over_the_tag_and_the_folded_name() {
    // One `i32` and nothing beside it: the digest of both halves.
    assert_eq!(size_of::<FixId>(), size_of::<i32>());
    let msgtype = id_of(35, "MsgType");
    assert_eq!(msgtype.to_string(), msgtype.digest().to_string());
    assert_eq!(FixId::from_digest(msgtype.digest()), msgtype);
    assert_eq!(id_of(35, "msg_type"), msgtype);
    assert_eq!(id_of(35, "MSG TYPE"), msgtype);
    assert_ne!(id_of(35, "MsgType "), id_of(35, "MsgType!"));
    // A tag alone is not an identity: the name is the other half.
    assert_ne!(id_of(35, ""), msgtype);
    assert_ne!(id_of(35, "MsgSeqNum"), msgtype);
    assert_ne!(id_of(34, "MsgType"), msgtype);
    // An entry keeps the tag; the field the tag names is the registry's to
    // answer, so an identity is assembled from the two owners.
    assert_eq!(
        id_of(entry(5001, "5001", "x").tag(), "VenueSym"),
        id_of(5001, "VenueSym")
    );

    // Both halves reach the hasher: sixty-four tags under one name and one
    // tag under sixty-four names each make sixty-four identities, and the
    // finalizer spreads them over the control-byte classes.
    let tags = 5_000..5_064;
    let counted = tags.len();
    let by_tag: HashSet<FixId> = tags.clone().map(|tag| id_of(tag, "VenueSym")).collect();
    assert_eq!(by_tag.len(), counted);
    let by_name: HashSet<FixId> = (0..counted)
        .map(|index| id_of(5_001, &format!("Field{index}")))
        .collect();
    assert_eq!(by_name.len(), counted);
    let classes: HashSet<u8> = tags
        .map(|tag| control_byte(id_of(tag, "VenueSym")))
        .collect();
    assert!(
        classes.len() > 16,
        "{counted} tags reached {} classes",
        classes.len()
    );

    // The tag half may not be signed, whatever the name.
    for name in ["MsgType", "", "cme"] {
        let error = FixId::of(-1, name).unwrap_err();
        assert!(
            matches!(&error, Error::InvalidMetadataValue { key, reason }
                if key == "fix:tag" && reason.contains("-1")),
            "{name:?}: {error}"
        );
    }
    assert!(FixId::of(0, "").is_ok());
    assert!(FixId::of(i32::MAX, "MsgType").is_ok());
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
fn membership_round_trips_and_is_no_half_of_the_identity() {
    let mut field = DataType::utf8().nullable_field("TradeID");

    // Absent means the specification alone, and no identity without a tag.
    assert_eq!(field.as_fix().branches().count(), 0);
    assert_eq!(field.as_fix().id().unwrap(), None);
    assert!(!field.has_metadata("fix:branches"));

    field.as_fix_mut().set_branches(["CME"]).unwrap();
    assert_eq!(field.as_fix().branches().collect::<Vec<_>>(), ["cme"]);
    assert_eq!(field.get_metadata("fix:branches"), Some("cme"));
    assert_eq!(field.as_fix().id().unwrap(), None, "still no tag");

    field.as_fix_mut().set_tag(5001).unwrap();
    let id = field.as_fix().id().unwrap().unwrap();
    assert_eq!(id, id_of(5001, "TradeID"));
    assert_eq!(id.to_string(), id.digest().to_string());

    // Membership is provenance and never identity: the same tag under the
    // same name is one field whatever dictionaries spoke it, and taking the
    // membership away moves nothing.
    assert_eq!(tagged("TradeID", 5001).as_fix().id().unwrap(), Some(id));
    assert_eq!(
        member("TradeID", "xnas", 5001).as_fix().id().unwrap(),
        Some(id)
    );
    field
        .as_fix_mut()
        .set_branches::<[&str; 0], &str>([])
        .unwrap();
    assert!(!field.has_metadata("fix:branches"));
    assert_eq!(field.as_fix().id().unwrap(), Some(id));

    // The name is the other half: a rename is a new identity, derived on
    // every read rather than stored stale.
    field.set_name("Trade_ID");
    assert_eq!(field.as_fix().id().unwrap(), Some(id), "the fold");
    field.set_name("TradeReportID");
    assert_ne!(field.as_fix().id().unwrap(), Some(id));
    assert_eq!(
        field.as_fix().id().unwrap(),
        Some(id_of(5001, "TradeReportID"))
    );
}

#[test]
fn registering_a_message_type_names_it_describes_it_and_never_rewrites_it() {
    let mut registry =
        FixRegistry::from_fields([tagged("msgtype", super::MSGTYPE_TAG_NAME.0)]).unwrap();
    let value = registry.register_msgtype("D", None, None).unwrap();
    assert_eq!(value.as_str(), "D");
    assert!(matches!(value.as_field().dtype(), DataType::Struct(_)));

    let held = registry
        .register_msgtype(
            "P Report Ack",
            Some("AllocationReportAck"),
            Some("Allocation Report ACK"),
        )
        .unwrap();
    assert_eq!(held.as_str(), "P Report Ack");
    assert!(std::ptr::eq(
        registry.msgtype("P Report Ack").unwrap(),
        registry.msgtype("AllocationReportAck").unwrap(),
    ));
    let code = |registry: &FixRegistry| {
        registry
            .field_by_tag(super::MSGTYPE_TAG_NAME.0)
            .unwrap()
            .as_fix()
            .codes()
            .map(Result::unwrap)
            .find(|code| code.name() == "AllocationReportAck")
            .map(super::FixCode::from)
            .unwrap()
    };
    let initial = code(&registry);
    assert_eq!(initial.value(), "P Report Ack");
    assert_eq!(initial.description(), Some("Allocation Report ACK"));
    registry
        .register_msgtype("P Report Ack", Some("allocationreportack"), Some("Other"))
        .unwrap();
    assert_eq!(code(&registry), initial);
    assert_eq!(registry.msgtypes().count(), 2);

    registry
        .register_msgtype("D", None, Some("Order - Single"))
        .unwrap();
    let field = registry.field_by_tag(super::MSGTYPE_TAG_NAME.0).unwrap();
    assert_eq!(
        field
            .as_fix()
            .codes()
            .map(Result::unwrap)
            .find(|code| code.value() == "D")
            .unwrap()
            .parse_doc()
            .unwrap(),
        Some("Order - Single".to_owned())
    );

    let before = registry.clone();
    let error = registry
        .register_msgtype("F", Some("AllocationReportAck"), None)
        .unwrap_err();
    assert!(error.is_conflict(), "{error}");
    assert_eq!(registry, before);
    assert!(registry.register_msgtype("", None, None).is_err());
    assert_eq!(registry, before);
}

#[test]
fn nothing_gates_a_tag_on_the_dictionary_that_speaks_it() {
    // Membership means "this dictionary speaks it", the specification's own
    // tags included: a venue file naming tag 35 stamps itself on tag 35.
    let spoken = member("MsgType", "cme", 35);
    assert!(spoken.as_fix().has_branch("cme"));
    assert_eq!(spoken.as_fix().id().unwrap(), Some(id_of(35, "MsgType")));

    // Every door that takes a tag takes any non-negative one, whatever the
    // field's membership: the identity is the tag and the name, and the
    // dictionary is not consulted.
    let mut vendor = member("TradeID", "cme", 5001);
    vendor.as_fix_mut().set_tag(35).unwrap();
    vendor.as_fix_mut().set_tags(&[5002, 40_000, 0]).unwrap();
    assert_eq!(vendor.as_fix().tag().unwrap(), Some(35));
    assert_eq!(vendor.as_fix().tags().unwrap(), [5002, 40_000, 0]);
    assert!(vendor.as_fix().has_branch("cme"));
    for tag in [0, 35, 4_999, 5_000, 39_999, 40_000, i32::MAX] {
        assert!(FixId::of(tag, "TradeID").is_ok(), "{tag}");
    }
    let mut counted = member("NoPartyIDs", "cme", 453);
    counted.as_fix_mut().set_counter(35).unwrap();
    assert_eq!(counted.as_fix().counter().unwrap(), Some(35));

    // A registry holds a member on a specification tag, membership and all.
    let registry = FixRegistry::from_fields([spoken.clone()]).unwrap();
    assert_eq!(registry.get_field_by_tag(35), Some(&spoken));
    assert!(
        registry
            .field_by_tag(35)
            .unwrap()
            .as_fix()
            .has_branch("cme")
    );
}

#[test]
fn two_fields_may_hold_one_tag_under_two_names() {
    let mut spec = tagged("Symbol", 5055);
    spec.as_fix_mut().set_aliases(["Ticker"]).unwrap();
    spec.as_fix_mut().set_tags(&[9055]).unwrap();
    let venue = member("VenueSymbol", "cme", 5055);

    let registry = FixRegistry::from_fields([spec.clone(), venue.clone()]).unwrap();
    assert_eq!(registry.len(), 2 + crated());

    // Each identity answers its own field. The holder of the tag gained the
    // arrival's name as an alias, so it is its stored self plus that.
    let spec_id = id_of(5055, "Symbol");
    let venue_id = id_of(5055, "VenueSymbol");
    let held = registry.field_by_id(spec_id).unwrap();
    assert_eq!(held.name(), "Symbol");
    assert_eq!(
        held.as_fix().aliases().collect::<Vec<_>>(),
        ["Ticker", "VenueSymbol"]
    );
    assert_eq!(held.as_fix().tags().unwrap(), [9055]);
    assert_eq!(held.as_fix().branches().count(), 0);
    assert_eq!(registry.field_by_id(venue_id).unwrap(), &venue);
    assert!(
        registry.get_field_by_id(id_of(9055, "Symbol")).is_none(),
        "an alternate tag is an index entry, never an identity"
    );

    // A bare wire tag answers the first holder; the newcomer is reached by
    // its name, which is canonical before it is the holder's alias.
    assert_eq!(registry.get_field_by_tag(5055), Some(held));
    assert_eq!(registry.get_field_by_tag(9055), Some(held));
    assert_eq!(registry.get_field("Symbol"), Some(held));
    assert_eq!(registry.get_field("Ticker"), Some(held));
    assert_eq!(registry.get_field_by_name("VENUESYMBOL"), Some(&venue));
    assert_eq!(registry.get_field("venue_symbol"), Some(&venue));
    // A colon-bearing string is a name, never an identifier.
    assert!(registry.get_field("5055:cme").is_none());
    assert!(registry.get_field_by_name("absent").is_none());
    assert!(registry.get_field_by_id(id_of(6000, "Absent")).is_none());

    // The order of arrival is what decides the first holder, and the alias
    // is lent the other way.
    let reversed = FixRegistry::from_fields([venue.clone(), spec.clone()]).unwrap();
    assert_eq!(
        reversed.get_field_by_tag(5055).map(Field::name),
        Some("VenueSymbol")
    );
    assert_eq!(
        reversed
            .field_by_tag(5055)
            .unwrap()
            .as_fix()
            .aliases()
            .collect::<Vec<_>>(),
        ["Symbol"]
    );
    assert_eq!(reversed.field_by_id(spec_id).unwrap(), &spec);
    assert_eq!(registry.len(), reversed.len());

    // A conflict is still a conflict, and it names both fields.
    let mut twice = member("VenueSym", "cme", 5099);
    twice.as_fix_mut().set_aliases(["TICKER"]).unwrap();
    let mut probed = registry.clone();
    let error = probed.insert(twice).unwrap_err();
    assert!(
        matches!(&error, Error::Conflict { path, .. }
            if path == "alias \"TICKER\" of VenueSym, held by Symbol"),
        "{error}"
    );
    assert_eq!(probed, registry);
    let error = probed.insert(tagged("VENUE_SYMBOL", 6000)).unwrap_err();
    assert!(
        matches!(&error, Error::Conflict { path, .. }
            if path == "name \"VENUE_SYMBOL\" of VENUE_SYMBOL, held by VenueSymbol"),
        "{error}"
    );
    assert_eq!(probed, registry);

    // The failing halves name the key the way it was asked.
    let absent = id_of(6000, "Absent");
    let by_id = registry.field_by_id(absent).unwrap_err();
    assert!(
        matches!(&by_id, Error::Absent { expected: "fix field", path }
            if *path == format!("identifier {absent}")),
        "{by_id}"
    );
    // The specialized and generic pairs answer alike for an identifier.
    assert_eq!(
        registry.get_field(venue_id),
        registry.get_field_by_id(venue_id)
    );
    assert_eq!(registry.get_field(FixKey::Id(venue_id)), Some(&venue));
    assert_eq!(
        registry.field(venue_id).map(Field::name).ok(),
        registry.field_by_id(venue_id).map(Field::name).ok()
    );
    assert!(registry.contains(venue_id));
    assert!(!registry.contains(absent));
}

/// One run of the fold table over `fold`, which is either verb that adds a
/// field to a dictionary it may already describe.
fn fold_table(verb: &str, fold: impl Fn(&mut FixRegistry, Field) -> (usize, usize)) {
    let mut registry = FixRegistry::from_fields([
        full("Symbol", 55, &[65], &["Ticker"]),
        full("Price", 44, &[31], &["Px"]),
    ])
    .unwrap();

    // Row 1: the same tag under the same folded name is the same field, and
    // the merge unions its tags, aliases and membership.
    let mut same = member("SYMBOL", "cme", 55);
    same.as_fix_mut().set_tags(&[66]).unwrap();
    same.as_fix_mut().set_aliases(["Sym"]).unwrap();
    assert_eq!(fold(&mut registry, same), (0, 1), "{verb}: row 1");
    assert_eq!(registry.len(), 2 + crated(), "{verb}");
    let symbol = registry.field_by_tag(55).unwrap();
    assert_eq!(symbol.name(), "Symbol", "{verb}");
    assert_eq!(symbol.as_fix().tags().unwrap(), [66, 65], "{verb}");
    assert_eq!(
        symbol.as_fix().aliases().collect::<Vec<_>>(),
        ["Sym", "Ticker"],
        "{verb}"
    );
    assert_eq!(
        symbol.as_fix().branches().collect::<Vec<_>>(),
        ["cme"],
        "{verb}"
    );

    // Row 2: the same tag under another name is a new field, registered
    // beside the holder, which gains the arrival's name as an alias. The
    // bare tag keeps answering the first holder; the newcomer is reached by
    // its name or its identity, and its membership is its own.
    assert_eq!(
        fold(&mut registry, member("VenueSymbol", "xnas", 55)),
        (1, 0),
        "{verb}: row 2"
    );
    assert_eq!(registry.len(), 3 + crated(), "{verb}");
    let symbol = registry.field_by_tag(55).unwrap();
    assert_eq!(symbol.name(), "Symbol", "{verb}: the bare tag");
    assert_eq!(
        symbol.as_fix().aliases().collect::<Vec<_>>(),
        ["Sym", "Ticker", "VenueSymbol"],
        "{verb}"
    );
    assert!(!symbol.as_fix().has_branch("xnas"), "{verb}");
    let newcomer = registry.field_by_id(id_of(55, "VenueSymbol")).unwrap();
    assert_eq!(newcomer.name(), "VenueSymbol", "{verb}");
    assert_eq!(
        newcomer.as_fix().branches().collect::<Vec<_>>(),
        ["xnas"],
        "{verb}"
    );
    assert_eq!(
        registry.field_by_name("venue_symbol").unwrap().name(),
        "VenueSymbol",
        "{verb}: canonical before alias"
    );
    assert_eq!(
        registry.field_by_tag(66).unwrap().name(),
        "Symbol",
        "{verb}"
    );

    // Row 3: the same folded name under another tag is the same field
    // spelled with another number: it merges into the holder, which gains
    // the tag as an alternate and the incoming spellings as aliases. No
    // second field.
    let mut spelled = member("symbol", "blp", 9055);
    spelled.as_fix_mut().set_aliases(["BlpSym"]).unwrap();
    assert_eq!(fold(&mut registry, spelled), (0, 1), "{verb}: row 3");
    assert_eq!(registry.len(), 3 + crated(), "{verb}");
    let symbol = registry.field_by_tag(9055).unwrap();
    assert_eq!(
        symbol.name(),
        "Symbol",
        "{verb}: the alternate reaches the holder"
    );
    assert_eq!(symbol.as_fix().tags().unwrap(), [66, 65, 9055], "{verb}");
    assert_eq!(
        symbol.as_fix().aliases().collect::<Vec<_>>(),
        ["Sym", "Ticker", "VenueSymbol", "BlpSym"],
        "{verb}"
    );
    assert_eq!(
        symbol.as_fix().branches().collect::<Vec<_>>(),
        ["blp", "cme"],
        "{verb}"
    );
    assert!(
        registry.get_field_by_id(id_of(9055, "symbol")).is_none(),
        "{verb}: no second field"
    );
    assert_eq!(
        registry.field_by_name("blpsym").unwrap().name(),
        "Symbol",
        "{verb}"
    );
    // The same row reached through an alias, with a tag another field
    // already answers: the tag stays with that field and is left out.
    assert_eq!(
        fold(&mut registry, member("TICKER", "blp", 31)),
        (0, 1),
        "{verb}: row 3 by alias"
    );
    assert_eq!(registry.field_by_tag(31).unwrap().name(), "Price", "{verb}");
    assert_eq!(
        registry.field_by_tag(55).unwrap().as_fix().tags().unwrap(),
        [66, 65, 9055],
        "{verb}: a tag another field answers is nobody's alternate"
    );

    // Row 4: neither, so it is inserted as it arrived.
    assert_eq!(
        fold(&mut registry, member("TransactTime", "cme", 60)),
        (1, 0),
        "{verb}: row 4"
    );
    assert_eq!(registry.len(), 4 + crated(), "{verb}");
    assert_eq!(
        registry.field_by_tag(60).unwrap(),
        &member("TransactTime", "cme", 60)
    );
    assert_eq!(registry.dialects(), ["blp", "cme", "xnas"], "{verb}");
}

#[test]
fn the_fold_table_holds_through_add_field_and_through_merge_with() {
    fold_table("add_field", |registry, field| {
        if registry.add_field(field).unwrap() {
            (1, 0)
        } else {
            (0, 1)
        }
    });
    fold_table("merge_with", |registry, field| {
        let other = FixRegistry::from_fields([field]).unwrap();
        registry.merge_with(&other).unwrap()
    });
}

#[test]
fn one_message_code_namespace_folds_a_restated_name_and_keeps_a_second_one() {
    let mut registry = FixRegistry::from_fields([tagged("MsgType", 35)]).unwrap();
    let message = |name: &str, code: &str| {
        let mut field = DataType::from_fields([]).unwrap().required_field(name);
        field.as_fix_mut().set_msgtype(code).unwrap();
        field
    };
    registry
        .insert_definition(FixCategory::Components, message("NewOrderSingle", "D"))
        .unwrap();
    assert_eq!(registry.msgtype("D").unwrap().name(), "NewOrderSingle");

    // A definition re-declaring the code under the same folded name folds
    // into the stored one.
    assert!(
        !registry
            .add_definition(FixCategory::Components, message("new_order_single", "D"))
            .unwrap()
    );
    assert_eq!(registry.msgtypes().count(), 1);
    assert_eq!(registry.msgtype("D").unwrap().name(), "NewOrderSingle");

    // Under another name it is a second message, whose bare code answers
    // the first holder; the second is reached by its name.
    assert!(
        registry
            .add_definition(FixCategory::Components, message("VenueOrder", "D"))
            .unwrap()
    );
    assert_eq!(registry.msgtypes().count(), 2);
    assert_eq!(registry.msgtype("D").unwrap().name(), "NewOrderSingle");
    assert_eq!(registry.msgtype("VenueOrder").unwrap().name(), "VenueOrder");
    assert_eq!(registry.msgtype("venue_order").unwrap().as_str(), "D");
    assert_eq!(registry.msgtype("neworder_single").unwrap().as_str(), "D");
    assert_eq!(
        registry
            .definitions(FixCategory::Components)
            .map(Field::name)
            .collect::<Vec<_>>(),
        ["NewOrderSingle", "VenueOrder"]
    );
}

#[test]
fn a_corrupt_stored_property_is_reported_under_its_full_key() {
    let cases = [
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
        .insert(DataType::utf8().nullable_field("Symbol"))
        .unwrap_err();
    assert!(error.is_absent(), "{error}");
    let message = error.to_string();
    assert!(message.contains("fix:tag"), "{message}");
    assert!(message.contains("Symbol"), "{message}");

    let error = FixRegistry::new()
        .update(DataType::utf8().nullable_field("Symbol"))
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
            registry.field_by_name(query).unwrap().name(),
            "Symbol",
            "{query}"
        );
        assert_eq!(registry.field(query).unwrap().name(), "Symbol", "{query}");
        assert!(registry.contains(query), "{query}");
    }
    assert_eq!(
        registry.field_by_name("clientorderid").unwrap().name(),
        "ClOrdID"
    );
    assert!(registry.get_field_by_name("Symbols").is_none());
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
        assert_eq!(registry.field_by_name("px").unwrap().name(), "Px");
        assert_eq!(registry.field_by_name("Price").unwrap().name(), "Px");
        assert_eq!(
            registry.field_by_name("LastPrice").unwrap().name(),
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
    assert_eq!(registry.field_by_name("35").unwrap().name(), "35");
    assert!(registry.get_field_by_name("1").is_none());
    assert!(registry.get_field_by_tag(2).is_none());
}

#[test]
fn an_insert_conflict_names_both_fields_for_each_key_kind() {
    let stored = full("Symbol", 55, &[65], &["Ticker"]);
    let registry = FixRegistry::from_fields([stored]).unwrap();
    let cases = [
        (full("symbol", 56, &[], &[]), "name \"symbol\""),
        (full("SymbolSfx", 56, &[65], &[]), "alternate tag 65"),
        (full("SymbolSfx", 56, &[], &["TICKER"]), "alias \"TICKER\""),
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
        assert_eq!(probed.len(), 1 + crated());
    }

    // The one thing this namespace admits twice: a held tag under another
    // name is a field of its own, inserted beside the holder, which gains
    // the arrival's name as an alias while the bare tag keeps answering it.
    let mut beside = registry.clone();
    assert_eq!(
        beside.insert(full("SymbolSfx", 55, &[], &[])).unwrap(),
        None
    );
    assert_eq!(beside.len(), 2 + crated());
    assert_eq!(beside.field_by_tag(55).unwrap().name(), "Symbol");
    assert_eq!(
        beside
            .field_by_tag(55)
            .unwrap()
            .as_fix()
            .aliases()
            .collect::<Vec<_>>(),
        ["Ticker", "SymbolSfx"]
    );
    assert_eq!(
        beside.field_by_id(id_of(55, "SymbolSfx")).unwrap().name(),
        "SymbolSfx"
    );
    assert_eq!(
        beside.field_by_name("symbolsfx").unwrap().name(),
        "SymbolSfx"
    );

    // Overlap between a canonical key and an alternate one is not a conflict.
    let mut registry = registry;
    assert_eq!(
        registry
            .insert(full("Ticker", 56, &[55], &["Symbol"]))
            .unwrap(),
        None
    );
    assert_eq!(registry.field_by_name("Ticker").unwrap().name(), "Ticker");
    assert_eq!(registry.field_by_name("Symbol").unwrap().name(), "Symbol");
    assert_eq!(registry.field_by_tag(55).unwrap().name(), "Symbol");
}

#[test]
fn reinserting_the_same_identity_replaces_properties_and_retains_its_spelling() {
    let mut registry = FixRegistry::from_fields([
        full("Symbol", 55, &[65], &["Ticker"]),
        full("Price", 44, &[], &["Px"]),
    ])
    .unwrap();

    // Same tag, same folded name: the canonical spelling stays while the
    // other properties are replaced and the prior definition comes back whole.
    let replacement = full("SYMBOL", 55, &[66], &["Sym"]);
    let prior = registry.insert(replacement.clone()).unwrap().unwrap();
    assert_eq!(prior.name(), "Symbol");
    assert_eq!(prior.as_fix().tags().unwrap(), [65]);
    assert_eq!(
        registry.field_by_tag(55).unwrap(),
        &replacement.with_name("Symbol")
    );
    assert_eq!(registry.field_by_name("symbol").unwrap().name(), "Symbol");
    assert_eq!(registry.field_by_tag(66).unwrap().name(), "Symbol");
    assert_eq!(registry.field_by_name("Sym").unwrap().name(), "Symbol");
    assert!(registry.get_field_by_tag(65).is_none());
    assert!(registry.get_field_by_name("Ticker").is_none());
    assert_eq!(registry.len(), 2 + crated());

    // A tag matching one field and a name matching another is never a
    // replacement, and a new key another field holds refuses the whole thing.
    let before = registry.clone();
    let error = registry.insert(full("Price", 55, &[], &[])).unwrap_err();
    assert!(error.is_conflict(), "{error}");
    let error = registry
        .insert(full("Symbol", 55, &[], &["px"]))
        .unwrap_err();
    assert!(
        matches!(&error, Error::Conflict { path, .. } if path == "alias \"px\" of Symbol, held by Price"),
        "{error}"
    );
    assert_eq!(registry, before);
    assert_eq!(
        probe(&registry, 55, 66, "SYMBOL", "Sym"),
        [Some("Symbol"); 4]
    );
}

#[test]
fn a_merge_follows_the_truth_table() {
    let mut stored = full("Symbol", 55, &[65, 66], &["Ticker", "Sym"]);
    stored.insert_metadata("display", "Symbol").unwrap();
    stored.insert_metadata("owner", "stored").unwrap();
    let mut registry = FixRegistry::from_fields([stored]).unwrap();

    let mut incoming = DataType::utf8().required_field("SYMBOL");
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
    // The canonical spelling stays; incoming nullability and shared keys win.
    assert_eq!(merged.name(), "Symbol");
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
            "Symbol",
            "{tag}"
        );
    }
    for name in ["symbol", "ticker", "SYM", "instrument"] {
        assert_eq!(
            registry.field_by_name(name).unwrap().name(),
            "Symbol",
            "{name}"
        );
    }
    assert_eq!(registry.len(), 1 + crated());

    // Another spelling retains the accumulated tags and canonical name.
    let before = registry.clone();
    registry.update(tagged("symbol", 55)).unwrap();
    assert_eq!(
        registry.field_by_tag(55).unwrap().as_fix().tags().unwrap(),
        [67, 66, 65]
    );
    assert_eq!(registry.field_by_tag(55).unwrap().name(), "Symbol");
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
    widened.set_dtype(DataType::large_utf8()).unwrap();
    let error = registry.update(widened).unwrap_err();
    assert!(matches!(error, Error::InvalidRecord { .. }), "{error}");
    let message = error.to_string();
    assert!(
        message.contains("utf8") && message.contains("large_utf8"),
        "{message}"
    );

    // A name disagreement is another identity, because the name is half of
    // it: the incoming field names no stored one, and the absence says which.
    let error = registry.update(tagged("Sym", 55)).unwrap_err();
    assert!(
        matches!(&error, Error::Absent { path, .. }
            if *path == format!("identifier {}", id_of(55, "Sym"))),
        "{error}"
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
        matches!(&error, Error::Absent { path, .. }
            if *path == format!("identifier {}", id_of(58, "Text"))),
        "{error}"
    );

    // A tag disagreement is that same absence: the tag is the other half of
    // the identity, so a venue's `Symbol` on 5055 names no stored one, and
    // its membership changes nothing about that.
    let error = registry.update(member("Symbol", "cme", 5055)).unwrap_err();
    assert!(
        matches!(&error, Error::Absent { path, .. }
            if *path == format!("identifier {}", id_of(5055, "Symbol"))),
        "{error}"
    );

    assert_eq!(registry, before);
    assert_eq!(format!("{registry:?}"), snapshot);
    assert_eq!(
        probe(&registry, 55, 65, "symbol", "ticker"),
        [Some("Symbol"); 4]
    );
    assert_eq!(probe(&registry, 44, 31, "price", "px"), [Some("Price"); 4]);
    assert_eq!(registry.len(), 2 + crated());
}

#[test]
fn add_fields_adds_what_is_absent_and_merges_what_is_present() {
    let mut registry =
        FixRegistry::from_fields([full("Symbol", 55, &[65], &["Ticker"]), tagged("Price", 44)])
            .unwrap();

    // Tag 55 is stored and folds; tag 44 is stored under another spelling of
    // the same name and folds too; tag 60 is new. The venue's own `Symbol`
    // on 5055 is the same folded name under another tag: the same field
    // spelled with another number, so it folds into the holder too.
    let mut priced = tagged("PRICE", 44);
    priced.as_fix_mut().set_aliases(["Px"]).unwrap();
    let (added, merged) = registry
        .add_fields([
            full("Symbol", 55, &[66], &["Sym"]),
            priced,
            tagged("TransactTime", 60),
            member("Symbol", "cme", 5_055),
        ])
        .unwrap();
    assert_eq!((added, merged), (1, 3));
    assert_eq!(registry.len(), 3 + crated());

    // The merge kept what only the stored field declared and added the rest:
    // the venue's tag as an alternate, and its membership.
    let symbol = registry.field_by_tag(55).unwrap();
    assert_eq!(symbol.as_fix().tags().unwrap(), [66, 65, 5_055]);
    assert_eq!(
        symbol.as_fix().aliases().collect::<Vec<_>>(),
        ["Sym", "Ticker"]
    );
    assert_eq!(symbol.as_fix().branches().collect::<Vec<_>>(), ["cme"]);
    assert_eq!(registry.field_by_tag(5_055).unwrap().name(), "Symbol");
    assert!(registry.get_field_by_id(id_of(5_055, "Symbol")).is_none());
    // Incoming metadata folds into the stored canonical spelling.
    assert_eq!(registry.field_by_tag(44).unwrap().name(), "Price");
    assert_eq!(registry.field_by_tag(60).unwrap().name(), "TransactTime");

    // The identity is the whole probe: the same tag under another name is
    // another field, added beside the specification's rather than folded
    // into it, and the holder learns the arrival's name.
    let (added, merged) = registry
        .add_fields([member("VenueTime", "cme", 60)])
        .unwrap();
    assert_eq!((added, merged), (1, 0));
    assert_eq!(registry.len(), 4 + crated());
    assert_eq!(registry.field_by_tag(60).unwrap().name(), "TransactTime");
    assert_eq!(
        registry
            .field_by_tag(60)
            .unwrap()
            .as_fix()
            .aliases()
            .collect::<Vec<_>>(),
        ["VenueTime"]
    );
    assert_eq!(
        registry.field_by_id(id_of(60, "VenueTime")).unwrap(),
        &member("VenueTime", "cme", 60)
    );
}

#[test]
fn add_fields_refuses_the_way_the_one_field_writes_refuse() {
    let mut registry = FixRegistry::from_fields([tagged("Symbol", 55)]).unwrap();

    // No `fix:tag` is no identity, so there is nothing to add or fold under.
    let error = registry
        .add_fields([DataType::utf8().nullable_field("Nameless")])
        .unwrap_err();
    assert!(error.is_absent(), "{error}");
    assert!(error.to_string().contains("fix:tag"), "{error}");

    // A datatype that disagrees with the stored definition is refused, never
    // widened - the shape a CBlock's generic `float` takes against a stored
    // `float64`.
    let mut widened = tagged("Symbol", 55);
    widened.set_dtype(DataType::large_utf8()).unwrap();
    let error = registry.add_fields([widened]).unwrap_err();
    assert!(matches!(error, Error::InvalidRecord { .. }), "{error}");

    // The whole fold is one mutation, so a refusal in the middle of it leaves
    // the dictionary exactly as it was - neither the field before nor the one
    // after arrives.
    let before = registry.clone();
    let mut clash = tagged("Symbol", 55);
    clash.set_dtype(DataType::large_utf8()).unwrap();
    let error = registry
        .add_fields([tagged("Price", 44), clash, tagged("TransactTime", 60)])
        .unwrap_err();
    assert!(matches!(error, Error::InvalidRecord { .. }), "{error}");
    assert_eq!(registry, before);
    assert_eq!(registry.len(), 1 + crated());
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
    assert_eq!(registry.len(), 2 + crated());
    assert_eq!(probe(&registry, 55, 65, "symbol", "ticker"), [None; 4]);
    assert_eq!(probe(&registry, 44, 45, "price", "px"), [Some("Price"); 4]);
    assert_eq!(
        probe(&registry, 58, 59, "text", "freetext"),
        [Some("Text"); 4]
    );
    assert_eq!(
        registry.iter().map(Field::name).collect::<Vec<_>>(),
        then_crated(&["Price", "Text"])
    );

    // A name key removes through the alias tier too; a path never does.
    assert!(registry.remove("Symbol").is_none());
    assert_eq!(registry.remove("PX").unwrap().name(), "Price");
    assert_eq!(probe(&registry, 44, 45, "price", "px"), [None; 4]);
    assert_eq!(
        probe(&registry, 58, 59, "text", "freetext"),
        [Some("Text"); 4]
    );
    assert_eq!(registry.len(), 1 + crated());
    assert_eq!(registry.remove("FreeText").unwrap().name(), "Text");
    assert_eq!(
        registry.len(),
        crated(),
        "nothing of the test's own is left"
    );
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
            registry.get_field_by_name(name),
            "{name}"
        );
        assert_eq!(
            registry.get_field(&name.to_owned()),
            registry.get_field_by_name(name)
        );
        assert_eq!(
            registry.field(name).map(Field::name).ok(),
            registry.field_by_name(name).map(Field::name).ok()
        );
        assert_eq!(
            registry.contains(name),
            registry.get_field_by_name(name).is_some()
        );
    }

    // The failing halves name the key the way it was asked.
    let by_tag = registry.field_by_tag(1).unwrap_err();
    assert!(matches!(&by_tag, Error::Absent { expected: "fix field", path } if path == "tag 1"));
    let by_name = registry.field_by_name("absent").unwrap_err();
    assert!(matches!(&by_name, Error::Absent { path, .. } if path == "name \"absent\""));
    let by_path = registry.field_by_path(&fpath("Symbol.absent")).unwrap_err();
    // A path renders its own canonical spelling, so the absence names the
    // reading rather than quoting a string nobody resolved.
    assert!(matches!(&by_path, Error::Absent { path, .. } if path == "path Symbol.absent"));
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
    let mut party_id = DataType::utf8().nullable_field("PartyID");
    party_id.as_fix_mut().set_tag(448).unwrap();
    let mut role = DataType::Int32.nullable_field("PartyRole");
    role.as_fix_mut().set_tag(452).unwrap();
    let mut group = DataType::list(
        DataType::from_fields([party_id.clone(), role])
            .unwrap()
            .required_field("Party"),
    )
    .nullable_field("Parties");
    group.as_fix_mut().set_counter(453).unwrap();
    let instrument = DataType::from_fields([tagged("Symbol", 55), tagged("SecurityID", 48)])
        .unwrap()
        .nullable_field("Instrument");

    let mut registry = FixRegistry::from_fields([counter("NoPartyIDs", 453)]).unwrap();
    registry
        .insert_definition(FixCategory::Groups, group)
        .unwrap();
    registry
        .insert_definition(FixCategory::Components, instrument)
        .unwrap();
    assert_eq!(
        registry
            .definition(FixCategory::Groups, "Parties")
            .unwrap()
            .as_fix()
            .counter()
            .unwrap(),
        Some(453)
    );
    assert_eq!(
        registry.field_by_path(&fpath("Parties.PartyID")).unwrap(),
        &party_id
    );
    assert_eq!(
        registry
            .field_by_path(&fpath("parties.PartyRole"))
            .unwrap()
            .name(),
        "PartyRole"
    );
    assert_eq!(
        registry
            .field_by_path(&fpath("Instrument.Symbol"))
            .unwrap()
            .name(),
        "Symbol"
    );
    assert_eq!(
        registry
            .field_by_path(&fpath("INSTRUMENT.SecurityID"))
            .unwrap()
            .name(),
        "SecurityID"
    );
    assert_eq!(
        registry.get_field("Instrument.Symbol"),
        registry.get_field_by_path(&fpath("Instrument.Symbol"))
    );
    assert!(registry.contains("Parties.PartyID"));
    // A member is reached through its parent only: the registry does not
    // index it.
    assert!(registry.get_field_by_name("PartyID").is_none());
    // The remainder of a path folds like the head does. One function that
    // folded its first segment and matched the rest exactly would refuse
    // `Parties.PartyID` on a dictionary that stores its members folded,
    // which is every dictionary this crate writes.
    assert_eq!(
        registry.get_field_by_path(&fpath("Parties.partyid")),
        registry.get_field_by_path(&fpath("Parties.PartyID")),
    );
    assert_eq!(
        registry
            .get_field_by_path(&fpath("Parties.PARTY_ID"))
            .map(Field::name),
        Some("PartyID"),
    );
    assert!(
        registry
            .get_field_by_path(&fpath("Instrument.Absent"))
            .is_none()
    );
    assert!(
        registry
            .get_field_by_path(&fpath("Absent.Symbol"))
            .is_none()
    );
}

#[test]
fn one_spelling_reaches_a_member_through_the_message_and_through_the_registry() {
    // The asymmetry this replaces: the registry wanted `Parties.PartyID` and
    // the message wanted `Parties.0.PartyID`, so neither string worked on the
    // other side. Now one does, because a schema answers the item every
    // occurrence of a group holds.
    let registry = committed();
    let message = FixCodec::new(Arc::clone(&registry))
        .parse_fix_line(b"8=FIX.4.4|35=D|453=1|448=BUYSIDE|447=D|452=1|10=0|")
        .expect("a readable frame");
    let member = fpath("Parties[0].PartyID");
    assert_eq!(
        registry
            .field_by_path(&member)
            .expect("the member the schema declares")
            .as_fix()
            .tag()
            .unwrap(),
        Some(448)
    );
    assert_eq!(
        message.by_path(&member).expect("the occurrence's value"),
        &Scalar::from("BUYSIDE")
    );

    // A bare decimal is a name and not a position, exactly as it is one layer
    // down where a text line's entry keyed `55` is reached by the path `55`.
    assert!(message.get_by_path(&fpath("Parties.0.PartyID")).is_none());
    assert!(
        registry
            .get_field_by_path(&fpath("Parties.0.PartyID"))
            .is_none()
    );

    // A position past what the message carried is absence and not an error,
    // and a negative one counts back from the end as the grammar states.
    assert!(message.get_by_path(&fpath("Parties[7].PartyID")).is_none());
    assert_eq!(
        message
            .by_path(&fpath("Parties[-1].PartyID"))
            .expect("the last occurrence"),
        &Scalar::from("BUYSIDE")
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

    // The crate's own fields close every walk: their tags are above any a
    // test claims.
    let crated_names = crate_names();
    let mut iter = registry.iter();
    assert_eq!(iter.len(), 3 + crated());
    assert_eq!(iter.next().map(Field::name), Some("Account"));
    assert_eq!(
        iter.next_back().map(Field::name),
        crated_names.last().copied()
    );
    assert_eq!(iter.len(), 1 + crated());
    assert_eq!(iter.next().map(Field::name), Some("Symbol"));
    assert_eq!(iter.next().map(Field::name), Some("Text"));
    assert_eq!(
        iter.map(Field::name).collect::<Vec<_>>(),
        crated_names[..crated_names.len() - 1].to_vec()
    );
    assert_eq!(
        (&registry).into_iter().map(Field::name).collect::<Vec<_>>(),
        then_crated(&["Account", "Symbol", "Text"])
    );
    // The cursor form walks the same order, and a binding advancing it with
    // only the last identifier it saw sees every field exactly once.
    let mut walked = Vec::new();
    let mut cursor = None;
    while let Some(field) = registry.next_field_after(cursor) {
        walked.push(field.name());
        cursor = field.as_fix().id().unwrap();
    }
    assert_eq!(walked, then_crated(&["Account", "Symbol", "Text"]));
    assert!(
        registry
            .next_field_after(Some(id_of(i32::MAX, "Nothing")))
            .is_none(),
        "an identity this registry does not hold has no place in its order"
    );
    assert_eq!(
        FixRegistry::new().next_field_after(None).map(Field::name),
        crated_names.first().copied(),
        "a new registry walks the crate's own fields"
    );
    // An alternate tag is an index entry, never a cursor stop.
    let mut aliased = tagged("MsgType", 35);
    aliased.as_fix_mut().set_tags(&[2]).unwrap();
    let with_alternate = FixRegistry::from_fields([aliased, tagged("Account", 1)]).unwrap();
    assert_eq!(
        with_alternate
            .next_field_after(Some(id_of(1, "Account")))
            .map(Field::name)
            .unwrap(),
        "MsgType"
    );

    assert_eq!(registry, other);
    assert_ne!(registry, FixRegistry::new());
    assert_eq!(FixRegistry::new(), FixRegistry::default());
    assert!(
        !FixRegistry::new().is_empty(),
        "a new registry holds the crate's own fields"
    );
    assert_eq!(FixRegistry::new().iter().len(), crated());

    // A conflict anywhere fails the whole build: a held name under another
    // tag is a conflict for the strict verb, where the same tag under
    // another name is a second field beside the first.
    let error = FixRegistry::from_fields([tagged("Text", 58), tagged("text", 59)]).unwrap_err();
    assert!(error.is_conflict(), "{error}");
    let beside = FixRegistry::from_fields([tagged("Text", 58), tagged("Symbol", 58)]).unwrap();
    assert_eq!(beside.len(), 2 + crated());
    assert_eq!(beside.field_by_tag(58).unwrap().name(), "Text");
    assert_eq!(
        beside.field_by_id(id_of(58, "Symbol")).unwrap().name(),
        "Symbol"
    );

    // Debug renders the fields under their tag and name, in order.
    let rendered = format!("{registry:?}");
    assert!(rendered.starts_with("{1 Account: "), "{rendered}");
    assert!(rendered.find("55 Symbol: ").unwrap() < rendered.find("58 Text: ").unwrap());
}

#[test]
fn iteration_and_the_cursor_are_tag_major_then_by_identity() {
    // Two fields on tag 35: the specification's and a venue's own reading
    // of the same number, which the holder answers on the wire.
    let venue_kind = member("MsgKind", "cme", 35);
    let registry = FixRegistry::from_fields([
        tagged("Account", 1),
        member("TradeID", "cme", 5001),
        tagged("MsgType", 35),
        venue_kind.clone(),
        member("Venue", "cme", 9000),
    ])
    .unwrap();
    assert_eq!(registry.len(), 5 + crated());
    assert_eq!(registry.field_by_tag(35).unwrap().name(), "MsgType");

    // Tags lead whatever the membership; on an equal tag the holder of the
    // bare tag comes first - it arrived first, and a store writes it first so
    // a reload keeps it the holder - and the identity orders the rest. The
    // crate's own fields, on the highest tags, close the walk.
    let on_35 = [
        (id_of(35, "MsgType"), "MsgType"),
        (id_of(35, "MsgKind"), "MsgKind"),
    ];
    let expected = then_crated(&["Account", "MsgType", "MsgKind", "TradeID", "Venue"]);
    assert_eq!(
        registry.iter().map(Field::name).collect::<Vec<_>>(),
        expected
    );
    let mut backwards = registry.iter().rev().map(Field::name).collect::<Vec<_>>();
    backwards.reverse();
    assert_eq!(backwards, expected);
    // The cursor form walks the same order, and a binding advancing it with
    // only the last identity it saw sees each of the two fields on one tag
    // exactly once.
    let mut walked = Vec::new();
    let mut cursor = None;
    while let Some(field) = registry.next_field_after(cursor) {
        walked.push(field.name());
        cursor = field.as_fix().id().unwrap();
    }
    assert_eq!(walked, expected);
    assert_eq!(
        registry.next_field_after(Some(on_35[0].0)).map(Field::name),
        Some(on_35[1].1)
    );
    assert_eq!(
        registry.next_field_after(Some(on_35[1].0)).map(Field::name),
        Some("TradeID")
    );
    let rendered = format!("{registry:?}");
    assert!(rendered.starts_with("{1 Account: "), "{rendered}");
    assert!(
        rendered.find("35 MsgType: ").unwrap() < rendered.find("5001 TradeID: ").unwrap(),
        "{rendered}"
    );

    // The order follows the tags and not the arrival, except that the holder
    // of a shared tag - decided by arrival, with the alias it lends - leads
    // its tag.
    let reversed = FixRegistry::from_fields([
        member("Venue", "cme", 9000),
        venue_kind,
        tagged("MsgType", 35),
        member("TradeID", "cme", 5001),
        tagged("Account", 1),
    ])
    .unwrap();
    assert_eq!(
        reversed.iter().map(Field::name).collect::<Vec<_>>(),
        then_crated(&["Account", "MsgKind", "MsgType", "TradeID", "Venue"])
    );
    assert_eq!(reversed.field_by_tag(35).unwrap().name(), "MsgKind");
    assert_eq!(
        reversed.next_field_after(Some(on_35[1].0)).map(Field::name),
        Some("MsgType")
    );
    // The holder's departure hands the tag, and the front of its order, to
    // the next arrival on the tag.
    let mut departed = registry.clone();
    departed.remove(on_35[0].0).expect("the holder leaves");
    assert_eq!(departed.field_by_tag(35).unwrap().name(), "MsgKind");
    assert_eq!(
        departed.iter().map(Field::name).collect::<Vec<_>>(),
        then_crated(&["Account", "MsgKind", "TradeID", "Venue"])
    );
}

#[test]
fn fields_reject_nested_shapes_and_keep_the_registry_unchanged() {
    let mut registry = FixRegistry::from_fields([tagged("Symbol", 55)]).unwrap();
    let original = registry.clone();
    for dtype in [
        DataType::from_fields([tagged("Member", 9_001)]).unwrap(),
        DataType::list(DataType::utf8().required_field("item")),
        DataType::dictionary(
            DataType::Int32,
            DataType::from_fields([tagged("Member", 9_002)]).unwrap(),
        )
        .unwrap(),
    ] {
        let mut field = dtype.nullable_field("InvalidWireField");
        field.as_fix_mut().set_tag(453).unwrap();
        let error = registry.insert(field).unwrap_err();
        assert!(error.to_string().contains("scalar"), "{error}");
        assert_eq!(registry, original);
    }
    let mut encoded = DataType::dictionary(DataType::Int32, DataType::utf8())
        .unwrap()
        .nullable_field("Coded");
    encoded.as_fix_mut().set_tag(60).unwrap();
    registry.insert(encoded).unwrap();
    assert_eq!(registry.len(), 2 + crated());
}

#[test]
fn a_derived_tag_identifies_one_definition_however_it_arrived() {
    let mut registry = FixRegistry::from_fields([counter("NoPartyIDs", 453)]).unwrap();
    registry
        .insert_definition(FixCategory::Groups, named_group("Parties", 453))
        .unwrap();
    let first = registry
        .definition(FixCategory::Groups, "Parties")
        .unwrap()
        .as_fix()
        .tag()
        .unwrap()
        .expect("a derived tag");

    // A definition cloned under a second name carries the first one's tag. It
    // is not that definition, so it does not keep that identity.
    let mut clone = registry
        .definition(FixCategory::Groups, "Parties")
        .unwrap()
        .clone();
    clone.set_name("Counterparties");
    registry
        .insert_definition(FixCategory::Groups, clone)
        .unwrap();
    let second = registry
        .definition(FixCategory::Groups, "Counterparties")
        .unwrap()
        .as_fix()
        .tag()
        .unwrap()
        .expect("a derived tag");
    assert_ne!(first, second);
    assert!(FixId::is_definition_tag(second), "{second}");

    // Re-stating a definition keeps the identity it already has.
    let again = registry
        .definition(FixCategory::Groups, "Parties")
        .unwrap()
        .clone();
    registry
        .insert_definition(FixCategory::Groups, again)
        .unwrap();
    assert_eq!(
        registry
            .definition(FixCategory::Groups, "Parties")
            .unwrap()
            .as_fix()
            .tag()
            .unwrap(),
        Some(first)
    );

    // A tag outside the block is refused whatever states it.
    let mut outside = named_group("Brokers", 453);
    outside.as_fix_mut().set_tag(42).unwrap();
    let refused = registry
        .insert_definition(FixCategory::Groups, outside)
        .unwrap_err();
    assert!(refused.to_string().contains("100000"), "{refused}");
}

#[test]
fn a_named_group_and_its_counter_keep_separate_identities() {
    let mut shadow = tagged("Shadow", 5);
    shadow.as_fix_mut().set_tags(&[453]).unwrap();
    let mut registry = FixRegistry::from_fields([shadow, counter("NoPartyIDs", 453)]).unwrap();
    registry
        .insert_definition(FixCategory::Groups, named_group("Parties", 453))
        .unwrap();
    assert_eq!(registry.field_by_tag(453).unwrap().name(), "NoPartyIDs");
    assert_eq!(
        registry.field_by_tag(453).unwrap().dtype(),
        &DataType::Int32
    );
    let group = registry.definition(FixCategory::Groups, "PARTIES").unwrap();
    // The group's own identity is derived into the definition block; the
    // counter it heads stays the published tag 453, and the two never meet.
    let derived = group.as_fix().tag().unwrap().expect("a derived tag");
    assert!(FixId::is_definition_tag(derived), "{derived}");
    assert_ne!(derived, 453);
    assert_eq!(group.as_fix().counter().unwrap(), Some(453));
    assert_eq!(registry.group_by_counter(453).unwrap(), group);
    assert_eq!(registry.field_by_tag(5).unwrap().name(), "Shadow");
}

#[test]
fn named_definitions_refuse_wire_tags_and_invalid_counters_atomically() {
    let mut registry =
        FixRegistry::from_fields([counter("NoPartyIDs", 453), tagged("Symbol", 55)]).unwrap();
    let original = registry.clone();
    let mut tagged_group = named_group("Parties", 453);
    tagged_group.as_fix_mut().set_tag(453).unwrap();
    for group in [
        tagged_group,
        named_group("AbsentCounter", 454),
        named_group("TextCounter", 55),
    ] {
        assert!(
            registry
                .insert_definition(FixCategory::Groups, group)
                .is_err()
        );
        assert_eq!(registry, original);
    }
}

#[test]
fn scalar_iteration_and_named_category_iteration_have_distinct_orders() {
    let mut registry = FixRegistry::from_fields([
        tagged("Text", 58),
        counter("NoPartyIDs", 453),
        tagged("Account", 1),
        tagged("Symbol", 55),
    ])
    .unwrap();
    registry
        .insert_definition(FixCategory::Groups, named_group("Parties", 453))
        .unwrap();
    let expected = then_crated(&["Account", "Symbol", "Text", "NoPartyIDs"]);
    assert_eq!(
        registry.iter().map(Field::name).collect::<Vec<_>>(),
        expected
    );
    assert_eq!(registry.iter().len(), expected.len());
    assert_eq!(registry.len(), expected.len());
    let mut backwards = registry.iter().rev().map(Field::name).collect::<Vec<_>>();
    backwards.reverse();
    assert_eq!(backwards, expected);
    let mut cursor = None;
    let mut walked = Vec::new();
    while let Some(field) = registry.next_field_after(cursor) {
        walked.push(field.name());
        cursor = field.as_fix().id().unwrap();
    }
    assert_eq!(walked, expected);
    assert_eq!(
        registry
            .definitions(FixCategory::Groups)
            .map(Field::name)
            .collect::<Vec<_>>(),
        ["Parties"]
    );
    assert_ne!(
        registry,
        FixRegistry::from_fields(registry.iter().cloned()).unwrap()
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

    // Nothing configured, or no home at all: a new registry, holding the
    // crate's own fields and nothing else.
    assert_eq!(
        autoload(None, Some(config.clone())).unwrap(),
        FixRegistry::new()
    );
    assert_eq!(autoload(None, None).unwrap(), FixRegistry::new());

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
        assert_eq!(loaded.len(), 1 + crated(), "{spelling}");
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
    std::fs::write(location.join("fields").join("0.json"), b"not json").unwrap();
    let error = autoload(Some(&location.to_string_lossy()), None).unwrap_err();
    let message = error.to_string();
    assert!(message.contains("0.json"), "{message}");
    std::fs::write(home.join(".config/fix/fields/0.json"), b"[1]").unwrap();
    let error = autoload(None, Some(config)).unwrap_err();
    let message = error.to_string();
    assert!(message.contains("0.json"), "{message}");
    assert!(message.contains("field mapping"), "{message}");

    let _ = std::fs::remove_dir_all(&root);
}

/// A registry, a root and a value the message tests share.
fn order() -> (Arc<FixRegistry>, Field, Scalar) {
    let mut party_id = DataType::utf8().nullable_field("PartyID");
    party_id.as_fix_mut().set_tag(448).unwrap();
    let mut role = DataType::Int32.nullable_field("PartyRole");
    role.as_fix_mut().set_tag(452).unwrap();
    let item = DataType::from_fields([party_id, role])
        .unwrap()
        .required_field("Party");
    let mut group = DataType::list(item).nullable_field("Parties");
    group.as_fix_mut().set_counter(453).unwrap();
    let instrument = DataType::from_fields([tagged("Symbol", 55)])
        .unwrap()
        .nullable_field("Instrument");
    let mut qty = DataType::Int64.required_field("OrderQty");
    qty.as_fix_mut().set_tag(38).unwrap();
    qty.as_fix_mut().set_aliases(["Qty"]).unwrap();
    let count = counter("NoPartyIDs", 453);
    let mut registry =
        FixRegistry::from_fields([count.clone(), qty.clone(), tagged("Symbol", 55)]).unwrap();
    registry
        .insert_definition(FixCategory::Groups, group.clone())
        .unwrap();
    registry
        .insert_definition(FixCategory::Components, instrument.clone())
        .unwrap();
    let registry = Arc::new(registry);
    let root = DataType::from_fields([
        qty,
        instrument,
        count,
        group,
        DataType::utf8().nullable_field("9999"),
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
            "Parties",
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
        ("NoPartyIDs", Scalar::from(2_i32)),
        ("9999", Scalar::from("custom")),
    ])
    .unwrap();
    (registry, root, value)
}

#[test]
fn folded_message_child_lookup_does_not_choose_between_colliding_names() {
    let field = DataType::from_fields([
        DataType::utf8().required_field("A"),
        DataType::utf8().required_field("a"),
    ])
    .unwrap()
    .required_field("message");
    let message = FixMsg::with_registry(
        Arc::new(FixRegistry::new()),
        field,
        Scalar::from_sequence([Scalar::from("first"), Scalar::from("second")]),
    )
    .unwrap();
    assert_eq!(message.by_name("A").unwrap().as_str(), Some("first"));
    assert_eq!(message.by_name("a").unwrap().as_str(), Some("second"));
    assert!(message.get_by_name("a_").is_none());
}

#[test]
fn a_message_resolves_values_through_its_registry() {
    let (registry, root, value) = order();
    let msg = FixMsg::with_registry(Arc::clone(&registry), root.clone(), value).unwrap();
    assert!(Arc::ptr_eq(msg.registry(), &registry));
    assert_eq!(msg.as_field(), &root);

    // A record input canonicalizes to the ordered sequence the root declares.
    let row = msg.as_value().as_sequence().unwrap();
    assert_eq!(row.len(), 5);
    assert_eq!(row[0], Scalar::from(100));
    assert_eq!(row[2], Scalar::from(2_i32));
    assert_eq!(row[4], Scalar::from("custom"));

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
        msg.by_path(&fpath("Instrument.symbol")).unwrap(),
        &Scalar::from("AAPL")
    );
    assert_eq!(
        msg.by_path(&fpath("Parties[1].PartyID")).unwrap(),
        &Scalar::from("CLIENT")
    );
    assert_eq!(
        msg.by_path(&fpath("parties[0].PartyRole")).unwrap(),
        &Scalar::from(1)
    );
    assert_eq!(msg.by_path(&fpath("Parties")).unwrap().len(), 2);
    assert!(
        msg.get_by_path(&fpath("Parties.PartyID")).is_none(),
        "a group member needs its index"
    );
    assert!(msg.get_by_path(&fpath("Parties[2].PartyID")).is_none());
    assert!(msg.get_by_path(&fpath("OrderQty.deeper")).is_none());
    assert!(
        msg.get_by_tag(55).is_none(),
        "Symbol is nested, not a root child"
    );
    // A member the registry does not know resolves its unique local spelling.
    assert_eq!(
        msg.by_path(&fpath("Parties[0].PartyID")).unwrap(),
        &Scalar::from("BROKER")
    );
    assert_eq!(
        msg.by_path(&fpath("Parties[0].partyid")).unwrap(),
        &Scalar::from("BROKER")
    );
    assert!(msg.get_by_tag(-1).is_none());

    // The generic pair matches the specialized one for every key.
    for tag in [38, 9999, 55, 453] {
        assert_eq!(msg.get(tag), msg.get_by_tag(tag), "{tag}");
        assert_eq!(msg.value(tag).ok(), msg.by_tag(tag).ok(), "{tag}");
    }
    // The generic pair matches the specialized one for every key: a name
    // reaches what the name door reaches, and a key spelling more than one
    // segment reaches what the path door reaches once that key is read.
    for name in ["OrderQty", "qty", "absent"] {
        assert_eq!(msg.get(name), msg.get_by_name(name), "{name}");
        assert_eq!(msg.value(name).ok(), msg.by_name(name).ok(), "{name}");
    }
    for spelling in ["Instrument.Symbol", "Parties[1].PartyID"] {
        assert_eq!(
            msg.get(spelling),
            msg.get_by_path(&fpath(spelling)),
            "{spelling}"
        );
        assert_eq!(
            msg.value(spelling).ok(),
            msg.by_path(&fpath(spelling)).ok(),
            "{spelling}"
        );
    }
    // A path of one named segment is that name lookup, which is what makes
    // the two doors one reading rather than two.
    for name in ["OrderQty", "qty", "absent"] {
        assert_eq!(
            msg.get_by_path(&fpath(name)),
            msg.get_by_name(name),
            "{name}"
        );
    }
    let error = msg.by_tag(55).unwrap_err();
    assert!(matches!(&error, Error::Absent { expected: "fix value", path } if path == "tag 55"));
    let error = msg.by_path(&fpath("absent.x")).unwrap_err();
    assert!(matches!(&error, Error::Absent { path, .. } if path == "path absent.x"));

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
fn a_message_resolves_a_bare_tag_to_its_first_holder_and_an_identity_exactly() {
    // Tag 5001 is held twice, under two names: the venue's arrived first
    // and so answers the bare tag; the specification's is reached by its
    // own name or identity. Tag 35 is held once.
    let mut venue_trade = member("TradeID", "cme", 5001);
    venue_trade.as_fix_mut().set_aliases(["TID"]).unwrap();
    let mut spec_trade = tagged("SecondaryTradeID", 5001);
    spec_trade.as_fix_mut().set_aliases(["STID"]).unwrap();
    let msg_type = tagged("MsgType", 35);
    let registry = Arc::new(
        FixRegistry::from_fields([venue_trade.clone(), spec_trade.clone(), msg_type.clone()])
            .unwrap(),
    );
    assert_eq!(registry.field_by_tag(5001).unwrap().name(), "TradeID");

    // A message root the venue's fields shape carries no membership of its
    // own: a message is not a dictionary member.
    let root = DataType::from_fields([venue_trade.clone(), msg_type.clone()])
        .unwrap()
        .required_field("VenueExecutionReport");
    let value = Scalar::from_record([
        ("TradeID", Scalar::from("T-1")),
        ("MsgType", Scalar::from("8")),
    ])
    .unwrap();
    let msg = FixMsg::with_registry(Arc::clone(&registry), root, value).unwrap();
    assert_eq!(msg.as_field().as_fix().branches().count(), 0);

    // The bare tag: its first holder, which is the child this root carries.
    assert_eq!(msg.by_tag(5001).unwrap(), &Scalar::from("T-1"));
    assert_eq!(msg.by_name("tid").unwrap(), &Scalar::from("T-1"));
    // One namespace, so MsgType is reachable from a venue message.
    assert_eq!(msg.by_tag(35).unwrap(), &Scalar::from("8"));
    assert_eq!(msg.by_name("msgtype").unwrap(), &Scalar::from("8"));
    // The specification's field on 5001 names a root child this message
    // does not hold, so its alias misses rather than answering the venue's
    // value.
    assert!(msg.get_by_name("stid").is_none());

    // An identity is exact: it names one field, and the child is the one
    // that field's name reaches.
    let venue_id = id_of(5001, "TradeID");
    let spec_id = id_of(5001, "SecondaryTradeID");
    assert_eq!(msg.by_id(venue_id).unwrap(), &Scalar::from("T-1"));
    assert!(
        msg.get_by_id(spec_id).is_none(),
        "the other field on the tag misses"
    );
    assert_eq!(msg.get(venue_id), msg.get_by_id(venue_id));
    assert_eq!(msg.value(venue_id).unwrap(), msg.by_id(venue_id).unwrap());
    let error = msg.by_id(spec_id).unwrap_err();
    assert!(
        matches!(&error, Error::Absent { expected: "fix value", path }
            if *path == format!("identifier {spec_id}")),
        "{error}"
    );
    let error = msg.by_id(id_of(5001, "Nobody")).unwrap_err();
    assert!(error.is_absent(), "{error}");

    // A message shaped by the specification's field reaches it by the tag
    // its own child carries, whoever holds the bare tag in the dictionary.
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
    assert_eq!(plain.as_field().as_fix().branches().count(), 0);
    assert_eq!(plain.by_tag(5001).unwrap(), &Scalar::from("S-1"));
    assert_eq!(plain.by_name("stid").unwrap(), &Scalar::from("S-1"));
    assert_eq!(plain.by_id(spec_id).unwrap(), &Scalar::from("S-1"));
    assert!(plain.get_by_name("tid").is_none());
    assert!(plain.get_by_id(venue_id).is_none());
}

#[test]
fn a_message_root_carrying_membership_is_read_as_any_root_is() {
    // Membership is provenance on a dictionary field; on a root it states
    // nothing the message reads, and a spelling nothing validates on read
    // is not a refusal.
    let mut root = DataType::from_fields([tagged("MsgType", 35)])
        .unwrap()
        .required_field("row");
    root.as_fix_mut().set_branches(["cme"]).unwrap();
    root.insert_metadata("fix:branches", "2cme,c me").unwrap();
    let message = FixMsg::with_registry(
        Arc::new(FixRegistry::new()),
        root,
        Scalar::from_record([("MsgType", Scalar::from("8"))]).unwrap(),
    )
    .unwrap();
    assert_eq!(message.by_tag(35).unwrap(), &Scalar::from("8"));
    assert_eq!(
        message.as_field().as_fix().branches().collect::<Vec<_>>(),
        ["2cme", "c me"],
        "read back as stored"
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
    assert_eq!(view.name_at(version("5.0.2")), Some("LastQty"));
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
    let mut field = DataType::utf8().nullable_field("BasisPoints");
    field.as_fix_mut().set_tag(9999).unwrap();
    field
        .as_fix_mut()
        .set_lineage(&[
            FixLineageEntry::new(FixPedigree::new(version("5.0.2"), Some(309)))
                .with_name("BasisPoints"),
            FixLineageEntry::new(FixPedigree::new(version("5.0.2"), Some(204)))
                .with_name("Superseded"),
            FixLineageEntry::new(FixPedigree::new(version("5.0.2"), None)).with_name("Base"),
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
        field.as_fix().name_at(version("5.0.2")),
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
    let mut retired = DataType::utf8().nullable_field("Retired");
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
    assert!(!view.defined_at(version("5.0.2")));

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
    let mut field = DataType::utf8().nullable_field("LastQty");
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
    let mut field = DataType::utf8().nullable_field("Twice");
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
    let mut edited = DataType::utf8().nullable_field("LastShares");
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
    super::FixCodec::new(committed())
}

#[test]
fn a_report_states_what_is_left_once_it_has_stated_the_rest() {
    let codec = enriching();
    // Appendix D: a part-filled working order. What is left is what was
    // ordered minus what was done, and the fill's worth is its quantity at
    // its price.
    let held = codec
        .parse_fix_line(b"8=FIX.4.4|35=8|39=1|150=F|38=100|14=40|32=40|31=10.5|54=1|10=0|")
        .and_then(|held| codec.enrich_message(held))
        .expect("a readable report");
    assert_eq!(held.by_tag(151).unwrap(), &Scalar::from(60.0_f64));
    assert_eq!(held.by_tag(381).unwrap(), &Scalar::from(420.0_f64));
    // One fill, so the average is that fill's price.
    assert_eq!(held.by_tag(6).unwrap(), &Scalar::from(10.5_f64));

    // A closed order leaves nothing, whatever the arithmetic of the other two
    // would say: Appendix D shows zero on every terminal row.
    let closed = codec
        .parse_fix_line(b"8=FIX.4.4|35=8|39=4|150=4|38=100|14=40|10=0|")
        .and_then(|held| codec.enrich_message(held))
        .expect("a readable report");
    assert_eq!(closed.by_tag(151).unwrap(), &Scalar::from(0.0_f64));

    // The same identity read backwards: what was ordered is what is left plus
    // what was done.
    let ordered = codec
        .parse_fix_line(b"8=FIX.4.4|35=8|39=1|14=40|151=60|10=0|")
        .and_then(|held| codec.enrich_message(held))
        .expect("a readable report");
    assert_eq!(ordered.by_tag(38).unwrap(), &Scalar::from(100.0_f64));
}

#[test]
fn a_stated_value_is_never_replaced_and_filling_twice_changes_nothing() {
    let codec = enriching();
    // The venue's own arithmetic wins even where it disagrees with the
    // specification's: the row says what was sent.
    let held = codec
        .parse_fix_line(b"8=FIX.4.4|35=8|39=1|38=100|14=40|151=999|10=0|")
        .and_then(|held| codec.enrich_message(held))
        .expect("a readable report");
    assert_eq!(held.by_tag(151).unwrap(), &Scalar::from(999.0_f64));

    // Idempotent: a value derived once is a stated value the second time, so
    // a second pass derives it to itself.
    let once = codec
        .parse_fix_line(b"8=FIX.4.4|35=8|39=1|38=100|14=40|10=0|")
        .and_then(|held| codec.enrich_message(held))
        .expect("a readable report");
    let twice = codec.enrich_message(once.clone()).expect("a second pass");
    assert_eq!(once, twice);
}

#[test]
fn filling_leaves_the_wire_exactly_as_it_arrived() {
    let codec = enriching();
    const LINE: &[u8] = b"8=FIX.4.4|35=8|39=1|38=100|14=40|32=40|31=10.5|54=1|10=0|";
    let bare = codec.parse_fix_line(LINE).expect("a readable report");
    let filled = codec
        .parse_fix_line(LINE)
        .and_then(|held| codec.enrich_message(held))
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
        .parse_fix_line(b"8=FIX.4.4|35=8|39=1|38=100|10=0|")
        .and_then(|held| codec.enrich_message(held))
        .expect("a readable report");
    assert_eq!(held.get_by_tag(151), None, "no CumQty to subtract");

    // A negative remainder means the two inputs were never about one order,
    // so the rule declines rather than stating a quantity that cannot exist.
    let crossed = codec
        .parse_fix_line(b"8=FIX.4.4|35=8|39=1|38=40|14=100|10=0|")
        .and_then(|held| codec.enrich_message(held))
        .expect("a readable report");
    assert_eq!(crossed.get_by_tag(151), None);

    // A status the matrices do not place answers nothing either.
    let unknown = codec
        .parse_fix_line(b"8=FIX.4.4|35=8|39=Z|38=100|14=40|10=0|")
        .and_then(|held| codec.enrich_message(held))
        .expect("a readable report");
    assert_eq!(unknown.get_by_tag(151), None);

    // A message type the rule does not speak for is left alone: an order has
    // no remainder to state until something reports on it.
    let order = codec
        .parse_fix_line(b"8=FIX.4.4|35=D|38=100|14=40|10=0|")
        .and_then(|held| codec.enrich_message(held))
        .expect("a readable order");
    assert_eq!(order.get_by_tag(151), None);
}

#[test]
fn a_foreign_exchange_trade_settles_in_the_currency_it_was_dealt_in() {
    let codec = enriching();
    // Appendix O: the settlement currency defaults to the dealt one, and the
    // settled amount is the traded amount at the stated rate.
    let held = codec
        .parse_fix_line(
            b"8=FIX.4.4|35=8|39=2|150=F|38=100|14=100|32=100|31=1.25|15=EUR|155=1.1|10=0|",
        )
        .and_then(|held| codec.enrich_message(held))
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
        .parse_fix_line(b"8=FIX.4.4|35=8|39=2|15=EUR|120=USD|10=0|")
        .and_then(|held| codec.enrich_message(held))
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
        .parse_fix_line(b"99=N/A|")
        .expect("a readable frame");
    assert_eq!(message.get_by_tag(99), Some(&Scalar::Null));
    let entry = message
        .entries()
        .iter()
        .find(|held| held.tag() == 99)
        .expect("the pair still arrived");
    assert_eq!(entry.value().as_str(), Some("N/A"));

    // A value the list does not name is read as the price it is.
    let message = super::FixCodec::new(registry)
        .parse_fix_line(b"99=12.5|")
        .expect("a readable frame");
    assert_eq!(message.by_tag(99).unwrap(), &Scalar::from(12.5_f64));
}

#[test]
fn a_null_spelling_is_refused_when_it_carries_the_separator_or_repeats() {
    let mut field = DataType::utf8().nullable_field("Account");
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
    let mut field = DataType::utf8().nullable_field("account");
    field.as_fix_mut().set_tag(1).unwrap();
    // The specification renamed the type without changing it: `char` and
    // `String` are one `string`.
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
        Some(r#"{"entries":[{"since":"2.7","name":"account","type":{"type":"string"}}]}"#)
    );
    // The oldest entry survives, so `since` still dates the field.
    assert_eq!(field.as_fix().since(), Some(version("2.7")));
    // And the type is answered at both versions, from the one entry left.
    assert_eq!(
        field.as_fix().dtype_at(version("4.4")).unwrap(),
        Some(DataType::utf8())
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
        let mut field = DataType::utf8().nullable_field("kept");
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
        FixLineageEntry::new(FixPedigree::new(version("5.0.2"), None)).with_dtype("char"),
        FixLineageEntry::new(FixPedigree::new(version("5.0.2"), Some(309))).with_dtype("String"),
    ])
    .expect("the entries render");
    assert_eq!(
        rendered,
        r#"{"entries":[{"since":"5.0.2","type":{"type":"string"}}]}"#
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
    let mut field = DataType::utf8().nullable_field("Side");
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
    let mut field = DataType::utf8().nullable_field("CommType");
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
            FixCode::new("BasisPoints", "7").with_since(version("5.0.2"), Some(208)),
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
    let mut field = DataType::utf8().nullable_field("Side");
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

    let mut field = DataType::utf8().nullable_field("Side");
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
    let mut field = DataType::utf8().nullable_field("Side");
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
    let mut aliased = DataType::utf8().nullable_field("Side");
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
    let mut field = DataType::utf8().nullable_field("TimeInForce");
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
        comm.code_value_at(version("5.0.2"), "BasisPoints"),
        Some("7")
    );
    assert_eq!(comm.code("7").unwrap().ep(), Some(208));

    let mut retired = DataType::utf8().nullable_field("OldFlag");
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
    let mut edited = DataType::utf8().nullable_field("Side");
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
    let mut field = DataType::utf8().nullable_field("Side");
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
    let mut field = DataType::utf8().nullable_field("Side");
    field.as_fix_mut().set_tag(54).unwrap();
    field
        .as_fix_mut()
        .set_codes(&[FixCode::new("Buy", "1")
            .with_description(r#"Buy; the "long" side"#)
            .with_aliases(["Bought"])
            .with_since(version("2.7"), Some(254))
            .with_deprecated(version("5.0.2"))
            .with_sort(10)
            .with_group("Directional")])
        .unwrap();

    let view = field.as_fix();
    let code = view.code("1").unwrap();
    assert_eq!(code.name(), "Buy");
    assert_eq!(code.since(), Some(version("2.7")));
    assert_eq!(code.ep(), Some(254));
    assert_eq!(code.deprecated(), Some(version("5.0.2")));
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
    let mut stored = DataType::utf8().nullable_field("LastQty");
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
    let mut incoming = DataType::utf8().nullable_field("LastQty");
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
            FixLineageEntry::new(FixPedigree::new(version("5.0.2"), None))
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
    assert_eq!(dated, [version("2.7"), version("4.3"), version("5.0.2")]);
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
    let mut stored = DataType::utf8().nullable_field("Symbol");
    stored.as_fix_mut().set_tag(55).unwrap();
    stored
        .as_fix_mut()
        .set_description("a very long stored wording nobody wants compared")
        .unwrap();

    let mut incoming = DataType::utf8().nullable_field("Symbol");
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
    let mut incoming = DataType::utf8().nullable_field("Symbol");
    incoming.as_fix_mut().set_tag(55).unwrap();
    incoming.as_fix_mut().set_aliases(["Ticker"]).unwrap();
    let before = incoming.clone();

    let mut other = DataType::utf8().nullable_field("Symbol");
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

    // Membership is no half of the identity: the same tag spoken by a venue
    // merges, and the merge records who spoke it.
    let vendor = member("Symbol", "cme", 5055);
    let mut mine = DataType::utf8().nullable_field("Symbol");
    mine.as_fix_mut().set_tag(5055).unwrap();
    mine.as_fix_mut().merge_with(&vendor.as_fix()).unwrap();
    assert_eq!(mine.as_fix().branches().collect::<Vec<_>>(), ["cme"]);
    assert_eq!(mine.as_fix().tag().unwrap(), Some(5055));
    // And the union is a union: two dictionaries each stamping itself leave
    // both names, sorted, whichever side held which.
    let mut theirs = member("Symbol", "xnas", 5055);
    theirs.as_fix_mut().merge_with(&mine.as_fix()).unwrap();
    assert_eq!(
        theirs.as_fix().branches().collect::<Vec<_>>(),
        ["cme", "xnas"]
    );
}

#[test]
fn a_merge_adding_nothing_leaves_the_field_byte_identical() {
    let mut field = DataType::utf8().nullable_field("LastQty");
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
    let mut bare = DataType::utf8().nullable_field("LastQty");
    bare.as_fix_mut().set_tag(32).unwrap();
    field.as_fix_mut().merge_with(&bare.as_fix()).unwrap();
    assert_eq!(field, before);
}

/// Fixture C: `Rule80A(47)`, stating every shape a replacement rule has.
///
/// Three entries in a deliberate order: a value rule, a rule scoped to two
/// message types and one repeating group, and a catch-all filling a group
/// occurrence whose members reach every fill source, one of them a group
/// again.
fn rule80a_rules() -> Vec<FixReplacement> {
    vec![
        FixReplacement::new(version("4.3"))
            .with_when("C")
            .with_fills([
                FixFill::Field {
                    tag: 528,
                    value: FixFillSource::Constant("P".into()),
                },
                FixFill::Field {
                    tag: 529,
                    value: FixFillSource::Constant("1 3".into()),
                },
            ])
            .with_doc(r#"Program order, non-index arbitrage, for "other" agency"#),
        FixReplacement::new(version("4.3"))
            .with_ep(12)
            .with_msgtypes(["8", "AE"])
            .with_in(["allocgrp"])
            .with_when("A")
            .with_fills([FixFill::Field {
                tag: 528,
                value: FixFillSource::Constant("A".into()),
            }]),
        FixReplacement::new(version("4.3")).with_fills([FixFill::Group {
            name: "parties".into(),
            members: vec![
                FixFill::Field {
                    tag: 448,
                    value: FixFillSource::Source,
                },
                FixFill::Field {
                    tag: 628,
                    value: FixFillSource::From(115),
                },
                FixFill::Group {
                    name: "ptyssubgrp".into(),
                    members: vec![FixFill::Field {
                        tag: 523,
                        value: FixFillSource::Join(vec![200, 205]),
                    }],
                },
            ],
        }]),
    ]
}

/// The one text fixture C renders to.
const RULE80A_DOCUMENT: &str = concat!(
    r#"{"replacements":["#,
    r#"{"since":"4.3","when":"C","fills":[{"tag":528,"value":"P"},{"tag":529,"value":"1 3"}],"#,
    r#""doc":"Program order, non-index arbitrage, for \"other\" agency"},"#,
    r#"{"since":"4.3","ep":12,"msgtypes":["8","AE"],"in":["allocgrp"],"when":"A","#,
    r#""fills":[{"tag":528,"value":"A"}]},"#,
    r#"{"since":"4.3","fills":[{"group":"parties","members":[{"tag":448},{"tag":628,"from":115},"#,
    r#"{"group":"ptyssubgrp","members":[{"tag":523,"join":[200,205]}]}]}]}]}"#,
);

fn rule80a() -> Field {
    let mut field = DataType::utf8().nullable_field("rule80a");
    field.as_fix_mut().set_tag(47).unwrap();
    field
        .as_fix_mut()
        .set_replacements(&rule80a_rules())
        .unwrap();
    field
}

/// A field carrying one hand-written `fix:replacements` text, unvalidated.
fn replacing(document: &str) -> Field {
    let mut field = DataType::utf8().nullable_field("rule80a");
    field
        .set_metadata([("fix:replacements", document)])
        .unwrap();
    field
}

#[test]
fn a_replacement_document_round_trips_canonically_and_in_order() {
    let field = rule80a();
    assert_eq!(
        field.get_metadata("fix:replacements"),
        Some(RULE80A_DOCUMENT)
    );

    // The borrowed read hands back every fact as a slice of that text.
    let view = field.as_fix();
    let entries: Vec<_> = view
        .replacements()
        .map(|entry| entry.expect("a readable entry"))
        .collect();
    assert_eq!(entries.len(), 3);
    let valued = entries[0];
    assert_eq!(valued.since(), version("4.3"));
    assert_eq!(valued.ep(), None);
    assert_eq!(valued.msgtypes().count(), 0, "every message");
    assert_eq!(valued.in_groups().count(), 0, "wherever the field sits");
    assert_eq!(valued.when(), Some("C"));
    assert_eq!(
        valued.doc(),
        Some(r#"Program order, non-index arbitrage, for \"other\" agency"#),
        "still escaped as stored"
    );
    assert_eq!(
        valued.parse_doc().unwrap().as_deref(),
        Some(r#"Program order, non-index arbitrage, for "other" agency"#)
    );
    let fills: Vec<_> = valued.fills().map(|fill| fill.unwrap()).collect();
    assert!(matches!(
        fills.as_slice(),
        [
            FixFillEntry::Field {
                tag: 528,
                value: FixFillValue::Constant("P")
            },
            FixFillEntry::Field {
                tag: 529,
                value: FixFillValue::Constant("1 3")
            },
        ]
    ));

    let scoped = entries[1];
    assert_eq!(scoped.ep(), Some(12));
    assert_eq!(scoped.msgtypes().collect::<Vec<_>>(), ["8", "AE"]);
    assert_eq!(scoped.in_groups().collect::<Vec<_>>(), ["allocgrp"]);
    assert_eq!(scoped.when(), Some("A"));
    assert_eq!(scoped.doc(), None);

    // A group fill nests, and its members are a walk of their own.
    let grouped = entries[2];
    assert_eq!(grouped.when(), None, "the catch-all comes last");
    let mut fills = grouped.fills();
    let Some(FixFillEntry::Group { name, mut members }) = fills.next_ok() else {
        panic!("a group fill");
    };
    assert_eq!(name, "parties");
    assert!(fills.next_ok().is_none());
    assert!(matches!(
        members.next_ok(),
        Some(FixFillEntry::Field {
            tag: 448,
            value: FixFillValue::Source
        })
    ));
    assert!(matches!(
        members.next_ok(),
        Some(FixFillEntry::Field {
            tag: 628,
            value: FixFillValue::From(115)
        })
    ));
    let Some(FixFillEntry::Group { name, mut members }) = members.next_ok() else {
        panic!("a nested group fill");
    };
    assert_eq!(name, "ptyssubgrp");
    let Some(FixFillEntry::Field {
        tag: 523,
        value: FixFillValue::Join(tags),
    }) = members.next_ok()
    else {
        panic!("a join fill");
    };
    assert_eq!(tags.collect::<Vec<_>>(), [200, 205]);
    assert!(members.next_ok().is_none());

    // Owning what was read answers exactly what was written, so taking the
    // rules away and putting them back produces the same text - and the
    // order is kept, because it is the rule.
    let owned: Vec<FixReplacement> = entries.into_iter().map(FixReplacement::from).collect();
    assert_eq!(owned, rule80a_rules());
    let mut rebuilt = field.clone();
    let taken = rebuilt.as_fix_mut().remove_replacements().unwrap().unwrap();
    assert_eq!(taken, rule80a_rules());
    assert_eq!(rebuilt.get_metadata("fix:replacements"), None);
    assert_eq!(rebuilt.as_fix().replacements().count(), 0);
    rebuilt.as_fix_mut().set_replacements(&taken).unwrap();
    assert_eq!(rebuilt, field);
    assert_eq!(
        rebuilt.as_fix_mut().remove_replacements().unwrap(),
        Some(taken)
    );
    assert_eq!(rebuilt.as_fix_mut().remove_replacements().unwrap(), None);
}

#[test]
fn an_empty_replacement_set_removes_the_property() {
    let mut field = rule80a();
    field.as_fix_mut().set_replacements(&[]).unwrap();
    assert_eq!(field.get_metadata("fix:replacements"), None);
    assert_eq!(field.as_fix().replacements().count(), 0);
    assert!(field.as_fix().replacements().next_ok().is_none());
    // Removing what is not there is not an error.
    assert_eq!(field.as_fix_mut().remove_replacements().unwrap(), None);
}

#[test]
fn the_replacement_writer_refuses_what_the_document_cannot_state() {
    let field_fill = |tag: i32, value: FixFillSource| FixFill::Field { tag, value };
    let constant = |text: &str| FixFillSource::Constant(text.into());
    let sound = || FixReplacement::new(version("4.3")).with_fills([field_fill(528, constant("A"))]);
    for (rules, names) in [
        // An entry stating no fill restates nothing.
        (
            vec![FixReplacement::new(version("4.3"))],
            "at least one fill",
        ),
        // A group occurrence with no member is no occurrence.
        (
            vec![
                FixReplacement::new(version("4.3")).with_fills([FixFill::Group {
                    name: "parties".into(),
                    members: Vec::new(),
                }]),
            ],
            "at least one member",
        ),
        // A join of one tag is a `from`.
        (
            vec![
                FixReplacement::new(version("4.3"))
                    .with_fills([field_fill(541, FixFillSource::Join(vec![200]))]),
            ],
            "at least two tags",
        ),
        // Tags are non-negative wherever they stand.
        (
            vec![FixReplacement::new(version("4.3")).with_fills([field_fill(-1, constant("A"))])],
            "a FIX tag, got -1",
        ),
        (
            vec![
                FixReplacement::new(version("4.3"))
                    .with_fills([field_fill(628, FixFillSource::From(-115))]),
            ],
            "a FIX tag, got -115",
        ),
        (
            vec![
                FixReplacement::new(version("4.3"))
                    .with_fills([field_fill(541, FixFillSource::Join(vec![200, -205]))]),
            ],
            "a FIX tag, got -205",
        ),
        // What the reader reads back as a word must be written as one.
        (vec![sound().with_when("")], r#""when""#),
        (
            vec![
                FixReplacement::new(version("4.3"))
                    .with_fills([field_fill(58, constant(r#"say "hi""#))]),
            ],
            r#""value""#,
        ),
        (vec![sound().with_in(["alloc\\grp"])], r#""in""#),
        (
            vec![
                FixReplacement::new(version("4.3")).with_fills([FixFill::Group {
                    name: "".into(),
                    members: vec![field_fill(448, FixFillSource::Source)],
                }]),
            ],
            r#""group""#,
        ),
        // A message type is held to what `set_msgtype` holds one to.
        (vec![sound().with_msgtypes([""])], "message-code"),
        // A refusal anywhere in the list refuses the whole list.
        (
            vec![sound(), FixReplacement::new(version("4.4"))],
            "at least one fill",
        ),
    ] {
        let mut field = DataType::utf8().nullable_field("rule80a");
        field.as_fix_mut().set_tag(47).unwrap();
        let error = field.as_fix_mut().set_replacements(&rules).unwrap_err();
        assert!(error.to_string().contains(names), "{names}: {error}");
        assert_eq!(
            field.get_metadata("fix:replacements"),
            None,
            "atomic: {names}"
        );
    }
}

#[test]
fn a_hand_edited_replacement_document_is_refused_at_its_own_byte() {
    // Each case: the stored text, the reason expected, and the text whose
    // first byte the refusal must name.
    for (document, reason, at) in [
        (
            r#"{"replacements":[{"fills":[{"tag":528}],"since":"4.3"}]}"#,
            "out of order",
            Some(r#""since""#),
        ),
        (
            r#"{"replacements":[{"since":"4.3","fills":[{"tag":528}],"note":"x"}]}"#,
            r#"unknown key "note""#,
            Some(r#""note""#),
        ),
        (
            r#"{"replacements":[{"fills":[{"tag":528}]}]}"#,
            r#"state "since""#,
            None,
        ),
        (
            r#"{"replacements":[{"since":"4.3"}]}"#,
            r#"state "fills""#,
            None,
        ),
        (
            r#"{"replacements":[{"since":"4.3","fills":[]}]}"#,
            r#""fills" to hold at least 1"#,
            Some(r#"[]"#),
        ),
        (
            r#"{"replacements":[{"since":"4.3","fills":[{"tag":528,"group":"parties","members":[{"tag":1}]}]}]}"#,
            r#""tag" and "group" never together"#,
            None,
        ),
        (
            r#"{"replacements":[{"since":"4.3","fills":[{"value":"A"}]}]}"#,
            r#"state "tag""#,
            None,
        ),
        (
            r#"{"replacements":[{"since":"4.3","fills":[{"tag":528,"value":"A","from":1}]}]}"#,
            r#""value" and "from" never together"#,
            None,
        ),
        (
            r#"{"replacements":[{"since":"4.3","fills":[{"tag":528,"from":1,"join":[2,3]}]}]}"#,
            r#""from" and "join" never together"#,
            None,
        ),
        (
            r#"{"replacements":[{"since":"4.3","fills":[{"tag":528,"members":[{"tag":1}]}]}]}"#,
            r#""tag" and "members" never together"#,
            None,
        ),
        (
            r#"{"replacements":[{"since":"4.3","fills":[{"value":"A","group":"parties","members":[{"tag":1}]}]}]}"#,
            r#""group" and "value" never together"#,
            None,
        ),
        (
            r#"{"replacements":[{"since":"4.3","fills":[{"group":"parties"}]}]}"#,
            r#"state "members""#,
            None,
        ),
        (
            r#"{"replacements":[{"since":"4.3","fills":[{"group":"parties","members":[]}]}]}"#,
            r#""members" to hold at least 1"#,
            Some(r#"[]"#),
        ),
        (
            r#"{"replacements":[{"since":"4.3","fills":[{"tag":541,"join":[200]}]}]}"#,
            r#""join" to hold at least 2"#,
            Some(r#"[200]"#),
        ),
        (
            r#"{"replacements":[{"since":"4.3","fills":[{"tag":541,"join":[200,4294967295]}]}]}"#,
            r#""join" to fit in 32 bits"#,
            Some("4294967295"),
        ),
        (
            r#"{"replacements":[{"since":"4.3","fills":[{"tag":2147483648}]}]}"#,
            r#""tag" to fit in 32 bits"#,
            Some("2147483648"),
        ),
        (
            r#"{"replacements":[{"since":"4.3","when":"a\"b","fills":[{"tag":528}]}]}"#,
            r#""when" to hold no escape"#,
            Some(r#""a\"b""#),
        ),
        (
            r#"{"replacements":[]}x"#,
            "expected the document to end",
            Some("x"),
        ),
        // A refusal inside a nested member names its byte in the whole
        // document, not in the slice the member walk was reading.
        (
            r#"{"replacements":[{"since":"4.3","fills":[{"group":"parties","members":[{"tag":1,"value":"A","from":2}]}]}]}"#,
            r#""value" and "from" never together"#,
            Some("]}]}]}"),
        ),
    ] {
        let field = replacing(document);
        let error = field
            .as_fix()
            .replacements()
            .next()
            .expect("a refusal")
            .expect_err("a refusal");
        let Error::Parse {
            target, position, ..
        } = &error
        else {
            panic!("{document}: {error}");
        };
        assert_eq!(*target, "fix replacements", "{document}");
        assert!(error.to_string().contains(reason), "{document}: {error}");
        if let Some(at) = at {
            assert_eq!(*position, document.find(at).unwrap(), "{document}: {error}");
        }
        // The infallible walk answers nothing rather than a wrong entry.
        assert!(
            field.as_fix().replacements().next_ok().is_none(),
            "{document}"
        );
        // And taking a document a reader refuses away reports the refusal,
        // having removed it.
        let mut taken = field.clone();
        assert!(
            taken.as_fix_mut().remove_replacements().is_err(),
            "{document}"
        );
        assert_eq!(taken.get_metadata("fix:replacements"), None, "{document}");
    }

    // A refusal is fused: the walk ends where it stopped.
    let field =
        replacing(r#"{"replacements":[{"since":"4.3"},{"since":"4.4","fills":[{"tag":1}]}]}"#);
    let mut walk = field.as_fix().replacements();
    assert!(walk.next().unwrap().is_err());
    assert!(walk.next().is_none());
}

#[test]
fn a_merge_lets_the_incoming_replacements_win_whole() {
    let mut stored = DataType::utf8().nullable_field("rule80a");
    stored.as_fix_mut().set_tag(47).unwrap();
    stored
        .as_fix_mut()
        .set_replacements(&[
            FixReplacement::new(version("4.3")).with_fills([FixFill::Field {
                tag: 528,
                value: FixFillSource::Constant("W".into()),
            }]),
        ])
        .unwrap();
    let stored_text = stored.get_metadata("fix:replacements").unwrap().to_owned();

    // Two documents have no order between them, so the incoming one is not
    // folded entry by entry: it replaces the stored one.
    let mut incoming = rule80a();
    incoming.as_fix_mut().merge_with(&stored.as_fix()).unwrap();
    assert_eq!(
        incoming.get_metadata("fix:replacements"),
        Some(RULE80A_DOCUMENT)
    );

    // The stored one keeps what only it has.
    let mut bare = DataType::utf8().nullable_field("rule80a");
    bare.as_fix_mut().set_tag(47).unwrap();
    bare.as_fix_mut().merge_with(&stored.as_fix()).unwrap();
    assert_eq!(
        bare.get_metadata("fix:replacements"),
        Some(stored_text.as_str())
    );
}

/// Immutable seed fixtures share parsing and compiled plans within this binary.
fn committed() -> Arc<FixRegistry> {
    static REGISTRY: std::sync::OnceLock<Arc<FixRegistry>> = std::sync::OnceLock::new();
    Arc::clone(REGISTRY.get_or_init(|| {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
        Arc::new(FixRegistry::from_handle(&Folder::new(root).unwrap()).unwrap())
    }))
}

#[test]
fn group_entry_names_singularize_published_collections() {
    use super::component::{entry_name, group_name};
    for (collection, entry) in [
        ("Parties", "party"),
        ("NestedParties2", "nestedparty2"),
        ("SecAltIDGrp", "secaltid"),
        ("ContractualMatrices", "contractualmatrix"),
        ("Indices", "index"),
        ("Appendices", "appendix"),
        ("Vertices", "vertex"),
        ("Address", "address"),
        ("Classes", "class"),
        ("Branches", "branch"),
        ("Brushes", "brush"),
        ("Buzzes", "buzz"),
        ("SideTrdRegTS", "sidetrdregts"),
    ] {
        assert_eq!(entry_name(collection).as_str(), entry, "{collection}");
    }
    let mut count = counter("nopartyids", 453);
    count.set_display("NoPartyIDs").unwrap();
    assert_eq!(group_name(&count).as_str(), "parties");
}

#[test]
fn the_catalog_names_every_shipped_group_and_entry_without_field_collisions() {
    let registry = committed();
    let mut groups = HashSet::new();
    let mut entries = HashSet::new();
    for field in registry.definitions(FixCategory::Groups) {
        assert!(groups.insert(field.name()));
        let DataType::List(item) = field.dtype() else {
            panic!("{}", field.dtype());
        };
        assert!(entries.insert(item.name()));
        assert!(!item.is_nullable());
        assert!(matches!(item.dtype(), DataType::Struct(_)));
        let component = registry
            .definition(FixCategory::Components, item.name())
            .unwrap();
        assert_eq!(item.dtype(), component.dtype());
        assert!(registry.get_field_by_name(item.name()).is_none());
        let counter = registry
            .field_by_tag(field.as_fix().counter().unwrap().unwrap())
            .unwrap();
        assert_eq!(counter.dtype(), &DataType::Int32);
    }
    assert_eq!(groups.len(), 580);
    assert_eq!(entries.len(), 580);
    assert_eq!(registry.definitions(FixCategory::Components).count(), 928);
    assert_eq!(registry.msgtypes().count(), 181);
}

#[test]
fn a_group_path_reaches_members_and_skips_its_occurrence_component() {
    let registry = committed();
    let member = registry.field_by_path(&fpath("Parties.PartyID")).unwrap();
    assert_eq!(member.as_fix().tag().unwrap(), Some(448));
    assert_eq!(member.name(), "partyid");
    assert_eq!(registry.field_by_tag(453).unwrap().name(), "nopartyids");
    assert_eq!(
        registry.field_by_tag(453).unwrap().dtype(),
        &DataType::Int32
    );
    for field in registry.definitions(FixCategory::Groups) {
        let DataType::List(item) = field.dtype() else {
            unreachable!()
        };
        for child in item.fields() {
            let path = format!("{}.{}", field.name(), child.name());
            let reached = registry.field_by_path(&fpath(&path)).unwrap();
            assert_eq!(reached, child, "{path}");
        }
    }
    assert!(
        registry
            .get_field_by_path(&fpath("Parties.Party.PartyRole"))
            .is_none()
    );
    assert!(
        registry
            .get_field_by_path(&fpath("Parties.Party"))
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
            .parse_pairs([(tag.to_string().as_bytes(), wire.as_bytes())])
            .expect("the row builds");
        assert_ne!(
            message.by_tag(tag).unwrap(),
            &Scalar::Null,
            "{spelling} reads {wire}",
        );
    }
}

/// One stored document with every `since` and `deprecated` value spelled as
/// `Version` spells it.
///
/// The generator writes the source file's spelling, the crate writes its own,
/// and the two parse to one version. Nothing else in the document is touched.
fn canonical_versions(document: &str) -> String {
    let mut out = String::with_capacity(document.len());
    let mut rest = document;
    let next_key = |rest: &str| {
        [r#""since":""#, r#""deprecated":""#]
            .into_iter()
            .filter_map(|key| rest.find(key).map(|at| (at, key)))
            .min()
    };
    while let Some((at, key)) = next_key(rest) {
        let (head, tail) = rest.split_at(at + key.len());
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
        // An entry stating no type is a deprecation or a removal, a dated
        // point about the field that has no type to state.
        for entry in &held {
            let dtype = entry.parse_dtype().expect("a resolvable type");
            assert!(
                dtype.is_some() || entry.is_deprecated() || entry.is_removed(),
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
    assert_eq!(lineages, 1_603, "fields carrying a lineage");
    // 1,926 before these two phases: 268 entries stated nothing their
    // predecessor did not once types were resolved and the temporal ones
    // adopted backward, and one more was a second statement about one dated
    // point. Two more since: `OrdStatus` and `ExecType` retyped to the crate's
    // `state`, which each lineage records, less the one the generic MsgType
    // datatype's removal collapses back, less the one tag 385's retype to a
    // direction datatype stated before decision 14 typed it as text again.
    assert_eq!(entries, 1_802, "lineage entries");
}

/// Every tag and group one owned fill names is one the dictionary has.
fn assert_fills_resolve(registry: &FixRegistry, fills: &[FixFill], owner: &str) {
    for fill in fills {
        match fill {
            FixFill::Field { tag, value } => {
                assert!(registry.get_field(*tag).is_some(), "{owner} fills {tag}");
                match value {
                    FixFillSource::Source | FixFillSource::Constant(_) => {}
                    FixFillSource::From(from) => {
                        assert!(registry.get_field(*from).is_some(), "{owner} reads {from}");
                    }
                    FixFillSource::Join(tags) => {
                        for tag in tags {
                            assert!(registry.get_field(*tag).is_some(), "{owner} joins {tag}");
                        }
                    }
                }
            }
            FixFill::Group { name, members } => {
                assert!(
                    registry.get_definition(FixCategory::Groups, name).is_some(),
                    "{owner} fills group {name}"
                );
                assert_fills_resolve(registry, members, owner);
            }
        }
    }
}

#[test]
fn every_committed_code_set_is_the_document_the_rust_writer_renders() {
    let registry = committed();
    let mut sets = 0_usize;
    let mut codes = 0_usize;
    for field in every_committed_field(&registry) {
        let Some(stored) = field.as_metadata().get("fix:codes") else {
            continue;
        };
        sets += 1;
        let held: Vec<FixCode> = field
            .as_fix()
            .codes()
            .map(|code| FixCode::from(code.expect("a readable code")))
            .collect();
        assert!(!held.is_empty(), "{} declares a code", field.name());
        codes += held.len();

        // The cross-host assertion, as for the lineage: the generator wrote
        // this set in Python, and the Rust writer must reproduce it byte for
        // byte - key order, sort order, escaping, the legacy codes' dates
        // and aliases - versions canonicalized for the reason the lineage
        // assertion states.
        assert_eq!(
            FixCodes::render(&held).expect("the codes render"),
            canonical_versions(stored),
            "{}",
            field.name()
        );
    }
    assert_eq!(sets, 2_026, "fields carrying a code set");
    assert_eq!(codes, 27_209, "code records");
}

#[test]
fn every_committed_replacement_is_the_document_the_rust_writer_renders() {
    let registry = committed();
    let mut documents = 0_usize;
    for field in every_committed_field(&registry) {
        let Some(stored) = field.as_metadata().get("fix:replacements") else {
            continue;
        };
        documents += 1;
        let held: Vec<FixReplacement> = field
            .as_fix()
            .replacements()
            .map(|entry| FixReplacement::from(entry.expect("a readable entry")))
            .collect();
        assert!(
            !held.is_empty(),
            "{} states at least one rule",
            field.name()
        );

        // The cross-host assertion: the dictionary generator wrote this
        // document in Python, and re-rendering the entries it holds through
        // the Rust writer must reproduce it byte for byte, or the two hosts
        // have forked on key order or spelling. Versions are canonicalized
        // first, for the reason the lineage assertion states.
        assert_eq!(
            FixReplacements::render(&held).expect("the entries render"),
            canonical_versions(stored),
            "{}",
            field.name()
        );

        // Every name a rule reaches for is one the dictionary resolves, so a
        // reader applying it never has to guess.
        for entry in &held {
            for msgtype in entry.msgtypes() {
                assert!(
                    registry.get_msgtype(msgtype).is_some(),
                    "{} applies to message type {msgtype}",
                    field.name()
                );
            }
            for group in entry.in_groups() {
                assert!(
                    registry
                        .get_definition(FixCategory::Groups, group)
                        .is_some(),
                    "{} applies inside {group}",
                    field.name()
                );
            }
            assert_fills_resolve(&registry, entry.fills(), field.name());
        }
    }
    // The generator writes these; a dictionary that carries none yet is a
    // dictionary with nothing to disagree about, so the count is reported
    // rather than pinned.
    eprintln!("{documents} committed fields carry fix:replacements");
}

#[test]
fn an_entry_is_a_range_of_its_line_and_the_registry_names_its_field() {
    // The measured footprint, so the report states the tree's number rather
    // than an estimate: a tag, two counted ranges of the one page the line was
    // read into, and the children vector.
    //
    // The shape this replaced, laid out by the same rules, is declared beside
    // it so the saving is measured rather than reasoned about: two owned
    // copies of bytes the line already held, and a per-pair copy of a branch
    // digest that no pair carries any more, the field a tag names being the
    // registry's to answer.
    struct Was {
        _tag: i32,
        _branch: i32,
        _key: SmolStr,
        _value: SmolStr,
        _children: Vec<Was>,
    }
    assert_eq!(std::mem::size_of::<Was>(), 80, "the copying entry");
    assert_eq!(
        std::mem::size_of::<FixEntry>(),
        64,
        "one entry, as this tree lays it out",
    );

    let registry = Arc::new(
        FixRegistry::from_fields([tagged("Symbol", 55), member("VenueSym", "cme", 5_055)]).unwrap(),
    );
    assert_eq!(registry.dialects(), ["cme"]);

    let codec = super::FixCodec::new(Arc::clone(&registry));
    let line = b"55=AAPL|5055=XYZ|VenueOwnThing=?|";
    let msg = codec.parse_fix_line(line).expect("a readable frame");
    let entries = msg.entries();
    assert!(!entries.is_empty());

    // A message root the codec builds carries no membership: a message is
    // not a dictionary member, whatever dictionaries spoke its fields.
    assert_eq!(msg.as_field().as_fix().branches().count(), 0);
    assert!(
        registry
            .field_by_tag(5_055)
            .unwrap()
            .as_fix()
            .has_branch("cme")
    );
    // An entry keeps the tag and nothing else of the identity: the field
    // that tag names is the registry's to answer, and the identity is
    // assembled from the two owners rather than copied onto every pair.
    let venue = entries.iter().find(|held| held.tag() == 5_055).unwrap();
    let named = registry.field_by_tag(venue.tag()).unwrap();
    assert_eq!(
        id_of(venue.tag(), named.name()),
        id_of(5_055, "VenueSym"),
        "an entry names its field with the tag it kept and the name its registry holds",
    );
    assert_eq!(
        msg.by_id(id_of(5_055, "VenueSym")).unwrap(),
        &Scalar::from("XYZ")
    );
    assert_eq!(
        msg.by_id(id_of(55, "Symbol")).unwrap(),
        &Scalar::from("AAPL")
    );
    // A key that named no field names no identity, whatever the message
    // resolved in.
    assert!(entries.iter().any(|held| held.tag() == 0));

    // Every key and value is a range of the line, never a copy of it: the
    // bytes are the same bytes, at the offsets the line wrote them.
    for entry in entries {
        for held in [entry.key(), entry.value()] {
            let at = held.start() as usize;
            assert_eq!(
                &line[at..held.end() as usize],
                held.as_bytes(),
                "an entry names the line at its own offsets",
            );
        }
    }
}

#[test]
fn the_entry_column_holds_the_pair_and_what_arrived_under_it() {
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
        // the same four members - what the line said and what FIX added, and
        // nothing the message already answers - the fourth a non-null
        // nofixentries that is a deeper list twice and the binary leaf at the
        // bottom.
        let mut held = item;
        for level in 1..=3 {
            assert_eq!(held.name(), "fixentry", "{column} level {level}");
            assert!(!held.is_nullable(), "{column} level {level}");
            let members = held.dtype().as_fields().expect("an occurrence struct");
            let names: Vec<&str> = members.iter().map(Field::name).collect();
            assert_eq!(
                names,
                ["tag", "key", "value", "nofixentries"],
                "{column} level {level}",
            );
            assert_eq!(
                members[0].dtype(),
                &DataType::Int32,
                "{column} level {level}"
            );
            let tail = &members[3];
            assert!(!tail.is_nullable(), "{column} level {level} tail");
            match tail.dtype() {
                DataType::List(deeper) if level < 3 => held = deeper,
                DataType::Bytes(bytes) if level == 3 && *bytes == BytesParameters::default() => {
                    break;
                }
                other => panic!("{column} level {level}: {other}"),
            }
        }
    }
}

/// One entry a fixture states by hand, its key and value copied into a page of
/// their own - which is what a message nothing read from a line has to do.
fn entry(tag: i32, key: &str, value: &str) -> FixEntry {
    FixEntry::new(
        tag,
        crate::media::text::TextBytes::from_bytes(key).expect("a key"),
        crate::media::text::TextBytes::from_bytes(value).expect("a value"),
    )
}

/// A chain of entries `depth` long, each child the sole passenger of the one
/// above, ending in a leaf pair carrying `value`.
fn nested_entries(depth: usize, value: &str) -> Vec<FixEntry> {
    let mut held = entry(523, "523", value);
    for level in (1..depth).rev() {
        let mut parent = entry(453, "453", &level.to_string());
        parent.push(held);
        held = parent;
    }
    vec![entry(35, "35", "D"), held]
}

#[test]
fn a_deep_arrival_materializes_three_levels_and_folds_the_rest() {
    let registry = committed();
    let root = DataType::from_fields([DataType::utf8().nullable_field("35")])
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
    let row = deep.into_row(&schema).unwrap();
    let columns = row.as_sequence().expect("a row").to_vec();
    let entries = columns[columns.len() - 2]
        .as_sequence()
        .expect("the arrival column");
    let level1 = entries[1].as_sequence().expect("the counter entry");
    let level2 = level1[3].as_sequence().expect("one child list")[0]
        .as_sequence()
        .expect("the level-2 entry")
        .to_vec();
    let level3 = level2[3].as_sequence().expect("one child list")[0]
        .as_sequence()
        .expect("the level-3 entry")
        .to_vec();
    let leaf = level3[3].as_bytes().expect("the binary leaf");
    assert!(!leaf.is_empty(), "two levels folded into it");

    // The leaf recovers exactly the folded entries through the one JSON
    // parser this crate has: level 4 carrying level 5.
    let decoded = crate::from_json_scalar(leaf).expect("a decodable leaf");
    let folded = decoded.as_sequence().expect("the folded children");
    assert_eq!(folded.len(), 1);
    let level4 = folded[0].as_sequence().expect("the level-4 entry").to_vec();
    assert_eq!(level4[0].as_i64(), Some(453));
    let level5 = level4[3].as_sequence().expect("its children")[0]
        .as_sequence()
        .expect("the level-5 entry")
        .to_vec();
    assert_eq!(level5[0].as_i64(), Some(523));
    assert_eq!(level5[2].as_str(), Some("x"));

    // A flat sibling's child list is empty: nothing arrived under it and
    // nothing was folded for it.
    let flat = entries[0].as_sequence().expect("the msgtype entry");
    assert_eq!(flat[3].as_sequence().map(<[Scalar]>::len), Some(0));

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
            entry(35, "35", "D"),
            entry(453, "453", "1"),
            entry(523, "523", "x"),
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
    let row = towering.into_row(&schema).expect("no depth refusal");
    assert!(row.as_sequence().is_some());
}
