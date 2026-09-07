//! One Ullink CBlock configuration, read into a dictionary and message roots.

use std::path::PathBuf;

use std::sync::Arc;
use yggdryl::holder::local::Folder;

use yggdryl::holder::Buffer;
use yggdryl::holder::fs::{File, FileSystem, MemoryFileSystem};
use yggdryl::{DataType, Error, Field, FixBranch, FixField, FixRegistry, IOBase, Version};

/// A CBlock in the exact shape a production file has: the same element order,
/// the same attribute order, the same escaping, the same self-closing forms.
const CBLOCK: &str = r#"<?xml version="1.0" encoding="US-ASCII"?>
<cplugin-configuration type="com.ullink.ulbridge2.toolkit.plugins.fix.model.state.cblock.BuySideFIXCPluginCBlock" version="1.2" logs="false" date="Fri 2019-09-13 13:07:00" fix-version="4.4" targetcompid="BLPFIX" sendercompid="OURDESK">
	<history>
		<version date="2006-09-06 11:46:49" owner="ullink">First version released by ULLINK</version>
		<version date="2006-09-22 16:41:44" owner="admin">3 main changes:
* change mapping SettlType and add 40=&gt;spot, 54=&gt;forward
* change type of tag 63 from char to String</version>
		<version date="2018-10-22 11:41:25" owner="fcolombat" />
	</history>
	<cvs-revision>$Revision: 1.3 $</cvs-revision>
	<description>Standard Buy Side 4.4</description>
	<message-types>
		<message-type value="P Report Ack" description="Allocation Report ACK" rejection="not supported" supported="false" />
	</message-types>
	<inbound-message-type-mappings />
	<outbound-message-type-mappings>
		<entry key="allocationreportack" value="P Report Ack" />
	</outbound-message-type-mappings>
	<vocabulary>
		<vocabulary-tag name="8" alt="BeginString" type="string" read-only="true" />
		<vocabulary-tag name="9" alt="BodyLength" type="integer" read-only="true" />
		<vocabulary-tag name="35" alt="MsgType" type="string" read-only="true" />
		<vocabulary-tag name="6" alt="AvgPx" type="float" read-only="false">
			<description>Calculated average price of all fills, with a trailing &lt;SOH&gt; and a &quot;quoted&quot; word.</description>
		</vocabulary-tag>
		<vocabulary-tag name="10001" alt="ExludedDealers" type="boolean" read-only="false">
			<description />
		</vocabulary-tag>
		<vocabulary-tag name="10015" alt="DealerParQuote" type="utc-date" />
		<vocabulary-tag name="22830" type="string" />
		<vocabulary-tag name="555" alt="NoLegs" type="integer" />
		<vocabulary-tag name="556" alt="LegCurrency" type="string" />
		<vocabulary-tag name="604" alt="NoLegSecurityAltID" type="integer" />
		<vocabulary-tag name="605" alt="LegSecurityAltID" type="string" />
		<vocabulary-tag name="60" alt="TransactTime" type="utc-timestamp" />
		<vocabulary-tag name="273" alt="MDEntryTime" type="utc-time-only" />
		<vocabulary-tag name="59" alt="TimeInForce" type="char" />
		<vocabulary-tag name="4" alt="AdvSide" type="char" />
	</vocabulary>
	<grammar-binding type="7">
		<grammar checkordering="false">
			<tag-constraint name="8" activated="false" read-only="true" part="header" required="true">
				<string-validity regexp=".*" domain="all-values" />
			</tag-constraint>
			<tag-constraint name="9" activated="false" read-only="true" part="header" required="true">
				<integer-validity domain="all-values">
					<integer-range min="minimum" max="maximum" />
				</integer-validity>
			</tag-constraint>
			<tag-constraint name="35" activated="true" read-only="true" part="header" required="true">
				<string-validity regexp="^7$" domain="ranges" />
			</tag-constraint>
			<tag-constraint name="59" activated="true" read-only="false" part="body" required="$59 = '6' and empty($126)">
				<string-validity regexp=".*" domain="all-values" />
			</tag-constraint>
			<tag-constraint name="6" part="body" />
			<grammar checkordering="false">
				<tag-constraint name="555" activated="true" read-only="false" part="body" required="false">
					<integer-validity domain="ranges">
						<integer-range min="1" max="maximum" />
					</integer-validity>
				</tag-constraint>
				<tag-constraint name="556" activated="true" read-only="false" part="body" required="true">
					<string-validity regexp=".*" domain="all-values" />
				</tag-constraint>
				<grammar checkordering="false">
					<tag-constraint name="604" activated="true" read-only="false" part="body" required="false" />
					<tag-constraint name="605" activated="true" read-only="false" part="body" required="true" />
				</grammar>
			</grammar>
			<tag-constraint name="8" part="trailer" required="false" />
		</grammar>
	</grammar-binding>
	<maps>
		<map name="ADVSIDE" read-only="false">
			<description>Used for the decoding of UlMessage tag ADVSIDE</description>
			<entries>
				<entry key="buy" value="B" />
				<entry key="cross" value="X" />
				<entry key="sell" value="S" />
				<entry key="trade" value="T" />
			</entries>
		</map>
		<map name="TimeInForce" read-only="false">
			<entries>
				<entry key="0" value="day" />
				<entry key="1" value="goodtillcancel" />
			</entries>
		</map>
		<map name="NOTAFIELD" read-only="false">
			<entries><entry key="a" value="1" /></entries>
		</map>
	</maps>
	<normalization-binding>
		<normalization type="7">
			<rule from="$35" to="$35" />
		</normalization>
	</normalization-binding>
	<reject-binding />
	<flow-filter-binding>
		<flow-filters flowType="outbound" />
	</flow-filter-binding>
	<maps />
	<options />
