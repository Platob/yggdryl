//! `rust/src/fix/mod.rs`: focused edge cases of the FIX module, driven with
//! explicit inputs.
//!
//! The module's own vocabulary - an identity, a key, the categories - and the
//! doors over it are a caller's throughout, so almost everything here reaches
//! the crate through `yggdryl::`. The handful of steps inside one that no
//! caller can name - the default's resolution from explicit inputs, the shard
//! arithmetic, the identifier spread, the retirement table, the compiled
//! derivations, the stored-document rewrite - are reached through
//! `yggdryl::internals`.

use super::SoleMessage;
use super::committed_registry;
use super::decimal;
use super::fixed_codec;
use super::format_target;
use super::sole_message;
use super::tag_index;
use yggdryl::{FixField, FixFieldMut};

/// `FixKey::from_text` reads a typed key the way the dictionary's own tag
/// reader reads a tag, and anything it does not read is a name, so a signed
/// or zero number reaches no field by tag and a leading zero is kept.
#[test]
fn a_typed_key_is_a_tag_by_the_dictionary_tag_reader_and_a_name_otherwise() {
    use yggdryl::FixKey;

    for (text, tag) in [("35", 35), ("035", 35), ("1", 1), ("2147483647", i32::MAX)] {
        assert_eq!(FixKey::from_text(text), FixKey::Tag(tag), "{text:?}");
    }
    for text in [
        "+35",
        "-1",
        "0",
        "00",
        " 35",
        "35 ",
        "3_5",
        "2147483648",
        "",
        "MsgType",
        "Symbol.absent",
        "35:MsgType",
        "\u{661}\u{662}",
    ] {
        assert_eq!(FixKey::from_text(text), FixKey::Name(text), "{text:?}");
    }
}

