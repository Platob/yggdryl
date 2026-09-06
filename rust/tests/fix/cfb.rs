//! One Ullink CBlock configuration, read into a dictionary and message roots.

use std::path::PathBuf;

use yggdryl::holder::local::Folder;
use std::sync::Arc;

use yggdryl::holder::fs::{File, FileSystem, MemoryFileSystem};
use yggdryl::{DataType, Field, FixBranch, FixRegistry, IOBase, Version};

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
    let filesystem: Arc<dyn FileSystem> = Arc::new(MemoryFileSystem::new());
    // A memory filesystem starts empty, where a real bucket already exists.
    filesystem
        .create_dir("cblock", true)
        .expect("a container to write into");
    let mut file =
        File::from_path(filesystem, "cblock/one.cfb", None).expect("a path under it");
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
    assert_eq!(registry.len(), 14);

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
        assert_eq!(registry.field_by_tag(tag).unwrap().dtype(), &dtype, "tag {tag}");
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
    let held = registry.branch_named("bloomberg").expect("the named branch");
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