</cplugin-configuration>
"#;

/// A file whose maps are named both ways round, over entries a rule reading
/// the shorter side as the wire value would orient backwards.
const ORIENTATIONS: &str = r#"<?xml version="1.0" encoding="US-ASCII"?>
<cplugin-configuration type="com.ullink.ulbridge2.toolkit.plugins.fix.model.state.cblock.BuySideFIXCPluginCBlock" version="1.2" fix-version="4.4" targetcompid="BLPFIX" sendercompid="OURDESK">
	<vocabulary>
		<vocabulary-tag name="167" alt="SecurityType" type="string" />
		<vocabulary-tag name="310" alt="UnderlyingSecurityType" type="string" />
		<vocabulary-tag name="22830" type="string" />
	</vocabulary>
	<maps>
		<map name="SecurityType" read-only="false">
			<entries>
				<entry key="FXSPOT" value="fx" />
				<entry key="CS" value="equity" />
			</entries>
		</map>
		<map name="UNDERLYINGSECURITYTYPE" read-only="false">
			<entries>
				<entry key="fx" value="FXSPOT" />
				<entry key="equity" value="CS" />
			</entries>
		</map>
		<map name="22830" read-only="false">
			<entries><entry key="1" value="one" /></entries>
		</map>
	</maps>
</cplugin-configuration>
"#;

/// A file that stresses what a map's name reaches and what one `entry` may
/// say: a spelling an earlier field folds onto, whitespace an editor left
/// behind, an entry stating nothing, and one repeating what another claimed.
const AWKWARD: &str = r#"<?xml version="1.0" encoding="US-ASCII"?>
<cplugin-configuration type="com.ullink.ulbridge2.toolkit.plugins.fix.model.state.cblock.BuySideFIXCPluginCBlock" version="1.2" fix-version="4.4" targetcompid="BLPFIX" sendercompid="OURDESK">
	<vocabulary>
		<vocabulary-tag name="100" alt="Ex_Destination" type="string" />
		<vocabulary-tag name="20000" alt="ExDestination" type="string" />
		<vocabulary-tag name="59" alt="TimeInForce" type="char" />
		<vocabulary-tag name="4" alt="AdvSide" type="char" />
	</vocabulary>
	<maps>
		<map name="ExDestination" read-only="false">
			<entries><entry key="XPAR" value="paris" /></entries>
		</map>
		<map name=" TimeInForce " read-only="false">
			<entries>
				<entry key="0" value="day" />
				<entry key="1" value="day" />
				<entry key="2" value="" />
				<entry value="atthecrossing" />
				<entry key="6" value="goodtilldate" />
			</entries>
		</map>
		<map name="ADVSIDE" read-only="false">
			<entries>
				<entry key="buy" value="B" />
				<entry key="bid" value="B" />
			</entries>
		</map>
	</maps>
</cplugin-configuration>
"#;

/// A second counterparty's file: one tag both declare, one only this one does.
const OVERLAY: &str = r#"<?xml version="1.0" encoding="US-ASCII"?>
<cplugin-configuration type="com.ullink.ulbridge2.toolkit.plugins.fix.model.state.cblock.SellSideFIXCPluginCBlock" version="1.2" fix-version="4.4" targetcompid="OURDESK" sendercompid="MSFIX">
	<vocabulary>
		<vocabulary-tag name="6" alt="AvgPx" type="float" />
		<vocabulary-tag name="44" alt="Price" type="float" />
	</vocabulary>
</cplugin-configuration>
"#;

/// The same file written by the other side of the session.
const SELLSIDE: &str = r#"<?xml version="1.0" encoding="US-ASCII"?>
<cplugin-configuration type="com.ullink.ulbridge2.toolkit.plugins.fix.model.state.cblock.SellSideFIXCPluginCBlock" version="1.2" fix-version="4.4" targetcompid="OURDESK" sendercompid="BLPFIX">
	<vocabulary>
		<vocabulary-tag name="35" alt="MsgType" type="string" />
	</vocabulary>
</cplugin-configuration>
"#;

/// One document behind a handle, the way the store cases build them.
fn handle(body: &str) -> impl IOBase {
    named_handle(body, "one.cfb")
}