/// A field-level merge refuses a `FIX:sources` text the setter never writes
/// (a text the read walks as nothing would merge as nothing and drop what
/// the other side said), naming the key, whichever side states it, and the
/// retired `FIX:branches` key by its own name; the held field stands.
#[test]
fn merge_with_refuses_a_sources_text_the_setter_never_writes_on_either_side() {
    use yggdryl::{DataType, Error, Field};

    fn tagged(name: &str, tag: i32) -> Field {
        let mut field = DataType::utf8().nullable_field(name);
        FixFieldMut::new(&mut field).set_tag(tag).unwrap();
        field
    }

    let mut sound = tagged("TradeID", 5001);
    FixFieldMut::new(&mut sound).set_sources(["venue"]).unwrap();
    for (key, stored, expected) in [
        ("FIX:sources", "venue", "a JSON array of source ids"),
        ("FIX:sources", r#"["b","a"]"#, "the source ids sorted"),
        ("FIX:sources", r#"["ve_nue","venue"]"#, "each source once"),
        ("FIX:branches", "venue", "retired"),
    ] {
        let mut edited = tagged("TradeID", 5001);
        edited.insert_metadata(key, stored).unwrap();
        let mut held = sound.clone();
        let error = FixFieldMut::new(&mut held)
            .merge_with(&FixField::new(&edited))
            .unwrap_err();
        assert!(
            matches!(&error, Error::InvalidMetadataValue { key: named, .. } if named == key),
            "{key} {stored}: {error}"
        );
        assert!(
            error.to_string().contains(expected),
            "{key} {stored}: {error}"
        );
        assert_eq!(held, sound, "{key} {stored}");
        let mut held = edited.clone();
        let error = FixFieldMut::new(&mut held)
            .merge_with(&FixField::new(&sound))
            .unwrap_err();
        assert!(
            matches!(&error, Error::InvalidMetadataValue { key: named, .. } if named == key),
            "{key} {stored}: {error}"
        );
        assert_eq!(held, edited, "{key} {stored}");
    }
}

#[cfg(feature = "internals")]
mod internal {
    use std::collections::HashSet;
    use std::path::PathBuf;
    use std::sync::Arc;

    use yggdryl::fix::FixCodes;
    use yggdryl::internals::fix_catalog::{add_definition, derived_definition_tag, group_by_tag};
    use yggdryl::internals::fix_codes::{create_codeset, render as render_codes};
    use yggdryl::internals::fix_global::autoload;
    use yggdryl::internals::fix_registry::control_byte;
    use yggdryl::internals::fix_store::shard_of;
    use yggdryl::internals::hashing_stable::stable_hash_of;
    use yggdryl::local::LocalFolder;

    use yggdryl::{
        DataType, Error, Field, FixCategory, FixCode, FixCodec, FixEntry, FixField, FixFieldMut,
        FixId, FixKey, FixMsg, FixRegistry, MimeType, Scalar, StructType, Version,
    };

    /// One path, resolved once, as every FIX navigator now takes it.
    /// One exact number, spelled the way a wire spells it.
    ///
    /// Every FIX quantity, price, price offset and amount is
    /// `decimal128(38, 18)`, so a pin states the number in text and never as a
    /// float: `82.5` is a value a `f64` holds approximately and a decimal holds
    /// exactly.
    fn decimal(text: &str) -> Scalar {
        Scalar::from(yggdryl::Decimal::parse(text).expect("an exact number"))
    }

    fn fpath(spelling: &str) -> yggdryl::FieldPath {
        yggdryl::FieldPath::from_str(spelling).unwrap_or_else(|error| panic!("{spelling}: {error}"))
    }

    /// A nullable text field carrying one canonical tag.
    fn tagged(name: &str, tag: i32) -> Field {
        let mut field = DataType::utf8().nullable_field(name);
        FixFieldMut::new(&mut field).set_tag(tag).unwrap();
        field
    }

    /// A nullable text field one venue dictionary contributed: a tag, and the
    /// membership that says whose it is.
    fn member(name: &str, dialect: &str, tag: i32) -> Field {
        let mut field = tagged(name, tag);
        FixFieldMut::new(&mut field).set_sources([dialect]).unwrap();
        field
    }

    /// The identity a tag and a name make, for a fixture that states both.
    fn id_of(tag: i32, name: &str) -> FixId {
        FixId::of(tag, name).unwrap()
    }

    /// A field carrying every `FIX:` property.
    fn full(name: &str, tag: i32, tags: &[i32], aliases: &[&str]) -> Field {
        let mut field = tagged(name, tag);
        FixFieldMut::new(&mut field).set_tags(tags).unwrap();
        FixFieldMut::new(&mut field).set_names(aliases).unwrap();
        FixFieldMut::new(&mut field)
            .set_description(format!("{name} described"))
            .unwrap();
        field
    }

    fn counter(name: &str, tag: i32) -> Field {
        let mut field = DataType::Int32.nullable_field(name);
        FixFieldMut::new(&mut field).set_tag(tag).unwrap();
        field
    }

    fn named_group(name: &str, tag: i32) -> Field {
        let item = StructType::from_fields([tagged("Member", 9_002)])
            .map(DataType::from)
            .unwrap()
            .required_field("MemberComponent");
        let mut field = DataType::serie(item).nullable_field(name);
        FixFieldMut::new(&mut field).set_counter(tag).unwrap();
        field
    }

    /// A fresh directory of this test's own under the platform temporary root.
    fn scratch(label: &str) -> PathBuf {
        let path = LocalFolder::temporary()
            .unwrap()
            .path()
            .unwrap()
            .join(format!("yggdryl-fix-unit-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        path
    }

    /// Scalar definitions present before any test insertion, including standard clocks.
    fn seeded_fields() -> usize {
        static COUNT: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
        *COUNT.get_or_init(|| FixRegistry::new().len())
    }

    /// Every message the catalog defines: a component carrying a `FIX:msgtype`.
    fn msgtypes(registry: &FixRegistry) -> usize {
        registry
            .definitions(FixCategory::Components)
            .filter(|field| FixField::new(field).msgtype().is_some())
            .count()
    }

    /// The named components this crate defines beside the dictionary's own.
    ///
    /// A definition is filed by the shape it has, so every crate column shaped as
    /// a Struct is a component.
    fn crated_components() -> usize {
        yggdryl::fix_crate_fields()
            .expect("the crate's own fields")
            .iter()
            .filter(|field| matches!(field.dtype(), DataType::Struct(_)))
            .count()
    }

    /// The crate's own scalars, or its own groups - the Maps - in the order
    /// every registry iterates them. `srcuuids` is a Serie of non-null
    /// scalars, which is one column rather than a group, so it is filed among
    /// the fields and walks with them.
    fn crate_names_of(groups: bool) -> Vec<&'static str> {
        yggdryl::fix_crate_fields()
            .unwrap()
            .iter()
            // A derived column is a FIX field's own statement under another
            // name, which no registry holds.
            .filter(|field| {
                !FixField::new(field)
                    .tag()
                    .ok()
                    .flatten()
                    .is_some_and(yggdryl::is_derived_tag)
            })
            .filter(|field| {
                if groups {
                    matches!(field.dtype(), DataType::Map(_) | DataType::SortedMap(_))
                } else {
                    !matches!(
                        field.dtype(),
                        DataType::Map(_) | DataType::SortedMap(_) | DataType::Struct(_)
                    )
                }
            })
            .map(Field::name)
            .collect()
    }

    /// The crate's own names in the order every registry iterates them: the
    /// scalars last among the scalars, because their tags are above every tag a
    /// test claims, and the definitions after every scalar.
    fn crate_names() -> Vec<&'static str> {
        let mut names = crate_names_of(false);
        names.extend(crate_names_of(true));
        names
    }

    /// `names`, then the crate's own: the order a registry holding `names`
    /// iterates.
    fn then_crated<'a>(names: &[&'a str]) -> Vec<&'a str> {
        names.iter().copied().chain(crate_names()).collect()
    }

    /// `names`, then the crate's own scalars: the order the cursor, which walks
    /// the scalars alone, answers for a registry holding `names`.
    fn then_crated_scalars<'a>(names: &[&'a str]) -> Vec<&'a str> {
        names.iter().copied().chain(crate_names_of(false)).collect()
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
        assert!(FixField::new(&vendor).has_source("cme"));
        assert!(!FixField::new(&standard).has_source("cme"));
        assert!(registry.get_field_by_name("cme").is_none());
        assert_eq!(registry.dialects(), ["cme"]);
    }

    #[test]
    fn three_spellings_of_one_name_under_one_tag_are_one_identity() {
        let registry = FixRegistry::from_fields([tagged("MsgType", 35)]).unwrap();
        let held = FixField::new(registry.field_by_tag(35).unwrap())
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
                // A JSON document is JSON, which is what it is, and it declares
                // no message type: the codec reads none and names the row
                // `unknown`, whatever the document says about itself.
                br#"{"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:name=ULMSG_BROKER_TO_DMZ,plugin-type=FIX,type=Plugin","type":"read"},"value":{"Category":"InterBridge"},"status":200}"#,
                MimeType::JSON,
                None,
            ),
            (
                // A wildcard read is the same document, and names the same nothing.
                br#"{"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:*","type":"read"},"value":{"com.ullink.ulbridge.sessioninterfaces.plugins:name=X,plugin-type=FIX,type=ConfigurationPlugin":{"Name":"X"}},"status":200}"#,
                MimeType::JSON,
                None,
            ),
            (
                // A request alone is a document too.
                br#"{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:*","type":"read"}"#,
                MimeType::JSON,
                None,
            ),
            (
                // JSON either way: no namespace separates one document from
                // another, and none names a message type.
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
    fn the_prose_in_front_of_a_json_document_names_its_half_and_the_document_nothing() {
        let reading = FixRegistry::new().msgdirection();

        const ANSWERED: &[u8] = br#"{"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:*","type":"read"},"value":{"com.ullink.ulbridge.sessioninterfaces.plugins:name=X,plugin-type=FIX,type=Plugin":{"ExtendedActions":[{"name":"send-test-request","description":"Send a test request message.","parameters":[{"name":"test-request-id","description":"The outgoing test request ID to send"}]}]}},"status":200}"#;
        const ASKED: &[u8] =
            br#"{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:*","type":"read"}"#;

        // A document states nothing of which way it moved: the
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
        // A document classifies as the JSON it is and names no message type:
        // whatever it says about itself is its own payload.
        assert_eq!(MimeType::infer_bytes(write), MimeType::JSON);
        assert_eq!(FixCodec::infer_msgtype_bytes(write), None);

        // A bulk read answers an array of these, and an array closes with `]`.
        let bulk = br#"[{"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:name=A,type=Plugin","type":"read"},"status":200},{"request":{"mbean":"com.ullink.ulbridge:type=Bridge","type":"read"},"status":200}]"#;
        assert_eq!(MimeType::infer_bytes(bulk), MimeType::JSON);
        assert_eq!(FixCodec::infer_msgtype_bytes(bulk), None);
        assert_eq!(reading.read_bytes(bulk), None);

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
        assert_eq!(MimeType::infer_bytes(&stamped), MimeType::JSON);
        assert_eq!(FixCodec::infer_msgtype_bytes(&stamped), None);

        // The prose's reading is filled as tag 385 on the line door, which takes
        // no pin; a bare document fills nothing there. A document states no
        // message type, so the codec is told to read the untyped row the
        // default refuses: the subject here is the direction, not the filter.
        let codec = FixCodec::new(Arc::new(FixRegistry::new()))
            .with_exclude_msgtypes::<[&str; 0], &str>([]);
        let message = codec
            .parse_line(&answered)
            .unwrap()
            .next()
            .unwrap()
            .unwrap();
        assert_eq!(message.by_tag(385).unwrap().as_str(), Some("R"));
        let message = codec.parse_line(ANSWERED).unwrap().next().unwrap().unwrap();
        assert_eq!(message.get_by_tag(385), None);

        // A class the bridge spells is not an MBean it names: every `$type`,
        // `className` and init file in these documents carries one, and a record
        // quoting one is an ordinary JSON record.
        let quoted =
            br#"{"level":"INFO","message":"reloading com.ullink.ulbridge2.plugins.ULMsg"}"#;
        assert_eq!(MimeType::infer_bytes(quoted), MimeType::JSON);
        assert_eq!(reading.read_bytes(quoted), None);
        // The prose in front of a document is prose, and the document behind it
        // is still the JSON it is.
        let prose =
            br#"reloading com.ullink.ulbridge.sessioninterfaces.plugins:* {"level":"INFO"}"#;
        assert_eq!(MimeType::infer_bytes(prose), MimeType::JSON);
        // An unterminated document is not one.
        let truncated =
            br#"{"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:*","#;
        assert_eq!(MimeType::infer_bytes(truncated), MimeType::OCTET_STREAM);
    }

    #[test]
    fn membership_folds_once_sorts_and_refuses_what_it_cannot_hold() {
        let mut field = tagged("TradeID", 5001);
        // Folded by ASCII case, deduplicated under the fold, and sorted, so two
        // registries built from the same dictionaries in any order hash alike.
        FixFieldMut::new(&mut field)
            .set_sources(["Globex", "CME", "cme", "globex"])
            .unwrap();
        assert_eq!(
            field.get_metadata("FIX:sources"),
            Some(r#"["cme","globex"]"#)
        );
        assert_eq!(
            FixField::new(&field).sources().collect::<Vec<_>>(),
            ["cme", "globex"]
        );
        assert!(
            FixField::new(&field).has_source("CME") && FixField::new(&field).has_source("globex")
        );
        assert!(!FixField::new(&field).has_source("cm"));

        // Adding is idempotent under the fold, and keeps the list sorted.
        FixFieldMut::new(&mut field).add_source("GLOBEX").unwrap();
        assert_eq!(
            field.get_metadata("FIX:sources"),
            Some(r#"["cme","globex"]"#)
        );
        FixFieldMut::new(&mut field).add_source("Blp").unwrap();
        assert_eq!(
            field.get_metadata("FIX:sources"),
            Some(r#"["blp","cme","globex"]"#)
        );

        // Held to the id grammar: a non-empty word holding no quote, no
        // backslash and no control character. A refusal leaves the field
        // exactly as it was.
        let before = field.clone();
        for refused in [
            vec!["cme", ""],
            vec!["cm\"e"],
            vec!["cm\\e"],
            vec!["cm\u{1}e"],
        ] {
            let error = FixFieldMut::new(&mut field)
                .set_sources(refused.clone())
                .unwrap_err();
            assert!(
                matches!(&error, Error::InvalidMetadataValue { key, .. } if key == "FIX:sources"),
                "{refused:?}: {error}"
            );
            assert_eq!(field, before, "{refused:?}");
        }
        assert!(FixFieldMut::new(&mut field).add_source("").is_err());
        assert_eq!(field, before);

        // A comma is an ordinary character of an id: the array is what
        // separates the ids, so nothing an id holds has to be kept out.
        FixFieldMut::new(&mut field)
            .add_source("ms,bloomberg")
            .unwrap();
        assert_eq!(
            field.get_metadata("FIX:sources"),
            Some(r#"["blp","cme","globex","ms,bloomberg"]"#)
        );
        assert!(FixField::new(&field).has_source("MS,Bloomberg"));

        // Empty input removes the property rather than storing "".
        FixFieldMut::new(&mut field)
            .set_sources::<[&str; 0], &str>([])
            .unwrap();
        assert!(!field.has_metadata("FIX:sources"));
        assert_eq!(FixField::new(&field).sources().count(), 0);
    }

    #[test]
    fn merge_with_folds_the_fields_and_the_dialects_beside_them() {
        let mut dictionary =
            FixRegistry::from_fields([tagged("symbol", 55), member("VenueSym", "cme", 5_055)])
                .unwrap();

        let other = FixRegistry::from_fields([
            member("SYMBOL", "globex", 55),
            member("VenueTime", "globex", 5_060),
        ])
        .unwrap();

        // The other dictionary holds the crate's own fields as every registry
        // does, and they are neither added nor merged: a fold never counts them.
        // Standard SendingTime and TransactTime are ordinary definitions and do merge.
        let yggdryl::FixMerge { added, merged, .. } = dictionary.merge_with(&other).unwrap();
        assert_eq!((added, merged), (1, 3));
        assert_eq!(dictionary.len(), 3 + seeded_fields());
        // A folded input name retains the canonical identity's stored spelling.
        assert_eq!(dictionary.field_by_tag(55).unwrap().name(), "symbol");

        // The dialects arrive beside the fields: membership unions onto the
        // stored field, and a field only one side held keeps saying whose it is.
        assert_eq!(
            FixField::new(dictionary.field_by_tag(55).unwrap())
                .sources()
                .collect::<Vec<_>>(),
            ["globex"]
        );
        assert!(FixField::new(dictionary.field_by_tag(5_055).unwrap()).has_source("cme"));
        assert!(FixField::new(dictionary.field_by_tag(5_060).unwrap()).has_source("globex"));
        assert_eq!(dictionary.dialects(), ["cme", "globex"]);

        // A declaration the dictionary already makes otherwise is passed over
        // and named; one it makes at another precision folds under the held
        // one; the rest of the source folds, its membership with it. Text is
        // the coarsest statement of any field, so the contradiction is a flag
        // stated as a count.
        let mut flag = tagged("PossDupFlag", 43);
        flag.set_dtype(DataType::Boolean).unwrap();
        dictionary.insert(flag).unwrap();
        let before = dictionary.clone();
        let mut disagreeing = FixRegistry::from_fields([tagged("symbol", 55)]).unwrap();
        disagreeing
            .insert({
                let mut widened = tagged("SYMBOL", 55);
                widened.set_dtype(DataType::large_utf8()).unwrap();
                widened
            })
            .unwrap();
        disagreeing
            .insert({
                let mut counted = tagged("possdupflag", 43);
                counted.set_dtype(DataType::Int32).unwrap();
                counted
            })
            .unwrap();
        disagreeing.insert(member("BlpSym", "blp", 5_070)).unwrap();
        let merge = dictionary.merge_with(&disagreeing).unwrap();
        assert_eq!(merge.restated, 1);
        assert_eq!(
            dictionary.field_by_tag(55).unwrap().dtype(),
            &DataType::utf8()
        );
        assert_eq!(merge.dropped.len(), 1, "{:?}", merge.dropped);
        assert_eq!(merge.dropped[0].incoming.dtype(), &DataType::Int32);
        assert_eq!(
            dictionary.field_by_tag(43).unwrap(),
            before.field_by_tag(43).unwrap()
        );
        assert!(FixField::new(dictionary.field_by_tag(5_070).unwrap()).has_source("blp"));
        assert_eq!(dictionary.dialects(), ["blp", "cme", "globex"]);
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
            id_of(FixEntry::new(5001, "5001", None).tag(), "VenueSym"),
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
                    if key == "FIX:tag" && reason.contains("-1")),
                "{name:?}: {error}"
            );
        }
        assert!(FixId::of(0, "").is_err());
        assert!(FixId::of(i32::MAX, "MsgType").is_ok());
    }

    #[test]
    fn the_identifier_finalizer_spreads_the_committed_dictionary_control_bytes() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("config")
            .join("fix");
        let folder = LocalFolder::new(root).unwrap();
        let registry = FixRegistry::from_handle(&folder).unwrap();
        let classes: HashSet<u8> = registry
            .iter()
            .map(|field| control_byte(FixField::new(field).id().unwrap().unwrap()))
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
        let view = FixField::new(&field);
        assert_eq!(view.tag().unwrap(), None);
        assert_eq!(view.tags().unwrap(), Vec::<i32>::new());
        assert_eq!(view.names().count(), 0);
        assert_eq!(view.description(), None);

        FixFieldMut::new(&mut field).set_tag(38).unwrap();
        FixFieldMut::new(&mut field).set_tags(&[152]).unwrap();
        FixFieldMut::new(&mut field).set_names(["Qty"]).unwrap();
        FixFieldMut::new(&mut field)
            .set_description("Quantity ordered.")
            .unwrap();
        assert_eq!(FixField::new(&field).tag().unwrap(), Some(38));
        assert_eq!(FixField::new(&field).tags().unwrap(), [152]);
        assert_eq!(FixField::new(&field).names().collect::<Vec<_>>(), ["Qty"]);
        assert_eq!(
            FixField::new(&field).description(),
            Some("Quantity ordered.")
        );
        // Each list is the compact JSON array it is.
        assert_eq!(field.get_metadata("FIX:tag"), Some("38"));
        assert_eq!(field.get_metadata("FIX:tags"), Some("[152]"));
        assert_eq!(field.get_metadata("FIX:names"), Some("[\"Qty\"]"));

        // Order is priority and is kept.
        FixFieldMut::new(&mut field).set_tags(&[3, 1, 2]).unwrap();
        FixFieldMut::new(&mut field)
            .set_names(["Quantity", "Qty", "OrderQuantity"])
            .unwrap();
        assert_eq!(FixField::new(&field).tags().unwrap(), [3, 1, 2]);
        assert_eq!(field.get_metadata("FIX:tags"), Some("[3,1,2]"));
        assert_eq!(
            FixField::new(&field).names().collect::<Vec<_>>(),
            ["Quantity", "Qty", "OrderQuantity"]
        );
        assert_eq!(
            field.get_metadata("FIX:names"),
            Some("[\"Quantity\",\"Qty\",\"OrderQuantity\"]")
        );

        // An empty list removes the property rather than storing "[]".
        FixFieldMut::new(&mut field).set_tags(&[]).unwrap();
        FixFieldMut::new(&mut field)
            .set_names(Vec::<&str>::new())
            .unwrap();
        assert!(!field.has_metadata("FIX:tags"));
        assert!(!field.has_metadata("FIX:names"));
        assert_eq!(FixField::new(&field).tags().unwrap(), Vec::<i32>::new());
        assert_eq!(FixField::new(&field).names().count(), 0);

        // The value outlives the view it was read through.
        let description = FixField::new(&field).description();
        assert_eq!(description, Some("Quantity ordered."));
    }

    #[test]
    fn a_property_write_rejects_bad_elements_and_leaves_the_field_unchanged() {
        let mut field = tagged("Symbol", 55);
        FixFieldMut::new(&mut field).set_names(["Ticker"]).unwrap();
        let before = field.clone();

        let refusals = [
            FixFieldMut::new(&mut field).set_tag(-1).unwrap_err(),
            FixFieldMut::new(&mut field).set_tags(&[1, -2]).unwrap_err(),
            FixFieldMut::new(&mut field)
                .set_tags(&[1, 2, 1])
                .unwrap_err(),
            FixFieldMut::new(&mut field)
                .set_names(["Sym", ""])
                .unwrap_err(),
            FixFieldMut::new(&mut field)
                .set_names(["Sym\"bol"])
                .unwrap_err(),
            FixFieldMut::new(&mut field)
                .set_names(["Sym\\bol"])
                .unwrap_err(),
            FixFieldMut::new(&mut field)
                .set_names(["Sym\tbol"])
                .unwrap_err(),
            FixFieldMut::new(&mut field)
                .set_names(["Sym", "SYM"])
                .unwrap_err(),
        ];
        for (index, error) in refusals.iter().enumerate() {
            assert!(
                matches!(error, Error::InvalidMetadataValue { key, .. } if key.starts_with("FIX:")),
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
        assert_eq!(FixField::new(&field).sources().count(), 0);
        assert_eq!(FixField::new(&field).id().unwrap(), None);
        assert!(!field.has_metadata("FIX:sources"));

        FixFieldMut::new(&mut field).set_sources(["CME"]).unwrap();
        assert_eq!(FixField::new(&field).sources().collect::<Vec<_>>(), ["cme"]);
        assert_eq!(field.get_metadata("FIX:sources"), Some(r#"["cme"]"#));
        assert_eq!(FixField::new(&field).id().unwrap(), None, "still no tag");

        FixFieldMut::new(&mut field).set_tag(5001).unwrap();
        let id = FixField::new(&field).id().unwrap().unwrap();
        assert_eq!(id, id_of(5001, "TradeID"));
        assert_eq!(id.to_string(), id.digest().to_string());

        // Membership is provenance and never identity: the same tag under the
        // same name is one field whatever dictionaries spoke it, and taking the
        // membership away moves nothing.
        assert_eq!(
            FixField::new(&tagged("TradeID", 5001)).id().unwrap(),
            Some(id)
        );
        assert_eq!(
            FixField::new(&member("TradeID", "xnas", 5001))
                .id()
                .unwrap(),
            Some(id)
        );
        FixFieldMut::new(&mut field)
            .set_sources::<[&str; 0], &str>([])
            .unwrap();
        assert!(!field.has_metadata("FIX:sources"));
        assert_eq!(FixField::new(&field).id().unwrap(), Some(id));

        // The name is the other half: a rename is a new identity, derived on
        // every read rather than stored stale.
        field.set_name("Trade_ID");
        assert_eq!(FixField::new(&field).id().unwrap(), Some(id), "the fold");
        field.set_name("TradeReportID");
        assert_ne!(FixField::new(&field).id().unwrap(), Some(id));
        assert_eq!(
            FixField::new(&field).id().unwrap(),
            Some(id_of(5001, "TradeReportID"))
        );
    }

    /// Registering both cases of one letter registers two messages.
    ///
    /// An unnamed code is named after its wire value, and a name is folded
    /// where a wire value is not - so deriving a definition's name from the
    /// fold of `B` produced `b`, which is the spelling the *other* FIX
    /// message answers to, and the next registration found that entry and
    /// added nothing.
    #[test]
    fn registering_both_cases_of_one_message_code_registers_two_messages() {
        let mut registry =
            FixRegistry::from_fields([tagged("msgtype", yggdryl::fix::MSGTYPE_TAG_NAME.0)])
                .unwrap();
        for code in ["B", "b", "C", "c", "S", "s"] {
            assert_eq!(
                registry
                    .register_msgtype(code, None, None)
                    .unwrap()
                    .as_str(),
                code
            );
        }
        let mut names = Vec::new();
        for code in ["B", "b", "C", "c", "S", "s"] {
            let held = registry.msgtype(code).unwrap();
            assert_eq!(held.as_str(), code, "{code:?}");
            names.push(held.name().to_owned());
        }
        // Six codes are six messages, each under a name of its own.
        names.sort();
        names.dedup();
        assert_eq!(names.len(), 6, "{names:?}");
        // And a named registration still names the message it reaches.
        let named = registry
            .register_msgtype("b", Some("MassQuoteAcknowledgement"), None)
            .unwrap();
        assert_eq!(named.as_str(), "b");
        assert!(std::ptr::eq(
            registry.msgtype("MassQuoteAcknowledgement").unwrap(),
            registry.msgtype("b").unwrap(),
        ));
        assert_eq!(registry.msgtype("B").unwrap().as_str(), "B");
    }

    #[test]
    fn registering_a_message_type_names_it_describes_it_and_never_rewrites_it() {
        let mut registry =
            FixRegistry::from_fields([tagged("msgtype", yggdryl::fix::MSGTYPE_TAG_NAME.0)])
                .unwrap();
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
        // The vocabulary tag 35 reads by is the dictionary's, so a registration
        // states its members there and the field only names the set.
        let code = |registry: &FixRegistry| {
            registry
                .codeset_of(
                    registry
                        .field_by_tag(yggdryl::fix::MSGTYPE_TAG_NAME.0)
                        .unwrap(),
                )
                .expect("the set tag 35 reads by")
                .codes()
                .map(Result::unwrap)
                .find(|code| code.name() == "AllocationReportAck")
                .map(yggdryl::FixCode::from)
                .unwrap()
        };
        let initial = code(&registry);
        assert_eq!(initial.value(), "P Report Ack");
        assert_eq!(initial.description(), Some("Allocation Report ACK"));
        registry
            .register_msgtype("P Report Ack", Some("allocationreportack"), Some("Other"))
            .unwrap();
        assert_eq!(code(&registry), initial);
        assert_eq!(msgtypes(&registry), 2);

        registry
            .register_msgtype("D", None, Some("Order - Single"))
            .unwrap();
        let set = registry
            .codeset_of(
                registry
                    .field_by_tag(yggdryl::fix::MSGTYPE_TAG_NAME.0)
                    .unwrap(),
            )
            .expect("the set tag 35 reads by");
        assert_eq!(
            set.codes()
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
        assert!(FixField::new(&spoken).has_source("cme"));
        assert_eq!(
            FixField::new(&spoken).id().unwrap(),
            Some(id_of(35, "MsgType"))
        );

        // Metadata takes any positive tag, whatever the
        // field's membership: the identity is the tag and the name, and the
        // dictionary is not consulted.
        let mut vendor = member("TradeID", "cme", 5001);
        FixFieldMut::new(&mut vendor).set_tag(35).unwrap();
        FixFieldMut::new(&mut vendor)
            .set_tags(&[5002, 40_000, 40_001])
            .unwrap();
        assert_eq!(FixField::new(&vendor).tag().unwrap(), Some(35));
        assert_eq!(
            FixField::new(&vendor).tags().unwrap(),
            [5002, 40_000, 40_001]
        );
        assert!(FixField::new(&vendor).has_source("cme"));
        for tag in [1, 35, 4_999, 5_000, 39_999, 40_000, i32::MAX] {
            assert!(FixId::of(tag, "TradeID").is_ok(), "{tag}");
        }
        let mut counted = member("NoPartyIDs", "cme", 453);
        FixFieldMut::new(&mut counted).set_counter(35).unwrap();
        assert_eq!(FixField::new(&counted).counter().unwrap(), Some(35));

        // A registry holds a member on a specification tag, membership and all.
        let registry = FixRegistry::from_fields([spoken.clone()]).unwrap();
        assert_eq!(registry.get_field_by_tag(35), Some(&spoken));
        assert!(FixField::new(registry.field_by_tag(35).unwrap()).has_source("cme"));
    }

    #[test]
    fn a_lent_spelling_stays_with_the_field_that_already_answers_for_it() {
        // The courtesy a second field on one tag buys the holder is an alias of
        // the arrival's name. An alias reaches exactly one field - `check_free`
        // refuses a second everywhere - and lending is the one write that does
        // not go through it, so it states the rule itself: a spelling a third
        // field already answers for is that field's and is not lent away.
        let mut registry =
            FixRegistry::from_fields([tagged("Symbol", 55), full("Price", 44, &[], &["Ticker"])])
                .unwrap();
        assert_eq!(registry.field_by_name("Ticker").unwrap().name(), "Price");

        // A second field on tag 55 named `Ticker`: the tag stays with `Symbol`,
        // the arrival is registered under its own name and identity, and the
        // alias stays where it was.
        assert!(registry.insert(tagged("Ticker", 55)).unwrap().is_none());
        assert_eq!(registry.field_by_tag(55).unwrap().name(), "Symbol");
        assert_eq!(
            registry.field_by_id(id_of(55, "Ticker")).unwrap().name(),
            "Ticker"
        );
        assert_eq!(
            FixField::new(registry.field_by_tag(55).unwrap())
                .names()
                .count(),
            0,
            "the holder is lent nothing it would have to take from another field"
        );
        assert_eq!(
            registry
                .iter()
                .filter(|field| FixField::new(field).names().any(|alias| alias == "Ticker"))
                .map(Field::name)
                .collect::<Vec<_>>(),
            ["Price"],
            "one alias, one holder"
        );

        // Which is what keeps the first holder reachable and writable: taking
        // the spelling made `Price` refuse its own update, because `check_free`
        // then found its alias held by `Symbol`.
        let mut price = registry.field_by_tag(44).unwrap().clone();
        FixFieldMut::new(&mut price)
            .set_description("the traded price")
            .unwrap();
        registry.update(price).unwrap();
        assert_eq!(
            registry.field_by_tag(44).unwrap().description(),
            Some("the traded price")
        );
        // The spelling now names a field of its own, and a canonical name is
        // read before an alias; `Price` keeps the claim it arrived with.
        assert_eq!(registry.field_by_name("Ticker").unwrap().name(), "Ticker");
        assert_eq!(
            FixField::new(registry.field_by_tag(44).unwrap())
                .names()
                .collect::<Vec<_>>(),
            ["Ticker"]
        );

        // The lenient verb reaches the same insert and answers the same way.
        let mut lenient =
            FixRegistry::from_fields([tagged("Symbol", 55), full("Price", 44, &[], &["Ticker"])])
                .unwrap();
        assert!(lenient.add_field(tagged("Ticker", 55)).unwrap());
        assert_eq!(lenient.field_by_name("Ticker").unwrap().name(), "Ticker");
        assert_eq!(
            FixField::new(lenient.field_by_tag(44).unwrap())
                .names()
                .collect::<Vec<_>>(),
            ["Ticker"]
        );
    }

    #[test]
    fn two_fields_may_hold_one_tag_under_two_names() {
        let mut spec = tagged("Symbol", 5055);
        FixFieldMut::new(&mut spec).set_names(["Ticker"]).unwrap();
        FixFieldMut::new(&mut spec).set_tags(&[9055]).unwrap();
        let venue = member("VenueSymbol", "cme", 5055);

        let registry = FixRegistry::from_fields([spec.clone(), venue.clone()]).unwrap();
        assert_eq!(registry.len(), 2 + seeded_fields());

        // Each identity answers its own field exactly as it was stated:
        // neither learns the other's name.
        let spec_id = id_of(5055, "Symbol");
        let venue_id = id_of(5055, "VenueSymbol");
        let held = registry.field_by_id(spec_id).unwrap();
        assert_eq!(held, &spec);
        assert_eq!(FixField::new(held).names().collect::<Vec<_>>(), ["Ticker"]);
        assert_eq!(FixField::new(held).tags().unwrap(), [9055]);
        assert_eq!(FixField::new(held).sources().count(), 0);
        assert_eq!(registry.field_by_id(venue_id).unwrap(), &venue);
        assert!(
            registry.get_field_by_id(id_of(9055, "Symbol")).is_none(),
            "an alternate tag is an index entry, never an identity"
        );

        // A bare wire tag answers the first holder; the newcomer is reached by
        // its name.
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

        // The order of arrival is what decides the first holder, and neither
        // learns a name the other way round either.
        let reversed = FixRegistry::from_fields([venue.clone(), spec.clone()]).unwrap();
        assert_eq!(reversed.field_by_tag(5055).unwrap(), &venue);
        assert_eq!(reversed.field_by_id(spec_id).unwrap(), &spec);
        assert_eq!(registry.len(), reversed.len());

        // A conflict is still a conflict, and it names both fields.
        let mut twice = member("VenueSym", "cme", 5099);
        FixFieldMut::new(&mut twice).set_names(["TICKER"]).unwrap();
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
    fn fold_table(
        verb: &str,
        seeded_merges: usize,
        fold: impl Fn(&mut FixRegistry, Field) -> (usize, usize),
    ) {
        let mut registry = FixRegistry::from_fields([
            full("Symbol", 55, &[65], &["Ticker"]),
            full("Price", 44, &[31], &["Px"]),
        ])
        .unwrap();

        // Row 1: the same tag under the same folded name is the same field, and
        // the merge unions its tags, aliases and membership.
        let mut same = member("SYMBOL", "cme", 55);
        FixFieldMut::new(&mut same).set_tags(&[66]).unwrap();
        FixFieldMut::new(&mut same).set_names(["Sym"]).unwrap();
        assert_eq!(
            fold(&mut registry, same),
            (0, 1 + seeded_merges),
            "{verb}: row 1"
        );
        assert_eq!(registry.len(), 2 + seeded_fields(), "{verb}");
        let symbol = registry.field_by_tag(55).unwrap();
        assert_eq!(symbol.name(), "Symbol", "{verb}");
        assert_eq!(FixField::new(symbol).tags().unwrap(), [66, 65], "{verb}");
        assert_eq!(
            FixField::new(symbol).names().collect::<Vec<_>>(),
            ["Sym", "Ticker"],
            "{verb}"
        );
        assert_eq!(
            FixField::new(symbol).sources().collect::<Vec<_>>(),
            ["cme"],
            "{verb}"
        );

        // Row 2: the same tag under another name is a new field, registered
        // beside the holder, and neither learns the other's name. The bare tag
        // keeps answering the first holder; the newcomer is reached by its
        // name or its identity, and its membership is its own.
        assert_eq!(
            fold(&mut registry, member("VenueSymbol", "xnas", 55)),
            (1, seeded_merges),
            "{verb}: row 2"
        );
        assert_eq!(registry.len(), 3 + seeded_fields(), "{verb}");
        let symbol = registry.field_by_tag(55).unwrap();
        assert_eq!(symbol.name(), "Symbol", "{verb}: the bare tag");
        assert_eq!(
            FixField::new(symbol).names().collect::<Vec<_>>(),
            ["Sym", "Ticker"],
            "{verb}"
        );
        assert!(!FixField::new(symbol).has_source("xnas"), "{verb}");
        let newcomer = registry.field_by_id(id_of(55, "VenueSymbol")).unwrap();
        assert_eq!(newcomer.name(), "VenueSymbol", "{verb}");
        assert!(FixField::new(newcomer).names().next().is_none(), "{verb}");
        assert_eq!(
            FixField::new(newcomer).sources().collect::<Vec<_>>(),
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
        FixFieldMut::new(&mut spelled)
            .set_names(["BlpSym"])
            .unwrap();
        assert_eq!(
            fold(&mut registry, spelled),
            (0, 1 + seeded_merges),
            "{verb}: row 3"
        );
        assert_eq!(registry.len(), 3 + seeded_fields(), "{verb}");
        let symbol = registry.field_by_tag(9055).unwrap();
        assert_eq!(
            symbol.name(),
            "Symbol",
            "{verb}: the alternate reaches the holder"
        );
        assert_eq!(
            FixField::new(symbol).tags().unwrap(),
            [66, 65, 9055],
            "{verb}"
        );
        assert_eq!(
            FixField::new(symbol).names().collect::<Vec<_>>(),
            ["Sym", "Ticker", "BlpSym"],
            "{verb}"
        );
        assert_eq!(
            FixField::new(symbol).sources().collect::<Vec<_>>(),
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
            (0, 1 + seeded_merges),
            "{verb}: row 3 by alias"
        );
        assert_eq!(registry.field_by_tag(31).unwrap().name(), "Price", "{verb}");
        assert_eq!(
            FixField::new(registry.field_by_tag(55).unwrap())
                .tags()
                .unwrap(),
            [66, 65, 9055],
            "{verb}: a tag another field answers is nobody's alternate"
        );

        // Row 4: neither, so it is inserted as it arrived.
        assert_eq!(
            fold(&mut registry, member("Text", "cme", 58)),
            (1, seeded_merges),
            "{verb}: row 4"
        );
        assert_eq!(registry.len(), 4 + seeded_fields(), "{verb}");
        assert_eq!(
            registry.field_by_tag(58).unwrap(),
            &member("Text", "cme", 58)
        );
        assert_eq!(registry.dialects(), ["blp", "cme", "xnas"], "{verb}");
    }

    #[test]
    fn the_fold_table_holds_through_add_field_and_through_merge_with() {
        fold_table("add_field", 0, |registry, field| {
            if registry.add_field(field).unwrap() {
                (1, 0)
            } else {
                (0, 1)
            }
        });
        fold_table("merge_with", 2, |registry, field| {
            let other = FixRegistry::from_fields([field]).unwrap();
            let merge = registry.merge_with(&other).unwrap();
            assert!(merge.is_clean(), "{:?}", merge.dropped);
            (merge.added, merge.merged)
        });
    }

    #[test]
    fn one_message_code_namespace_folds_a_restated_name_and_keeps_a_second_one() {
        let mut registry = FixRegistry::from_fields([tagged("MsgType", 35)]).unwrap();
        let message = |name: &str, code: &str| {
            let mut field =
                DataType::from(StructType::from_fields([]).unwrap()).required_field(name);
            FixFieldMut::new(&mut field).set_msgtype(code).unwrap();
            field
        };
        registry
            .insert_definition(FixCategory::Components, message("NewOrderSingle", "D"))
            .unwrap();
        assert_eq!(registry.msgtype("D").unwrap().name(), "NewOrderSingle");

        // A definition re-declaring the code under the same folded name folds
        // into the stored one.
        assert!(
            !add_definition(
                &mut registry,
                FixCategory::Components,
                message("new_order_single", "D")
            )
            .unwrap()
        );
        assert_eq!(msgtypes(&registry), 1);
        assert_eq!(registry.msgtype("D").unwrap().name(), "NewOrderSingle");

        // Under another name it is a second message, whose bare code answers
        // the first holder; the second is reached by its name.
        assert!(
            add_definition(
                &mut registry,
                FixCategory::Components,
                message("VenueOrder", "D")
            )
            .unwrap()
        );
        assert_eq!(msgtypes(&registry), 2);
        assert_eq!(registry.msgtype("D").unwrap().name(), "NewOrderSingle");
        assert_eq!(registry.msgtype("VenueOrder").unwrap().name(), "VenueOrder");
        assert_eq!(registry.msgtype("venue_order").unwrap().as_str(), "D");
        assert_eq!(registry.msgtype("neworder_single").unwrap().as_str(), "D");
        // The two this test added.
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
            ("FIX:tag", "3x"),
            ("FIX:tag", "+35"),
            ("FIX:tag", "-35"),
            ("FIX:tag", ""),
            // The alternates are one compact JSON array: the comma text an older
            // writer wrote is not one, and neither is a spaced, signed, empty,
            // unclosed or repeated element.
            ("FIX:tags", "1,2"),
            ("FIX:tags", "[1,,2]"),
            ("FIX:tags", "[1,1]"),
            ("FIX:tags", "[1, 2]"),
            ("FIX:tags", "[1,-2]"),
            ("FIX:tags", "[0]"),
            ("FIX:tags", "[1"),
            ("FIX:tags", "[1]]"),
            ("FIX:tags", "[2147483648]"),
            ("FIX:tags", "[1,]"),
            ("FIX:tags", "[,1]"),
            ("FIX:tags", "[01]"),
            ("FIX:tags", "[1 2]"),
        ];
        for (key, stored) in cases {
            let mut field = tagged("Symbol", 55);
            field.insert_metadata(key, stored).unwrap();
            let error = match key {
                "FIX:tag" => FixField::new(&field).tag().unwrap_err(),
                _ => FixField::new(&field).tags().unwrap_err(),
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

        // A stored names text that is not the array of words the setter writes
        // reads as nothing rather than as a partial list: the comma text an
        // older writer wrote, an escaped word, an unclosed array, a trailing
        // byte. A read stays infallible, so it cannot report; it can decline to
        // mis-read.
        for stored in [
            "Ticker,Sym",
            "[\"Ticker\",\"Sy\\\"m\"]",
            "[\"Ticker\"",
            "[\"Ticker\"]x",
            "{\"Ticker\"}",
        ] {
            let mut field = tagged("Symbol", 55);
            field.insert_metadata("FIX:names", stored).unwrap();
            assert_eq!(FixField::new(&field).names().count(), 0, "{stored:?}");
        }
        // A well-formed array reads whatever produced it.
        let mut field = tagged("Symbol", 55);
        field
            .insert_metadata("FIX:names", "[\"Ticker\",\"Sym\"]")
            .unwrap();
        assert_eq!(
            FixField::new(&field).names().collect::<Vec<_>>(),
            ["Ticker", "Sym"]
        );
    }

    #[test]
    fn a_names_text_the_read_walks_as_nothing_never_enters_a_registry() {
        // The read is infallible and answers nothing for such a text, so the
        // registry is where it is refused: at intake, under the full key, and on
        // a merge, so a stored text is never silently replaced by nothing. An
        // empty word, an escaped one and a trailing separator are not the array
        // the setter writes; a name stated twice under the fold is not either.
        for stored in [
            "Ticker,Sym",
            "[\"\"]",
            "[\"Ticker\",\"\"]",
            "[\"Ticker\",\"Sy\\\"m\"]",
            "[\"Ticker\",]",
            "[\"Ticker\" \"Sym\"]",
            "[\"Ticker\",\"TICKER\"]",
            "[\"Ticker\",\"a\tb\"]",
        ] {
            let mut field = tagged("Symbol", 55);
            field.insert_metadata("FIX:names", stored).unwrap();
            let error = FixRegistry::new().insert(field.clone()).unwrap_err();
            assert!(
                matches!(&error, Error::InvalidMetadataValue { key, .. } if key == "FIX:names"),
                "{stored:?}: {error}"
            );
            let mut incoming = tagged("Symbol", 55);
            FixFieldMut::new(&mut incoming).set_names(["Sym"]).unwrap();
            let error = FixFieldMut::new(&mut incoming)
                .merge_with(&FixField::new(&field))
                .unwrap_err();
            assert!(
                matches!(&error, Error::InvalidMetadataValue { key, .. } if key == "FIX:names"),
                "{stored:?}: {error}"
            );
            assert_eq!(
                FixField::new(&incoming).names().collect::<Vec<_>>(),
                ["Sym"]
            );
        }
    }

    #[test]
    fn a_field_without_a_tag_never_enters() {
        let error = FixRegistry::new()
            .insert(DataType::utf8().nullable_field("Symbol"))
            .unwrap_err();
        assert!(error.is_absent(), "{error}");
        let message = error.to_string();
        assert!(message.contains("FIX:tag"), "{message}");
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
        // `Prc` is Price's canonical name and also an alias LastPx declares; the
        // canonical claim wins whatever order the fields entered in. The same
        // holds for a tag: 31 is LastPx's own tag and Price lists it as an
        // alternate.
        let price = full("Prc", 44, &[31], &["Price"]);
        let last = full("LastPx", 31, &[], &["Prc", "LastPrice"]);
        for order in [[price.clone(), last.clone()], [last, price]] {
            let registry = FixRegistry::from_fields(order).unwrap();
            assert_eq!(registry.field_by_name("prc").unwrap().name(), "Prc");
            assert_eq!(registry.field_by_name("Price").unwrap().name(), "Prc");
            assert_eq!(
                registry.field_by_name("LastPrice").unwrap().name(),
                "LastPx"
            );
            assert_eq!(registry.field_by_tag(31).unwrap().name(), "LastPx");
            assert_eq!(registry.field_by_tag(44).unwrap().name(), "Prc");
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
            assert_eq!(probed.len(), 1 + seeded_fields());
        }

        // The one thing this namespace admits twice: a held tag under another
        // name is a field of its own, inserted beside the holder, the bare tag
        // answering the holder and neither learning the other's name.
        let mut beside = registry.clone();
        assert_eq!(
            beside.insert(full("SymbolSfx", 55, &[], &[])).unwrap(),
            None
        );
        assert_eq!(beside.len(), 2 + seeded_fields());
        assert_eq!(
            beside.field_by_tag(55).unwrap(),
            registry.field_by_tag(55).unwrap()
        );
        assert!(
            FixField::new(beside.field_by_name("SymbolSfx").unwrap())
                .names()
                .next()
                .is_none()
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
        assert_eq!(FixField::new(&prior).tags().unwrap(), [65]);
        assert_eq!(
            registry.field_by_tag(55).unwrap(),
            &replacement.with_name("Symbol")
        );
        assert_eq!(registry.field_by_name("symbol").unwrap().name(), "Symbol");
        assert_eq!(registry.field_by_tag(66).unwrap().name(), "Symbol");
        assert_eq!(registry.field_by_name("Sym").unwrap().name(), "Symbol");
        assert!(registry.get_field_by_tag(65).is_none());
        assert!(registry.get_field_by_name("Ticker").is_none());
        assert_eq!(registry.len(), 2 + seeded_fields());

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
        FixFieldMut::new(&mut incoming).set_tag(55).unwrap();
        FixFieldMut::new(&mut incoming).set_tags(&[67, 66]).unwrap();
        FixFieldMut::new(&mut incoming)
            .set_names(["Instrument", "sym"])
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
        assert_eq!(
            FixField::new(merged).description(),
            Some("Symbol described")
        );
        // Lists concatenate, incoming first, deduplicated with case folded.
        assert_eq!(FixField::new(merged).tags().unwrap(), [67, 66, 65]);
        assert_eq!(
            FixField::new(merged).names().collect::<Vec<_>>(),
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
        assert_eq!(registry.len(), 1 + seeded_fields());

        // Another spelling retains the accumulated tags and canonical name.
        let before = registry.clone();
        registry.update(tagged("symbol", 55)).unwrap();
        assert_eq!(
            FixField::new(registry.field_by_tag(55).unwrap())
                .tags()
                .unwrap(),
            [67, 66, 65]
        );
        assert_eq!(registry.field_by_tag(55).unwrap().name(), "Symbol");
        assert_eq!(registry.len(), before.len());
    }

    #[test]
    fn a_rejected_merge_leaves_the_registry_untouched() {
        let mut registry = FixRegistry::from_fields([
            full("Symbol", 55, &[65], &["Ticker"]),
            full("Price", 44, &[31], &["Prc"]),
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
            .update(full("Symbol", 55, &[], &["PRC"]))
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
        assert_eq!(probe(&registry, 44, 31, "price", "prc"), [Some("Price"); 4]);
        assert_eq!(registry.len(), 2 + seeded_fields());
    }

    #[test]
    fn add_fields_adds_what_is_absent_and_merges_what_is_present() {
        let mut registry =
            FixRegistry::from_fields([full("Symbol", 55, &[65], &["Ticker"]), tagged("Price", 44)])
                .unwrap();

        // Tag 55 is stored and folds; tag 44 is stored under another spelling of
        // the same name and folds too; tag 58 is new. The venue's own `Symbol`
        // on 5055 is the same folded name under another tag: the same field
        // spelled with another number, so it folds into the holder too.
        let mut priced = tagged("PRICE", 44);
        FixFieldMut::new(&mut priced).set_names(["Px"]).unwrap();
        let (added, merged) = registry
            .add_fields([
                full("Symbol", 55, &[66], &["Sym"]),
                priced,
                tagged("Text", 58),
                member("Symbol", "cme", 5_055),
            ])
            .unwrap();
        assert_eq!((added, merged), (1, 3));
        assert_eq!(registry.len(), 3 + seeded_fields());

        // The merge kept what only the stored field declared and added the rest:
        // the venue's tag as an alternate, and its membership.
        let symbol = registry.field_by_tag(55).unwrap();
        assert_eq!(FixField::new(symbol).tags().unwrap(), [66, 65, 5_055]);
        assert_eq!(
            FixField::new(symbol).names().collect::<Vec<_>>(),
            ["Sym", "Ticker"]
        );
        assert_eq!(FixField::new(symbol).sources().collect::<Vec<_>>(), ["cme"]);
        assert_eq!(registry.field_by_tag(5_055).unwrap().name(), "Symbol");
        assert!(registry.get_field_by_id(id_of(5_055, "Symbol")).is_none());
        // Incoming metadata folds into the stored canonical spelling.
        assert_eq!(registry.field_by_tag(44).unwrap().name(), "Price");
        assert_eq!(registry.field_by_tag(58).unwrap().name(), "Text");

        // The identity is the whole probe: the same tag under another name is
        // another field, added beside the specification's rather than folded
        // into it, and neither learns the other's name - two fields on one
        // tag are two fields, not two spellings of one.
        let text = registry.field_by_tag(58).unwrap().clone();
        let (added, merged) = registry
            .add_fields([member("VenueText", "cme", 58)])
            .unwrap();
        assert_eq!((added, merged), (1, 0));
        let beside = |registry: &FixRegistry| {
            assert_eq!(registry.len(), 4 + seeded_fields());
            // The bare tag answers the first holder, exactly as it was held.
            assert_eq!(registry.field_by_tag(58).unwrap(), &text);
            assert!(FixField::new(&text).names().next().is_none());
            // The arrival exactly as it was stated: no `FIX:names` either.
            assert_eq!(
                registry.field_by_id(id_of(58, "VenueText")).unwrap(),
                &member("VenueText", "cme", 58)
            );
            assert_eq!(
                registry.field_by_name("venuetext").unwrap().name(),
                "VenueText"
            );
        };
        beside(&registry);

        // Both survive the snapshot and the store.
        let snapshot = FixRegistry::from_json(&registry.into_json().unwrap()).unwrap();
        assert_eq!(snapshot, registry);
        beside(&snapshot);
        let root = scratch("two-on-one-tag-unaliased");
        let mut folder = LocalFolder::new(&root).unwrap();
        registry.commit(&mut folder).unwrap();
        let stored = FixRegistry::from_handle(&folder).unwrap();
        assert_eq!(stored, registry);
        beside(&stored);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn add_fields_refuses_the_way_the_one_field_writes_refuse() {
        let mut registry = FixRegistry::from_fields([tagged("Symbol", 55)]).unwrap();

        // No `FIX:tag` is no identity, so there is nothing to add or fold under.
        let error = registry
            .add_fields([DataType::utf8().nullable_field("Nameless")])
            .unwrap_err();
        assert!(error.is_absent(), "{error}");
        assert!(error.to_string().contains("FIX:tag"), "{error}");

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
        assert_eq!(registry.len(), 1 + seeded_fields());
    }

    #[test]
    fn removal_keeps_every_position_consistent() {
        let mut registry = FixRegistry::from_fields([
            full("Symbol", 55, &[65], &["Ticker"]),
            full("Price", 44, &[45], &["Prc"]),
            full("Text", 58, &[59], &["FreeText"]),
        ])
        .unwrap();

        // Removing the first field moves the last into its slot.
        let removed = registry.remove(55).unwrap();
        assert_eq!(removed.name(), "Symbol");
        assert_eq!(registry.len(), 2 + seeded_fields());
        assert_eq!(probe(&registry, 55, 65, "symbol", "ticker"), [None; 4]);
        assert_eq!(probe(&registry, 44, 45, "price", "prc"), [Some("Price"); 4]);
        assert_eq!(
            probe(&registry, 58, 59, "text", "freetext"),
            [Some("Text"); 4]
        );
        assert_eq!(
            registry.iter().map(Field::name).collect::<Vec<_>>(),
            then_crated(&["Price", "sendingtime", "Text", "transacttime"])
        );

        // A name key removes through the alias tier too; a path never does.
        assert!(registry.remove("Symbol").is_none());
        assert_eq!(registry.remove("PRC").unwrap().name(), "Price");
        assert_eq!(probe(&registry, 44, 45, "price", "prc"), [None; 4]);
        assert_eq!(
            probe(&registry, 58, 59, "text", "freetext"),
            [Some("Text"); 4]
        );
        assert_eq!(registry.len(), 1 + seeded_fields());
        assert_eq!(registry.remove("FreeText").unwrap().name(), "Text");
        assert_eq!(
            registry.len(),
            seeded_fields(),
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
        assert!(
            matches!(&by_tag, Error::Absent { expected: "fix field", path } if path == "tag 1")
        );
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
        FixFieldMut::new(&mut party_id).set_tag(448).unwrap();
        let mut role = DataType::Int32.nullable_field("PartyRole");
        FixFieldMut::new(&mut role).set_tag(452).unwrap();
        let mut group = DataType::serie(
            StructType::from_fields([party_id.clone(), role])
                .map(DataType::from)
                .unwrap()
                .required_field("Party"),
        )
        .nullable_field("Parties");
        FixFieldMut::new(&mut group).set_counter(453).unwrap();
        let instrument = StructType::from_fields([tagged("Symbol", 55), tagged("SecurityID", 48)])
            .map(DataType::from)
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
            FixField::new(registry.definition(FixCategory::Groups, "Parties").unwrap())
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
            FixField::new(
                registry
                    .field_by_path(&member)
                    .expect("the member the schema declares")
            )
            .tag()
            .unwrap(),
            Some(448)
        );
        assert_eq!(
            message.by_path(&member).expect("the occurrence's value"),
            Scalar::from("BUYSIDE")
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
            Scalar::from("BUYSIDE")
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
        assert_eq!(iter.len(), 3 + seeded_fields());
        assert_eq!(iter.next().map(Field::name), Some("Account"));
        assert_eq!(
            iter.next_back().map(Field::name),
            crated_names.last().copied()
        );
        assert_eq!(iter.len(), 1 + seeded_fields());
        assert_eq!(iter.next().map(Field::name), Some("sendingtime"));
        assert_eq!(iter.next().map(Field::name), Some("Symbol"));
        assert_eq!(iter.next().map(Field::name), Some("Text"));
        assert_eq!(iter.next().map(Field::name), Some("transacttime"));
        assert_eq!(
            iter.map(Field::name).collect::<Vec<_>>(),
            crated_names[..crated_names.len() - 1].to_vec()
        );
        assert_eq!(
            (&registry).into_iter().map(Field::name).collect::<Vec<_>>(),
            then_crated(&["Account", "sendingtime", "Symbol", "Text", "transacttime"])
        );
        // The cursor form walks the same order, and a binding advancing it with
        // only the last identifier it saw sees every field exactly once.
        let mut walked = Vec::new();
        let mut cursor = None;
        while let Some(field) = registry.next_field_after(cursor) {
            walked.push(field.name());
            cursor = FixField::new(field).id().unwrap();
        }
        assert_eq!(
            walked,
            then_crated_scalars(&["Account", "sendingtime", "Symbol", "Text", "transacttime"])
        );
        assert!(
            registry
                .next_field_after(Some(id_of(i32::MAX, "Nothing")))
                .is_none(),
            "an identity this registry does not hold has no place in its order"
        );
        assert_eq!(
            FixRegistry::new().next_field_after(None).map(Field::name),
            Some("sendingtime"),
            "standard clocks precede the crate's own fields"
        );
        // An alternate tag is an index entry, never a cursor stop.
        let mut aliased = tagged("MsgType", 35);
        FixFieldMut::new(&mut aliased).set_tags(&[2]).unwrap();
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
        assert_eq!(FixRegistry::new().iter().len(), seeded_fields());

        // A conflict anywhere fails the whole build: a held name under another
        // tag is a conflict for the strict verb, where the same tag under
        // another name is a second field beside the first.
        let error = FixRegistry::from_fields([tagged("Text", 58), tagged("text", 59)]).unwrap_err();
        assert!(error.is_conflict(), "{error}");
        let beside = FixRegistry::from_fields([tagged("Text", 58), tagged("Symbol", 58)]).unwrap();
        assert_eq!(beside.len(), 2 + seeded_fields());
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
        assert_eq!(registry.len(), 5 + seeded_fields());
        assert_eq!(registry.field_by_tag(35).unwrap().name(), "MsgType");

        // Tags lead whatever the membership; on an equal tag the holder of the
        // bare tag comes first - it arrived first, and a store writes it first so
        // a reload keeps it the holder - and the identity orders the rest. The
        // crate's own fields, on the highest tags, close the walk.
        let on_35 = [
            (id_of(35, "MsgType"), "MsgType"),
            (id_of(35, "MsgKind"), "MsgKind"),
        ];
        let scalars = [
            "Account",
            "MsgType",
            "MsgKind",
            "sendingtime",
            "transacttime",
            "TradeID",
            "Venue",
        ];
        let expected = then_crated(&scalars);
        assert_eq!(
            registry.iter().map(Field::name).collect::<Vec<_>>(),
            expected
        );
        let mut backwards = registry.iter().rev().map(Field::name).collect::<Vec<_>>();
        backwards.reverse();
        assert_eq!(backwards, expected);
        // The cursor form walks the scalars in the same order, and a binding
        // advancing it with only the last identity it saw sees each of the two
        // fields on one tag exactly once.
        let mut walked = Vec::new();
        let mut cursor = None;
        while let Some(field) = registry.next_field_after(cursor) {
            walked.push(field.name());
            cursor = FixField::new(field).id().unwrap();
        }
        assert_eq!(walked, then_crated_scalars(&scalars));
        assert_eq!(
            registry.next_field_after(Some(on_35[0].0)).map(Field::name),
            Some(on_35[1].1)
        );
        assert_eq!(
            registry.next_field_after(Some(on_35[1].0)).map(Field::name),
            Some("sendingtime")
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
            then_crated(&[
                "Account",
                "MsgKind",
                "MsgType",
                "sendingtime",
                "transacttime",
                "TradeID",
                "Venue"
            ])
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
            then_crated(&[
                "Account",
                "MsgKind",
                "sendingtime",
                "transacttime",
                "TradeID",
                "Venue"
            ])
        );
    }

    #[test]
    fn nested_shapes_file_as_definitions_and_refuse_a_wire_tag_unchanged() {
        // A Struct inserts as a component and a Serie of Structs as a group, so
        // the tag such a field states is a definition's, derived into the
        // definition block: a wire tag on one is refused, naming the block, and
        // the registry stands as it was.
        let mut registry = FixRegistry::from_fields([tagged("Symbol", 55)]).unwrap();
        let original = registry.clone();
        let occurrence =
            DataType::from(StructType::from_fields([tagged("Member", 9_001)]).unwrap());
        for dtype in [
            occurrence.clone(),
            DataType::serie(occurrence.required_field("item")),
        ] {
            let mut field = dtype.nullable_field("InvalidWireField");
            FixFieldMut::new(&mut field).set_tag(453).unwrap();
            let error = registry.insert(field).unwrap_err();
            assert!(error.to_string().contains("100000"), "{error}");
            assert_eq!(registry, original);
        }
        // A Serie of scalars and a dictionary-encoded Struct are neither a
        // definition nor a wire field: refused as the scalar they are not.
        for dtype in [
            DataType::serie(DataType::utf8().required_field("item")),
            DataType::dictionary(
                DataType::Int32,
                DataType::from(StructType::from_fields([tagged("Member", 9_002)]).unwrap()),
            )
            .unwrap(),
        ] {
            let mut field = dtype.nullable_field("InvalidWireField");
            FixFieldMut::new(&mut field).set_tag(453).unwrap();
            let error = registry.insert(field).unwrap_err();
            assert!(error.to_string().contains("scalar"), "{error}");
            assert_eq!(registry, original);
        }
        let mut encoded = DataType::dictionary(DataType::Int32, DataType::utf8())
            .unwrap()
            .nullable_field("Coded");
        FixFieldMut::new(&mut encoded).set_tag(60).unwrap();
        registry.insert(encoded).unwrap();
        assert_eq!(registry.len(), 2 + seeded_fields());
    }

    #[test]
    fn derived_definition_tags_keep_the_exact_initial_slots() {
        let registry = FixRegistry::new();
        // Independent XXH32 vectors placed into the half-open definition block.
        for (name, expected) in [
            ("acorn", 475_337),
            ("birch", 534_447),
            ("cedar", 312_212),
            ("delta.group", 830_555),
        ] {
            assert_eq!(
                derived_definition_tag(&registry, name).unwrap(),
                expected,
                "{name}"
            );
        }
    }

    #[test]
    fn derived_definition_tags_probe_past_scalar_and_component_occupants() {
        let mut registry = FixRegistry::from_fields([tagged("OccupiedScalar", 475_337)]).unwrap();
        let dtype = DataType::from(StructType::from_fields([] as [Field; 0]).unwrap());
        let mut occupied = dtype.clone().nullable_field("OccupiedComponent");
        FixFieldMut::new(&mut occupied).set_tag(475_338).unwrap();
        registry
            .insert_definition(FixCategory::Components, occupied)
            .unwrap();
        assert_eq!(derived_definition_tag(&registry, "acorn").unwrap(), 475_339);
        registry
            .insert_definition(FixCategory::Components, dtype.nullable_field("acorn"))
            .unwrap();
        assert_eq!(
            FixField::new(
                registry
                    .definition(FixCategory::Components, "acorn")
                    .unwrap()
            )
            .tag()
            .unwrap(),
            Some(475_339)
        );
        assert_eq!(
            registry.field_by_tag(475_337).unwrap().name(),
            "OccupiedScalar"
        );
        assert_eq!(
            FixField::new(
                registry
                    .definition(FixCategory::Components, "OccupiedComponent")
                    .unwrap()
            )
            .tag()
            .unwrap(),
            Some(475_338)
        );
    }

    #[test]
    fn a_derived_tag_identifies_one_definition_however_it_arrived() {
        let mut registry = FixRegistry::from_fields([counter("NoPartyIDs", 453)]).unwrap();
        registry
            .insert_definition(FixCategory::Groups, named_group("Parties", 453))
            .unwrap();
        let first = FixField::new(registry.definition(FixCategory::Groups, "Parties").unwrap())
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
        let second = FixField::new(
            registry
                .definition(FixCategory::Groups, "Counterparties")
                .unwrap(),
        )
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
            FixField::new(registry.definition(FixCategory::Groups, "Parties").unwrap())
                .tag()
                .unwrap(),
            Some(first)
        );

        // A tag outside the block is refused whatever states it.
        let mut outside = named_group("Brokers", 453);
        FixFieldMut::new(&mut outside).set_tag(42).unwrap();
        let refused = registry
            .insert_definition(FixCategory::Groups, outside)
            .unwrap_err();
        assert!(refused.to_string().contains("100000"), "{refused}");
    }

    #[test]
    fn a_named_group_and_its_counter_keep_separate_identities() {
        let mut shadow = tagged("Shadow", 5);
        FixFieldMut::new(&mut shadow).set_tags(&[453]).unwrap();
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
        let derived = FixField::new(group).tag().unwrap().expect("a derived tag");
        assert!(FixId::is_definition_tag(derived), "{derived}");
        assert_ne!(derived, 453);
        assert_eq!(FixField::new(group).counter().unwrap(), Some(453));
        assert_eq!(group_by_tag(&registry, 453).unwrap(), group);
        assert_eq!(registry.field_by_tag(5).unwrap().name(), "Shadow");
    }

    #[test]
    fn named_definitions_refuse_wire_tags_and_invalid_counters_atomically() {
        let mut registry =
            FixRegistry::from_fields([counter("NoPartyIDs", 453), tagged("Symbol", 55)]).unwrap();
        let original = registry.clone();
        let mut tagged_group = named_group("Parties", 453);
        FixFieldMut::new(&mut tagged_group).set_tag(453).unwrap();
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
        // Every scalar, the test's own and the crate's, then every definition:
        // the test's group before the crate's.
        let scalars = [
            "Account",
            "sendingtime",
            "Symbol",
            "Text",
            "transacttime",
            "NoPartyIDs",
        ];
        let mut expected = then_crated_scalars(&scalars);
        expected.push("Parties");
        expected.extend(crate_names_of(true));
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
            cursor = FixField::new(field).id().unwrap();
        }
        assert_eq!(walked, then_crated_scalars(&scalars));
        assert_eq!(
            registry
                .definitions(FixCategory::Groups)
                .map(Field::name)
                .collect::<Vec<_>>(),
            ["Parties", "metadata"]
        );
        // The walk holds the definitions too, so a registry rebuilt from its
        // own walk is the registry.
        assert_eq!(
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
        let config = LocalFolder::new(home.join(".config")).unwrap();

        // Nothing configured, or no home at all: a new registry, holding the
        // crate's own fields and nothing else.
        assert_eq!(
            autoload(None, Some(config.clone())).unwrap(),
            FixRegistry::new()
        );
        assert_eq!(autoload(None, None).unwrap(), FixRegistry::new());

        // A present dictionary under the configuration directory loads.
        let mut configured = LocalFolder::new(home.join(".config").join("fix")).unwrap();
        FixRegistry::from_fields([tagged("Symbol", 55)])
            .unwrap()
            .commit(&mut configured)
            .unwrap();
        let loaded = autoload(None, Some(config.clone())).unwrap();
        assert_eq!(loaded.field_by_tag(55).unwrap().name(), "Symbol");

        // The explicit location beats it, spelled as a path or as a URL.
        let mut located = LocalFolder::new(&location).unwrap();
        FixRegistry::from_fields([tagged("Price", 44)])
            .unwrap()
            .commit(&mut located)
            .unwrap();
        let as_path = location.to_string_lossy().into_owned();
        let as_url = located.url().to_string();
        for spelling in [as_path, as_url] {
            let loaded = autoload(Some(&spelling), Some(config.clone())).unwrap();
            assert_eq!(loaded.len(), 1 + seeded_fields(), "{spelling}");
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
        FixFieldMut::new(&mut party_id).set_tag(448).unwrap();
        let mut role = DataType::Int32.nullable_field("PartyRole");
        FixFieldMut::new(&mut role).set_tag(452).unwrap();
        let item = StructType::from_fields([party_id, role])
            .map(DataType::from)
            .unwrap()
            .required_field("Party");
        let mut group = DataType::serie(item).nullable_field("Parties");
        FixFieldMut::new(&mut group).set_counter(453).unwrap();
        let instrument = StructType::from_fields([tagged("Symbol", 55)])
            .map(DataType::from)
            .unwrap()
            .nullable_field("Instrument");
        let mut qty = DataType::Int64.required_field("OrderQty");
        FixFieldMut::new(&mut qty).set_tag(38).unwrap();
        FixFieldMut::new(&mut qty).set_names(["Quantity"]).unwrap();
        let count = counter("NoPartyIDs", 453);
        let mut registry =
            FixRegistry::from_fields([count, qty.clone(), tagged("Symbol", 55)]).unwrap();
        registry
            .insert_definition(FixCategory::Groups, group.clone())
            .unwrap();
        registry
            .insert_definition(FixCategory::Components, instrument.clone())
            .unwrap();
        let registry = Arc::new(registry);
        let root = StructType::from_fields([
            qty,
            instrument,
            group,
            DataType::utf8().nullable_field("9999"),
            registry.field_by_tag(52).unwrap().clone(),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("NewOrderSingle");
        let value = Scalar::from_struct([
            ("OrderQty", Scalar::from(100)),
            (
                "Instrument",
                Scalar::from_struct([("Symbol", Scalar::from("AAPL"))]).unwrap(),
            ),
            (
                "Parties",
                Scalar::from_sequence([
                    Scalar::from_struct([
                        ("PartyID", Scalar::from("BROKER")),
                        ("PartyRole", Scalar::from(1)),
                    ])
                    .unwrap(),
                    Scalar::from_struct([
                        ("PartyID", Scalar::from("CLIENT")),
                        ("PartyRole", Scalar::from(3)),
                    ])
                    .unwrap(),
                ]),
            ),
            ("9999", Scalar::from("custom")),
            (
                "sendingtime",
                Scalar::datetime64(0, yggdryl::TimeUnit::Nanosecond, yggdryl::Timezone::UTC)
                    .unwrap(),
            ),
        ])
        .unwrap();
        (registry, root, value)
    }

    #[test]
    fn folded_message_child_lookup_does_not_choose_between_colliding_names() {
        let field = StructType::from_fields([
            DataType::utf8().required_field("A"),
            DataType::utf8().required_field("a"),
        ])
        .map(DataType::from)
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
        let msg =
            FixMsg::with_registry(Arc::clone(&registry), root.clone(), value.clone()).unwrap();
        assert!(Arc::ptr_eq(msg.registry(), &registry));
        assert_eq!(msg.as_field().name(), root.name());
        assert_eq!(&msg.as_field().fields()[..3], &root.fields()[1..4]);

        // A record input canonicalizes to the ordered sequence the root declares.
        let row = msg.as_value().as_sequence().unwrap();
        assert_eq!(
            row.len(),
            3,
            "three business fields; the clock is the header's and OrderQty is the \
         event's own quantity"
        );
        assert_eq!(row[1].as_sequence().map(<[Scalar]>::len), Some(2));
        assert_eq!(row[2], Scalar::from("custom"));
        assert_eq!(
            msg.by_tag(52).unwrap(),
            Scalar::datetime64(0, yggdryl::TimeUnit::Nanosecond, yggdryl::Timezone::UTC).unwrap()
        );

        // By tag, through the registry's canonical name. `OrderQty` is the
        // quantity the event holds, so it answers exact whatever the row's own
        // column would have typed it as.
        let hundred = Scalar::from(yggdryl::Decimal::from_int(100));
        assert_eq!(msg.by_tag(38).unwrap(), hundred);
        // By name, folded through the registry, and by alias.
        assert_eq!(msg.by_name("orderqty").unwrap(), hundred);
        assert_eq!(msg.by_name("QUANTITY").unwrap(), hundred);
        // An unknown tag is kept under its rendered name.
        assert_eq!(msg.by_tag(9999).unwrap(), Scalar::from("custom"));
        assert_eq!(msg.by_name("9999").unwrap(), Scalar::from("custom"));
        // A path descends a component by name and a group by index.
        assert_eq!(
            msg.by_path(&fpath("Instrument.symbol")).unwrap(),
            Scalar::from("AAPL")
        );
        assert_eq!(
            msg.by_path(&fpath("Parties[1].PartyID")).unwrap(),
            Scalar::from("CLIENT")
        );
        assert_eq!(
            msg.by_path(&fpath("parties[0].PartyRole")).unwrap(),
            Scalar::from(1)
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
            Scalar::from("BROKER")
        );
        assert_eq!(
            msg.by_path(&fpath("Parties[0].partyid")).unwrap(),
            Scalar::from("BROKER")
        );
        assert!(msg.get_by_tag(-1).is_none());

        // The generic pair matches the specialized one for every key; a
        // group's counter tag reaches nothing, the group being its list.
        assert!(msg.get_by_tag(453).is_none());
        for tag in [38, 9999, 55, 453] {
            assert_eq!(msg.get(tag), msg.get_by_tag(tag), "{tag}");
            assert_eq!(msg.value(tag).ok(), msg.by_tag(tag).ok(), "{tag}");
        }
        // The generic pair matches the specialized one for every key: a name
        // reaches what the name door reaches, and a key spelling more than one
        // segment reaches what the path door reaches once that key is read.
        for name in ["OrderQty", "quantity", "absent"] {
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
        for name in ["OrderQty", "quantity", "absent"] {
            assert_eq!(
                msg.get_by_path(&fpath(name)),
                msg.get_by_name(name),
                "{name}"
            );
        }
        let error = msg.by_tag(55).unwrap_err();
        assert!(
            matches!(&error, Error::Absent { expected: "fix value", path } if path == "tag 55")
        );
        let error = msg.by_path(&fpath("absent.x")).unwrap_err();
        assert!(matches!(&error, Error::Absent { path, .. } if path == "path absent.x"));

        // Equality and hashing follow the facts, the schema and the value, so
        // the same parts build the same message: the stated clock keeps the
        // identity it settles to deterministic.
        let same = FixMsg::with_registry(Arc::clone(&registry), root, value).unwrap();
        assert_eq!(msg, same);
        assert_eq!(stable_hash_of(&msg), stable_hash_of(&same));
    }

    #[test]
    fn a_message_rejects_a_value_its_field_refuses() {
        let (registry, root, _) = order();
        let error = FixMsg::with_registry(
            Arc::clone(&registry),
            root.clone(),
            Scalar::from_struct([("OrderQty", Scalar::from("many"))]).unwrap(),
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
        assert!(
            matches!(&error, Error::InvalidRecord { path, reason }
            if path == "$" && reason == "expected a struct root, got field \"scalar\" of int64"),
            "{error}"
        );
    }

    #[test]
    fn a_message_resolves_a_bare_tag_to_its_first_holder_and_an_identity_exactly() {
        // Tag 5001 is held twice, under two names: the venue's arrived first
        // and so answers the bare tag; the specification's is reached by its
        // own name or identity. Tag 35 is held once.
        let mut venue_trade = member("TradeID", "cme", 5001);
        FixFieldMut::new(&mut venue_trade)
            .set_names(["TID"])
            .unwrap();
        let mut spec_trade = tagged("SecondaryTradeID", 5001);
        FixFieldMut::new(&mut spec_trade)
            .set_names(["STID"])
            .unwrap();
        let msg_type = tagged("MsgType", 35);
        let registry = Arc::new(
            FixRegistry::from_fields([venue_trade.clone(), spec_trade.clone(), msg_type.clone()])
                .unwrap(),
        );
        assert_eq!(registry.field_by_tag(5001).unwrap().name(), "TradeID");

        // A message root the venue's fields shape carries no membership of its
        // own: a message is not a dictionary member.
        let root = StructType::from_fields([venue_trade.clone(), msg_type.clone()])
            .map(DataType::from)
            .unwrap()
            .required_field("VenueExecutionReport");
        let value = Scalar::from_struct([
            ("TradeID", Scalar::from("T-1")),
            ("MsgType", Scalar::from("8")),
        ])
        .unwrap();
        let msg = FixMsg::with_registry(Arc::clone(&registry), root, value).unwrap();
        assert_eq!(FixField::new(msg.as_field()).sources().count(), 0);

        // The bare tag: its first holder, which is the child this root carries.
        assert_eq!(msg.by_tag(5001).unwrap(), Scalar::from("T-1"));
        assert_eq!(msg.by_name("tid").unwrap(), Scalar::from("T-1"));
        // One namespace, so MsgType is reachable from a venue message.
        assert_eq!(msg.by_tag(35).unwrap(), Scalar::from("8"));
        assert_eq!(msg.by_name("msgtype").unwrap(), Scalar::from("8"));
        // The specification's field on 5001 names a root child this message
        // does not hold, so its alias misses rather than answering the venue's
        // value.
        assert!(msg.get_by_name("stid").is_none());

        // An identity is exact: it names one field, and the child is the one
        // that field's name reaches.
        let venue_id = id_of(5001, "TradeID");
        let spec_id = id_of(5001, "SecondaryTradeID");
        assert_eq!(msg.by_id(venue_id).unwrap(), Scalar::from("T-1"));
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
        let plain_root = StructType::from_fields([spec_trade, msg_type])
            .map(DataType::from)
            .unwrap()
            .required_field("ExecutionReport");
        let plain = FixMsg::with_registry(
            registry,
            plain_root,
            Scalar::from_struct([
                ("SecondaryTradeID", Scalar::from("S-1")),
                ("MsgType", Scalar::from("8")),
            ])
            .unwrap(),
        )
        .unwrap();
        assert_eq!(FixField::new(plain.as_field()).sources().count(), 0);
        assert_eq!(plain.by_tag(5001).unwrap(), Scalar::from("S-1"));
        assert_eq!(plain.by_name("stid").unwrap(), Scalar::from("S-1"));
        assert_eq!(plain.by_id(spec_id).unwrap(), Scalar::from("S-1"));
        assert!(plain.get_by_name("tid").is_none());
        assert!(plain.get_by_id(venue_id).is_none());
    }

    #[test]
    fn a_message_root_carrying_membership_is_read_as_any_root_is() {
        // Membership is provenance on a dictionary field; on a root it states
        // nothing the message reads, and a spelling nothing validates on read
        // is not a refusal.
        let mut root = StructType::from_fields([tagged("MsgType", 35)])
            .map(DataType::from)
            .unwrap()
            .required_field("row");
        FixFieldMut::new(&mut root).set_sources(["cme"]).unwrap();
        root.insert_metadata("FIX:sources", r#"["2cme","c me"]"#)
            .unwrap();
        let message = FixMsg::with_registry(
            Arc::new(FixRegistry::new()),
            root,
            Scalar::from_struct([("MsgType", Scalar::from("8"))]).unwrap(),
        )
        .unwrap();
        assert_eq!(message.by_tag(35).unwrap(), Scalar::from("8"));
        assert_eq!(
            FixField::new(message.as_field())
                .sources()
                .collect::<Vec<_>>(),
            ["2cme", "c me"],
            "read back as stored"
        );
    }

    /// The committed dictionary, as the codec every derivation case reads with:
    /// a parse runs the dictionary's rules, so a parsed report already states
    /// what its rules imply.
    fn deriving() -> yggdryl::FixCodec {
        yggdryl::FixCodec::new(committed())
    }

    #[test]
    fn a_report_states_what_is_left_once_it_has_stated_the_rest() {
        let codec = deriving();
        // Appendix D: a part-filled working order. What is left is what was
        // ordered minus what was done, and the fill's worth is its quantity at
        // its price.
        let held = codec
            .parse_fix_line(b"8=FIX.4.4|35=8|39=1|150=F|38=100|14=40|32=40|31=10.5|54=1|10=0|")
            .expect("a readable report");
        assert_eq!(held.by_tag(151).unwrap(), decimal("60"));
        assert_eq!(held.by_tag(381).unwrap(), decimal("420"));
        // One fill, so the average is that fill's price.
        assert_eq!(held.by_tag(6).unwrap(), decimal("10.5"));

        // A closed order leaves nothing, whatever the arithmetic of the other two
        // would say: Appendix D shows zero on every terminal row.
        let closed = codec
            .parse_fix_line(b"8=FIX.4.4|35=8|39=4|150=4|38=100|14=40|10=0|")
            .expect("a readable report");
        assert_eq!(closed.by_tag(151).unwrap(), decimal("0"));

        // The same identity read backwards: what was ordered is what is left plus
        // what was done.
        let ordered = codec
            .parse_fix_line(b"8=FIX.4.4|35=8|39=1|14=40|151=60|10=0|")
            .expect("a readable report");
        assert_eq!(ordered.by_tag(38).unwrap(), decimal("100"));
    }

    #[test]
    fn a_message_states_each_market_number_once_and_reads_the_market_off_its_codes() {
        use yggdryl::graph::{Market, Operation};

        let codec = deriving();
        let held = codec
            .parse_fix_line(b"8=FIX.4.4|35=8|55=BRN|54=1|44=82.5|38=300|31=82.5|59=1|326=17|10=0|")
            .expect("a readable report");
        // Every number is FIX's own field and has a column of its own:
        // `Price(44)`, `OrderQty(38)` and `Quantity(53)` alike, each exact.
        let columns = yggdryl::fix_schema_tags();
        for tag in [44, 38, 53] {
            assert!(columns.contains(&tag), "{tag} is a column");
        }
        assert_eq!(held.by_tag(44).unwrap(), decimal("82.5"));
        assert_eq!(held.by_tag(38).unwrap(), decimal("300"));
        // And what the message states is what the trait reads off them.
        assert_eq!(held.get_price(), yggdryl::Decimal::parse("82.5").ok());
        // `OrderQty(38)` is what the order asked for; the quantity is what it
        // has left open, which a report stating no `LeavesQty(151)`, no state
        // and no fill does not say.
        assert_eq!(held.get_ordqty(), yggdryl::Decimal::parse("300").ok());
        assert_eq!(held.get_quantity(), None);
        // `Quantity(53)` is the newer spelling and its own slot: a line that
        // said `53=` holds it there, and `OrderQty` stays empty.
        assert!(held.get_by_tag(53).is_none());
        let spelled = codec
            .parse_fix_line(b"8=FIX.4.4|35=8|53=300|10=0|")
            .expect("a readable report");
        assert_eq!(spelled.by_tag(53).unwrap(), decimal("300"));
        assert!(spelled.get_by_tag(38).is_none());
        assert_eq!(
            spelled.get_quantity(),
            yggdryl::Decimal::parse("300").ok(),
            "and the quantity the message states reads either spelling"
        );
        // The last trade is its own fact beside them, under FIX's own tag.
        assert_eq!(held.get_lastpx(), yggdryl::Decimal::parse("82.5").ok());
        // How long it stands: `TimeInForce(59)`'s `1` is good till canceled.
        assert_eq!(
            held.get_timeinforce().map(|held| held.as_str()),
            Some("GTC")
        );

        // What the market said about trading it, read off the status it stated:
        // `ReadyToTrade` trades.
        assert_eq!(held.get_tradable(), Some(true));
        // And the ticker, off `Symbol`.
        assert_eq!(held.get_ticker(), Some("BRN"));

        // A halt says the opposite, and a status that is about something else
        // says nothing either way.
        for (status, expected) in [("2", Some(false)), ("21", Some(false)), ("7", None)] {
            let line = format!("8=FIX.4.4|35=8|55=BRN|326={status}|10=0|");
            let held = codec
                .parse_fix_line(line.as_bytes())
                .expect("a readable report");
            assert_eq!(held.get_tradable(), expected, "{status}");
        }
        // The session answers where the security says nothing, and the listing
        // behind it.
        let held = codec
            .parse_fix_line(b"8=FIX.4.4|35=8|55=BRN|340=3|965=1|10=0|")
            .expect("a readable report");
        assert_eq!(held.get_tradable(), Some(false), "a closed session");
        let held = codec
            .parse_fix_line(b"8=FIX.4.4|35=8|55=BRN|965=5|10=0|")
            .expect("a readable report");
        assert_eq!(held.get_tradable(), Some(false), "delisted");

        // FIX's one non-answer is not a ticker: `[N/A]` says the instrument has
        // none and is named by its `SecurityID` instead, so the column says so
        // too rather than repeating the convention.
        let held = codec
            .parse_fix_line(b"8=FIX.4.4|35=8|55=[N/A]|48=US0378331005|22=4|10=0|")
            .expect("a readable report");
        assert_eq!(held.get_ticker(), None);
        // A message carrying no `Symbol` at all still names one where the
        // exchange's own identifier is the ticker: that is what `Symbol`
        // derives to, and this column is what it settled on.
        let held = codec
            .parse_fix_line(b"8=FIX.4.4|35=8|48=IBM|22=8|10=0|")
            .expect("a readable report");
        assert_eq!(held.get_ticker(), Some("IBM"));
    }

    #[test]
    fn a_stated_value_is_never_replaced_and_filling_twice_changes_nothing() {
        let codec = deriving();
        // The venue's own arithmetic wins even where it disagrees with the
        // specification's: the row says what was sent.
        let held = codec
            .parse_fix_line(b"8=FIX.4.4|35=8|39=1|38=100|14=40|151=999|10=0|")
            .expect("a readable report");
        assert_eq!(held.by_tag(151).unwrap(), decimal("999"));

        // Idempotent: a value derived once is a stated value the second time, so
        // the message read back from its own wire - which spells what it derived
        // - derives it to itself.
        let once = codec
            .parse_fix_line(b"8=FIX.4.4|35=8|52=20260102-10:15:30|39=1|38=100|14=40|10=0|")
            .expect("a readable report");
        let twice = codec
            .parse_fix_line(&once.into_bytes(b'|'))
            .expect("a second reading");
        assert_eq!(once, twice);
    }

    #[test]
    fn a_rule_answers_nothing_rather_than_a_guess() {
        let codec = deriving();
        // An input the message never stated: nothing is derived from an absence.
        let held = codec
            .parse_fix_line(b"8=FIX.4.4|35=8|39=1|38=100|10=0|")
            .expect("a readable report");
        assert_eq!(held.get_by_tag(151), None, "no CumQty to subtract");

        // A negative remainder means the two inputs were never about one order,
        // so the rule declines rather than stating a quantity that cannot exist.
        let crossed = codec
            .parse_fix_line(b"8=FIX.4.4|35=8|39=1|38=40|14=100|10=0|")
            .expect("a readable report");
        assert_eq!(crossed.get_by_tag(151), None);

        // A status the matrices do not place answers nothing either.
        let unknown = codec
            .parse_fix_line(b"8=FIX.4.4|35=8|39=Z|38=100|14=40|10=0|")
            .expect("a readable report");
        assert_eq!(unknown.get_by_tag(151), None);

        // A message type the rule does not speak for is left alone: an order has
        // no remainder to state until something reports on it.
        let order = codec
            .parse_fix_line(b"8=FIX.4.4|35=D|38=100|14=40|10=0|")
            .expect("a readable order");
        assert_eq!(order.get_by_tag(151), None);
    }

    #[test]
    fn a_foreign_exchange_trade_settles_in_the_currency_it_was_dealt_in() {
        let codec = deriving();
        // Appendix O: the settlement currency defaults to the dealt one, and the
        // settled amount is the traded amount at the stated rate.
        let held = codec
            .parse_fix_line(
                b"8=FIX.4.4|35=8|39=2|150=F|38=100|14=100|32=100|31=1.25|15=EUR|155=1.1|10=0|",
            )
            .expect("a readable report");
        assert_eq!(held.by_tag(381).unwrap(), decimal("125"));
        assert_eq!(
            held.by_tag(119).unwrap(),
            decimal("137.5"),
            "the traded amount at the stated rate",
        );
        let settled = held.by_tag(120).unwrap();
        assert_eq!(settled.as_str(), Some("EUR"));

        // A trade that states its own settlement currency keeps it.
        let stated = codec
            .parse_fix_line(b"8=FIX.4.4|35=8|39=2|15=EUR|120=USD|10=0|")
            .expect("a readable report");
        assert_eq!(stated.by_tag(120).unwrap().as_str(), Some("USD"));
    }

    #[test]
    fn the_published_typed_tags_are_exactly_the_ones_a_message_holds() {
        // A binding walking a message's typed facts reads `FIX_TYPED_TAGS`, so
        // the listing has to be the whole of what `is_typed_tag` answers outside
        // the crate's own block - nothing missing, because a tag missing here is
        // a fact no column holds either, and nothing extra, because a tag the
        // message never lifted would read as a fact it does not have.
        let published: Vec<i32> = yggdryl::FIX_TYPED_TAGS.to_vec();
        let answered: Vec<i32> = (1..10_000)
            .filter(|tag| !(yggdryl::CRATE_TAG_MIN..yggdryl::CRATE_TAG_MAX).contains(tag))
            .filter(|tag| yggdryl::internals::fix_identity::is_typed_tag(*tag))
            .collect();
        let mut sorted = published.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, answered, "published {published:?}");
        // And it is stated in the order the wire states it: the header, the
        // lifted band in tag order - the six FX parts of a price among it -
        // the trailer, then the text.
        let trailer = published.len() - 4;
        assert_eq!(published.len(), 35);
        assert_eq!(&published[..8], [8, 35, 49, 56, 34, 43, 52, 385]);
        assert_eq!(&published[trailer..], [93, 89, 10, 58]);
        assert!(
            published[8..trailer]
                .windows(2)
                .all(|pair| pair[0] < pair[1]),
            "the lifted band is swept in tag order: {:?}",
            &published[8..trailer]
        );
        // Every one of them is a tag no content row keeps.
        let registry = committed();
        let codec = FixCodec::new(registry);
        let held = codec
            .parse_fix_line(b"8=FIX.4.4|35=D|11=A1|44=10.5|38=100|55=AAPL|58=note|10=0|")
            .expect("a readable order");
        for tag in yggdryl::FIX_TYPED_TAGS {
            assert!(
                !held.entries().iter().any(|entry| entry.tag() == tag),
                "{tag} is typed, so it is no entry"
            );
        }
    }

    #[test]
    fn a_field_states_the_spellings_that_mean_nothing_was_sent() {
        let mut field = DataType::Float64.nullable_field("StopPx");
        FixFieldMut::new(&mut field).set_tag(99).unwrap();
        FixFieldMut::new(&mut field)
            .set_nulls(["N/A", "NONE", ""])
            .unwrap();
        assert_eq!(field.get_metadata("FIX:nulls"), Some("N/A,NONE,"));
        assert_eq!(
            FixField::new(&field).nulls().collect::<Vec<_>>(),
            ["N/A", "NONE"],
            "an empty spelling is stored and walked past, as every list here does",
        );

        // Matched case-insensitively against the trimmed text, exactly as the
        // capture-wide list matches.
        for held in ["N/A", "n/a", " none ", "NONE"] {
            assert!(FixField::new(&field).is_null_value(held), "{held}");
        }
        for held in ["12.5", "NA", "N/A/"] {
            assert!(!FixField::new(&field).is_null_value(held), "{held}");
        }

        // The row types it as null, and the entries are the row read as a tree,
        // so a stated absence is no entry.
        let registry = Arc::new(FixRegistry::from_fields([field]).unwrap());
        let codec = yggdryl::FixCodec::new(registry).with_null_values::<[&str; 0], &str>([]);
        let message = codec.parse_fix_line(b"99=N/A|").expect("a readable frame");
        assert_eq!(message.get_by_tag(99), Some(Scalar::Null));
        assert!(!message.entries().iter().any(|held| held.tag() == 99));

        // A value the list does not name is read as the price it is - a float
        // here, because the field this registry holds for the tag is one.
        let message = codec.parse_fix_line(b"99=12.5|").expect("a readable frame");
        assert_eq!(message.by_tag(99).unwrap(), Scalar::from(12.5_f64));
    }

    #[test]
    fn a_null_spelling_is_refused_when_it_carries_the_separator_or_repeats() {
        let mut field = DataType::utf8().nullable_field("Account");
        FixFieldMut::new(&mut field).set_tag(1).unwrap();

        let error = FixFieldMut::new(&mut field).set_nulls(["a,b"]).unwrap_err();
        assert!(error.to_string().contains("without ','"), "{error}");
        let error = FixFieldMut::new(&mut field)
            .set_nulls(["NONE", "none"])
            .unwrap_err();
        assert!(error.to_string().contains("twice"), "{error}");
        // A refusal leaves the field exactly as it was.
        assert_eq!(field.get_metadata("FIX:nulls"), None);

        // Empty input removes the property.
        FixFieldMut::new(&mut field).set_nulls(["NONE"]).unwrap();
        FixFieldMut::new(&mut field)
            .set_nulls::<[&str; 0], &str>([])
            .unwrap();
        assert_eq!(field.get_metadata("FIX:nulls"), None);
    }

    /// Fixture A: the standard `SideCodeSet`, dated as the specification dates it.
    ///
    /// A vocabulary is the dictionary's and a field only names it, so the fixture
    /// is the pair: the dictionary holding `sidecodeset`, and the field that
    /// reads by it.
    fn side() -> (FixRegistry, Field) {
        let mut registry = FixRegistry::new();
        registry
            .set_codeset(
                "sidecodeset",
                &[
                    FixCode::new("Buy", "1"),
                    FixCode::new("Sell", "2"),
                    FixCode::new("Undisclosed", "7"),
                    FixCode::new("CrossShort", "9"),
                    FixCode::new("CrossShortExempt", "A"),
                ],
            )
            .unwrap();
        let mut field = DataType::utf8().nullable_field("Side");
        FixFieldMut::new(&mut field).set_tag(54).unwrap();
        FixFieldMut::new(&mut field)
            .set_codeset("sidecodeset")
            .unwrap();
        registry.insert(field.clone()).unwrap();
        (registry, field)
    }

    /// Fixture B: `CommTypeCodeSet`, whose long names are what tier 2 folds.
    fn comm_type() -> (FixRegistry, Field) {
        let mut registry = FixRegistry::new();
        registry
            .set_codeset(
                "commtypecodeset",
                &[
                    FixCode::new("PerUnit", "1"),
                    FixCode::new("Percent", "2"),
                    FixCode::new("Absolute", "3"),
                    FixCode::new("PercentageWaivedCashDiscount", "4"),
                    FixCode::new("PercentageWaivedEnhancedUnits", "5"),
                    FixCode::new("PointsPerBondOrContract", "6")
                        .with_description("Good Till Date (GTD) points per bond"),
                    FixCode::new("BasisPoints", "7"),
                    FixCode::new("AmountPerContract", "8"),
                ],
            )
            .unwrap();
        let mut field = DataType::utf8().nullable_field("CommType");
        FixFieldMut::new(&mut field).set_tag(13).unwrap();
        FixFieldMut::new(&mut field)
            .set_codeset("commtypecodeset")
            .unwrap();
        registry.insert(field.clone()).unwrap();
        (registry, field)
    }

    #[test]
    fn a_code_set_resolves_by_value_by_name_and_by_every_folding_of_a_name() {
        let (registry, field) = comm_type();
        let view = registry
            .codeset_of(&field)
            .expect("the set the field names");

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
        let mut registry = FixRegistry::new();
        registry
            .set_codeset(
                "sidecodeset",
                &[
                    FixCode::new("Buy", "1").with_aliases(["Bought", "BUYSIDE"]),
                    FixCode::new("Sell", "2"),
                ],
            )
            .unwrap();
        let view = registry.codeset("sidecodeset").unwrap();

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

        let mut registry = FixRegistry::new();
        registry
            .set_codeset("sidecodeset", &[buy, FixCode::new("Sell", "2")])
            .unwrap();
        let view = registry.codeset("sidecodeset").unwrap();

        // The grown code is one code with two spellings, not two codes.
        assert_eq!(view.codes().count(), 2);
        assert_eq!(view.code_value("Bought"), Some("1"));
        assert_eq!(view.code_name("1"), Some("Buy"));
    }

    #[test]
    fn an_ambiguous_spelling_resolves_to_nothing_rather_than_the_first_match() {
        let mut registry = FixRegistry::new();
        registry
            .set_codeset(
                "sidecodeset",
                &[
                    FixCode::new("Cross", "8"),
                    FixCode::new("CrossOther", "9").with_aliases(["cross"]),
                ],
            )
            .unwrap();
        let view = registry.codeset("sidecodeset").unwrap();

        // Two codes reach one spelling, so picking either would be a guess.
        assert_eq!(view.code_value("Cross"), None);
        assert_eq!(view.code_by_name("cross"), None);
        // Tier 1 still answers, because a legal wire value is never a spelling.
        assert_eq!(view.code_value("8"), Some("8"));
        // Two names sharing one value are an alias, not an ambiguity.
        let mut aliased = FixRegistry::new();
        aliased
            .set_codeset(
                "sidecodeset",
                &[
                    FixCode::new("Cross", "8"),
                    FixCode::new("CrossSame", "8").with_aliases(["cross"]),
                ],
            )
            .unwrap();
        assert_eq!(
            aliased.codeset("sidecodeset").unwrap().code_value("cross"),
            Some("8")
        );
    }

    #[test]
    fn tier_three_reads_a_leading_abbreviation_and_leaves_both_traps_alone() {
        let mut registry = FixRegistry::new();
        registry
            .set_codeset(
                "timeinforcecodeset",
                &[
                    FixCode::new("GoodTillDate", "6").with_description("Good Till Date (GTD)"),
                    FixCode::new("BrokenDate", "7")
                        .with_description("Broken date; SettlDate (64) is required"),
                    FixCode::new("SwapValueFactor", "8").with_description(
                        "Swap Value Factor (SVP) through a central counterparty (CCP)",
                    ),
                ],
            )
            .unwrap();
        let view = registry.codeset("timeinforcecodeset").unwrap();

        assert_eq!(view.code_value("gtd"), Some("6"));
        assert_eq!(view.code_value("GTD"), Some("6"));
        // A numeric parenthesization is a tag cross-reference, never a spelling.
        assert_eq!(view.code_value("64"), None);
        // Only the abbreviation on the leading phrase counts.
        assert_eq!(view.code_value("svp"), Some("8"));
        assert_eq!(view.code_value("ccp"), None);
    }

    #[test]
    fn a_code_set_round_trips_canonically_and_a_hand_edit_names_its_byte_position() {
        let (registry, field) = side();
        // The field states the name; the dictionary holds the document.
        assert_eq!(field.get_metadata("FIX:codeset"), Some("sidecodeset"));
        let stored = registry
            .codeset_of(&field)
            .expect("the code set is stored")
            .document()
            .to_owned();
        assert_eq!(
            stored,
            concat!(
                r#"["#,
                r#"{"value":"1","name":"Buy"},"#,
                r#"{"value":"2","name":"Sell"},"#,
                r#"{"value":"7","name":"Undisclosed"},"#,
                r#"{"value":"9","name":"CrossShort"},"#,
                r#"{"value":"A","name":"CrossShortExempt"}]"#,
            )
        );

        // Taking the set away and putting it back produces the same text. A
        // vocabulary a held field reads by may not be taken away, so the round
        // trip runs on a dictionary that holds the set alone.
        let mut rebuilt = FixRegistry::new();
        rebuilt
            .set_codeset("sidecodeset", &FixCodes::parse(&stored).unwrap())
            .unwrap();
        let taken = rebuilt.remove_codeset("sidecodeset").unwrap().unwrap();
        assert!(rebuilt.get_codeset("sidecodeset").is_none());
        rebuilt.set_codeset("sidecodeset", &taken).unwrap();
        assert_eq!(
            rebuilt.codeset("sidecodeset").unwrap().document(),
            stored.as_str()
        );

        // Keys follow the document's declared order, so a reordered one is
        // refused rather than mis-scanned.
        let reordered = r#"[{"name":"Buy","value":"1"}]"#;
        let mut held = FixRegistry::new();
        create_codeset(&mut held, "sidecodeset", reordered.to_owned()).unwrap();
        let edited = held.codeset("sidecodeset").unwrap();
        let error = edited.codes().next().unwrap().unwrap_err();
        assert!(
            matches!(&error, Error::Parse { target, position, .. }
                if *target == "fix codes" && *position == reordered.find(r#""value""#).unwrap()),
            "{error}"
        );
        // A read that cannot parse answers nothing rather than a wrong answer.
        assert_eq!(edited.code_value("Buy"), None);
        assert_eq!(edited.code("1"), None);

        // A document is the array of its entries, so the wrapper object an older
        // writer put around one is refused on its first byte like any other
        // hand edit - there is one shape, and this is not it. A code set is
        // stored on its own, so the dictionary is what holds that edit.
        let mut wrapping = FixRegistry::new();
        create_codeset(
            &mut wrapping,
            "sidecodeset",
            r#"{"codes":[{"value":"1","name":"Buy"}]}"#.to_owned(),
        )
        .unwrap();
        let error = wrapping
            .codeset("sidecodeset")
            .unwrap()
            .codes()
            .next()
            .unwrap()
            .unwrap_err();
        assert!(
            matches!(&error, Error::Parse { position, .. } if *position == 0),
            "FIX:codeset: {error}"
        );
        assert!(error.to_string().contains("'['"), "FIX:codeset: {error}");
        for (property, wrapped) in [
            (
                "FIX:directions",
                r#"{"directions":[{"code":"S","patterns":["^TX"]}]}"#,
            ),
            (
                "FIX:idmap",
                r#"{"idmap":[{"map":"identifiers","key":"side"}]}"#,
            ),
        ] {
            let mut wrapper = DataType::utf8().nullable_field("Side");
            wrapper.set_metadata([(property, wrapped)]).unwrap();
            let view = FixField::new(&wrapper);
            let error = match property {
                "FIX:directions" => view.directions().next().unwrap().unwrap_err(),
                _ => view.idmap().next().unwrap().unwrap_err(),
            };
            assert!(
                matches!(&error, Error::Parse { position, .. } if *position == 0),
                "{property}: {error}"
            );
            assert!(error.to_string().contains("'['"), "{property}: {error}");
        }
    }

    #[test]
    fn two_codes_may_share_a_value_but_never_a_name_and_neither_may_be_empty() {
        let mut registry = FixRegistry::new();

        let error = registry
            .set_codeset(
                "sidecodeset",
                &[FixCode::new("Buy", "1"), FixCode::new("BUY", "2")],
            )
            .unwrap_err();
        assert!(error.to_string().contains("BUY"), "{error}");
        assert!(registry.get_codeset("sidecodeset").is_none(), "atomic");

        let error = registry
            .set_codeset("sidecodeset", &[FixCode::new("Buy", "")])
            .unwrap_err();
        assert!(error.to_string().contains("value"), "{error}");

        // Two names on one value is an alias, which is legal.
        registry
            .set_codeset(
                "sidecodeset",
                &[FixCode::new("Buy", "1"), FixCode::new("Bought", "1")],
            )
            .unwrap();
        let view = registry.codeset("sidecodeset").unwrap();
        assert_eq!(view.codes().count(), 2);
        assert_eq!(view.code_value("Bought"), Some("1"));
    }

    #[test]
    fn a_code_set_carries_every_fact_the_specification_states_about_a_member() {
        let mut registry = FixRegistry::new();
        registry
            .set_codeset(
                "sidecodeset",
                &[FixCode::new("Buy", "1")
                    .with_description(r#"Buy; the "long" side"#)
                    .with_aliases(["Bought"])
                    .with_group("Directional")],
            )
            .unwrap();

        let view = registry.codeset("sidecodeset").unwrap();
        let code = view.code("1").unwrap();
        assert_eq!(code.name(), "Buy");
        assert_eq!(code.group(), Some("Directional"));
        // A description holding a quote survives the round trip through the one
        // codec that escaped it.
        assert_eq!(
            code.parse_doc().unwrap().as_deref(),
            Some(r#"Buy; the "long" side"#)
        );
        assert_eq!(code.aliases().collect::<Vec<_>>(), ["Bought"]);
        // An empty slice removes the set rather than holding an empty one.
        let mut cleared = registry.clone();
        cleared.set_codeset("sidecodeset", &[]).unwrap();
        assert!(cleared.get_codeset("sidecodeset").is_none());
        assert_eq!(
            cleared.codesets().count(),
            4,
            "the crate's market data type, MsgCat, state and plugin side sets remain"
        );
    }

    #[test]
    fn a_field_merge_folds_every_key_by_its_own_rule() {
        // Stored: the older, lower-priority source.
        let mut stored = DataType::utf8().nullable_field("LastQty");
        FixFieldMut::new(&mut stored).set_tag(32).unwrap();
        FixFieldMut::new(&mut stored).set_tags(&[65, 66]).unwrap();
        FixFieldMut::new(&mut stored)
            .set_names(["lastshares"])
            .unwrap();
        FixFieldMut::new(&mut stored)
            .set_description("the stored wording")
            .unwrap();
        FixFieldMut::new(&mut stored)
            .set_codeset("storedcodeset")
            .unwrap();

        // Incoming: the newer, higher-priority source.
        let mut incoming = DataType::utf8().nullable_field("LastQty");
        FixFieldMut::new(&mut incoming).set_tag(32).unwrap();
        FixFieldMut::new(&mut incoming).set_tags(&[67, 66]).unwrap();
        FixFieldMut::new(&mut incoming).set_names(["qty"]).unwrap();
        FixFieldMut::new(&mut incoming)
            .set_description("the incoming wording")
            .unwrap();
        FixFieldMut::new(&mut incoming)
            .set_codeset("lastqtycodeset")
            .unwrap();

        FixFieldMut::new(&mut incoming)
            .merge_with(&FixField::new(&stored))
            .unwrap();
        let merged = FixField::new(&incoming);

        // Identity is not merged, it is agreed.
        assert_eq!(merged.tag().unwrap(), Some(32));
        // Lists union, incoming first, deduplicated.
        assert_eq!(merged.tags().unwrap(), [67, 66, 65]);
        // The description is never compared: incoming has one, so it wins.
        assert_eq!(merged.description(), Some("the incoming wording"));
        // A field keeps the vocabulary it already reads by, and the stored field
        // is that one: a registry fold hands the incoming field in as `self`, and
        // `unify_codeset` has already folded the incoming set into the held one,
        // so taking the incoming name here would move the field to a set holding
        // strictly less than the one it reads by.
        assert_eq!(merged.codeset(), Some("storedcodeset"));
        // Aliases union, incoming first, folded and deduplicated.
        assert_eq!(merged.names().collect::<Vec<_>>(), ["qty", "lastshares"]);

        // And the members fold where they are held: by wire value, the reading
        // the dictionary already states winning a shared one, the other source
        // keeping a value only it has.
        let mut registry = FixRegistry::new();
        registry
            .set_codeset(
                "lastqtycodeset",
                &[
                    FixCode::new("IncomingOnly", "5"),
                    FixCode::new("Shared", "1").with_description("the incoming reading"),
                ],
            )
            .unwrap();
        registry
            .merge_codeset(
                "lastqtycodeset",
                &[
                    FixCode::new("StoredOnly", "9"),
                    FixCode::new("Shared", "1").with_description("the stored reading"),
                ],
            )
            .unwrap();
        let folded = registry.codeset("lastqtycodeset").unwrap();
        assert_eq!(folded.code_name("1"), Some("Shared"));
        assert_eq!(
            folded.code("1").unwrap().parse_doc().unwrap().as_deref(),
            Some("the incoming reading")
        );
        assert_eq!(folded.code_name("5"), Some("IncomingOnly"));
        assert_eq!(folded.code_name("9"), Some("StoredOnly"));
    }

    #[test]
    fn a_merge_keeps_a_stored_description_the_incoming_does_not_state() {
        let mut stored = DataType::utf8().nullable_field("Symbol");
        FixFieldMut::new(&mut stored).set_tag(55).unwrap();
        FixFieldMut::new(&mut stored)
            .set_description("a very long stored wording nobody wants compared")
            .unwrap();

        let mut incoming = DataType::utf8().nullable_field("Symbol");
        FixFieldMut::new(&mut incoming).set_tag(55).unwrap();
        FixFieldMut::new(&mut incoming)
            .set_names(["Ticker"])
            .unwrap();

        // The FIX half folds the `FIX:` keys and nothing else, so on its own it
        // leaves a description alone in both directions: it is not FIX's key.
        FixFieldMut::new(&mut incoming)
            .merge_with(&FixField::new(&stored))
            .unwrap();
        assert_eq!(FixField::new(&incoming).description(), None);
        assert_eq!(
            FixField::new(&incoming).names().collect::<Vec<_>>(),
            ["Ticker"]
        );

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
        assert_eq!(FixField::new(held).names().collect::<Vec<_>>(), ["Ticker"]);

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
        FixFieldMut::new(&mut incoming).set_tag(55).unwrap();
        FixFieldMut::new(&mut incoming)
            .set_names(["Ticker"])
            .unwrap();
        let before = incoming.clone();

        let mut other = DataType::utf8().nullable_field("Symbol");
        FixFieldMut::new(&mut other).set_tag(56).unwrap();
        let error = FixFieldMut::new(&mut incoming)
            .merge_with(&FixField::new(&other))
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
        FixFieldMut::new(&mut mine).set_tag(5055).unwrap();
        FixFieldMut::new(&mut mine)
            .merge_with(&FixField::new(&vendor))
            .unwrap();
        assert_eq!(FixField::new(&mine).sources().collect::<Vec<_>>(), ["cme"]);
        assert_eq!(FixField::new(&mine).tag().unwrap(), Some(5055));
        // And the union is a union: two dictionaries each stamping itself leave
        // both names, sorted, whichever side held which.
        let mut theirs = member("Symbol", "xnas", 5055);
        FixFieldMut::new(&mut theirs)
            .merge_with(&FixField::new(&mine))
            .unwrap();
        assert_eq!(
            FixField::new(&theirs).sources().collect::<Vec<_>>(),
            ["cme", "xnas"]
        );
    }

    #[test]
    fn a_merge_adding_nothing_leaves_the_field_byte_identical() {
        let mut field = DataType::utf8().nullable_field("LastQty");
        FixFieldMut::new(&mut field).set_tag(32).unwrap();
        FixFieldMut::new(&mut field).set_tags(&[65]).unwrap();
        FixFieldMut::new(&mut field)
            .set_description("wording")
            .unwrap();
        FixFieldMut::new(&mut field)
            .set_names(["lastshares"])
            .unwrap();
        FixFieldMut::new(&mut field)
            .set_codeset("lastqtycodeset")
            .unwrap();

        let before = field.clone();
        let other = field.clone();
        FixFieldMut::new(&mut field)
            .merge_with(&FixField::new(&other))
            .unwrap();
        assert_eq!(field, before);
        // Merging a bare field into a full one is also a no-op.
        let mut bare = DataType::utf8().nullable_field("LastQty");
        FixFieldMut::new(&mut bare).set_tag(32).unwrap();
        FixFieldMut::new(&mut field)
            .merge_with(&FixField::new(&bare))
            .unwrap();
        assert_eq!(field, before);
    }

    fn committed() -> Arc<FixRegistry> {
        static REGISTRY: std::sync::OnceLock<Arc<FixRegistry>> = std::sync::OnceLock::new();
        Arc::clone(REGISTRY.get_or_init(|| {
            let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
            Arc::new(FixRegistry::from_handle(&LocalFolder::new(root).unwrap()).unwrap())
        }))
    }

    #[test]
    fn group_entry_names_singularize_published_collections() {
        use yggdryl::internals::fix_component::{
            canonical_group_name, entry_name, group_name, group_names,
        };
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
        let mut count = counter("nosecurityaltid", 454);
        count.set_display("NoSecurityAltID").unwrap();
        assert_eq!(group_name(&count).as_str(), "secaltids");
        assert_eq!(canonical_group_name("SecAltIDGrp").as_str(), "secaltids");
        let mut count = counter("noregulatorytradeids", 1907);
        count.set_display("NoRegulatoryTradeIDs").unwrap();
        assert_eq!(group_name(&count).as_str(), "regulatorytradeids");
        assert_eq!(
            canonical_group_name("RegulatoryTradeIDGrp").as_str(),
            "regulatorytradeids"
        );
        let mut count = counter("nopartysubids", 802);
        count.set_display("NoPartySubIDs").unwrap();
        assert_eq!(group_name(&count).as_str(), "partysubids");
        assert_eq!(canonical_group_name("PtysSubGrp").as_str(), "partysubids");
        assert_eq!(canonical_group_name("HopGrp").as_str(), "hops");
        let names = group_names(&count, Some("PtysSubGrp"));
        assert_eq!(names.0.as_str(), "partysubids");
        assert_eq!(names.1.as_str(), "PartySubIDs");
        assert_eq!(names.2.as_str(), "ptyssub");
        assert_eq!(names.3.as_str(), "PtysSub");
    }

    #[test]
    fn the_catalog_names_every_shipped_group_and_entry_without_field_collisions() {
        let registry = committed();
        let mut groups = HashSet::new();
        let mut entries = HashSet::new();
        for field in registry.definitions(FixCategory::Groups) {
            assert!(groups.insert(field.name()));
            if let Some(map) = (field.dtype()).as_mapping() {
                // The crate's one Map group, counted by its own tag and
                // reached through the counter door, as every group is.
                let (tag, name) = [yggdryl::METADATA_TAG_NAME]
                    .into_iter()
                    .find(|(_, name)| *name == field.name())
                    .unwrap_or_else(|| panic!("{} is no crate group", field.name()));
                assert_eq!(FixField::new(field).tag().unwrap(), Some(tag), "{name}");
                assert_eq!(FixField::new(field).counter().unwrap(), Some(tag), "{name}");
                assert!(map.keys_sorted());
                assert!(!map.entries().is_nullable());
                assert!(!map.entries().fields()[0].is_nullable());
                assert_eq!(registry.get_field_by_counter(tag), Some(field), "{name}");
                continue;
            }
            let DataType::Serie(item) = field.dtype() else {
                panic!("{}", field.dtype());
            };
            let display = field
                .display()
                .unwrap_or_else(|| panic!("{} has no collection display", field.name()));
            assert_eq!(display.to_ascii_lowercase(), field.name(), "{display}");
            assert_eq!(field.name().ends_with("grp"), display.ends_with("Grp"));
            assert!(entries.insert(item.name()));
            assert!(!item.is_nullable());
            assert!(matches!(item.dtype(), DataType::Struct(_)));
            let component = registry
                .definition(FixCategory::Components, item.name())
                .unwrap();
            assert_eq!(item.dtype(), component.dtype());
            // No scalar shares the occurrence's name, and the field door reaches
            // the component by it once no scalar answers.
            assert!(
                registry
                    .get_definition(FixCategory::Fields, item.name())
                    .is_none()
            );
            assert_eq!(registry.get_field_by_name(item.name()), Some(component));
            let counter = registry
                .field_by_tag(FixField::new(field).counter().unwrap().unwrap())
                .unwrap();
            assert_eq!(counter.dtype(), &DataType::Int32);
        }
        // The shipped dictionary's groups, beside the crate's one Map group.
        assert_eq!(groups.len(), 580 + 1);
        assert_eq!(entries.len(), 580);
        // The shipped dictionary's own, beside the crate's own components,
        // which every registry carries.
        assert_eq!(
            registry.definitions(FixCategory::Components).count(),
            928 + crated_components()
        );
        assert_eq!(msgtypes(&registry), 181);
    }

    #[test]
    fn a_group_path_reaches_members_and_skips_its_occurrence_component() {
        let registry = committed();
        let member = registry.field_by_path(&fpath("Parties.PartyID")).unwrap();
        assert_eq!(FixField::new(member).tag().unwrap(), Some(448));
        assert_eq!(member.name(), "partyid");
        assert_eq!(registry.field_by_tag(453).unwrap().name(), "nopartyids");
        assert_eq!(
            registry.field_by_tag(453).unwrap().dtype(),
            &DataType::Int32
        );
        for field in registry.definitions(FixCategory::Groups) {
            let DataType::Serie(item) = field.dtype() else {
                assert!(
                    matches!(field.dtype(), DataType::Map(_) | DataType::SortedMap(_)),
                    "{}: a group is a List, or one of the crate's Maps",
                    field.name()
                );
                continue;
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
                DataType::time64(yggdryl::TimeUnit::Nanosecond).unwrap(),
            ),
            (
                9_002,
                "LocalMktTime",
                "10:15:30",
                DataType::time64(yggdryl::TimeUnit::Nanosecond).unwrap(),
            ),
            (
                9_003,
                "UTCTimestamp",
                "20240102-10:15:30.123",
                DataType::datetime64(yggdryl::TimeUnit::Nanosecond, yggdryl::Timezone::UTC)
                    .unwrap(),
            ),
            (
                9_004,
                "TZTimeOnly",
                "10:15:30-05:00",
                DataType::datetime64(yggdryl::TimeUnit::Nanosecond, yggdryl::Timezone::UTC)
                    .unwrap(),
            ),
            (
                9_005,
                "LocalMktDate",
                "20240102",
                DataType::datetime64(yggdryl::TimeUnit::Nanosecond, yggdryl::Timezone::NAIVE)
                    .unwrap(),
            ),
        ] {
            let held: DataType = spelling.parse().expect("a resolvable FIX datatype");
            assert_eq!(held, dtype, "{spelling}");

            let mut field = held.nullable_field("dated");
            FixFieldMut::new(&mut field).set_tag(tag).unwrap();
            let registry = Arc::new(FixRegistry::from_fields([field]).unwrap());
            let message = yggdryl::FixCodec::new(registry)
                .parse_pairs([(tag.to_string().as_bytes(), wire.as_bytes())])
                .expect("the row builds");
            assert_ne!(
                message.by_tag(tag).unwrap(),
                Scalar::Null,
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

    #[test]
    fn every_committed_code_set_is_the_document_the_rust_writer_renders() {
        let registry = committed();
        let mut sets = 0_usize;
        let mut codes = 0_usize;
        // The dictionary holds each vocabulary once, under its own name, however
        // many fields read by it.
        for set in registry.codesets() {
            sets += 1;
            let held: Vec<FixCode> = set
                .codes()
                .map(|code| FixCode::from(code.expect("a readable code")))
                .collect();
            assert!(!held.is_empty(), "{} declares a code", set.name());
            codes += held.len();

            // The cross-host assertion: the generator wrote this set in Python,
            // and the Rust writer must reproduce it byte for byte - key order,
            // sort order, escaping, the legacy codes' dates and aliases - with
            // versions canonicalized first, because the two hosts spell a
            // version their own way and neither spelling is the document.
            assert_eq!(
                render_codes(&held).expect("the codes render"),
                canonical_versions(set.document()),
                "{}",
                set.name()
            );
        }
        // The crate adds MarketDataKind's 26 categories, the 62 states, the 118
        // market data types and the 19 sides a plugin's role can store to the
        // 735 published sets.
        assert_eq!(sets, 739, "code sets held");
        assert_eq!(codes, 7_954, "code records");
    }

    #[test]
    fn the_entry_column_is_one_sorted_map_of_text() {
        let root = yggdryl::fix_schema(&FixRegistry::new(), "row").unwrap();
        let column = yggdryl::fix::FIXENTRIES_COLUMN;
        let held = root
            .fields()
            .iter()
            .find(|field| field.name() == column)
            .unwrap_or_else(|| panic!("a {column} column"));
        // One map, whatever a message holds: each key a field's `tag:name`,
        // each value text - a scalar's wire text, anything nested its JSON -
        // so no depth bound and no counter column stand beside it.
        assert_eq!(
            held.dtype(),
            &DataType::map_of(DataType::utf8(), DataType::utf8(), true).unwrap()
        );
        assert!(held.is_nullable());
        assert_eq!(FixField::new(held).counter().unwrap(), None);
        assert!(root.index_of("nofixentries").is_none());
    }

    /// A root of `depth` components nested one inside the next, each the sole
    /// member of the one above, the innermost holding one text child: the
    /// deepest tree a row states without a dictionary naming any of it.
    fn towering(depth: usize) -> (Field, Scalar) {
        let mut field = DataType::utf8().nullable_field("leaf");
        let mut value = Scalar::from("deep");
        for level in (1..depth).rev() {
            field = StructType::from_fields([field])
                .map(DataType::from)
                .unwrap()
                .nullable_field(format!("level{level}"));
            value = Scalar::from_sequence([value]);
        }
        let root = DataType::from(StructType::from_fields([field]).unwrap()).required_field("D");
        (root, Scalar::from_sequence([value]))
    }

    /// The residual record of one row: each key under the text it holds.
    fn residual(row: &Scalar, schema: &Field) -> Vec<(String, String)> {
        let at = schema
            .index_of(yggdryl::fix::FIXENTRIES_COLUMN)
            .expect("the residual column");
        row.as_sequence().expect("a row")[at]
            .as_mapping()
            .expect("the residual map")
            .iter()
            .map(|(key, value)| {
                (
                    key.as_str().expect("a text key").to_owned(),
                    value.as_str().expect("a text value").to_owned(),
                )
            })
            .collect()
    }

    #[test]
    fn a_deep_arrival_is_held_whole_and_its_record_is_its_json() {
        let registry = committed();
        let codec = FixCodec::new(Arc::clone(&registry));
        // A party carrying a sub-party: the group entry, its occurrence, the
        // nested group entry, its occurrence and the member it states - five
        // levels of entries out of two levels of groups.
        let message = |value: &str| {
            codec
                .parse_fix_line(
                    format!(
                        "8=FIX.4.4|35=D|11=ORDER-1|55=AAPL|453=1|448=BUYSIDE|452=1|802=1|523={value}|10=0|"
                    )
                    .as_bytes(),
                )
                .expect("a readable order")
        };
        let deep = message("x");

        // The Rust tree is never truncated: all five levels are held whole.
        let mut held = deep
            .entries()
            .iter()
            .find(|held| held.tag() == 453)
            .expect("the group entry");
        let mut levels = 1;
        while let Some(next) = held
            .entries()
            .iter()
            .find(|held| !held.entries().is_empty())
            .or_else(|| held.entries().first())
        {
            held = next;
            levels += 1;
        }
        assert_eq!(levels, 5, "the record holds what the wire nested");
        assert_eq!((held.tag(), held.value()), (523, Some("x")));

        // Fully projected fields are absent from the residual record; the
        // party group the fixed row projects no column for is held in it, and
        // the party it names is the row's `partyids`.
        let full = yggdryl::fix_schema(&registry, "row").unwrap();
        let row = deep.into_row(&full).unwrap();
        let columns = row.as_sequence().expect("a row");
        assert_eq!(
            yggdryl::Identifiers::from_scalar(
                &columns[full.index_of("partyids").expect("the partyids column")]
            )
            .expect("the partyids")
            .to_string(),
            "[executingfirm=BUYSIDE]"
        );
        assert_eq!(
            columns[full.index_of("symbol").expect("the projected symbol")].as_str(),
            Some("AAPL")
        );
        let held = residual(&row, &full);
        assert!(
            held.iter().all(|(key, _)| !key.starts_with("55:")),
            "fully projected fields are absent from the residual record"
        );
        assert!(
            held.iter().any(|(key, _)| key == "453:parties"),
            "the party group is held whole: {held:?}"
        );

        // A projection without the typed symbol and the partyids carries the
        // symbol in the residual record as its text and the group as the JSON
        // of its occurrences, every level keyed `tag:name`, whole at any
        // depth.
        let schema = StructType::from_fields(
            full.fields()
                .iter()
                .filter(|field| !matches!(field.name(), "partyids" | "symbol"))
                .cloned(),
        )
        .map(DataType::from)
        .unwrap()
        .required_field("row");
        let row = deep.into_row(&schema).unwrap();
        let held = residual(&row, &schema);
        let text = |key: &str| {
            held.iter()
                .find(|(held, _)| held == key)
                .map(|(_, value)| value.clone())
                .unwrap_or_else(|| panic!("{key} in {held:?}"))
        };
        assert_eq!(text("55:symbol"), "AAPL");
        let parties = yggdryl::from_json_scalar(text("453:parties").as_bytes())
            .expect("the group's JSON decodes");
        let occurrences = parties.as_sequence().expect("the occurrences");
        assert_eq!(occurrences.len(), 1);
        let party = occurrences[0]
            .as_struct()
            .expect("one occurrence's members");
        assert_eq!(
            party.get("448:partyid").and_then(Scalar::as_str),
            Some("BUYSIDE")
        );
        let subs = party
            .get("802:partysubids")
            .and_then(Scalar::as_sequence)
            .expect("the nested group");
        assert_eq!(
            subs[0]
                .as_struct()
                .and_then(|members| members.get("523:partysubid"))
                .and_then(Scalar::as_str),
            Some("x")
        );
        // And a row holding it is its own fixed point.
        let back = FixMsg::from_row(Arc::clone(&registry), &schema, &row).unwrap();
        assert_eq!(back.into_row(&schema).unwrap(), row);

        // Wire emission walks the whole tree pre-order, so what comes back is
        // what went in.
        let text = deep.into_text('|').unwrap();
        assert!(text.contains("|453=1|448=BUYSIDE|452=1|"), "{text}");
        assert!(text.contains("|802=1|523=x|"), "{text}");

        // Two messages that differ deep down still hash apart, because the
        // digest walks the whole tree.
        assert_ne!(message("x").digest(), message("y").digest());

        // Depth is never a refusal: thirty levels read and render without a
        // complaint. No dictionary names any of them, so the row states the
        // tower in its metadata, as the JSON of what it holds.
        let (root, value) = towering(30);
        let towering = FixMsg::with_registry(Arc::clone(&registry), root, value).unwrap();
        let mut held = &towering.entries()[0];
        let mut levels = 1;
        while let Some(next) = held.entries().first() {
            held = next;
            levels += 1;
        }
        assert_eq!(levels, 30);
        let row = towering.into_row(&schema).expect("no depth refusal");
        assert!(residual(&row, &schema).is_empty());
        let metadata = &row.as_sequence().expect("a row")[schema.index_of("metadata").unwrap()];
        let tower = metadata
            .as_mapping()
            .expect("the metadata")
            .iter()
            .find(|(key, _)| key.as_str() == Some("level1"))
            .and_then(|(_, value)| value.as_str())
            .expect("the tower under its own name");
        assert!(tower.starts_with(r#"{"level2":{"level3":"#), "{tower}");
    }

    #[test]
    fn a_nested_group_states_its_counter_once() {
        // A group's count is the group entry's own value, so the counter beside
        // it states nothing the entries do not already: at the root the counter
        // is left out of the entries, and an occurrence's counter beside the
        // group it heads is the same fact one level down.
        let codec = FixCodec::new(committed());
        let order = codec
            .parse_fix_line(b"8=FIX.4.4|35=D|11=ORDER-1|453=1|448=BUYSIDE|802=1|523=x|10=0|")
            .expect("a readable order");
        // `ClOrdID(11)` is a fact the message lifts and holds, so the entries
        // open at the group.
        let at = order
            .entries()
            .iter()
            .position(|held| held.name() == "parties")
            .expect("the parties group");
        let party = &order.entries()[at].entries()[0];
        assert_eq!(party.name(), "party");
        assert_eq!(
            party
                .entries()
                .iter()
                .filter(|held| held.tag() == 802)
                .count(),
            1,
            "{:?}",
            party.entries()
        );
        let text = order.into_text('|').unwrap();
        assert_eq!(text.matches("|802=1|").count(), 1, "{text}");
    }

    #[test]
    fn a_composed_key_fills_the_field_its_last_segment_names() {
        let codec = yggdryl::FixCodec::new(committed());
        let one = |row: &str| {
            codec
                .parse_line(row.as_bytes())
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
        };
        // A bridge writes a field under its own namespace, and the fact is the
        // field's however the writer spelled the key.
        let filled = one("MSGTYPE=8|TECH.ACCOUNT=ACCT-000117|SIDE=1|");
        assert_eq!(
            filled.by_name("Account").unwrap().as_str(),
            Some("ACCT-000117")
        );

        // What the row states is never overwritten: a namespace's spelling of a
        // fact is not the fact. The corpus states both on one line, disagreeing.
        let stated = one("MSGTYPE=8|CLIENT.SYMBOL=XAU|SYMBOL=XAU/USD|");
        assert_eq!(stated.by_name("Symbol").unwrap().as_str(), Some("XAU/USD"));

        // One voice or silence: two namespaces naming one absent field, and
        // disagreeing, fill nothing. Nine lines of the corpus do exactly this.
        let split = one("MSGTYPE=8|FIRM.ORIG.CLIENTID=2540498.003|ULLINK.CLIENTID=trader1|");
        assert_eq!(split.get_by_name("ClientID"), None);
        // Agreeing, they fill.
        let agreed = one("MSGTYPE=8|FIRM.ORIG.CLIENTID=trader1|ULLINK.CLIENTID=trader1|");
        assert_eq!(
            agreed.by_name("ClientID").unwrap().as_str(),
            Some("trader1")
        );

        // A segment naming no field of this dictionary names nothing.
        let stranger = one("MSGTYPE=8|METAL.LOCO=LDN|");
        assert!(stranger.get_by_name("Loco").is_none());
    }

    #[test]
    fn a_registry_of_the_crates_own_fields_derives_nothing_it_holds_no_field_for() {
        // The crate owns no derived column, so a registry holding only the
        // crate's own fields lands no derivation: every target is a standard
        // field it does not hold. A message still answers its market, because
        // the traits read the FIX fields rather than a column: the ISIN is
        // the one the line stated under the source that names it.
        let registry = FixRegistry::new();
        let codec = FixCodec::new(Arc::new(registry));
        let held = codec
            .parse_line(b"8=FIX.4.4|35=D|11=A|48=US0378331005|22=4|10=0|")
            .unwrap()
            .next()
            .unwrap()
            .unwrap();
        assert_eq!(
            yggdryl::graph::Market::get_securityids(&held).get(&yggdryl::IdType::Isin),
            Some("US0378331005")
        );
        assert_eq!(
            held.get_by_tag(470),
            None,
            "no CountryOfIssue field to fill"
        );
    }
}

mod capture {
    use super::SoleMessage;

    use std::sync::Arc;

    use yggdryl::holder::Buffer;
    use yggdryl::media::RecordOptions;
    use yggdryl::text::{TextBytes, TextLine, TextOptions, read_text_lines};
    use yggdryl::{FixCodec, FixField, FixRegistry, IOMedia, Scalar, Url};

    /// The committed dictionary.
    fn registry() -> Arc<FixRegistry> {
        super::committed_registry()
    }

    /// A framed order behind the prose its process printed around it.
    const TAGGED: &str = "sending >> 8=FIX.4.4|9=176|35=D|49=BUYSIDE|56=VENUE|11=ORDER-1|55=AAPL|54=1|38=100|44=10.5|59=0|60=20240102-10:15:30.000|10=203| << queued seq=1092";
    /// A part-filled execution report: what is left is implied, never stated.
    const WORKING: &str = "8=FIX.4.4|9=224|35=8|49=VENUE|56=BUYSIDE|37=O-9|17=E-1|39=1|150=F|55=AAPL|54=1|38=100|14=40|32=40|31=10.5|64=20240104|10=118|";
    /// The fill that closes it, and states nothing more than it must.
    const FILLED: &str = "8=FIX.4.4|9=224|35=8|49=VENUE|56=BUYSIDE|37=O-9|17=E-2|39=2|150=F|55=AAPL|54=1|38=100|14=100|32=60|31=10.5|15=EUR|155=1.1|10=119|";
    /// A bridge row, keyed by name rather than by tag, with a group packed in.
    const NAMED: &str = "recv |MSGTYPE=D|SYMBOL=TTF|SIDE=1|ORDERQTY=1200|#NOPARTYIDS=1|#NOPARTYIDS[0]=PARTYID=BUYSIDE\u{4}\u{3}PARTYIDSOURCE=D\u{4}\u{3}PARTYROLE=1|";
    /// A FIXML row, whose fields are attributes.
    const FIXML: &str = r#"<FIXML><Order ClOrdID="ORDER-2" Side="1" Qty="50"/></FIXML>"#;
    /// A Jolokia read of a session interface: a JSON document rather than pairs,
    /// and a body the codec does not read - one `unknown` message, no entries.
    const PLUGIN: &str = concat!(
        r#"{"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:name=ULMSG_BROKER_TO_DMZ,"#,
        r#"plugin-type=FIX,type=Plugin","type":"read"},"value":{"SenderCompID":"ULB_BKRBDG","#,
        r#""TargetCompID":"ULB_PTBDG","BeginString":"FIX.4.2","State":"logged"},"status":200}"#,
    );
    /// A line that is not a message at all, which is still a text row.
    ///
    /// It opens no frame, holds no pair and carries no document, so it states
    /// nothing and yields no message - not even the entry-less `unknown` it used
    /// to build.
    const PROSE: &str = "no level printed by this plugin, and no pairs either";

    /// A sentence whose prose carries an `=`, which is not a pair it separated.
    ///
    /// The shape the none-one-or-many rule is named for. The run of named pairs is a bridge row
    /// only where the line named a separator for it - a pipe, a `SOH`, one of the
    /// spellings a log escapes it with, never whitespace - or the bridge marked a
    /// key with `#`. This line does neither, so `seq=7` is prose and the line
    /// states no message, where it used to state an `unknown` carrying `seq`.
    const CHATTER: &str = "heartbeat emitted seq=7 to VENUE, no reply yet";

    /// Every shape, in the order the capture holds them.
    const CAPTURE: [&str; 8] = [
        TAGGED, WORKING, FILLED, NAMED, FIXML, PLUGIN, PROSE, CHATTER,
    ];

    /// The messages the capture states: one per line but the two silent ones,
    /// and the execution the parse splits off each of the two fills (A12).
    ///
    /// `PROSE` and `CHATTER` state no message - the first holds no pair at all,
    /// the second holds one no separator was named for - so a message count is
    /// two under the line count, and `WORKING` and `FILLED` each state two: the
    /// report and its execution. A line is a text row whatever it holds, which
    /// is why `CAPTURE.len()` is what a line count is compared against and
    /// this is what a message count is.
    const MESSAGES: usize = CAPTURE.len() - 2 + 2;

    /// The capture line each message is read from, in message order.
    const LINE_OF: [usize; MESSAGES] = [0, 1, 1, 2, 2, 3, 4, 5];

    /// The capture as the bytes a log file holds.
    fn corpus() -> Vec<u8> {
        let mut bytes = Vec::new();
        for line in CAPTURE {
            bytes.extend_from_slice(line.as_bytes());
            bytes.push(b'\n');
        }
        bytes
    }

    /// A handle whose media type comes from its name, so `.log` reads as records.
    fn handle() -> Buffer {
        Buffer::from_bytes(corpus()).with_media_type(
            Url::from_str("file:///capture.log")
                .expect("a URL")
                .media_type(),
        )
    }

    /// The text options a capture is read under.
    fn text() -> RecordOptions {
        let mut options = TextOptions::new();
        options.parse_mimetype = true;
        options.into()
    }

    /// Every line the capture holds, as the text reader decodes them.
    ///
    /// The one decode entry point, which is what the line door takes: the same
    /// lines the batch path is built from, handed over rather than made again,
    /// and therefore the honest comparison.
    fn lines_of(source: &Buffer) -> Vec<TextLine> {
        let RecordOptions::Text(options) = text() else {
            panic!("a text read")
        };
        read_text_lines(source, &options)
            .expect("a line reader")
            .map(|line| line.expect("a line"))
            .collect()
    }

    /// One value as the reading it is, independent of how it is stored.
    ///
    /// A packed code and the text it holds are one reading, and a null is one
    /// reading whatever column it sits in. This is what lets the row-by-row path
    /// and the batched one be compared without comparing the Arrow boundary
    /// between them.
    fn rendered(value: &Scalar) -> Option<String> {
        match value {
            Scalar::Null => None,
            held => Some(
                held.as_str()
                    .map_or_else(|| format!("{held:?}"), ToString::to_string),
            ),
        }
    }

    /// One column of one batch, by name.
    fn column(batch: &arrow_array::RecordBatch, name: &str) -> Vec<Scalar> {
        let at = batch
            .schema()
            .index_of(name)
            .unwrap_or_else(|_| panic!("a {name} column"));
        column_at(batch, at)
    }

    /// One column of one batch, by the tag its field carries.
    fn tag_column(batch: &arrow_array::RecordBatch, tag: i32) -> Vec<Scalar> {
        column_at(batch, super::tag_index(batch, tag))
    }

    /// One column of one batch, by position.
    fn column_at(batch: &arrow_array::RecordBatch, at: usize) -> Vec<Scalar> {
        let held =
            yggdryl::Serie::from_arrow_batch(None, batch, yggdryl::ArrowCastOptions::default())
                .expect("the batch reads");
        held.rows()
            .iter()
            .map(|row| {
                row.as_sequence()
                    .expect("a row")
                    .get(at)
                    .cloned()
                    .unwrap_or(Scalar::Null)
            })
            .collect()
    }

    /// A Jolokia answer as a log line writes it: prose in front, prose behind.
    ///
    /// The shape a bridge actually prints - a timestamp, the reader that logged
    /// it, the level, then the document - with a duration written after it, which
    /// is what a reader that assumed the document ended the line never saw.
    const LOGGED: &str = concat!(
        r#"2026-08-14 06:46:22.150 [Jolokia] (DEBUG) Response: {"request":{"mbean":"#,
        r#""com.ullink.ulbridge.sessioninterfaces.plugins:name=Router_OrderRouting,"#,
        r#"plugin-type=FIX,type=Plugin","type":"read"},"value":{"Name":"Router_OrderRouting","#,
        r#""Version":"4.7.0","Category":"Fix BuySide","SenderCompID":"CLI.PROD.TRD","#,
        r#""TargetCompID":"ST.PROD","BeginString":"FIX.4.4","PrimaryHost":"172.97.127.90","#,
        r#""CurrentPort":9726,"State":"logged","Type":"I","NeedCFBReload":false,"#,
        r#""cm-extension":"4.7.0","IncomingMsgSeqNum":18336},"status":200} (12 ms)"#,
    );

    /// A wildcard read: one answer, a plugin per key, each named by its ObjectName.
    const WILDCARD: &str = concat!(
        r#"{"request": {"mbean": "com.ullink.ulbridge.sessioninterfaces.plugins:*", "type": "read"},"#,
        r#" "value": {"com.ullink.ulbridge.sessioninterfaces.plugins:name=ULMSG_BROKER_BDG_DMZ_PCO,"#,
        r#"plugin-type=FIX,type=ConfigurationPlugin": {"Comment": "", "Category": "InterBridge","#,
        r#" "Prefix": "", "Name": "ULMSG_BROKER_BDG_DMZ_PCO", "LoadIsolation": 0, "Suffix": "","#,
        r#" "PriorityLevel": 5, "Version": "2.0.3"},"#,
        r#" "com.ullink.ulbridge.sessioninterfaces.plugins:name=ULMSG_BROKER_TO_DMZ,"#,
        r#"plugin-type=FIX,type=Plugin": {"Name": "ULMSG_BROKER_TO_DMZ", "Version": "4.7.0"}},"#,
        r#" "status": 200}"#,
    );

    /// A JSON body that is not a Jolokia answer: a row's own bytes, and nothing
    /// in them a FIX reader can read.
    const STRANGER: &str = r#"{"a":1}"#;

    /// The codec every shape in this capture is read under.
    ///
    /// A capture of every dialect holds rows that state no type - a document,
    /// and a FIXML element whose name no message code spells - and those are
    /// what `DEFAULT_REFUSED_MSGTYPES` keeps out of a live session's read. A
    /// fixture written to hold them is asking for them, and says so here; what
    /// the default refuses is pinned on its own below.
    fn codec() -> FixCodec {
        super::fixed_codec(registry()).with_exclude_msgtypes::<[&str; 0], &str>([])
    }

    #[test]
    fn the_default_read_refuses_every_row_that_states_no_type() {
        let default = super::fixed_codec(registry());
        // Four of the six messages this capture states name a type. The FIXML
        // `Order` element and the Jolokia answer name none - `unknown` is what
        // the codec calls a row that states no type - and `unknown` is one of
        // the three types a codec refuses until a caller says otherwise.
        assert!(yggdryl::DEFAULT_REFUSED_MSGTYPES.contains(&"unknown"));
        assert_eq!(default.parse_lines(CAPTURE).count(), MESSAGES - 2);
        assert_eq!(codec().parse_lines(CAPTURE).count(), MESSAGES);
        for body in [FIXML, PLUGIN] {
            assert!(
                default
                    .parse_line(body.as_bytes())
                    .expect("a readable line")
                    .next()
                    .is_none(),
                "{body} states no type and is refused",
            );
            assert_eq!(
                codec()
                    .sole_line(body.as_bytes())
                    .expect("a message")
                    .header()
                    .msgtype(),
                "",
                "and states none when it is read",
            );
        }
    }

    #[test]
    fn a_mixed_capture_reads_row_by_row_and_batched_to_the_same_messages() {
        let source = handle();
        let codec = codec();

        // Line by line: the text reader answers lines, and every line is read for
        // the messages its own body spells - none, one or many - the dialect
        // chosen per line, never per capture.
        let lines = lines_of(&source);
        assert_eq!(lines.len(), CAPTURE.len(), "a line in is a line out");

        let one_at_a_time: Vec<_> = codec
            .parse_text_lines(lines)
            .map(|held| held.expect("a message"))
            .collect();
        // A message per line but the last two: the prose opens no frame, states
        // no bridge pair and carries no document, and the chatter's `seq=7` is a
        // pair the line named no separator for - so both state nothing at all.
        assert_eq!(one_at_a_time.len(), MESSAGES);
        // The document's row is the one entry-less `unknown` - a body the codec
        // does not read, stating nothing - and every other row arrived with
        // entries, the FIXML row's `Order` among them.
        for (at, held) in one_at_a_time.iter().enumerate() {
            assert_eq!(
                held.entries().is_empty(),
                CAPTURE[LINE_OF[at]] == PLUGIN,
                "row {at}"
            );
        }

        // Batched: the same capture through the batch door, which is the same
        // read with the rows held in Arrow instead of one at a time.
        let batched: Vec<_> = codec
            .parse_text_arrow_reader(source.read_arrow_reader(&text()).expect("a reader"))
            .expect("the batch reader opens")
            .map(|batch| batch.expect("a batch"))
            .collect();
        let rows: usize = batched.iter().map(arrow_array::RecordBatch::num_rows).sum();
        assert_eq!(rows, MESSAGES, "the batch path reads the same messages");
        // One batch, because the capture is far under the 128 MiB target.
        assert_eq!(batched.len(), 1);

        // The two paths agree on what each line said. They are compared on the
        // reading rather than on the storage: a batch value has crossed the Arrow
        // boundary and a message's has not, so logical values may differ in
        // physical representation while retaining the same declared reading.
        let batch = &batched[0];
        for tag in [11, 55, 37, 17, 39, 151, 6, 120] {
            let column = tag_column(batch, tag);
            for (at, message) in one_at_a_time.iter().enumerate() {
                let alone = message.get_by_tag(tag).unwrap_or(Scalar::Null);
                assert_eq!(
                    rendered(&alone),
                    rendered(&column[at]),
                    "tag {tag} on row {at} differs between the two paths",
                );
            }
        }

        // The batch retains only arrival content the fixed columns cannot state;
        // the typed-column checks above prove represented facts arrive identically.
        let entries = column(batch, "fixentries");
        assert_eq!(entries.len(), one_at_a_time.len());
        assert!(entries.iter().all(|entry| entry.as_mapping().is_some()));

        // Which way a message moved is FIX's own tag 385, a code of its set,
        // retained once per row and shared by every message that row states.
        let directions = tag_column(batch, 385);
        assert_eq!(directions.len(), MESSAGES);
        // The bridge row wrote `recv` in front of its frame, and a verb the
        // transport wrote wins over everything else; a bare document states
        // nothing of which way it moved, so the batch door's own pin fills it.
        assert_eq!(directions[5].as_str(), Some("R"));
        assert_eq!(directions[7].as_str(), Some("S"));
    }

    #[test]
    fn every_dialect_in_one_capture_is_read_as_itself() {
        let codec = codec();

        // A framed order behind prose: the frame is located and the prose dropped.
        let order = codec.sole_line(TAGGED.as_bytes()).expect("an order");
        assert_eq!(order.by_tag(11).unwrap().as_str(), Some("ORDER-1"));
        assert_eq!(order.by_tag(55).unwrap().as_str(), Some("AAPL"));

        // A bridge row keyed by name, with its group packed into one occurrence.
        let bridge = codec.sole_line(NAMED.as_bytes()).expect("a bridge row");
        assert_eq!(bridge.by_tag(55).unwrap().as_str(), Some("TTF"));
        let group = bridge
            .by_name("parties")
            .expect("the group the counter heads");
        let parties = group.as_sequence().expect("a serie");
        assert_eq!(parties.len(), 1);
        let party = parties[0].as_sequence().expect("one occurrence");
        assert!(
            party.iter().any(|held| held.as_str() == Some("BUYSIDE")),
            "the occurrence carries the members it packed",
        );
        // What makes that run of named keys a bridge row rather than prose
        // carrying an `=` is the separator the line named for it - the pipe - and
        // the `#` the bridge marked its counter with. Named neither way, the same
        // shape of run is prose: whitespace names no separator, so the line states
        // no message at all.
        let paired = codec
            .sole_line(b"ACCOUNT=A1|SIDE=1")
            .expect("a bridge row the pipe separated");
        assert_eq!(paired.by_tag(1).unwrap().as_str(), Some("A1"));
        assert!(
            codec
                .parse_line(b"After Enrichment -> ACCOUNT=A1 CLIENTID=B2")
                .expect("a readable line")
                .next()
                .is_none(),
            "a run of named pairs the line separated with whitespace is prose",
        );

        // A FIXML row, whose fields are attributes rather than pairs.
        let fixml = codec.sole_line(FIXML.as_bytes()).expect("a FIXML row");
        assert_eq!(fixml.by_tag(11).unwrap().as_str(), Some("ORDER-2"));
        // The same document behind a transport's prose, with whitespace either
        // side: the document opens where the tag opens, whatever was trimmed off
        // the line's end.
        let prosed = format!("  Sending : {FIXML}  \t");
        let behind = codec
            .sole_line(prosed.as_bytes())
            .expect("a FIXML row behind prose");
        assert_eq!(behind.by_tag(11).unwrap().as_str(), Some("ORDER-2"));
        assert_eq!(behind.by_tag(54).unwrap(), fixml.by_tag(54).unwrap());
        assert_eq!(behind.entries().len(), fixml.entries().len());

        // A line that is not a message states no message at all - never an error:
        // it opens no frame, states no bridge pair and carries no document, so
        // there is nothing in it to read. It reads without failing, which is the
        // fact that matters: one such line must not end a run over ten million.
        for line in [PROSE, CHATTER] {
            assert!(
                codec
                    .parse_line(line.as_bytes())
                    .expect("a readable line")
                    .next()
                    .is_none(),
                "{line:?} states no message",
            );
        }
    }

    #[test]
    fn a_parse_fills_the_columns_and_the_wire_re_emits_them() {
        let codec = codec();

        // The part-filled report states what was ordered and what was done, so it
        // has stated what is left and what the fill was worth - filled as it is
        // read, with no second pass.
        let filled = codec.sole_line(WORKING.as_bytes()).expect("a report");
        assert_eq!(filled.by_tag(151).unwrap(), super::decimal("60"));
        assert_eq!(filled.by_tag(381).unwrap(), super::decimal("420"));

        // A date arrives compact and reads as that day's midnight, stating no zone
        // because a local market date has none.
        assert_ne!(filled.by_tag(64).expect("a settlement date"), Scalar::Null);

        // The closing fill settles in the currency it was dealt in, at the rate it
        // stated - Appendix O read as the implication it is.
        let closed = codec.sole_line(FILLED.as_bytes()).expect("a report");
        assert_eq!(closed.by_tag(151).unwrap(), super::decimal("0"));
        assert_eq!(closed.by_tag(120).unwrap().as_str(), Some("EUR"));

        // What was derived is the message's, so the wire carries it beside what
        // arrived: a re-emitted report states the leaves neither line stated.
        // `LeavesQty(151)` is one of the event's own facts rather than a child
        // of the content row, so it is on the wire without being an entry.
        for line in [WORKING, FILLED] {
            let held = codec.sole_line(line.as_bytes()).expect("a report");
            assert!(
                !held.entries().iter().any(|entry| entry.tag() == 151),
                "{line}"
            );
            let wire = held.into_text('|').unwrap();
            assert!(
                wire.starts_with("8=FIX.4.4|35=8|49=VENUE|56=BUYSIDE|"),
                "{wire}"
            );
            assert!(wire.contains("|151="), "{wire}");
        }
    }

    #[test]
    fn a_document_is_one_unknown_row_at_every_door() {
        // The classifier reads past the prose in front and answers
        // `application/json`, which is what the document is, and it names no
        // message type for it: what a document says is not read. The codec
        // locates the document to its own close rather than to the end of the
        // line, so what a transport writes behind it is prose too, and the row
        // is one `unknown` message - no entries, no type - carrying what the
        // prose in front stated: the half `Response:` names.
        assert_eq!(
            yggdryl::MimeType::infer_bytes(LOGGED.as_bytes()),
            yggdryl::MimeType::JSON
        );
        assert_eq!(FixCodec::infer_msgtype_bytes(LOGGED.as_bytes()), None);

        let codec = codec();
        let message = codec
            .sole_line(LOGGED.as_bytes())
            .expect("the line carries one document, and so one message");
        assert_eq!(message.as_field().name(), "unknown");
        assert!(message.entries().is_empty());
        assert!(message.get_by_tag(35).is_none_or(|held| held.is_null()));
        assert!(
            message
                .get_by_name("SenderCompID")
                .is_none_or(|held| held.is_null())
        );
        assert_eq!(message.by_tag(385).unwrap().as_str(), Some("R"));

        // A stranger's `{"a":1}` answers exactly what a Jolokia answer does, at
        // the byte door and at the line door alike: one message stating no type,
        // no entries, and nothing on the wire but the version every built
        // message states.
        let line = TextLine::from_bytes(
            0,
            TextBytes::from_bytes(STRANGER.as_bytes()).unwrap(),
            std::sync::Arc::new(yggdryl::text::TextOptions::new()),
        )
        .unwrap();
        let unknown = |message: yggdryl::FixMsg, door: &str| {
            assert_eq!(message.as_field().name(), "unknown", "{door}");
            assert!(message.entries().is_empty(), "{door}");
            assert_eq!(message.into_bytes(b'|'), b"8=FIX.4.4|", "{door}");
            assert!(
                message.get_by_tag(35).is_none_or(|held| held.is_null()),
                "{door}"
            );
        };
        unknown(
            codec
                .sole_line(STRANGER.as_bytes())
                .expect("the byte door reads one message"),
            "the byte door",
        );
        unknown(
            super::sole_message(codec.parse_text_line(&line).expect("a readable row"))
                .expect("the line door reads one message"),
            "the line door",
        );

        // A wildcard read answers for two plugins, and the row is one message all
        // the same: a document is not read, so there is no per-plugin expansion.
        let wildcard = codec
            .sole_line(WILDCARD.as_bytes())
            .expect("one message, not one per plugin");
        assert_eq!(wildcard.as_field().name(), "unknown");
        assert!(wildcard.get_by_name("Name").is_none());

        // And on the batch door a row that carried a document is a row carrying
        // its own source columns, exactly as it is at the other two.
        let field = yggdryl::StructType::from_fields([
            yggdryl::DataType::utf8().required_field("url"),
            yggdryl::DataType::Int64.required_field("rownum"),
            yggdryl::DataType::binary().required_field("body"),
        ])
        .map(yggdryl::DataType::from)
        .unwrap()
        .required_field("capture");
        let value = Scalar::from_sequence([(41_i64, WILDCARD), (42_i64, WORKING)].map(
            |(rownum, body)| {
                Scalar::from_sequence([
                    Scalar::from("file:///bulk.log"),
                    Scalar::from(rownum),
                    Scalar::from(body.as_bytes().to_vec()),
                ])
            },
        ));
        let rows = value.as_sequence().expect("a run of rows").to_vec();
        let source = yggdryl::Serie::from_scalars(field, rows)
            .unwrap()
            .into_arrow_batch()
            .unwrap();
        let batch = codec
            .parse_text_arrow_reader(yggdryl::arrow::batch_reader(source.schema(), [source]))
            .expect("the batch door opens")
            .next()
            .expect("one batch")
            .expect("a batch");
        // The fill's row is its report and the execution its parse splits off
        // (A12), each carrying the row's own cells.
        assert_eq!(batch.num_rows(), 3);
        assert_eq!(
            column(&batch, "rownum"),
            [
                Scalar::from(41_i64),
                Scalar::from(42_i64),
                Scalar::from(42_i64)
            ]
        );
        assert_eq!(
            column(&batch, "url"),
            vec![Scalar::from("file:///bulk.log"); 3]
        );
        assert_eq!(
            column(&batch, "body"),
            [
                Scalar::from(WILDCARD.as_bytes().to_vec()),
                Scalar::from(WORKING.as_bytes().to_vec()),
                Scalar::from(WORKING.as_bytes().to_vec()),
            ]
        );
        assert_eq!(
            tag_column(&batch, 35),
            [Scalar::Null, Scalar::from("8"), Scalar::from("8")]
        );
    }

    #[test]
    fn a_json_document_is_one_unknown_and_any_other_unreadable_body_is_none() {
        // Reading is not refusing. Every one of these is a body a row really
        // carried, and none has a `Result` left to unwrap. What makes a body a
        // document is its shape alone: an object, or an array of objects,
        // opening before any `=` on the line and closing on its own last byte.
        // Such a body is one `unknown`, whatever it says - an error-only
        // answer, a request not yet answered, a wildcard that selected nothing
        // and a bulk answer of one object alike.
        let codec = codec();
        for body in [
            STRANGER,
            r#"[{"request":{"mbean":"com.ullink.ulbridge:type=Bridge","type":"read"}},2]"#,
            r#"{"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:name=Gone,plugin-type=FIX,type=Plugin","type":"read"},"error":"InstanceNotFoundException","status":404}"#,
            r#"{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:*","type":"read"}"#,
            r#"{"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:*","type":"read"},"value":{},"status":200}"#,
        ] {
            let message = codec
                .sole_line(body.as_bytes())
                .unwrap_or_else(|error| panic!("{body:?}: {error}"));
            assert_eq!(message.as_field().name(), "unknown", "{body:?}");
            assert!(message.entries().is_empty(), "{body:?}");
        }
        // Everything else JSON could spell is no document, and a body that
        // opens a brace without a member is not JSON at all: each opens no
        // frame, states no bridge pair and carries no document, so the row
        // states no message - never an error.
        for body in [
            "null",
            "true",
            "1",
            r#""text""#,
            "[]",
            "[1]",
            "{not json at all",
        ] {
            assert!(
                codec
                    .parse_line(body.as_bytes())
                    .unwrap_or_else(|error| panic!("{body:?}: {error}"))
                    .next()
                    .is_none(),
                "{body:?} states a message",
            );
        }
    }

    #[test]
    fn a_capture_writes_back_what_each_message_emits() {
        let source = handle();
        let codec = codec().with_separator(b'|');
        let messages = codec
            .parse_lines(CAPTURE)
            .map(|message| message.expect("a source message"))
            .collect::<Vec<_>>();
        assert_eq!(messages.len(), MESSAGES);

        let reader = codec
            .parse_text_arrow_reader(source.read_arrow_reader(&text()).expect("a reader"))
            .expect("the batch reader opens");

        // The wire is rebuilt from each row's arrival record and the facts it
        // holds typed, which is what the line read emits for the same line.
        let mut written: Vec<u8> = Vec::new();
        let rows = codec
            .write_arrow_reader(reader, &mut written)
            .expect("the capture writes");
        // One line written per message, and the two lines that state none reach
        // the writer as no row at all, so the capture comes back two lines
        // shorter than it went in.
        assert_eq!(rows, MESSAGES as u64);

        let held = String::from_utf8(written).expect("the wire is text here");
        let lines: Vec<&str> = held.lines().collect();
        assert_eq!(lines.len(), MESSAGES);
        let target = super::format_target(codec.registry());
        let group_columns = target
            .fields()
            .iter()
            .enumerate()
            .filter_map(|(at, field)| {
                FixField::new(field)
                    .counter()
                    .expect("valid FIX metadata")
                    .filter(|counter| {
                        !(yggdryl::CRATE_TAG_MIN..=yggdryl::CRATE_TAG_MAX).contains(counter)
                    })
                    .map(|_| (at, field.name()))
            })
            .collect::<Vec<_>>();
        assert!(
            !group_columns.is_empty(),
            "the fixed row has protocol groups"
        );
        for (at, (message, line)) in messages.iter().zip(&lines).enumerate() {
            let emitted = message.into_text('|').expect("the source message emits");
            let mut source_tokens = emitted
                .split('|')
                .filter(|token| !token.is_empty())
                .collect::<Vec<_>>();
            let mut written_tokens = line
                .split('|')
                .filter(|token| !token.is_empty())
                .collect::<Vec<_>>();
            source_tokens.sort_unstable();
            written_tokens.sort_unstable();
            assert_eq!(written_tokens, source_tokens, "row {at} changed a token");

            // Root fields may move into schema order, but header/trailer bands and
            // every repeating group's member and occurrence order remain wire
            // structure rather than a token-set property.
            if emitted.contains("|10=") {
                assert!(line.starts_with("8="), "row {at}: {line}");
                assert!(
                    line.rsplit('|')
                        .nth(1)
                        .is_some_and(|token| token.starts_with("10=")),
                    "row {at}: {line}"
                );
            }
            let reparsed = codec
                .sole_line(line.as_bytes())
                .unwrap_or_else(|error| panic!("row {at} did not parse: {error}"));
            let source_row = message.into_row(&target).expect("the source row");
            let reparsed_row = reparsed.into_row(&target).expect("the reparsed row");
            let source_cells = source_row.as_sequence().expect("source columns");
            let reparsed_cells = reparsed_row.as_sequence().expect("reparsed columns");
            for (column, name) in &group_columns {
                assert_eq!(
                    reparsed_cells[*column], source_cells[*column],
                    "row {at} changed {name} occurrence order"
                );
            }
        }
        for silent in [PROSE, CHATTER] {
            assert!(
                !lines.contains(&silent),
                "a line that stated no message writes back none",
            );
        }
        // And the capture's own columns are the capture's: a row read back off
        // a capture states the message, never the body it was read from.
        for line in &lines {
            assert!(!line.contains("|body="), "{line}");
            assert!(!line.contains("|url="), "{line}");
        }
    }
}