/// The same, under a chosen file name, for the cases that read the stem.
fn named_handle(body: &str, name: &str) -> impl IOBase {
    let filesystem: Arc<dyn FileSystem> = Arc::new(MemoryFileSystem::new());
    // A memory filesystem starts empty, where a real bucket already exists.
    filesystem
        .create_dir("cblock", true)
        .expect("a container to write into");
    let mut file =
        File::from_path(filesystem, format!("cblock/{name}"), None).expect("a path under it");
    file.write_all_bytes(body.as_bytes()).expect("the document");
    file
}

fn branch() -> FixBranch {
    FixBranch::from_str("bloomberg").expect("a valid branch name")
}

fn parse(body: &str) -> (FixRegistry, Vec<Field>) {
    FixRegistry::from_cfb(&handle(body), Some(&branch())).expect("a readable CBlock")
}

/// One root's children by name, in document order.
fn children(root: &Field) -> Vec<&str> {
    root.dtype()
        .as_fields()
        .expect("a struct root")
        .iter()
        .map(yggdryl::Field::name)
        .collect()
}

#[test]
fn the_vocabulary_becomes_a_dictionary_of_lower_cased_names() {
    let (registry, _) = parse(CBLOCK);
    assert_eq!(registry.len(), 15);

    // Named by `alt` lower-cased, with the file's own spelling kept beside it,
    // so a caller spelling it the file's way still resolves.
    let field = registry.field_by_tag(6).expect("AvgPx");
    assert_eq!(field.name(), "avgpx");
    // Tag 6 is FIX's, not a venue's: a dialect claims only the user-defined
    // range, so a CBlock's standard tags land in the standard branch and
    // resolve there.
    assert!(registry.get_field_by_name("AvgPx", None).is_some());
    assert!(
        registry
            .get_field_by_name("ExludedDealers", Some(&branch()))
            .is_some(),
        "a custom tag is the named dialect's",
    );

    // The description's entities are unescaped, never kept as opaque bytes.
    let described = field
        .as_fix()
        .description()
        .expect("a description")
        .to_owned();
    assert!(described.contains("<SOH>"), "{described}");
    assert!(described.contains("\"quoted\""), "{described}");

    // `<description />` contributes no key rather than an empty one.
    let empty = registry.field_by_tag(10001).expect("ExludedDealers");
    assert_eq!(empty.as_fix().description(), None);

    // A missing `alt` falls back to the tag rendered as text.
    assert_eq!(registry.field_by_tag(22830).unwrap().name(), "22830");
}

#[test]
fn the_eight_types_resolve_through_the_schema_grammars_own_names() {
    let (registry, _) = parse(CBLOCK);
    for (tag, dtype) in [
        (35, DataType::Utf8),
        (59, DataType::Utf8),
        (9, DataType::Int32),
        (6, DataType::Float32),
        (10001, DataType::Boolean),
        (10015, DataType::Date32),
        (273, DataType::Time64(yggdryl::TimeUnit::Nanosecond)),
    ] {
        assert_eq!(
            registry.field_by_tag(tag).unwrap().dtype(),
            &dtype,
            "tag {tag}"
        );
    }
    // `utc-timestamp` is the one that carries a zone.
    assert!(matches!(
        registry.field_by_tag(60).unwrap().dtype(),
        DataType::DateTime64 { .. }
    ));
}

#[test]
fn a_grammar_becomes_one_root_flattened_across_part() {
    let (_, roots) = parse(CBLOCK);
    assert_eq!(roots.len(), 1);
    let root = &roots[0];

    // Named by its MsgType verbatim, and non-null.
    assert_eq!(root.name(), "7");
    assert!(!root.is_nullable());

    // Document order, `part` dropped: a header constraint and a body one sit
    // as siblings, and the trailer's duplicate tag 8 is kept under a suffix
    // rather than deduplicated or refused.
    assert_eq!(
        children(root),
        [
            "beginstring",
            "bodylength",
            "msgtype",
            "timeinforce",
            "avgpx",
            "nolegs",
            "beginstring2"
        ],
    );

    // The duplicate keeps the tag, which is what recovers it.
    let fields = root.dtype().as_fields().unwrap();
    assert_eq!(fields[0].as_fix().tag().unwrap(), Some(8));
    assert_eq!(fields[6].as_fix().tag().unwrap(), Some(8));
}

#[test]
fn required_decides_nullability_and_an_expression_counts_as_absent() {
    let (_, roots) = parse(CBLOCK);
    let fields = roots[0].dtype().as_fields().unwrap();

    // `required="true"` is a value that must be there.
    assert!(!fields[0].is_nullable(), "beginstring");
    assert!(!fields[2].is_nullable(), "msgtype");
    // A condition expression is read as not-required rather than as a failure:
    // a conditionally required field is one that may be absent.
    assert!(fields[3].is_nullable(), "timeinforce");
    // An absent `required` defaults to nullable too.
    assert!(fields[4].is_nullable(), "avgpx");

    // The dictionary is untouched by any of it.
    let (registry, _) = parse(CBLOCK);
    assert!(registry.field_by_tag(8).unwrap().is_nullable());
}

#[test]
fn a_nested_grammar_is_a_group_whose_counter_names_it_and_is_consumed() {
    let (_, roots) = parse(CBLOCK);
    let fields = roots[0].dtype().as_fields().unwrap();
    let group = fields.iter().find(|held| held.name() == "nolegs").unwrap();

    // The group takes the counter's name and tag; the counter's own integer
    // type is gone, because a list's length already carries it.
    assert_eq!(group.as_fix().tag().unwrap(), Some(555));
    let DataType::List(item) = group.dtype() else {
        panic!("a list, got {}", group.dtype());
    };
    assert_eq!(item.name(), "item");
    assert!(!item.is_nullable());

    // Everything after the counter, in document order, by the same rules.
    let members = item.dtype().as_fields().expect("an item struct");
    assert_eq!(
        members.iter().map(yggdryl::Field::name).collect::<Vec<_>>(),
        ["legcurrency", "nolegsecurityaltid"],
    );
    assert!(!members[0].is_nullable(), "556 is required");

    // And it recurses: a group inside a group.
    let DataType::List(inner) = members[1].dtype() else {
        panic!("a nested list, got {}", members[1].dtype());
    };
    assert_eq!(members[1].as_fix().tag().unwrap(), Some(604));
    assert_eq!(
        inner
            .dtype()
            .as_fields()
            .unwrap()
            .iter()
            .map(yggdryl::Field::name)
            .collect::<Vec<_>>(),
        ["legsecurityaltid"],
    );
}

#[test]
fn the_root_element_is_the_branch_record() {
    let (registry, _) = parse(CBLOCK);
    let held = registry
        .branch_named("bloomberg")
        .expect("the named branch");
    assert_eq!(held.version(), "4.4".parse::<Version>().unwrap());
    assert_eq!(held.sender_comp_id(), "OURDESK");
    assert_eq!(held.target_comp_id(), "BLPFIX");

    // The same session written from the other side declares the pair
    // reversed, and both are matched because a reader tries both orders.
    let (other, _) = FixRegistry::from_cfb(&handle(SELLSIDE), Some(&branch())).unwrap();
    let reversed = other.branch_named("bloomberg").expect("the named branch");
    assert_eq!(reversed.sender_comp_id(), "BLPFIX");
    assert_eq!(reversed.target_comp_id(), "OURDESK");
    assert!(registry.branch_for_session("OURDESK", "BLPFIX").is_some());
    assert!(other.branch_for_session("BLPFIX", "OURDESK").is_some());

    // A file parsed with no branch lands in the standard branch, which is
    // right for one read only for its vocabulary.
    let (standard, _) = FixRegistry::from_cfb(&handle(CBLOCK), None).unwrap();
    assert!(standard.field_by_tag(6).is_ok());
    assert!(standard.branch_named("bloomberg").is_none());
}

#[test]
fn merging_a_cblock_vocabulary_replaces_on_an_identity_match() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("config")
        .join("fix");
    let folder = Folder::new(root).expect("the seed folder");
    let mut seeded = FixRegistry::from_handle(&folder).expect("the committed dictionary");
    // The committed dictionary spells tag 6 as money.
    assert!(matches!(
        seeded.field_by_tag(6).unwrap().dtype(),
        DataType::Decimal128 { .. } | DataType::Decimal64 { .. } | DataType::Float64
    ));

    let (vocabulary, _) = FixRegistry::from_cfb(&handle(CBLOCK), None).unwrap();
    let avgpx = vocabulary.field_by_tag(6).unwrap().clone();
    seeded.insert(avgpx).expect("same tag, same name replaces");
    // The phase's principal known loss, stated rather than hidden: a CBlock
    // says nothing about which tag is money, so the generic answer wins.
    assert_eq!(seeded.field_by_tag(6).unwrap().dtype(), &DataType::Float32);

    // Same tag, a different name, is a conflict rather than a silent replace.
    let mut renamed = vocabulary.field_by_tag(35).unwrap().clone();
    renamed.set_name("somethingelse");
    assert!(seeded.insert(renamed).is_err());
}

#[test]
fn the_structural_exceptions_are_a_statement_or_a_named_refusal() {
    // An empty root grammar is an empty non-null struct rather than a failure.
    let empty = r#"<?xml version="1.0"?>
<cplugin-configuration fix-version="4.4">
	<vocabulary><vocabulary-tag name="35" alt="MsgType" type="string" /></vocabulary>
	<grammar-binding type="0"><grammar /></grammar-binding>
</cplugin-configuration>"#;
    let (_, roots) = parse(empty);
    assert_eq!(roots.len(), 1);
    assert!(children(&roots[0]).is_empty());
    assert!(!roots[0].is_nullable());

    // A nested grammar with no counter is refused, and the refusal names the
    // position rather than a line of prose.
    let counterless = r#"<?xml version="1.0"?>
<cplugin-configuration fix-version="4.4">
	<vocabulary><vocabulary-tag name="35" alt="MsgType" type="string" /></vocabulary>
	<grammar-binding type="0"><grammar><grammar /></grammar></grammar-binding>
</cplugin-configuration>"#;
    let refused = FixRegistry::from_cfb(&handle(counterless), None).unwrap_err();
    assert!(refused.to_string().contains("counter"), "{refused}");

    // A constraint naming a tag the vocabulary does not have is a genuine
    // error: no dangling reference exists in any real file.
    let dangling = r#"<?xml version="1.0"?>
<cplugin-configuration fix-version="4.4">
	<vocabulary><vocabulary-tag name="35" alt="MsgType" type="string" /></vocabulary>
	<grammar-binding type="0"><grammar><tag-constraint name="99" part="body" /></grammar></grammar-binding>
</cplugin-configuration>"#;
    let refused = FixRegistry::from_cfb(&handle(dangling), None).unwrap_err();
    assert!(refused.to_string().contains("99"), "{refused}");
}

#[test]
fn a_bad_type_a_bad_domain_and_a_malformed_file_each_name_their_position() {
    for (body, wanted) in [
        (
            r#"<?xml version="1.0"?>
<cplugin-configuration fix-version="4.4">
	<vocabulary><vocabulary-tag name="35" alt="MsgType" type="decimal" /></vocabulary>
</cplugin-configuration>"#,
            "decimal",
        ),
        (
            r#"<?xml version="1.0"?>
<cplugin-configuration fix-version="4.4">
	<vocabulary><vocabulary-tag name="35" alt="MsgType" type="string" /></vocabulary>
	<grammar-binding type="0"><grammar><tag-constraint name="35" part="body">
		<string-validity regexp=".*" domain="some-values" />
	</tag-constraint></grammar></grammar-binding>
</cplugin-configuration>"#,
            "some-values",
        ),
        (
            r#"<?xml version="1.0"?>
<cplugin-configuration fix-version="4.4">
	<vocabulary><vocabulary-tag name="35"</vocabulary>
</cplugin-configuration>"#,
            "",
        ),
    ] {
        let refused = FixRegistry::from_cfb(&handle(body), None).unwrap_err();
        let rendered = refused.to_string();
        assert!(rendered.contains("cfb"), "{rendered}");
        assert!(rendered.contains(wanted), "{rendered}");
    }
}

#[test]
fn nesting_past_the_guard_is_refused_rather_than_overflowing() {
    let mut body = String::from(
        r#"<?xml version="1.0"?>
<cplugin-configuration fix-version="4.4">
	<vocabulary><vocabulary-tag name="555" alt="NoLegs" type="integer" /></vocabulary>
	<grammar-binding type="0"><grammar>"#,
    );
    let depth = 40;
    for _ in 0..depth {
        body.push_str(r#"<grammar><tag-constraint name="555" part="body" />"#);
    }
    for _ in 0..depth {
        body.push_str("</grammar>");
    }
    body.push_str("</grammar></grammar-binding></cplugin-configuration>");

    let refused = FixRegistry::from_cfb(&handle(&body), None).unwrap_err();
    assert!(refused.to_string().contains("deep"), "{refused}");
}

#[test]
fn a_map_becomes_the_code_set_of_the_tag_it_decodes() {
    let (registry, _) = parse(CBLOCK);

    // `ADVSIDE` is not how the vocabulary displays `AdvSide`, so the map is
    // written the UlMessage way: the name keys and the value is the value,
    // which is already the order a code set wants.
    let advside = registry.field_by_tag(4).expect("AdvSide");
    let view = advside.as_fix();
    assert_eq!(view.code_value("buy"), Some("B"));
    assert_eq!(view.code_name("X"), Some("cross"));
    assert_eq!(view.codes().count(), 4);

    // `TimeInForce` is exactly how the vocabulary displays it, so that map is
    // written the FIX way and the key is the wire value.
    let timeinforce = registry.field_by_tag(59).expect("TimeInForce");
    let view = timeinforce.as_fix();
    assert_eq!(view.code_value("day"), Some("0"));
    assert_eq!(view.code_name("1"), Some("goodtillcancel"));

    // A map naming no field is skipped rather than refused: a CBlock maps
    // things that are not fields.
    assert_eq!(registry.get_field_by_name("notafield", None), None);
}

#[test]
fn a_map_is_oriented_by_its_name_and_never_by_the_shape_of_an_entry() {
    let (registry, _) = parse(ORIENTATIONS);

    // Named exactly as the vocabulary displays the field, so the key is the
    // wire value however long it runs. Reading the shorter side as the value
    // would have put `fx` on the wire and `FXSPOT` in a name.
    let security = registry.field_by_tag(167).expect("SecurityType");
    let view = security.as_fix();
    assert_eq!(view.code_value("fx"), Some("FXSPOT"));
    assert_eq!(view.code_name("CS"), Some("equity"));

    // The mirror of that map under a name the file does not display the field
    // by lands identically, which is the whole rule: the name decides, and the
    // entries are read whichever way it says.
    let underlying = registry.field_by_tag(310).expect("UnderlyingSecurityType");
    let view = underlying.as_fix();
    assert_eq!(view.code_value("fx"), Some("FXSPOT"));
    assert_eq!(view.code_name("CS"), Some("equity"));

    // A tag declaring no `alt` is displayed as the tag itself, so a map named
    // for it matches and is read the FIX way.
    let extension = registry.field_by_tag(22830).expect("22830");
    assert_eq!(extension.as_fix().code_value("one"), Some("1"));
}

#[test]
fn a_map_reaches_the_field_it_spells_and_one_entry_never_refuses_the_file() {
    let (registry, _) = parse(AWKWARD);

    // `Ex_Destination` folds onto `ExDestination` and is declared first, so
    // resolving by the fold alone would put the set on the wrong tag and,
    // failing the strict compare there, read it backwards as well.
    let destination = registry.field_by_tag(20000).expect("ExDestination");
    assert_eq!(destination.as_fix().code_value("paris"), Some("XPAR"));
    assert_eq!(
        registry.field_by_tag(100).unwrap().as_fix().codes().count(),
        0,
    );

    // Whitespace around a name is not a spelling, so it neither breaks the
    // match nor flips the orientation. An entry stating nothing on a side is
    // dropped whether it says so with an empty attribute or with none, and so
    // is one repeating a name an earlier entry claimed: a code set may not
    // name one member twice, and one contradictory entry is not a reason to
    // refuse the file.
    let timeinforce = registry.field_by_tag(59).expect("TimeInForce");
    let view = timeinforce.as_fix();
    assert_eq!(view.code_value("day"), Some("0"));
    assert_eq!(view.code_value("goodtilldate"), Some("6"));
    assert_eq!(view.codes().count(), 2);

    // Two names for one wire value is an alias rather than a contradiction,
    // so the second is kept as one rather than dropped or made a second code.
    let advside = registry.field_by_tag(4).expect("AdvSide");
    let view = advside.as_fix();
    assert_eq!(view.code_value("buy"), Some("B"));
    assert_eq!(view.code_value("bid"), Some("B"));
    assert_eq!(view.codes().count(), 1);
    assert_eq!(view.code_name("B"), Some("buy"));
    assert_eq!(
        view.code("B")
            .expect("the code")
            .aliases()
            .collect::<Vec<_>>(),
        ["bid"],
    );
}

#[test]
fn a_file_answers_its_vocabulary_alone_and_in_declaration_order() {
    let fields =
        FixField::from_cfb_file(&handle(CBLOCK), Some("bloomberg")).expect("a readable CBlock");

    // Declaration order, where a registry answers tag-major: the file's own
    // order is what a reader folding two sources wants to see.
    assert_eq!(
        fields.iter().map(Field::name).collect::<Vec<_>>(),
        [
            "beginstring",
            "bodylength",
            "msgtype",
            "avgpx",
            "exludeddealers",
            "dealerparquote",
            "22830",
            "nolegs",
            "legcurrency",
            "nolegsecurityaltid",
            "legsecurityaltid",
            "transacttime",
            "mdentrytime",
            "timeinforce",
            "advside",
        ],
    );

    // Every field is keyed, so it enters a dictionary as it stands. A dialect
    // claims only the user-defined range, so the standard tags stay standard.
    let avgpx = &fields[3];
    assert_eq!(avgpx.as_fix().tag().unwrap(), Some(6));
    assert_eq!(avgpx.as_fix().branch().unwrap(), FixBranch::STANDARD);
    assert_eq!(fields[4].as_fix().branch().unwrap(), branch());

    // The maps sit past the grammar bindings, so a code set proves the whole
    // document was read and not just its first pass.
    assert_eq!(fields[14].as_fix().code_value("buy"), Some("B"));

    // The roots and the branch record are what a registry holds instead.
    let (registry, roots) = FixRegistry::from_cfb(&handle(CBLOCK), Some(&branch())).unwrap();
    assert_eq!(registry.len(), fields.len());
    assert_eq!(roots.len(), 1);
    assert_eq!(
        registry.branch_named("bloomberg").unwrap().version(),
        "4.4".parse::<Version>().unwrap(),
    );
}

#[test]
fn an_unnamed_file_takes_its_branch_from_its_own_stem() {
    // A CBlock never names itself, so the file standing in for the caller is
    // the stem and nothing else of the path.
    let fields = FixField::from_cfb_file(&named_handle(CBLOCK, "MSFIX44.cfb"), None)
        .expect("a readable CBlock");
    let named = FixBranch::from_str("msfix44").unwrap();
    assert_eq!(fields[4].as_fix().branch().unwrap(), named);
    assert_eq!(fields[3].as_fix().branch().unwrap(), FixBranch::STANDARD);

    // An explicit name still wins over the stem.
    let fields = FixField::from_cfb_file(&named_handle(CBLOCK, "MSFIX44.cfb"), Some("bloomberg"))
        .expect("a readable CBlock");
    assert_eq!(fields[4].as_fix().branch().unwrap(), branch());

    // A buffer's URL is an identity and not a location, so its stem names no
    // dialect and is refused rather than guessed at: bytes held in memory are
    // named by the caller or not at all.
    let mut buffer = Buffer::new();
    buffer.write_all_bytes(CBLOCK.as_bytes()).unwrap();
    assert!(FixField::from_cfb_file(&buffer, None).is_err());
    let fields = FixField::from_cfb_file(&buffer, Some("bloomberg")).expect("a readable CBlock");
    assert_eq!(fields[4].as_fix().branch().unwrap(), branch());
}

#[test]
fn a_stem_that_is_not_a_branch_is_refused_rather_than_folded_into_one() {
    // A dictionary keyed on a guess is worse than a refusal, so neither the
    // leading digit nor the over-long name is repaired.
    let error = FixField::from_cfb_file(&named_handle(CBLOCK, "4.4-ms.cfb"), None).unwrap_err();
    assert!(
        matches!(&error, Error::Parse { target, .. } if *target == "fix branch"),
        "{error}"
    );
    assert!(error.to_string().contains("ASCII letter"), "{error}");

    let error = FixField::from_cfb_file(
        &named_handle(CBLOCK, "a-name-well-past-the-inline-cap.cfb"),
        None,
    )
    .unwrap_err();
    assert!(error.to_string().contains("at most 23 bytes"), "{error}");

    // A supplied name is held to the same rule as a stem standing in for one.
    let error = FixField::from_cfb_file(&handle(CBLOCK), Some("4bloomberg")).unwrap_err();
    assert!(error.to_string().contains("ASCII letter"), "{error}");
}

#[test]
fn a_cblock_vocabulary_folds_into_a_dictionary_that_already_exists() {
    // A dialect claims only the user-defined range, so two counterparties'
    // files meet in the standard branch rather than each shadowing FIX - which
    // is the case the fold exists for.
    let mut dictionary = FixRegistry::from_fields(
        FixField::from_cfb_file(&handle(CBLOCK), Some("bloomberg")).unwrap(),
    )
    .expect("one file's vocabulary");
    let before = dictionary.len();

    let (added, merged) = dictionary
        .add_fields(FixField::from_cfb_file(&handle(OVERLAY), Some("morgan")).unwrap())
        .expect("the second file folds into the first");
    assert_eq!((added, merged), (1, 1));
    assert_eq!(dictionary.len(), before + added);

    // The fold keeps what only the stored definition declared, where the
    // wholesale replace an `insert` performs would drop it.
    let avgpx = dictionary.field_by_tag(6).unwrap();
    assert_eq!(avgpx.name(), "avgpx");
    assert!(
        avgpx
            .description()
            .is_some_and(|held| held.contains("average price")),
        "the first file's description survived a file that carries none",
    );
    // And a tag only the second file declares arrives.
    assert_eq!(
        dictionary.field_by_name("price", None).unwrap().name(),
        "price"
    );

    // The second file's own custom tags land in its own branch, so the two
    // dialects never collide on the user range.
    let (added, _) = dictionary
        .add_fields(FixField::from_cfb_file(&named_handle(CBLOCK, "morgan.cfb"), None).unwrap())
        .expect("the same vocabulary under a second dialect");
    assert_eq!(added, 3, "one branch's user-range tags, and no other");
}

#[test]
fn folding_a_cblock_into_the_committed_dictionary_refuses_what_it_would_lose() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("config")
        .join("fix");
    let mut seeded =
        FixRegistry::from_handle(&Folder::new(root).expect("the seed folder")).expect("the seed");
    let before = seeded.clone();

    // A CBlock says nothing about which tag is money or which is a MsgType, so
    // its generic answers disagree with the committed dictionary's typed ones.
    // The fold refuses rather than widening, which is the phase's principal
    // known loss stated as a refusal instead of a silent replacement.
    let fields = FixField::from_cfb_file(&handle(CBLOCK), Some("bloomberg")).unwrap();
    let error = seeded.add_fields(fields).unwrap_err();
    assert!(matches!(error, Error::InvalidRecord { .. }), "{error}");
    let message = error.to_string();
    assert!(
        message.contains("msgtype") && message.contains("utf8"),
        "{message}"
    );
    assert_eq!(seeded.field_by_tag(35).unwrap().dtype(), &DataType::MsgType);
    assert_eq!(seeded.field_by_tag(6).unwrap().dtype(), &DataType::Float64);

    // The fold is one mutation, so the tags read before the refusal - 8 and 9,
    // which agree and would have merged - are not in the dictionary either.
    assert_eq!(seeded, before, "a refused fold writes nothing");
}

#[test]
fn both_doors_refuse_a_file_that_names_one_field_twice() {
    // Two tags whose `alt` folds to one name, both outside the user range and
    // so both in the standard branch. The dictionary build is what catches it,
    // which is why the vocabulary door builds one and throws it away.
    let clashing = r#"<?xml version="1.0"?>
<cplugin-configuration fix-version="4.4">
	<vocabulary>
		<vocabulary-tag name="44" alt="Price" type="float" />
		<vocabulary-tag name="3044" alt="Price" type="float" />
	</vocabulary>
</cplugin-configuration>"#;
    let refused = FixRegistry::from_cfb(&handle(clashing), Some(&branch())).unwrap_err();
    assert!(refused.is_conflict(), "{refused}");
    let also = FixField::from_cfb_file(&handle(clashing), Some("bloomberg")).unwrap_err();
    assert_eq!(also.to_string(), refused.to_string());

    // Same tag twice under two spellings is the other half of that check.
    let doubled = r#"<?xml version="1.0"?>
<cplugin-configuration fix-version="4.4">
	<vocabulary>
		<vocabulary-tag name="44" alt="Price" type="float" />
		<vocabulary-tag name="44" alt="LastPx" type="float" />
	</vocabulary>
</cplugin-configuration>"#;
    assert!(FixField::from_cfb_file(&handle(doubled), None).is_err());

    // And the one difference that is not a loss: a dictionary keeps one entry
    // per identity, so a tag declared twice identically arrives twice here.
    let repeated = r#"<?xml version="1.0"?>
<cplugin-configuration fix-version="4.4">
	<vocabulary>
		<vocabulary-tag name="44" alt="Price" type="float" />
		<vocabulary-tag name="44" alt="Price" type="float" />
	</vocabulary>
</cplugin-configuration>"#;
    let fields = FixField::from_cfb_file(&handle(repeated), None).expect("a readable CBlock");
    assert_eq!(fields.len(), 2);
    let (registry, _) = FixRegistry::from_cfb(&handle(repeated), None).unwrap();
    assert_eq!(registry.len(), 1);
}

#[test]
fn a_cblock_reads_in_whole_with_its_dialect_and_the_file_it_arrived_as() {
    let mut dictionary = FixRegistry::new();
    let (added, merged) = dictionary
        .add_cfb_file(
            &named_handle(CBLOCK, "MSFIX44.cfb"),
            Some("morgan"),
            Some(&["mstanley"]),
        )
        .expect("a readable CBlock");
    assert_eq!((added, merged), (15, 0));
    assert_eq!(dictionary.len(), 15);

    // The dialect the root element declared, which reading the fields alone
    // would have lost: a field carries its branch's name and nothing else.
    let branch = dictionary.branch_named("morgan").expect("the named branch");
    assert_eq!(branch.version(), "4.4".parse::<Version>().unwrap());
    assert_eq!(branch.sender_comp_id(), "OURDESK");
    assert_eq!(branch.target_comp_id(), "BLPFIX");

    // The file a definition arrived as is a spelling people use for it, so the
    // stem answers beside the name and beside what the caller asked for.
    assert_eq!(branch.aliases(), ["mstanley", "msfix44"]);
    for spelling in ["morgan", "MSTANLEY", "msfix44"] {
        assert_eq!(
            dictionary.branch_named(spelling).map(FixBranch::name),
            Some("morgan"),
            "{spelling}",
        );
    }

    // Reading a second file is not a statement that the first one's names were
    // wrong, so the spellings accumulate.
    let (added, merged) = dictionary
        .add_cfb_file(
            &named_handle(SELLSIDE, "morgan-2024.cfb"),
            Some("morgan"),
            None,
        )
        .expect("the same dialect, read again");
    assert_eq!((added, merged), (0, 1), "SELLSIDE declares only tag 35");
    let branch = dictionary.branch_named("morgan").expect("the named branch");
    assert_eq!(branch.aliases(), ["mstanley", "msfix44", "morgan-2024"]);
    // And the record is the second file's, whole: it points the other way.
    assert_eq!(branch.sender_comp_id(), "BLPFIX");

    // With no branch named, the stem is the name - and a name is not an alias
    // of itself, so nothing is invented.
    let mut standalone = FixRegistry::new();
    standalone
        .add_cfb_file(&handle(CBLOCK), None, None)
        .expect("a readable CBlock");
    let branch = standalone.branch_named("one").expect("the stem named it");
    assert_eq!(branch.aliases(), [] as [&str; 0]);
}

#[test]
fn reading_a_cblock_in_whole_is_one_mutation() {
    // The committed dictionary types tag 35 as a MsgType, which a CBlock's
    // generic `string` disagrees with - so this file refuses partway.
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("config")
        .join("fix");
    let mut seeded =
        FixRegistry::from_handle(&Folder::new(root).expect("the seed folder")).expect("the seed");
    let before = seeded.clone();

    let error = seeded
        .add_cfb_file(&handle(CBLOCK), Some("bloomberg"), None)
        .unwrap_err();
    assert!(matches!(error, Error::InvalidRecord { .. }), "{error}");
    assert_eq!(seeded, before, "neither the branch nor a field arrived");
    assert!(seeded.branch_named("bloomberg").is_none());
}
