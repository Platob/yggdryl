//! `rust/src/fix/cfb.rs`: one Ullink CBlock configuration, read into a
//! dictionary and message roots.

use super::path;

use std::sync::Arc;

use yggdryl::fs::{FileSystem, FsFile, MemoryFileSystem};
use yggdryl::holder::Buffer;
use yggdryl::{DataType, Error, Field, FixCodec, FixId, FixRegistry, IOBase, PluginSide};

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
		<message-type value="AR Inbound" description="Trade Capture Report Ack" rejection="not supported" supported="false" />
		<message-type value="AR Outbound" description="Trade Capture Report Ack" rejection="not supported" supported="true" />
		<message-type value="c SDR" description="Security Definition" supported="true" />
		<message-type value="c SLR" description="Security Definition" supported="true" />
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
const CODESET_OVERLAY: &str = r#"<?xml version="1.0" encoding="US-ASCII"?>
<cplugin-configuration fix-version="4.4">
    <vocabulary><vocabulary-tag name="4" alt="AdvSide" type="char" /></vocabulary>
    <maps><map name="ADVSIDE"><entries><entry key="venue_buy" value="B" /></entries></map></maps>
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

/// A file whose normalization binding spells its tags the way the corpus
/// does: the shape a production `.cfb` writes, nested groups included, over a
/// vocabulary where some tags are named and some are not.
const NORMALIZED: &str = r#"<?xml version="1.0" encoding="US-ASCII"?>
<cplugin-configuration type="com.ullink.ulbridge2.toolkit.plugins.fix.model.state.cblock.BuySideFIXCPluginCBlock" version="1.2" fix-version="4.4" targetcompid="BLPFIX" sendercompid="OURDESK">
	<vocabulary>
		<vocabulary-tag name="602" alt="LegSecurityID" type="string" />
		<vocabulary-tag name="603" alt="LegSecurityIDSource" type="string" />
		<vocabulary-tag name="604" alt="NoLegSecurityAltID" type="integer" />
		<vocabulary-tag name="605" alt="LegSecurityAltID" type="string" />
		<vocabulary-tag name="608" type="string" />
		<vocabulary-tag name="609" type="string" />
		<vocabulary-tag name="22830" type="string" />
		<vocabulary-tag name="22831" type="string" />
		<vocabulary-tag name="22832" type="string" />
		<vocabulary-tag name="22833" alt="VenueSym" type="string" />
		<vocabulary-tag name="22834" type="string" />
	</vocabulary>
	<normalization-binding>
		<normalization type="inbound">
			<tag-normalization tag-name="LEGSECURITYID" part="body">
				<mapping-expression>
					<expression value="$602" />
				</mapping-expression>
			</tag-normalization>
			<tag-normalization tag-name="LEGSECURITYIDSOURCE" part="body">
				<mapping-expression>
					<expression value="lookup(&quot;SecurityIDSource&quot;, $603)" />
				</mapping-expression>
			</tag-normalization>
			<tag-normalization tag-name="LEGISINCODE" part="body">
				<mapping-condition>
					<expression value="$603 = &quot;4&quot;" />
				</mapping-condition>
				<mapping-expression>
					<expression value="$602" />
				</mapping-expression>
			</tag-normalization>
			<tag-normalization tag-name="LEGEXCHANGECODE" part="body">
				<mapping-condition>
					<expression value="$603 = &quot;8&quot;" />
				</mapping-condition>
				<mapping-expression>
					<expression value="$602" />
				</mapping-expression>
			</tag-normalization>
			<tag-normalization tag-name="NOLEGSECURITYALTID" part="body">
				<mapping-expression>
					<expression value="$604" />
				</mapping-expression>
			</tag-normalization>
			<normalization type="inbound" rg-name="NOLEGSECURITYALTID" rg-context="604">
				<tag-normalization tag-name="LegSecurityAltId" part="body">
					<mapping-expression>
						<expression value="$605" />
					</mapping-expression>
				</tag-normalization>
				<tag-normalization tag-name="EXCLUDEDDEALERS" part="body">
					<mapping-expression>
						<expression value="$22830" />
					</mapping-expression>
				</tag-normalization>
				<condition-expression />
			</normalization>
			<tag-normalization tag-name="LEGCFICODE" part="body">
				<mapping-expression>
					<expression value="$608 " />
				</mapping-expression>
			</tag-normalization>
			<tag-normalization tag-name="LEGSECURITYTYPE" part="body">
				<mapping-expression>
					<expression value="lookup(&quot;SecurityType&quot;, $609) " />
				</mapping-expression>
			</tag-normalization>
			<tag-normalization tag-name="EXCLUDED_DEALERS" part="body">
				<mapping-expression>
					<expression value="$22830" />
				</mapping-expression>
			</tag-normalization>
			<tag-normalization tag-name="EXCLUDEDDEALERS" part="body">
				<mapping-expression>
					<expression value="$22831" />
				</mapping-expression>
			</tag-normalization>
			<tag-normalization tag-name="BUILT" part="body">
				<mapping-expression>
					<expression value="$22830" />
					<expression value="$22831" />
				</mapping-expression>
			</tag-normalization>
			<tag-normalization tag-name="VENUESYM" part="body">
				<mapping-expression>
					<expression value="$22832" />
				</mapping-expression>
			</tag-normalization>
			<tag-normalization tag-name="LEGSECURITYID" part="body">
				<mapping-expression>
					<expression value="$22834" />
				</mapping-expression>
			</tag-normalization>
			<tag-normalization tag-name="" part="body">
				<mapping-expression>
					<expression value="$22832" />
				</mapping-expression>
			</tag-normalization>
			<tag-normalization tag-name="UNDECLARED" part="body">
				<mapping-expression>
					<expression value="$999" />
				</mapping-expression>
			</tag-normalization>
			<tag-normalization tag-name="COMMA,SPELLING" part="body">
				<mapping-expression>
					<expression value="$22832" />
				</mapping-expression>
			</tag-normalization>
			<tag-normalization tag-name="NOTHING" part="body" />
		</normalization>
	</normalization-binding>
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
        FsFile::from_path(filesystem, format!("cblock/{name}"), None).expect("a path under it");
    file.write_all_bytes(body.as_bytes()).expect("the document");
    file
}

/// The dialect the cases read under: a membership every produced field
/// carries, and nothing a lookup consults.
const DIALECT: &str = "bloomberg";

fn parse(body: &str) -> (FixRegistry, Vec<Field>) {
    FixRegistry::from_cfb_file(&handle(body), Some(DIALECT)).expect("a readable CBlock")
}

/// The dialects a field is a member of, for one assertion over the list.
fn sources(field: &Field) -> Vec<&str> {
    field.as_fix().sources().collect()
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
    assert_eq!(super::scalars(&registry), 14 + super::seeded_fields());

    // Named by `alt` lower-cased, with the file's own spelling kept beside it,
    // so a caller spelling it the file's way still resolves.
    let field = registry.field_by_tag(6).expect("AvgPx");
    assert_eq!(field.name(), "avgpx");
    // One namespace: a standard tag and a venue's own both resolve by name,
    // and both carry the dialect as a membership - tag 6 is FIX's, and this
    // dictionary speaks it, which is what the membership records.
    assert!(registry.get_field_by_name("AvgPx").is_some());
    assert!(
        registry.get_field_by_name("ExludedDealers").is_some(),
        "a custom tag resolves in the same namespace",
    );
    assert_eq!(sources(field), [DIALECT]);
    assert_eq!(sources(registry.field_by_tag(10001).unwrap()), [DIALECT]);
    assert_eq!(registry.dialects(), [DIALECT]);

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
        (35, DataType::utf8()),
        (59, DataType::utf8()),
        (9, DataType::Int32),
        // `float` is FIX's float family, which states no width and which
        // this crate types float64 - never the 32-bit float the grammar
        // reads the SQL word as.
        (6, DataType::Float64),
        (10001, DataType::Boolean),
        // `utc-date` is a day, and a day is that day's midnight in UTC.
        (
            10015,
            DataType::DateTime64 {
                unit: yggdryl::TimeUnit::Nanosecond,
                timezone: yggdryl::Timezone::UTC,
            },
        ),
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
            "legs",
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
fn a_nested_grammar_is_its_group_alone_and_its_counter_a_dictionary_field() {
    // The counter heads the group on the wire and is no member of the
    // grammar's root: the group is its list alone, its length the count,
    // and the counter's tag stands on the group as `FIX:counter` while the
    // dictionary keeps the counter as a field of its own.
    let (registry, roots) = parse(CBLOCK);
    let fields = roots[0].dtype().as_fields().unwrap();
    assert!(fields.iter().all(|held| held.name() != "nolegs"));
    assert_eq!(
        registry.field_by_tag(555).unwrap().dtype(),
        &DataType::Int32
    );
    let group = fields.iter().find(|held| held.name() == "legs").unwrap();
    assert_eq!(group.display(), Some("Legs"));
    assert_eq!(group.as_fix().tag().unwrap(), None);
    assert_eq!(group.as_fix().counter().unwrap(), Some(555));
    let DataType::Serie(item) = group.dtype() else {
        panic!("a serie, got {}", group.dtype());
    };
    assert_eq!(item.name(), "leg");
    assert_eq!(item.display(), Some("Leg"));
    assert!(!item.is_nullable());
    let members = item.dtype().as_fields().expect("an item struct");
    assert_eq!(
        members.iter().map(yggdryl::Field::name).collect::<Vec<_>>(),
        ["legcurrency", "legsecurityaltidgrp"]
    );
    assert!(!members[0].is_nullable(), "556 is required");
    assert_eq!(
        registry.field_by_tag(604).unwrap().dtype(),
        &DataType::Int32
    );
    let DataType::Serie(inner) = members[1].dtype() else {
        panic!("a nested serie, got {}", members[1].dtype());
    };
    assert_eq!(members[1].as_fix().tag().unwrap(), None);
    assert_eq!(members[1].as_fix().counter().unwrap(), Some(604));
    assert_eq!(members[1].display(), Some("LegSecurityAltIDGrp"));
    assert_eq!(inner.display(), Some("LegSecurityAltIDComponent"));
    assert_eq!(
        inner
            .fields()
            .iter()
            .map(yggdryl::Field::name)
            .collect::<Vec<_>>(),
        ["legsecurityaltid"]
    );
}

#[test]
fn the_root_element_is_read_past_and_the_dialect_is_a_membership() {
    // The root states a FIX version and a session pair, and neither is
    // recorded: which version a run reads at is the codec's pin, and which
    // two parties spoke a vocabulary is a fact about a run rather than about
    // the dictionary. What the caller states - the dialect - is what every
    // field carries.
    let (registry, _) = parse(CBLOCK);
    assert_eq!(registry.dialects(), [DIALECT]);
    for field in registry.iter() {
        // Crate fields and unstated SendingTime are seeds, not this file's
        // declarations. Its TransactTime replaces the standard seed.
        if field
            .as_fix()
            .tag()
            .unwrap()
            .is_some_and(|tag| yggdryl::is_crate_tag(tag) || tag == 52)
        {
            assert!(!field.as_fix().has_source(DIALECT), "{}", field.name());
            continue;
        }
        assert!(field.as_fix().has_source(DIALECT), "{}", field.name());
    }

    // The same file written from the other side of the session lands
    // identically: the session pair is read past.
    let (buy, _) = FixRegistry::from_cfb_file(&handle(SELLSIDE), Some(DIALECT)).unwrap();
    let mut other = SELLSIDE.replace("targetcompid=\"OURDESK\"", "targetcompid=\"BLPFIX\"");
    other = other.replace("sendercompid=\"BLPFIX\"", "sendercompid=\"OURDESK\"");
    let (sell, _) = FixRegistry::from_cfb_file(&handle(&other), Some(DIALECT)).unwrap();
    assert_eq!(sell, buy);

    // A file parsed with no dialect stamps nothing, which is right for one
    // read only for its vocabulary.
    let (bare, _) = FixRegistry::from_cfb_file(&handle(CBLOCK), None).unwrap();
    assert!(bare.field_by_tag(6).is_ok());
    assert!(bare.dialects().is_empty());
    assert!(!bare.field_by_tag(6).unwrap().as_fix().has_source(DIALECT));
}

#[test]
fn replacing_a_referenced_cblock_field_is_atomic_and_unreferenced_fields_replace() {
    let mut seeded = super::committed_registry().as_ref().clone();
    let (vocabulary, _) = FixRegistry::from_cfb_file(&handle(CBLOCK), None).unwrap();
    let avgpx = vocabulary.field_by_tag(6).unwrap().clone();
    let before = seeded.clone();
    let error = seeded.insert(avgpx.clone()).unwrap_err();
    assert!(matches!(error, Error::InvalidRecord { .. }), "{error}");
    assert!(error.to_string().contains("avgpx"), "{error}");
    assert_eq!(
        seeded, before,
        "referenced layouts remain coherent after refusal"
    );
    let mut standalone =
        FixRegistry::from_fields([seeded.field_by_tag(6).unwrap().clone()]).unwrap();
    standalone.insert(avgpx).unwrap();
    assert_eq!(
        standalone.field_by_tag(6).unwrap().dtype(),
        &DataType::Float64
    );
    // The same tag under another name is another field: it is registered
    // beside the holder under its own identity, the bare tag keeps answering
    // the holder, and the holder learns nothing of the arrival's name - two
    // fields one tag carries are two fields, never one aliasing the other.
    let mut renamed = vocabulary.field_by_tag(6).unwrap().clone();
    renamed.set_name("somethingelse");
    let before = standalone.len();
    assert_eq!(standalone.insert(renamed).unwrap(), None);
    assert_eq!(standalone.len(), before + 1);
    assert_eq!(standalone.field_by_tag(6).unwrap().name(), "avgpx");
    assert_eq!(
        standalone
            .field_by_tag(6)
            .unwrap()
            .as_fix()
            .names()
            .collect::<Vec<_>>(),
        [] as [&str; 0],
        "the holder lends no alias",
    );
    assert_eq!(
        standalone
            .get_field_by_name("somethingelse")
            .map(Field::name),
        Some("somethingelse"),
    );
    assert_eq!(
        standalone
            .field_by_id(FixId::of(6, "somethingelse").unwrap())
            .unwrap()
            .name(),
        "somethingelse"
    );
}

#[test]
fn catalog_members_resolve_codes_declared_after_their_grammar() {
    let (registry, roots) = parse(CBLOCK);
    let message = registry.msgtype("7").unwrap();
    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0].name(), "7");
    assert!(!roots[0].is_nullable());
    for (name, tag, nullable) in [("msgtype", 35, false), ("timeinforce", 59, true)] {
        let occurrence = message.as_field().get_field(name).unwrap();
        let canonical = registry.field_by_tag(tag).unwrap();
        assert_eq!(roots[0].get_field(name), Some(occurrence));
        assert_eq!(occurrence.as_fix().field_ref(), Some(canonical.name()));
        assert_eq!(occurrence.is_nullable(), nullable);
        // The occurrence reads by the set the canonical field reads by, which
        // the maps and the message types declared after this grammar.
        assert_eq!(
            registry.codeset_of(occurrence),
            registry.codeset_of(canonical)
        );
        assert!(registry.codeset_of(occurrence).is_some(), "{name}");
    }
    let repeated = message.as_field().get_field("beginstring2").unwrap();
    assert_eq!(roots[0].get_field("beginstring2"), Some(repeated));
    assert!(repeated.is_nullable());
    assert_eq!(repeated.as_fix().field_ref(), Some("beginstring"));
}

#[test]
fn venue_groups_and_their_components_carry_the_membership_and_key_on_the_counter_tag() {
    let body = r#"<cplugin-configuration fix-version="4.4">
      <vocabulary>
        <vocabulary-tag name="55" alt="Symbol" type="string" />
        <vocabulary-tag name="5000" alt="NoVendorEntries" type="integer" />
        <vocabulary-tag name="5001" alt="VendorID" type="string" />
        <vocabulary-tag name="5002" alt="NoVendorSubEntries" type="integer" />
        <vocabulary-tag name="5003" alt="VendorSubID" type="string" />
      </vocabulary>
      <grammar-binding type="D"><grammar>
        <grammar rg-name="VendorEntries">
          <tag-constraint name="5000" />
          <tag-constraint name="5001" required="true" />
          <tag-constraint name="55" />
          <grammar rg-name="VendorSubEntries">
            <tag-constraint name="5002" />
            <tag-constraint name="5003" required="true" />
          </grammar>
        </grammar>
      </grammar></grammar-binding>
    </cplugin-configuration>"#;
    let (registry, roots) = FixRegistry::from_cfb_file(&handle(body), Some("venue")).unwrap();
    for (counter, name, component) in [
        (5000, "VendorEntries", "VendorEntry"),
        (5002, "VendorSubEntries", "VendorSubEntry"),
    ] {
        // Every definition the file produced is a member of the dialect.
        let group = registry.field_by_name(name).unwrap();
        let component_name = component;
        let component = registry.field_by_name(component_name).unwrap();
        assert_eq!(group.display(), Some(name));
        assert_eq!(component.display(), Some(component_name));
        assert_eq!(sources(group), ["venue"]);
        assert_eq!(sources(component), ["venue"]);
        // The counter is its tag and its name, and the group tables key on
        // the tag, since every caller holds one.
        let held = registry.field_by_tag(counter).unwrap();
        assert_eq!(held.dtype(), &DataType::Int32);
        let id = FixId::of(counter, held.name()).unwrap();
        assert_eq!(registry.field(id).unwrap().dtype(), &DataType::Int32);
        assert_eq!(held.as_fix().id().unwrap(), Some(id));
        assert!(
            registry
                .msgtype("D")
                .unwrap()
                .get_group_by_tag(counter)
                .is_some()
        );
        assert!(registry.get_field_by_counter(counter).is_some());
    }
    let DataType::Serie(item) = roots[0].get_field("vendorentries").unwrap().dtype() else {
        panic!("a serie group");
    };
    assert_eq!(sources(item), ["venue"]);
    // A standard tag the file speaks is this dictionary's member too:
    // membership means "this dictionary speaks it", not "this dictionary
    // invented it".
    assert_eq!(sources(registry.field_by_tag(55).unwrap()), ["venue"]);
    assert_eq!(
        FixRegistry::from_json(&registry.into_json().unwrap()).unwrap(),
        registry
    );
}

/// Three grammars declaring one repeating group three ways: plainly under
/// `8`, with one more member under `D`, which tag 35's own map names
/// `NewOrderSingle`, and with another under `AE`, which nothing names.
const SPLIT: &str = r#"<cplugin-configuration fix-version="4.4">
      <vocabulary>
        <vocabulary-tag name="35" alt="MsgType" type="string" />
        <vocabulary-tag name="711" alt="NoUnderlyings" type="integer" />
        <vocabulary-tag name="311" alt="UnderlyingSymbol" type="string" />
        <vocabulary-tag name="1044" alt="UnderlyingAdjustedQuantity" type="float" />
        <vocabulary-tag name="879" alt="UnderlyingQty" type="float" />
      </vocabulary>
      <grammar-binding type="8"><grammar>
        <grammar rg-name="Underlyings">
          <tag-constraint name="711" />
          <tag-constraint name="311" required="true" />
        </grammar>
      </grammar></grammar-binding>
      <grammar-binding type="D"><grammar>
        <grammar rg-name="Underlyings">
          <tag-constraint name="711" />
          <tag-constraint name="311" required="true" />
          <tag-constraint name="1044" />
        </grammar>
      </grammar></grammar-binding>
      <grammar-binding type="AE"><grammar>
        <grammar rg-name="Underlyings">
          <tag-constraint name="711" />
          <tag-constraint name="311" required="true" />
          <tag-constraint name="879" />
        </grammar>
      </grammar></grammar-binding>
      <maps>
        <map name="MSGTYPE"><entries><entry key="NewOrderSingle" value="D" /></entries></map>
      </maps>
    </cplugin-configuration>"#;

#[test]
fn a_split_definition_is_named_for_the_message_it_was_read_in() {
    use yggdryl::FixCategory::{Components, Groups};

    let (registry, _) = parse(SPLIT);
    // The first shape keeps the plain names; every later shape is split under
    // the name the message is catalogued by - tag 35's own name for `D`, the
    // wire value's hex where nothing names `AE` - and never under a bare hex
    // fragment of the wire value alone.
    assert_eq!(registry.msgtype("8").unwrap().name(), "message38");
    assert_eq!(registry.msgtype("D").unwrap().name(), "newordersingle");
    assert_eq!(registry.msgtype("AE").unwrap().name(), "message4145");
    for (message, component, group, member) in [
        ("8", "underlying", "underlyings", None),
        (
            "D",
            "underlying_newordersingle",
            "underlyings_newordersingle",
            Some("underlyingadjustedquantity"),
        ),
        (
            "AE",
            "underlying_message4145",
            "underlyings_message4145",
            Some("underlyingqty"),
        ),
    ] {
        let occurrence = registry.definition(Components, component).unwrap();
        assert_eq!(
            occurrence.fields().len(),
            1 + usize::from(member.is_some()),
            "{component}"
        );
        if let Some(member) = member {
            assert!(occurrence.get_field(member).is_some(), "{component}");
        }
        let held = registry.definition(Groups, group).unwrap();
        assert_eq!(held.as_fix().component(), Some(component), "{group}");
        // The message reaches the split it was read with.
        let root = registry.msgtype(message).unwrap().as_field();
        assert!(
            root.fields()
                .iter()
                .any(|member| member.as_fix().group() == Some(group)),
            "{message}"
        );
    }
    for hex in ["underlying44", "underlyings44", "underlying4145"] {
        assert!(registry.get_definition(Components, hex).is_none(), "{hex}");
        assert!(registry.get_definition(Groups, hex).is_none(), "{hex}");
    }

    // A second binding of one wire type is that message again: it folds, and
    // its split is the one the first binding took.
    let rebound = SPLIT.replace(
        "<maps>",
        r#"<grammar-binding type="D"><grammar>
        <grammar rg-name="Underlyings">
          <tag-constraint name="711" />
          <tag-constraint name="311" required="true" />
          <tag-constraint name="1044" />
        </grammar>
      </grammar></grammar-binding>
      <maps>"#,
    );
    let (again, _) = parse(&rebound);
    assert_eq!(
        again.definitions(Components).count(),
        registry.definitions(Components).count()
    );
    assert_eq!(
        again.definitions(Groups).count(),
        registry.definitions(Groups).count()
    );
}

/// A second binding of one wire type that widens a definition the first
/// one read into the structure of another definition the file already split
/// leaves one definition, whichever of the two messages is bound first: the
/// widened one keeps its name, the split folds into it, and every message
/// reads it.
#[test]
fn a_second_binding_that_widens_a_held_definition_into_a_split_leaves_one_definition() {
    use yggdryl::FixCategory::{Components, Groups};

    let rebinding = r#"<grammar-binding type="8"><grammar>
        <grammar rg-name="Underlyings">
          <tag-constraint name="711" />
          <tag-constraint name="311" required="true" />
          <tag-constraint name="1044" />
        </grammar>
      </grammar></grammar-binding>
      "#;
    // After `D`, so the second binding reads `D`'s split and widens the
    // held `underlying` into its structure; and before `D`, so `D` finds
    // `underlying` already widened.
    let after = SPLIT.replace("<maps>", &format!("{rebinding}<maps>"));
    let before = SPLIT.replace(
        r#"<grammar-binding type="D">"#,
        &format!(r#"{rebinding}<grammar-binding type="D">"#),
    );
    let mut definitions = Vec::new();
    for body in [&after, &before] {
        let (registry, _) = parse(body);
        for split in ["underlying_newordersingle", "underlyings_newordersingle"] {
            assert!(
                registry.get_definition(Components, split).is_none(),
                "{split}"
            );
            assert!(registry.get_definition(Groups, split).is_none(), "{split}");
        }
        let underlying = registry.definition(Components, "underlying").unwrap();
        assert_eq!(
            children(underlying),
            ["underlyingsymbol", "underlyingadjustedquantity"]
        );
        for message in ["8", "D"] {
            let root = registry.msgtype(message).unwrap().as_field();
            assert!(
                root.fields()
                    .iter()
                    .any(|member| member.as_fix().group() == Some("underlyings")),
                "{message}"
            );
        }
        // The other shape keeps its own split.
        assert!(
            registry
                .get_definition(Components, "underlying_message4145")
                .is_some()
        );
        let names = |category| {
            registry
                .definitions(category)
                .map(|field| field.name().to_owned())
                .collect::<Vec<_>>()
        };
        definitions.push((names(Components), names(Groups)));
    }
    assert_eq!(definitions[0], definitions[1]);
}

#[test]
fn a_published_group_spelling_uses_the_shipped_collection_and_occurrence_names() {
    let body = r#"<cplugin-configuration fix-version="4.4">
      <vocabulary>
        <vocabulary-tag name="802" alt="NoPartySubIDs" type="integer" />
        <vocabulary-tag name="523" alt="PartySubID" type="string" />
      </vocabulary>
      <grammar-binding type="D"><grammar>
        <grammar rg-name="PtysSubGrp">
          <tag-constraint name="802" />
          <tag-constraint name="523" required="true" />
        </grammar>
      </grammar></grammar-binding>
    </cplugin-configuration>"#;
    let (_, roots) = FixRegistry::from_cfb_file(&handle(body), None).unwrap();
    let group = roots[0]
        .get_field("partysubids")
        .expect("the semantic collection name");
    assert_eq!(group.display(), Some("PartySubIDs"));
    let DataType::Serie(item) = group.dtype() else {
        panic!("a serie group");
    };
    assert_eq!(item.name(), "ptyssub");
    assert_eq!(item.display(), Some("PtysSub"));
}

#[test]
fn a_custom_singular_ending_in_s_keeps_grp_and_its_whole_occurrence_name() {
    let body = r#"<cplugin-configuration fix-version="4.4">
      <vocabulary>
        <vocabulary-tag name="5000" alt="NoStatus" type="integer" />
        <vocabulary-tag name="5001" alt="StatusCode" type="string" />
      </vocabulary>
      <grammar-binding type="D"><grammar>
        <grammar rg-name="StatusGrp">
          <tag-constraint name="5000" />
          <tag-constraint name="5001" required="true" />
        </grammar>
      </grammar></grammar-binding>
    </cplugin-configuration>"#;
    let (_, roots) = FixRegistry::from_cfb_file(&handle(body), None).unwrap();
    let group = roots[0]
        .get_field("statusgrp")
        .expect("no plural was published");
    assert_eq!(group.display(), Some("StatusGrp"));
    let DataType::Serie(item) = group.dtype() else {
        panic!("a serie group");
    };
    assert_eq!(item.name(), "status");
    assert_eq!(item.display(), Some("Status"));
}

#[test]
fn the_structural_exceptions_are_a_statement_or_a_named_drop() {
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

    // A nested grammar with no counter is dropped, named by the position
    // rather than by a line of prose, and the message keeps the rest.
    let counterless = r#"<?xml version="1.0"?>
<cplugin-configuration fix-version="4.4">
	<vocabulary><vocabulary-tag name="35" alt="MsgType" type="string" /></vocabulary>
	<grammar-binding type="0"><grammar><tag-constraint name="35" part="body" /><grammar /></grammar></grammar-binding>
</cplugin-configuration>"#;
    let (read, warnings) =
        super::warned::during(|| FixRegistry::from_cfb_file(&handle(counterless), None));
    let (_, roots) = read.expect("a readable CBlock");
    assert!(warnings[0].contains("counter"), "{warnings:?}");
    assert_eq!(children(&roots[0]), ["msgtype"]);

    // A constraint naming a tag the vocabulary does not have is still the
    // file saying the tag is on the wire in this message, so the constraint
    // declares it: a text field named by nothing but its digits, which the
    // message keeps as a member and a file that names the tag names when the
    // two fold. The warning names the constraint and says so.
    let dangling = r#"<?xml version="1.0"?>
<cplugin-configuration fix-version="4.4">
	<vocabulary><vocabulary-tag name="35" alt="MsgType" type="string" /></vocabulary>
	<grammar-binding type="0"><grammar><tag-constraint name="35" part="body" /><tag-constraint name="99" part="body" /></grammar></grammar-binding>
</cplugin-configuration>"#;
    let (read, warnings) =
        super::warned::during(|| FixRegistry::from_cfb_file(&handle(dangling), None));
    let (registry, roots) = read.expect("a readable CBlock");
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    for held in [
        "tag 99",
        "a tag this file's vocabulary declares",
        "; the constraint declares the tag as text, named by its digits",
    ] {
        assert!(
            warnings[0].contains(held),
            "{held} missing from {warnings:?}"
        );
    }
    assert_eq!(children(&roots[0]), ["msgtype", "99"]);
    let declared = registry
        .field_by_tag(99)
        .expect("the constraint declared tag 99");
    assert_eq!(declared.name(), "99");
    assert_eq!(declared.dtype(), &DataType::utf8());
    assert!(declared.is_nullable());
}

#[test]
fn a_message_the_catalog_will_not_hold_is_named_at_its_grammar_binding() {
    // A NumInGroup counter stated beside the group it counts - the nested
    // grammar opens with 555 and the message states 555 again beside it - is
    // a grammar contradicting itself, since a group is its list and its
    // length the count. The message is dropped, every field it declared is
    // kept and every other message still binds, and the warning names the
    // binding that declared it by its line and column, as every other
    // warning names what it read, rather than the start of the file.
    let body = r#"<?xml version="1.0"?>
<cplugin-configuration fix-version="4.4">
	<vocabulary>
		<vocabulary-tag name="55" alt="Symbol" type="string" />
		<vocabulary-tag name="555" alt="NoLegs" type="integer" />
		<vocabulary-tag name="556" alt="LegCurrency" type="string" />
	</vocabulary>
	<grammar-binding type="8"><grammar><tag-constraint name="55" /></grammar></grammar-binding>
	<grammar-binding type="D">
		<grammar>
			<tag-constraint name="55" />
			<tag-constraint name="555" />
			<grammar><tag-constraint name="555" /><tag-constraint name="556" /></grammar>
		</grammar>
	</grammar-binding>
</cplugin-configuration>"#;
    let binding = r#"<grammar-binding type="D">"#;
    let at = body.find(binding).expect("the binding") + binding.len();
    let (read, warnings) =
        super::warned::during(|| FixRegistry::from_cfb_file(&handle(body), None));
    let (registry, roots) = read.expect("a readable CBlock");
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    for held in [
        format!("at byte {at}: line 9, column 28: message \"D\""),
        "expected no NumInGroup counter beside the group it counts, whose length is its count, got nolegs (555)".to_owned(),
        "; the message is dropped and every field it declared kept".to_owned(),
    ] {
        assert!(
            warnings[0].contains(&held),
            "{held} missing from {warnings:?}"
        );
    }
    assert_eq!(roots.len(), 1, "message 8 still binds");
    for tag in [55, 555, 556] {
        assert!(registry.field_by_tag(tag).is_ok(), "tag {tag} is kept");
    }
}

/// Asserts that `rendered` names the byte it states by the line and column
/// that byte falls on in `body`.
///
/// Every warning a CBlock read raises reads `... at byte N: line L, column
/// C: ...`, and this recomputes `L` and `C` from the document rather than
/// trusting the sentence, so a pin is a pin on the location and not on its
/// spelling alone.
#[track_caller]
fn assert_located(body: &str, rendered: &str) {
    let (_, after) = rendered
        .split_once(" at byte ")
        .unwrap_or_else(|| panic!("no byte in {rendered}"));
    let (byte, after) = after
        .split_once(": ")
        .unwrap_or_else(|| panic!("no reason in {rendered}"));
    let byte: usize = byte.parse().unwrap_or_else(|_| panic!("{rendered}"));
    let before = &body[..byte.min(body.len())];
    let line = before.matches('\n').count() + 1;
    let column = before.rfind('\n').map_or(byte, |at| byte - at - 1) + 1;
    let wanted = format!("line {line}, column {column}: ");
    assert!(
        after.starts_with(&wanted),
        "{wanted:?} missing from {rendered}"
    );
}

#[test]
fn a_warning_quotes_the_element_and_the_content_it_read() {
    // Each case: the document, every span the warning has to carry for a
    // reader to find the declaration in a file that is megabytes of them, and
    // what the reader did about it, after the semicolon.
    for (body, wanted, consequence) in [
        (
            r#"<?xml version="1.0"?>
<cplugin-configuration fix-version="4.4">
	<vocabulary><vocabulary-tag name="35" alt="MsgType" type="widget" /></vocabulary>
</cplugin-configuration>"#,
            // The eight are named, the word nothing reads is quoted, and the
            // element that declared it is quoted whole.
            vec![
                "string, char, integer, float, boolean, utc-date, utc-timestamp, utc-time-only",
                "or a datatype name",
                "\"widget\"",
                "<vocabulary-tag name=\\\"35\\\"",
            ],
            "the tag is typed string, which every FIX datatype is on the wire",
        ),
        (
            r#"<?xml version="1.0"?>
<cplugin-configuration fix-version="4.4">
	<vocabulary><vocabulary-tag name="MsgType" type="string" /></vocabulary>
</cplugin-configuration>"#,
            vec!["a decimal tag", "\"MsgType\""],
            "the declaration is dropped",
        ),
        (
            // The one tag parser the crate has is strict: a signed spelling
            // the writer never emits is no tag here either.
            r#"<?xml version="1.0"?>
<cplugin-configuration fix-version="4.4">
	<vocabulary><vocabulary-tag name="+35" alt="MsgType" type="string" /></vocabulary>
</cplugin-configuration>"#,
            vec!["a decimal tag", "\"+35\""],
            "the declaration is dropped",
        ),
        (
            r#"<?xml version="1.0"?>
<cplugin-configuration fix-version="4.4">
	<vocabulary><vocabulary-tag name="35" alt="MsgType" type="string" /></vocabulary>
	<grammar-binding type="P Report Ack"><grammar><tag-constraint name="99" part="body" /></grammar></grammar-binding>
</cplugin-configuration>"#,
            // A dangling tag names the message it dangles in, because a file
            // binds hundreds of them.
            vec![
                "tag 99",
                "message \"P Report Ack\"",
                "<tag-constraint name=\\\"99\\\"",
            ],
            "the constraint declares the tag as text, named by its digits",
        ),
        (
            r#"<?xml version="1.0"?>
<cplugin-configuration fix-version="4.4">
	<vocabulary><vocabulary-tag name="35" alt="MsgType" type="string" /></vocabulary>
	<grammar-binding type="0"><grammar><tag-constraint name="35" part="body">
		<string-validity regexp=".*" domain="some-values" />
	</tag-constraint></grammar></grammar-binding>
</cplugin-configuration>"#,
            vec!["all-values", "\"some-values\"", "<string-validity"],
            "the validity is read past, as every validity is",
        ),
    ] {
        let (read, warnings) =
            super::warned::during(|| FixRegistry::from_cfb_file(&handle(body), None));
        read.expect("a readable CBlock");
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        // The file's name first, where the handle has one, then the refusal.
        let rendered = warnings[0].clone();
        assert!(
            rendered.starts_with("one.cfb invalid cfb expression at byte "),
            "{rendered}"
        );
        // The byte, then the line and column it falls on.
        assert_located(body, &rendered);
        for held in wanted {
            assert!(rendered.contains(held), "{held} missing from {rendered}");
        }
        // What the reader did, stated once and last.
        assert!(
            rendered.ends_with(&format!("; {consequence}")),
            "{consequence:?} missing from {rendered}"
        );
        // Bounded: a warning never grows with the document it read.
        assert!(rendered.len() < 400, "{rendered}");
        // Both doors read the same documents, and warn with the same sentence
        // - the folding one naming the dialect it read the file under beside
        // the file's name, which is what tells one file's warnings from
        // another's in a glob.
        let mut folded = FixRegistry::new();
        let (also, spelled) =
            super::warned::during(|| folded.add_cfb_file(&handle(body), Some("bloomberg")));
        also.expect("a readable CBlock");
        let sentence = &rendered["one.cfb ".len()..];
        assert_eq!(spelled, [format!("one.cfb [bloomberg] {sentence}")]);
    }

    // A document that is not XML at all has no element to name and nothing
    // left to keep, so it is the one thing still refused.
    let malformed = r#"<?xml version="1.0"?>
<cplugin-configuration fix-version="4.4">
	<vocabulary><vocabulary-tag name="35"</vocabulary>
</cplugin-configuration>"#;
    let refused = FixRegistry::from_cfb_file(&handle(malformed), None).unwrap_err();
    let rendered = refused.to_string();
    for held in ["well-formed CBlock", "reading", "vocabulary-tag"] {
        assert!(rendered.contains(held), "{held} missing from {rendered}");
    }
    assert!(rendered.len() < 400, "{rendered}");
    let also = FixRegistry::new()
        .add_cfb_file(&handle(malformed), Some("bloomberg"))
        .unwrap_err();
    assert_eq!(also.to_string(), rendered);

    // An element longer than the budget is quoted up to it and elided, so a
    // vocabulary tag carrying a paragraph of attributes still names itself.
    let wide = format!(
        r#"<?xml version="1.0"?>
<cplugin-configuration fix-version="4.4">
	<vocabulary><vocabulary-tag name="35" alt="MsgType" type="widget" note="{}" /></vocabulary>
</cplugin-configuration>"#,
        "n".repeat(200)
    );
    let (read, warnings) =
        super::warned::during(|| FixRegistry::from_cfb_file(&handle(&wide), None));
    read.expect("a readable CBlock");
    let rendered = warnings.first().expect("one warning");
    assert!(rendered.contains("<vocabulary-tag name="), "{rendered}");
    assert!(rendered.contains('\u{2026}'), "{rendered}");
    assert!(rendered.len() < 400, "{rendered}");
}

#[test]
fn a_refusal_never_grows_with_the_document_that_raised_it() {
    // The reader's own sentence quotes the document too - an unmatched end tag
    // names both spellings - so a file can make one of them enormous, and the
    // budget every other span crosses is the one it crosses.
    let long = "a".repeat(2_000);
    let body = format!(
        "<?xml version=\"1.0\"?>\n<cplugin-configuration fix-version=\"4.4\">\n\t<{long}></vocabulary>\n</cplugin-configuration>"
    );
    let refused = FixRegistry::from_cfb_file(&handle(&body), None).unwrap_err();
    let rendered = refused.to_string();
    assert!(rendered.contains("well-formed CBlock"), "{rendered}");
    assert!(rendered.len() < 400, "{} bytes: {rendered}", rendered.len());
}

#[test]
fn a_warning_the_core_raised_names_the_declaration_that_asked_for_it() {
    // A spelling holding a control character is a broken identifier, not
    // layout: the warning names the tag, quotes the spelling, and keeps the
    // core's own sentence behind them.
    let spelling = r#"<?xml version="1.0"?>
<cplugin-configuration fix-version="4.4">
	<vocabulary><vocabulary-tag name="35" alt="Msg&#1;Type" type="string" /></vocabulary>
</cplugin-configuration>"#;
    let (read, warnings) =
        super::warned::during(|| FixRegistry::from_cfb_file(&handle(spelling), None));
    let (registry, _) = read.expect("a readable CBlock");
    assert!(
        registry.get_field_by_tag(35).is_none(),
        "the tag is dropped"
    );
    let rendered = warnings.first().expect("one warning");
    assert!(rendered.contains("tag 35 spelling"), "{rendered}");
    assert!(rendered.contains("Msg\\u{1}Type"), "{rendered}");
    assert!(rendered.contains("control characters"), "{rendered}");

    // Two declarations of one tag under two names are two fields: a name is
    // what identifies a field to a reader, so the second is registered under
    // its own identity beside the first, which keeps the bare tag and learns
    // nothing of the second's name. Nothing is dropped, so nothing is warned.
    let doubled = r#"<?xml version="1.0"?>
<cplugin-configuration fix-version="4.4">
	<vocabulary>
		<vocabulary-tag name="35" alt="MsgType" type="string" />
		<vocabulary-tag name="35" alt="SomethingElse" type="string" />
	</vocabulary>
	<grammar-binding type="0"><grammar><tag-constraint name="35" part="body" /></grammar></grammar-binding>
</cplugin-configuration>"#;
    let (read, warnings) =
        super::warned::during(|| FixRegistry::from_cfb_file(&handle(doubled), None));
    let (registry, roots) = read.expect("a readable CBlock");
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(super::scalars(&registry), 2 + super::seeded_fields());
    // The first declaration is the one the bare tag answers.
    let holder = registry.field_by_tag(35).unwrap();
    assert_eq!(holder.name(), "msgtype");
    assert_eq!(
        holder.as_fix().names().collect::<Vec<_>>(),
        [] as [&str; 0],
        "the holder lends no alias",
    );
    let second = registry
        .field_by_id(FixId::of(35, "SomethingElse").unwrap())
        .unwrap();
    assert_eq!(second.name(), "somethingelse");
    assert_eq!(second.as_fix().tag().unwrap(), Some(35));
    assert!(second.as_fix().names().next().is_none());
    assert_eq!(
        registry.get_field_by_name("SomethingElse").map(Field::name),
        Some("somethingelse"),
        "the second is reached by its own name",
    );
    // The constraint named tag 35, and the message keeps a child on it.
    let bound = roots[0].dtype().as_fields().unwrap();
    assert_eq!(bound.len(), 1);
    assert_eq!(bound[0].as_fix().tag().unwrap(), Some(35));
}

#[test]
fn a_fix_version_the_root_declares_is_read_past_whatever_it_says() {
    // Which version a run reads at is the codec's pin and not a vocabulary's,
    // so the root's `fix-version` is read past: one the version grammar could
    // not read, one an editor padded, and one absent altogether all read to
    // the same dictionary, and none of them is a thing to warn about.
    let unreadable = r#"<?xml version="1.0"?>
<cplugin-configuration fix-version="FIX.4.4">
	<vocabulary><vocabulary-tag name="35" alt="MsgType" type="string" /></vocabulary>
</cplugin-configuration>"#;
    let padded = r#"<?xml version="1.0"?>
<cplugin-configuration fix-version=" 4.4 ">
	<vocabulary><vocabulary-tag name="35" alt="MsgType" type="string" /></vocabulary>
</cplugin-configuration>"#;
    let silent = r#"<?xml version="1.0"?>
<cplugin-configuration>
	<vocabulary><vocabulary-tag name="35" alt="MsgType" type="string" /></vocabulary>
</cplugin-configuration>"#;
    let (baseline, warnings) =
        super::warned::during(|| FixRegistry::from_cfb_file(&handle(padded), Some(DIALECT)));
    let (baseline, _) = baseline.expect("a readable CBlock");
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(baseline.field_by_tag(35).unwrap().name(), "msgtype");
    assert_eq!(baseline.dialects(), [DIALECT]);
    for body in [unreadable, silent] {
        let (read, warnings) =
            super::warned::during(|| FixRegistry::from_cfb_file(&handle(body), Some(DIALECT)));
        let (registry, _) = read.expect("a readable CBlock");
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(registry, baseline);
    }
}

#[test]
fn a_description_keeps_its_words_and_loses_its_layout() {
    // The shape a production file has: one description wrapped over indented
    // lines and holding the separator byte it describes, and one that is
    // nothing but layout.
    let body = r#"<?xml version="1.0"?>
<cplugin-configuration fix-version="4.4">
	<vocabulary>
		<vocabulary-tag name="58" alt="Text" type="string">
			<description>Free format text string.
			May hold the separator SEPARATOR, an escaped one &#1;, and a &lt;SOH&gt;.</description>
		</vocabulary-tag>
		<vocabulary-tag name="59" alt="TimeInForce" type="char">
			<description>
			</description>
		</vocabulary-tag>
		<vocabulary-tag name="60" alt="TransactTime" type="utc-timestamp">
			<description><![CDATA[Held as <yyyymmdd-hh:mm:ss> & nothing else.]]></description>
		</vocabulary-tag>
	</vocabulary>
</cplugin-configuration>"#
        .replace("SEPARATOR", "\u{1}");

    let (registry, _) =
        FixRegistry::from_cfb_file(&handle(&body), None).expect("a readable CBlock");
    assert_eq!(
        registry.field_by_tag(58).unwrap().as_fix().description(),
        Some("Free format text string. May hold the separator , an escaped one , and a <SOH>."),
    );
    // Layout alone is the file saying nothing, exactly as `<description />` is.
    assert_eq!(
        registry.field_by_tag(59).unwrap().as_fix().description(),
        None
    );
    // A CDATA section is how a description holds a `<` or an `&` without
    // escaping one, so it is content and is never unescaped again.
    assert_eq!(
        registry.field_by_tag(60).unwrap().as_fix().description(),
        Some("Held as <yyyymmdd-hh:mm:ss> & nothing else."),
    );

    // The folding door reads the same file the same way.
    let mut folded = FixRegistry::new();
    folded
        .add_cfb_file(&handle(&body), Some("bloomberg"))
        .unwrap();
    assert_eq!(
        folded.field_by_tag(58).unwrap().as_fix().description(),
        registry.field_by_tag(58).unwrap().as_fix().description(),
    );
}

#[test]
fn only_the_description_element_describes_a_tag() {
    // A `vocabulary-tag` may carry other text-bearing children, and their
    // words are theirs: folding them in would be inventing a sentence. A
    // validity child's own `description` is the deepest form of that, and two
    // of the tag's own are two sentences rather than one longer word.
    let body = r#"<?xml version="1.0"?>
<cplugin-configuration fix-version="4.4">
	<vocabulary>
		<vocabulary-tag name="35" alt="MsgType" type="string">
			<string-validity domain="all-values"><description>Any string.</description></string-validity>
			<description>The message type.</description>
			<comment>Never read.</comment>
			<description>Case-bearing.</description>
		</vocabulary-tag>
	</vocabulary>
</cplugin-configuration>"#;
    let (registry, _) = FixRegistry::from_cfb_file(&handle(body), None).expect("a readable CBlock");
    assert_eq!(
        registry.field_by_tag(35).unwrap().as_fix().description(),
        Some("The message type. Case-bearing."),
    );
}

#[test]
fn a_document_cut_short_is_refused_rather_than_read_as_a_shorter_one() {
    // What a partial download and a half-written file look like: the reader
    // answers no error for an element left open, so every loop that reads to
    // its own end tag says so itself.
    let whole = r#"<?xml version="1.0"?>
<cplugin-configuration fix-version="4.4">
	<vocabulary>
		<vocabulary-tag name="35" alt="MsgType" type="string">
			<description>The message type.</description>
		</vocabulary-tag>
		<vocabulary-tag name="55" alt="Symbol" type="string" />
	</vocabulary>
	<grammar-binding type="D"><grammar><tag-constraint name="35" part="body" /></grammar></grammar-binding>
</cplugin-configuration>"#;
    let (registry, roots) =
        FixRegistry::from_cfb_file(&handle(whole), None).expect("a readable CBlock");
    assert_eq!(
        (super::scalars(&registry), roots.len()),
        (2 + super::seeded_fields(), 1)
    );

    for (cut, wanted) in [
        ("</description>", "vocabulary-tag"),
        ("\t\t<vocabulary-tag name=\"55\"", "vocabulary"),
        ("</grammar>", "grammar"),
    ] {
        let at = whole.find(cut).expect("a cut inside the document");
        let refused = FixRegistry::from_cfb_file(&handle(&whole[..at]), None).unwrap_err();
        let rendered = refused.to_string();
        assert!(
            rendered.contains(&format!("a closed <{wanted}>")),
            "{rendered}"
        );
        assert!(rendered.contains("end of the document"), "{rendered}");
    }

    // A cut between two of the root's children left nothing open but the
    // root, so it reads as what it holds. Only the top-level loop's end of
    // document is an ending, and this is the shape that says so.
    let at = whole.find("\t<grammar-binding").expect("the binding");
    let (registry, roots) =
        FixRegistry::from_cfb_file(&handle(&whole[..at]), None).expect("a readable prefix");
    assert_eq!(
        (super::scalars(&registry), roots.len()),
        (2 + super::seeded_fields(), 0)
    );
}

#[test]
fn nesting_past_the_guard_is_dropped_rather_than_overflowing() {
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

    let (read, warnings) =
        super::warned::during(|| FixRegistry::from_cfb_file(&handle(&body), None));
    let (registry, roots) = read.expect("a readable CBlock");
    let rendered = warnings.first().expect("one warning");
    assert!(rendered.contains("deep"), "{rendered}");
    // The message it nested in, because a file binds hundreds of them.
    assert!(rendered.contains("message \"0\""), "{rendered}");
    // The guard stops the reader descending; it does not throw the file away,
    // nor the message: thirty grammars over one tag are thirty shapes of one
    // name - each nested one level deeper than the last, and nesting is
    // structure - and each is a definition of its own, split for the message
    // in the order it declares them.
    assert_eq!(roots.len(), 1, "{warnings:?}");
    assert!(
        !warnings.iter().any(|held| held.contains("per context")),
        "{warnings:?}"
    );
    for name in [
        "legs",
        "legs_message30",
        "legs_message30_2",
        "legs_message30_29",
    ] {
        assert!(
            registry
                .get_definition(yggdryl::FixCategory::Groups, name)
                .is_some(),
            "{name}"
        );
    }
    // The vocabulary the file declared is still a dictionary.
    assert_eq!(registry.field_by_tag(555).unwrap().name(), "nolegs");
}

#[test]
fn the_message_types_a_file_declares_become_the_code_set_of_tag_35() {
    let (registry, _) = parse(CBLOCK);
    let msgtype = registry.field_by_tag(35).expect("MsgType");
    // The field names the set and the dictionary holds its members.
    let view = registry
        .codeset_of(msgtype)
        .expect("the set tag 35 reads by");

    // A CBlock spells a type as the wire value and a qualifier, and the wire
    // value is what tag 35 carries: `P Report Ack` is `P` used as a report
    // ack, under the wording the listing gave it. The qualified spelling is
    // only ever a spelling - a file that knows `P` exists has not said what
    // anyone calls it, so the code is named after the value it is and never
    // after the qualifier.
    assert_eq!(view.code_value("P Report Ack"), Some("P"));
    assert_eq!(view.code_name("P"), Some("P"));
    assert_eq!(
        view.code_by_name("P Report Ack")
            .and_then(|code| code.parse_doc().ok().flatten()),
        Some("Allocation Report ACK".to_owned()),
    );

    // The qualifier is the direction as often as a role, and both directions
    // are one type on the wire: one code, answering to both spellings.
    assert_eq!(view.code_value("AR Inbound"), Some("AR"));
    assert_eq!(view.code_value("AR Outbound"), Some("AR"));
    assert_eq!(view.code_name("AR"), Some("AR"));
    assert_eq!(
        view.codes().count(),
        4,
        "AR was declared twice and is one code"
    );

    // And a name the dictionary this folds into already carries takes the
    // placeholder's place, rather than the dialect renaming the type.
    let mut dictionary = FixRegistry::new();
    dictionary
        .set_codeset(
            "msgtypecodeset",
            &[yggdryl::FixCode::new("AllocationReportAck", "P")],
        )
        .unwrap();
    let mut held = DataType::utf8().nullable_field("MsgType");
    held.as_fix_mut().set_tag(35).unwrap();
    held.as_fix_mut().set_codeset("msgtypecodeset").unwrap();
    dictionary.insert(held).unwrap();
    dictionary.merge_with(&registry).unwrap();
    let merged = dictionary.field_by_tag(35).unwrap();
    let set = dictionary
        .codeset_of(merged)
        .expect("the set both dictionaries name");
    assert_eq!(set.code_name("P"), Some("AllocationReportAck"));
    assert_eq!(set.code_value("P Report Ack"), Some("P"));

    // Two roles of one wire type are one code too: `c SDR` and `c SLR` are
    // both tag 35 `c`, and a code set keys on the wire.
    assert_eq!(view.code_value("c SLR"), Some("c"));

    // The mapping table spells the same type the way UlMessage does, so that
    // spelling reaches the value rather than declaring a second type.
    assert_eq!(view.code_value("allocationreportack"), Some("P"));

    // A bound type the listing never mentioned is still a type this dialect
    // carries: the binding declares `7`.
    assert_eq!(view.code_value("7"), Some("7"));

    // The folding door reads the same file the same way.
    let mut folded = FixRegistry::new();
    folded
        .add_cfb_file(&handle(CBLOCK), Some("bloomberg"))
        .unwrap();
    let held = folded.field_by_tag(35).expect("MsgType");
    assert_eq!(
        folded
            .codeset_of(held)
            .expect("the set tag 35 reads by")
            .code_value("AR Outbound"),
        Some("AR")
    );
}

#[test]
fn a_refused_cfb_field_cannot_replace_a_builtin_code_set() {
    let body = r#"<?xml version="1.0"?>
<cplugin-configuration fix-version="4.4">
    <vocabulary><vocabulary-tag name="9001" alt="MarketDataKind" type="string" /></vocabulary>
    <maps><map name="MarketDataKind"><entries><entry key="X" value="Injected" /></entries></map></maps>
</cplugin-configuration>"#;
    let baseline = FixRegistry::new();
    let codes = baseline
        .codeset("marketdatakindcodeset")
        .expect("the builtin categories")
        .document()
        .to_owned();
    let (read, warnings) =
        super::warned::during(|| FixRegistry::from_cfb_file(&handle(body), None));
    let (registry, _) = read.expect("the rejected declaration is a warning");
    assert!(registry.get_field_by_tag(9001).is_none());
    let held = registry
        .codeset("marketdatakindcodeset")
        .expect("the builtin categories");
    assert_eq!(held.document(), codes);
    assert_eq!(held.codes().count(), 26);
    assert!(
        warnings
            .iter()
            .any(|warning| warning.contains("marketdatakind")),
        "{warnings:?}"
    );
}

#[test]
fn adding_a_cfb_code_set_forgets_warm_typed_parse_memos() {
    let mut registry = Arc::new(parse(CBLOCK).0);
    {
        let codec = super::fixed_codec(Arc::clone(&registry));
        let message = codec
            .parse_line(b"8=FIX.4.4|35=P|4=venue_buy|10=0|")
            .unwrap()
            .next()
            .unwrap()
            .unwrap();
        assert_eq!(message.by_tag(4).unwrap().as_str(), Some("venue_buy"));
    }
    assert_eq!(Arc::strong_count(&registry), 1);
    // Cached plans retain Weak references. `make_mut` dissociates those
    // without cloning while this remains the sole strong owner.
    Arc::make_mut(&mut registry)
        .add_cfb_file(&handle(CODESET_OVERLAY), Some(DIALECT))
        .unwrap();
    let codec = super::fixed_codec(Arc::clone(&registry));
    let message = codec
        .parse_line(b"8=FIX.4.4|35=P|4=venue_buy|10=0|")
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(message.by_tag(4).unwrap().as_str(), Some("B"));
}

#[test]
fn a_file_declaring_no_message_type_tag_keeps_its_types_out_of_the_dictionary() {
    // A message type is a code of tag 35 and never a field of its own, so a
    // file that declares no tag 35 has nowhere to put one.
    let body = r#"<?xml version="1.0"?>
<cplugin-configuration fix-version="4.4">
	<message-types><message-type value="D" description="Order - Single" /></message-types>
	<vocabulary><vocabulary-tag name="55" alt="Symbol" type="string" /></vocabulary>
</cplugin-configuration>"#;
    let (registry, _) = FixRegistry::from_cfb_file(&handle(body), None).expect("a readable CBlock");
    assert_eq!(super::scalars(&registry), 1 + super::seeded_fields());
    assert!(registry.get_field_by_tag(35).is_none());
}

#[test]
fn a_map_becomes_the_code_set_of_the_tag_it_decodes() {
    let (registry, _) = parse(CBLOCK);

    // `ADVSIDE` is not how the vocabulary displays `AdvSide`, so the map is
    // written the UlMessage way: the name keys and the value is the value,
    // which is already the order a code set wants.
    let advside = registry.field_by_tag(4).expect("AdvSide");
    let view = registry.codeset_of(advside).expect("the set it reads by");
    assert_eq!(view.code_value("buy"), Some("B"));
    assert_eq!(view.code_name("X"), Some("cross"));
    assert_eq!(view.codes().count(), 4);

    // `TimeInForce` is exactly how the vocabulary displays it, so that map is
    // written the FIX way and the key is the wire value.
    let timeinforce = registry.field_by_tag(59).expect("TimeInForce");
    let view = registry
        .codeset_of(timeinforce)
        .expect("the set it reads by");
    assert_eq!(view.code_value("day"), Some("0"));
    assert_eq!(view.code_name("1"), Some("goodtillcancel"));

    // A map naming no field is skipped rather than refused: a CBlock maps
    // things that are not fields.
    assert_eq!(registry.get_field_by_name("notafield"), None);
}

#[test]
fn a_map_is_oriented_by_its_name_and_never_by_the_shape_of_an_entry() {
    let (registry, _) = parse(ORIENTATIONS);

    // Named exactly as the vocabulary displays the field, so the key is the
    // wire value however long it runs. Reading the shorter side as the value
    // would have put `fx` on the wire and `FXSPOT` in a name.
    let security = registry.field_by_tag(167).expect("SecurityType");
    let view = registry.codeset_of(security).expect("the set it reads by");
    assert_eq!(view.code_value("fx"), Some("FXSPOT"));
    assert_eq!(view.code_name("CS"), Some("equity"));

    // The mirror of that map under a name the file does not display the field
    // by lands identically, which is the whole rule: the name decides, and the
    // entries are read whichever way it says.
    let underlying = registry.field_by_tag(310).expect("UnderlyingSecurityType");
    let view = registry
        .codeset_of(underlying)
        .expect("the set it reads by");
    assert_eq!(view.code_value("fx"), Some("FXSPOT"));
    assert_eq!(view.code_name("CS"), Some("equity"));

    // A tag declaring no `alt` is displayed as the tag itself, so a map named
    // for it matches and is read the FIX way.
    let extension = registry.field_by_tag(22830).expect("22830");
    assert_eq!(
        registry
            .codeset_of(extension)
            .expect("the set it reads by")
            .code_value("one"),
        Some("1")
    );
}

#[test]
fn a_map_reaches_the_field_it_spells_and_one_entry_never_refuses_the_file() {
    let (registry, _) = parse(AWKWARD);

    // `Ex_Destination` folds onto `ExDestination` and is declared first, so
    // resolving by the fold alone would put the set on the wrong tag and,
    // failing the strict compare there, read it backwards as well.
    let destination = registry.field_by_tag(20000).expect("ExDestination");
    assert_eq!(
        registry
            .codeset_of(destination)
            .expect("the set it reads by")
            .code_value("paris"),
        Some("XPAR")
    );
    assert_eq!(registry.field_by_tag(100).unwrap().as_fix().codeset(), None);

    // Whitespace around a name is not a spelling, so it neither breaks the
    // match nor flips the orientation. An entry stating nothing on a side is
    // dropped whether it says so with an empty attribute or with none. One
    // repeating a name an earlier entry claimed loses the name and keeps its
    // wire value under none: a code set may not name one member twice, the
    // value is still a fact about the wire, and one contradictory entry is
    // not a reason to refuse the file.
    let timeinforce = registry.field_by_tag(59).expect("TimeInForce");
    let view = registry
        .codeset_of(timeinforce)
        .expect("the set it reads by");
    assert_eq!(view.code_value("day"), Some("0"));
    assert_eq!(view.code_value("goodtilldate"), Some("6"));
    assert_eq!(view.code_name("1"), Some("1"));
    assert_eq!(view.code_name("2"), None);
    assert_eq!(view.codes().count(), 3);

    // Two names for one wire value is an alias rather than a contradiction,
    // so the second is kept as one rather than dropped or made a second code.
    let advside = registry.field_by_tag(4).expect("AdvSide");
    let view = registry.codeset_of(advside).expect("the set it reads by");
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
fn a_files_whole_vocabulary_lands_with_the_maps_that_sit_past_its_grammar() {
    let (registry, roots) = parse(CBLOCK);

    // Every tag the file declared is in the dictionary, and every one is a
    // member of the dialect - a standard tag the file speaks as much as a
    // venue's own. A dictionary answers tag-major, so what the file declared
    // is what is pinned here and not the order it declared them in.
    for name in [
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
    ] {
        let held = registry.field_by_name(name).expect(name);
        assert!(held.as_fix().has_source(DIALECT), "{name}");
    }

    // Every field is keyed, so it enters a dictionary as it stands.
    let avgpx = registry.field_by_name("avgpx").expect("AvgPx");
    assert_eq!(avgpx.as_fix().tag().unwrap(), Some(6));
    assert_eq!(sources(avgpx), [DIALECT]);
    assert_eq!(
        sources(registry.field_by_name("exludeddealers").unwrap()),
        [DIALECT]
    );

    // The maps sit past the grammar bindings, so a code set proves the whole
    // document was read and not just its first pass.
    let advside = registry.field_by_name("advside").expect("AdvSide");
    assert_eq!(
        registry
            .codeset_of(advside)
            .expect("the set it reads by")
            .code_value("buy"),
        Some("B")
    );

    // Fifteen declarations, one of them the standard clock seed's own tag.
    assert_eq!(super::scalars(&registry), 14 + super::seeded_fields());
    assert_eq!(roots.len(), 1);
    assert_eq!(registry.dialects(), [DIALECT]);
}

#[test]
fn an_unnamed_file_takes_its_dialect_from_its_own_stem() {
    // A CBlock never names itself, so the file standing in for the caller is
    // the stem and nothing else of the path, folded by case as every
    // membership is.
    let mut stemmed = FixRegistry::new();
    stemmed
        .add_cfb_file(&named_handle(CBLOCK, "MSFIX44.cfb"), None)
        .expect("a readable CBlock");
    assert_eq!(
        sources(stemmed.field_by_name("exludeddealers").unwrap()),
        ["msfix44"]
    );
    assert_eq!(
        sources(stemmed.field_by_name("avgpx").unwrap()),
        ["msfix44"]
    );

    // An explicit name still wins over the stem.
    let mut named = FixRegistry::new();
    named
        .add_cfb_file(&named_handle(CBLOCK, "MSFIX44.cfb"), Some(DIALECT))
        .expect("a readable CBlock");
    assert_eq!(
        sources(named.field_by_name("exludeddealers").unwrap()),
        [DIALECT]
    );

    // Bytes held in memory are named by the caller: a buffer's URL is an
    // identity and not a location, and a name the caller states is the
    // dialect whatever the handle answers.
    let mut buffer = Buffer::new();
    buffer.write_all_bytes(CBLOCK.as_bytes()).unwrap();
    let mut buffered = FixRegistry::new();
    buffered
        .add_cfb_file(&buffer, Some(DIALECT))
        .expect("a readable CBlock");
    assert_eq!(
        sources(buffered.field_by_name("exludeddealers").unwrap()),
        [DIALECT]
    );
    for field in buffered.iter() {
        // Crate fields and unstated SendingTime are seeds, not this file's
        // declarations.
        if field
            .as_fix()
            .tag()
            .unwrap()
            .is_some_and(|tag| yggdryl::is_crate_tag(tag) || tag == 52)
        {
            continue;
        }
        assert!(field.as_fix().has_source(DIALECT), "{}", field.name());
    }
}

#[test]
fn a_name_that_cannot_be_a_source_id_is_refused_rather_than_folded_into_one() {
    // A supplied membership is held to the source id grammar - a non-empty
    // word holding no quote, backslash or control character - and nothing
    // else: a leading digit, a comma or a long name is a name. A stem stands
    // in for a name the caller did not supply, so it is taken only where it
    // reads as one, opening with a letter: `4.4-ms.cfb` names nothing on its
    // own and stamps nothing, while the same spelling supplied is a
    // membership. A dictionary keyed on a guess is worse than a refusal, so
    // the two that cannot be one are refused rather than repaired, and by
    // every door before a byte of the file is read.
    let mut numbered = FixRegistry::new();
    numbered
        .add_cfb_file(&named_handle(CBLOCK, "4.4-ms.cfb"), None)
        .expect("a stem that is not a name stands in for nothing");
    assert!(sources(numbered.field_by_name("exludeddealers").unwrap()).is_empty());
    let mut supplied = FixRegistry::new();
    supplied
        .add_cfb_file(&named_handle(CBLOCK, "4.4-ms.cfb"), Some("4.4-ms"))
        .expect("a supplied name beginning with a digit is a name");
    assert_eq!(
        sources(supplied.field_by_name("exludeddealers").unwrap()),
        ["4.4-ms"]
    );
    let mut long = FixRegistry::new();
    long.add_cfb_file(
        &named_handle(CBLOCK, "a-name-well-past-the-old-inline-cap.cfb"),
        None,
    )
    .expect("a long stem is a name");
    assert_eq!(
        sources(long.field_by_name("exludeddealers").unwrap()),
        ["a-name-well-past-the-old-inline-cap"]
    );
    // A buffer is identified by an address, which is a stem and not a name.
    let mut buffered = FixRegistry::new();
    buffered
        .add_cfb_file(&Buffer::from_bytes(CBLOCK.as_bytes().to_vec()), None)
        .expect("bytes held in memory are named by the caller or not at all");
    assert!(sources(buffered.field_by_name("exludeddealers").unwrap()).is_empty());
    // A stem opening with a letter that the id grammar refuses - a quote in
    // it - is no id either: it stands in for nothing, by the one door and by
    // the plural, and the file folds as a bare vocabulary rather than being
    // left out for a name nobody supplied.
    let mut quoted = FixRegistry::new();
    quoted
        .add_cfb_file(&named_handle(CBLOCK, "ms\"fix.cfb"), None)
        .expect("a stem that is no id stands in for nothing");
    assert!(sources(quoted.field_by_name("exludeddealers").unwrap()).is_empty());
    assert!(quoted.dialects().is_empty());
    assert_eq!(quoted.sources().count(), 0);
    let tree = cblock_tree(&[("ms\"fix.cfb", &one_tag(9001, "MsVenueRef"))]);
    let mut plural = FixRegistry::new();
    let merge = plural
        .add_cfb_files(std::slice::from_ref(&tree), None)
        .expect("a stem that is no id stands in for nothing");
    assert!(merge.failed.is_empty(), "{:?}", merge.failed);
    assert_eq!(merge.sources, 1);
    assert!(sources(plural.field_by_tag(9001).unwrap()).is_empty());
    assert_eq!(plural.sources().count(), 0);

    // A comma is an ordinary character of an id: the stored array is what
    // separates the ids.
    let mut comma = FixRegistry::new();
    comma
        .add_cfb_file(&handle(CBLOCK), Some("MS,Bloomberg"))
        .expect("a name holding a comma is a name");
    assert_eq!(
        sources(comma.field_by_name("exludeddealers").unwrap()),
        ["ms,bloomberg"]
    );
    assert_eq!(
        comma.get_source("ms,bloomberg").unwrap().file(),
        Some("one.cfb"),
        "the handle names the file"
    );

    let refused = |error: &Error| matches!(error, Error::InvalidMetadataValue { key, .. } if key == "FIX:sources");
    let error = FixRegistry::from_cfb_file(&handle(CBLOCK), Some("ms\"bloomberg")).unwrap_err();
    assert!(refused(&error), "{error}");
    assert!(error.to_string().contains("quote"), "{error}");
    assert!(error.to_string().contains("\"ms\\\"bloomberg\""), "{error}");
    let error = FixRegistry::from_cfb_file(&handle(CBLOCK), Some("")).unwrap_err();
    assert!(refused(&error), "{error}");
    assert!(error.to_string().contains("non-empty"), "{error}");
    let mut dictionary = FixRegistry::new();
    let error = dictionary
        .add_cfb_file(&handle(CBLOCK), Some("ms\"bloomberg"))
        .unwrap_err();
    assert!(refused(&error), "{error}");
    assert_eq!(
        super::scalars(&dictionary),
        super::seeded_fields(),
        "a refused read writes nothing"
    );
    assert!(dictionary.dialects().is_empty());
}

#[test]
fn a_cblock_vocabulary_folds_into_a_dictionary_that_already_exists() {
    // Two counterparties' files meet in the one namespace a dictionary is:
    // a tag both declare under one name is one field, and each file's name
    // is recorded on it - which is the case the fold exists for.
    let mut dictionary = FixRegistry::new();
    dictionary
        .add_cfb_file(&handle(CBLOCK), Some(DIALECT))
        .expect("one file's vocabulary");
    let before = dictionary.len();

    let yggdryl::FixMerge { added, merged, .. } = dictionary
        .add_cfb_file(&handle(OVERLAY), Some("morgan"))
        .expect("the second file folds into the first");
    // One tag the second file alone declares, and three that fold: the one
    // both files declare, and the two standard clocks every parsed dictionary
    // carries from construction and so states back at whatever it folds into.
    assert_eq!((added, merged), (1, 3));
    assert_eq!(dictionary.len(), before + added);

    // The fold keeps what only the stored definition declared, where the
    // wholesale replace an `insert` performs would drop it, and unions the
    // membership: both dictionaries speak tag 6.
    let avgpx = dictionary.field_by_tag(6).unwrap();
    assert_eq!(avgpx.name(), "avgpx");
    assert!(
        avgpx
            .description()
            .is_some_and(|held| held.contains("average price")),
        "the first file's description survived a file that carries none",
    );
    assert_eq!(sources(avgpx), [DIALECT, "morgan"]);
    // And a tag only the second file declares arrives, as its member alone.
    let price = dictionary.field_by_name("price").unwrap();
    assert_eq!(price.name(), "price");
    assert_eq!(sources(price), ["morgan"]);
    assert_eq!(dictionary.dialects(), [DIALECT, "morgan"]);

    // The same vocabulary read again under a second dialect is the same
    // fields: nothing is added, every one merges, and every one now names
    // both dictionaries - the user range included, because a tag is what
    // identifies a field on the wire and no dictionary owns a range of them.
    // Sixteen rather than fifteen: the parsed dictionary states the standard
    // clock this one already holds back at it, and a restated field merges.
    let yggdryl::FixMerge { added, merged, .. } = dictionary
        .add_cfb_file(&named_handle(CBLOCK, "morgan.cfb"), None)
        .expect("the same vocabulary under a second dialect");
    assert_eq!(
        (added, merged),
        (0, 16),
        "one vocabulary, whichever file spoke it"
    );
    assert_eq!(
        sources(dictionary.field_by_tag(10001).unwrap()),
        [DIALECT, "morgan"]
    );
    assert_eq!(dictionary.dialects(), [DIALECT, "morgan"]);
}

#[test]
fn folding_a_cblock_into_the_committed_dictionary_keeps_what_it_holds_and_restates_what_the_file_said_less_precisely()
 {
    let mut seeded = super::committed_registry().as_ref().clone();
    let before = seeded.clone();

    // Two declarations say less than the dictionary rather than something
    // else: AvgPx's `float` is a coarser number than its committed
    // `decimal128(38, 18)`, and LegCurrency's `string` a coarser statement of
    // its `ccy`. Each folds under the datatype the dictionary holds, counted
    // as restated, and nothing is passed over.
    let (vocabulary, _) = FixRegistry::from_cfb_file(&handle(CBLOCK), Some("bloomberg")).unwrap();
    assert_eq!(
        vocabulary.field_by_tag(6).unwrap().dtype(),
        &DataType::Float64
    );
    let merge = seeded.merge_with(&vocabulary).unwrap();
    assert!(merge.dropped.is_empty(), "{:?}", merge.dropped);
    assert_eq!(merge.restated, 2);
    for tag in [6, 556] {
        let held = seeded.field_by_tag(tag).unwrap();
        assert_eq!(
            held.dtype(),
            before.field_by_tag(tag).unwrap().dtype(),
            "tag {tag} keeps its stored datatype"
        );
        assert!(held.as_fix().has_source("bloomberg"), "tag {tag}");
    }
    let held = seeded.field_by_tag(6).unwrap();
    assert_eq!(held.dtype(), &DataType::DECIMAL);
    assert!(
        seeded
            .field_by_tag(10001)
            .unwrap()
            .as_fix()
            .has_source("bloomberg"),
        "the rest of the file arrived"
    );
    assert_eq!(seeded.field_by_tag(35).unwrap().dtype(), &DataType::utf8());

    // A field written on its own is still refused whole: a caller stating one
    // field asked for that field at that datatype, and nothing else is there
    // to keep - the strict doors restate nothing.
    let before = seeded.clone();
    let avgpx = vocabulary.field_by_tag(6).unwrap().clone();
    let error = seeded.add_fields([avgpx]).unwrap_err();
    assert!(matches!(error, Error::InvalidRecord { .. }), "{error}");
    let message = error.to_string();
    assert!(
        message.contains("avgpx") && message.contains("float64"),
        "{message}"
    );
    assert_eq!(seeded, before, "a refused write writes nothing");
}

#[test]
fn a_spelling_two_tags_share_names_neither_of_them() {
    // Two tags whose `alt` folds to one name, which contends in the one
    // namespace a dictionary is.
    let clashing = r#"<?xml version="1.0"?>
<cplugin-configuration fix-version="4.4">
	<vocabulary>
		<vocabulary-tag name="44" alt="Price" type="float" />
		<vocabulary-tag name="3044" alt="Price" type="float" />
	</vocabulary>
</cplugin-configuration>"#;
    let (registry, _) =
        FixRegistry::from_cfb_file(&handle(clashing), Some(DIALECT)).expect("a readable CBlock");
    // Each named by its own decimal - the identity a tag declaring no `alt`
    // already takes - and each keeping what the file called it.
    for tag in [44, 3044] {
        let field = registry.field_by_tag(tag).expect("a declared tag");
        assert_eq!(field.name(), tag.to_string());
        assert_eq!(field.display(), Some("Price"));
    }
    // And the spelling names neither.
    assert!(registry.get_field_by_name("Price").is_none());

    // Each names the other's tag, so a reader holding either reaches the one
    // the file said the same thing about.
    assert_eq!(
        registry.field_by_tag(44).unwrap().as_fix().tags().unwrap(),
        vec![3044]
    );
    assert_eq!(
        registry
            .field_by_tag(3044)
            .unwrap()
            .as_fix()
            .tags()
            .unwrap(),
        vec![44]
    );

    // Three tags sharing a spelling link none of each other: an alternate
    // identifier names one field, and three would each claim the other two.
    let crowded = r#"<?xml version="1.0"?>
<cplugin-configuration fix-version="4.4">
	<vocabulary>
		<vocabulary-tag name="44" alt="Price" type="float" />
		<vocabulary-tag name="3044" alt="Price" type="float" />
		<vocabulary-tag name="3045" alt="Price" type="float" />
	</vocabulary>
</cplugin-configuration>"#;
    let (registry, _) =
        FixRegistry::from_cfb_file(&handle(crowded), Some(DIALECT)).expect("a readable CBlock");
    for tag in [44, 3044, 3045] {
        let field = registry.field_by_tag(tag).expect("a declared tag");
        assert_eq!(field.name(), tag.to_string());
        assert!(field.as_fix().tags().unwrap().is_empty(), "tag {tag}");
    }

    // The folding door reads the same file the same way.
    let mut folded = FixRegistry::new();
    folded
        .add_cfb_file(&handle(clashing), Some("bloomberg"))
        .expect("a readable CBlock");
    for tag in [44, 3044] {
        assert_eq!(
            folded.field_by_tag(tag).expect("a declared tag").name(),
            tag.to_string()
        );
    }

    // A spelling that is another tag's own decimal contends with that tag's
    // identity, so it names neither either - and the tag it named keeps the
    // spelling nothing contends.
    let numbered = r#"<?xml version="1.0"?>
<cplugin-configuration fix-version="4.4">
	<vocabulary>
		<vocabulary-tag name="44" alt="3044" type="float" />
		<vocabulary-tag name="3044" alt="LastPx" type="float" />
	</vocabulary>
</cplugin-configuration>"#;
    let (registry, _) =
        FixRegistry::from_cfb_file(&handle(numbered), Some(DIALECT)).expect("a readable CBlock");
    assert_eq!(registry.field_by_tag(44).unwrap().name(), "44");
    assert_eq!(registry.field_by_tag(44).unwrap().display(), Some("3044"));
    assert_eq!(registry.field_by_tag(3044).unwrap().name(), "lastpx");

    // Contended by the fold the dictionary hashes a name with, not by the
    // spelling: a name that differs only by a separator is the same name
    // there, so it contends here.
    let separated = r#"<?xml version="1.0"?>
<cplugin-configuration fix-version="4.4">
	<vocabulary>
		<vocabulary-tag name="44" alt="Last_Px" type="float" />
		<vocabulary-tag name="3044" alt="LastPx" type="float" />
	</vocabulary>
</cplugin-configuration>"#;
    let (registry, _) =
        FixRegistry::from_cfb_file(&handle(separated), Some(DIALECT)).expect("a readable CBlock");
    assert_eq!(registry.field_by_tag(44).unwrap().name(), "44");
    assert_eq!(
        registry.field_by_tag(44).unwrap().display(),
        Some("Last_Px")
    );
    assert_eq!(registry.field_by_tag(3044).unwrap().name(), "3044");
    assert!(registry.get_field_by_name("LastPx").is_none());

    // A map naming that spelling decodes neither: one code set and nothing
    // in the file saying which of the two tags it belongs on.
    let mapped = r#"<?xml version="1.0"?>
<cplugin-configuration fix-version="4.4">
	<vocabulary>
		<vocabulary-tag name="44" alt="Price" type="string" />
		<vocabulary-tag name="3044" alt="Price" type="string" />
		<vocabulary-tag name="4" alt="AdvSide" type="char" />
	</vocabulary>
	<maps>
		<map name="Price"><entries><entry key="1" value="one" /></entries></map>
		<map name="AdvSide"><entries><entry key="B" value="buy" /></entries></map>
	</maps>
</cplugin-configuration>"#;
    let (registry, _) =
        FixRegistry::from_cfb_file(&handle(mapped), Some(DIALECT)).expect("a readable CBlock");
    for tag in [44, 3044] {
        assert_eq!(
            registry.field_by_tag(tag).unwrap().as_fix().codeset(),
            None,
            "tag {tag} keeps no code set from an ambiguous map",
        );
    }
    assert_eq!(
        registry
            .codeset_of(registry.field_by_tag(4).unwrap())
            .expect("the set AdvSide reads by")
            .codes()
            .count(),
        1,
        "a map naming one tag still decodes it",
    );

    // A dictionary is one namespace and no dialect owns a range of it, so a
    // venue's own tag spelled with a name FIX already has contends exactly as
    // two standard tags do: neither is named by it, and each names the other.
    let ranged = r#"<?xml version="1.0"?>
<cplugin-configuration fix-version="4.4">
	<vocabulary>
		<vocabulary-tag name="44" alt="Price" type="float" />
		<vocabulary-tag name="20044" alt="Price" type="float" />
	</vocabulary>
</cplugin-configuration>"#;
    let (registry, _) =
        FixRegistry::from_cfb_file(&handle(ranged), Some(DIALECT)).expect("a readable CBlock");
    assert_eq!(registry.field_by_tag(44).unwrap().name(), "44");
    assert_eq!(registry.field_by_tag(20044).unwrap().name(), "20044");
    assert!(registry.get_field_by_name("Price").is_none());
    assert_eq!(
        registry.field_by_tag(44).unwrap().as_fix().tags().unwrap(),
        vec![20044]
    );
}

#[test]
fn both_doors_keep_the_second_declaration_of_one_tag_as_a_second_field() {
    // Same tag twice under two spellings: two identities, because a field is
    // its tag and its name. Both doors keep both, and the dictionary holds
    // them in one tag's slot in identity order: the first declared answers
    // the bare tag and learns nothing of the second's name; the second is
    // reached by its name or its identity.
    let doubled = r#"<?xml version="1.0"?>
<cplugin-configuration fix-version="4.4">
	<vocabulary>
		<vocabulary-tag name="44" alt="Price" type="float" />
		<vocabulary-tag name="44" alt="LastPx" type="float" />
	</vocabulary>
</cplugin-configuration>"#;
    // The folding door keeps both, and warns about neither.
    let mut folded = FixRegistry::new();
    let (read, warnings) = super::warned::during(|| folded.add_cfb_file(&handle(doubled), None));
    read.expect("a readable CBlock");
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(super::scalars(&folded), 2 + super::seeded_fields());
    let (registry, _) = FixRegistry::from_cfb_file(&handle(doubled), None).unwrap();
    assert_eq!(super::scalars(&registry), 2 + super::seeded_fields());
    let holder = registry.field_by_tag(44).unwrap();
    assert_eq!(holder.name(), "price");
    assert_eq!(
        holder.as_fix().names().collect::<Vec<_>>(),
        [] as [&str; 0],
        "the holder lends no alias",
    );
    let lastpx = FixId::of(44, "LastPx").unwrap();
    assert_eq!(registry.field_by_id(lastpx).unwrap().name(), "lastpx");
    assert!(
        registry
            .field_by_id(lastpx)
            .unwrap()
            .as_fix()
            .names()
            .next()
            .is_none()
    );
    assert_eq!(
        registry.get_field_by_name("LastPx").map(Field::name),
        Some("lastpx")
    );
    assert_eq!(
        registry.get_field_by_name("Price").map(Field::name),
        Some("price")
    );

    // Iteration is tag-major and then by identity, so the two sit together
    // in the one tag's slot, ordered by their ids and not by declaration.
    let ids: Vec<(i32, FixId)> = registry
        .iter()
        .filter(|field| field.as_fix().tag().unwrap() == Some(44))
        .map(|field| (44, field.as_fix().id().unwrap().unwrap()))
        .collect();
    let mut sorted = ids.clone();
    sorted.sort_by_key(|(_, id)| id.digest());
    assert_eq!(ids.len(), 2);
    assert_eq!(ids, sorted, "identity order within a tag");
    let held: Vec<i32> = registry
        .iter()
        .filter_map(|field| field.as_fix().tag().unwrap())
        .filter(|tag| !yggdryl::is_crate_tag(*tag))
        .collect();
    assert_eq!(
        held,
        [44, 44, 52, 60],
        "tag-major, the pair adjacent before the clock seeds"
    );
    let price = FixId::of(44, "Price").unwrap();
    assert_eq!(
        registry.next_field_after(None).map(|field| field.name()),
        Some(if price.digest() < lastpx.digest() {
            "price"
        } else {
            "lastpx"
        })
    );

    // And a dictionary keeps one entry per identity, so a tag declared twice
    // identically is one field and neither door warns about it.
    let repeated = r#"<?xml version="1.0"?>
<cplugin-configuration fix-version="4.4">
	<vocabulary>
		<vocabulary-tag name="44" alt="Price" type="float" />
		<vocabulary-tag name="44" alt="Price" type="float" />
	</vocabulary>
</cplugin-configuration>"#;
    let (read, warnings) =
        super::warned::during(|| FixRegistry::from_cfb_file(&handle(repeated), None));
    let (registry, _) = read.expect("a readable CBlock");
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(super::scalars(&registry), 1 + super::seeded_fields());
    let mut folded = FixRegistry::new();
    folded
        .add_cfb_file(&handle(repeated), None)
        .expect("a readable CBlock");
    assert_eq!(super::scalars(&folded), 1 + super::seeded_fields());
}

#[test]
fn a_cblock_reads_in_whole_with_its_dialect_and_the_file_it_arrived_as() {
    let mut dictionary = FixRegistry::new();
    let yggdryl::FixMerge { added, merged, .. } = dictionary
        .add_cfb_file(&named_handle(CBLOCK, "MSFIX44.cfb"), Some("morgan"))
        .expect("a readable CBlock");
    // Fourteen new fields; declared TransactTime merges into its seed, and
    // the parsed dictionary's SendingTime seed participates too. Crate fields
    // never count as folds.
    assert_eq!((added, merged), (14, 2));
    assert_eq!(super::scalars(&dictionary), 14 + super::seeded_fields());

    // The dialect the caller named is what every field the file produced is
    // a member of - the name wins over the stem - and the version the root
    // declared is read past: which version a run reads at is the codec's pin.
    assert_eq!(dictionary.dialects(), ["morgan"]);
    for field in dictionary.iter() {
        if field
            .as_fix()
            .tag()
            .unwrap()
            .is_some_and(|tag| yggdryl::is_crate_tag(tag) || tag == 52)
        {
            continue;
        }
        assert_eq!(sources(field), ["morgan"], "{}", field.name());
    }
    assert_eq!(
        sources(dictionary.msgtype("7").unwrap().as_field()),
        ["morgan"],
        "a message the file bound is a member too",
    );

    // Reading a second file is not a statement that the first one's names were
    // wrong: a field both speak is one field naming both dictionaries, and
    // with no dialect named the stem is the name.
    let yggdryl::FixMerge { added, merged, .. } = dictionary
        .add_cfb_file(&named_handle(SELLSIDE, "morgan-2024.cfb"), None)
        .expect("the same dialect, read again");
    assert_eq!(
        (added, merged),
        (0, 3),
        "SELLSIDE declares tag 35 and carries both standard clock seeds"
    );
    assert_eq!(
        sources(dictionary.field_by_tag(35).unwrap()),
        ["morgan", "morgan-2024"]
    );
    assert_eq!(sources(dictionary.field_by_tag(6).unwrap()), ["morgan"]);
    assert_eq!(dictionary.dialects(), ["morgan", "morgan-2024"]);

    // With no dialect named, the stem is the name, folded by case.
    let mut standalone = FixRegistry::new();
    standalone
        .add_cfb_file(&handle(CBLOCK), None)
        .expect("a readable CBlock");
    assert_eq!(standalone.dialects(), ["one"]);
    assert_eq!(sources(standalone.field_by_tag(6).unwrap()), ["one"]);
}

#[test]
fn a_cblock_merged_under_a_dialect_stamps_what_it_touched_and_unions_onto_the_standard_field() {
    // The shipped dictionary declares no membership: what the specification
    // alone defines is nobody's dialect. A CBlock folded into it under a name
    // stamps that name on every field, group, component and message it
    // produced, and a standard field the file speaks gains the membership
    // without losing what the specification said about it.
    let mut seeded = super::committed_registry().as_ref().clone();
    assert!(seeded.dialects().is_empty());
    let symbol = seeded.field_by_tag(55).unwrap().clone();
    assert!(symbol.description().is_some());
    assert!(sources(&symbol).is_empty());
    let first_holder = seeded
        .msgtype("D")
        .expect("the specification's D")
        .name()
        .to_owned();
    assert_ne!(first_holder, "message44");

    let body = r#"<cplugin-configuration fix-version="4.4">
      <vocabulary>
        <vocabulary-tag name="55" alt="Symbol" type="string" />
        <vocabulary-tag name="5000" alt="NoVendorEntries" type="integer" />
        <vocabulary-tag name="5001" alt="VendorID" type="string" />
      </vocabulary>
      <grammar-binding type="D"><grammar>
        <tag-constraint name="55" />
        <grammar rg-name="VendorEntries">
          <tag-constraint name="5000" />
          <tag-constraint name="5001" required="true" />
        </grammar>
      </grammar></grammar-binding>
    </cplugin-configuration>"#;
    let yggdryl::FixMerge { added, merged, .. } = seeded
        .add_cfb_file(&named_handle(body, "VENUE.cfb"), None)
        .expect("a compatible vocabulary folds");
    assert_eq!(
        (added, merged),
        (2, 3),
        "two venue tags added; Symbol and both standard clock seeds merged"
    );
    assert_eq!(seeded.dialects(), ["venue"]);

    // The standard field: the same tag under the same folded name, so the
    // membership unions onto it and the specification's words survive.
    let merged = seeded.field_by_tag(55).unwrap();
    assert_eq!(merged.name(), symbol.name());
    assert_eq!(merged.description(), symbol.description());
    assert_eq!(merged.dtype(), symbol.dtype());
    assert_eq!(sources(merged), ["venue"]);
    assert_eq!(
        seeded.get_field_by_name("Symbol").map(Field::name),
        Some(symbol.name())
    );

    // The venue's own tags, the group, its component and the message it
    // bound: every one a member.
    for tag in [5000, 5001] {
        assert_eq!(
            sources(seeded.field_by_tag(tag).unwrap()),
            ["venue"],
            "tag {tag}"
        );
    }
    assert_eq!(
        sources(seeded.field_by_name("VendorEntries").unwrap()),
        ["venue"]
    );
    assert_eq!(
        sources(seeded.field_by_name("VendorEntry").unwrap()),
        ["venue"]
    );
    assert!(seeded.get_field_by_counter(5000).is_some());
    // A field no file touched states nothing still.
    assert!(sources(seeded.field_by_tag(35).unwrap()).is_empty());

    // Message codes live in one namespace under the same rule as fields: the
    // file bound `D` under a name of its own, so it is a second message whose
    // bare code keeps answering the first holder, and the newcomer is reached
    // by its name and carries the membership.
    assert_eq!(seeded.msgtype("D").unwrap().name(), first_holder);
    let bound = seeded.msgtype("message44").expect("the file's own message");
    assert_eq!(bound.as_field().as_fix().msgtype(), Some("D"));
    assert_eq!(sources(bound.as_field()), ["venue"]);
    assert!(sources(seeded.msgtype("D").unwrap().as_field()).is_empty());

    // The same file under a second name unions, and the list is sorted and
    // folded whichever order the dictionaries arrived in.
    let yggdryl::FixMerge { added, merged, .. } = seeded
        .add_cfb_file(&handle(body), Some("Other"))
        .expect("the same vocabulary again");
    assert_eq!(
        (added, merged),
        (0, 5),
        "three vocabulary fields and two standard clock seeds"
    );
    assert_eq!(
        sources(seeded.field_by_tag(55).unwrap()),
        ["other", "venue"]
    );
    assert_eq!(
        sources(seeded.field_by_tag(5001).unwrap()),
        ["other", "venue"]
    );
    assert_eq!(
        sources(seeded.msgtype("message44").unwrap().as_field()),
        ["other", "venue"],
        "a definition re-declared under the same name folds into the stored one",
    );
    assert_eq!(seeded.dialects(), ["other", "venue"]);
}

#[test]
fn reading_a_cblock_in_whole_restates_a_changed_width_and_names_the_file_it_passes_over() {
    // A changed width is the file saying less than the dictionary: the
    // declaration folds under the stored datatype, and the rest of the file
    // is a dictionary with it.
    let mut seeded = super::committed_registry().as_ref().clone();
    let before = seeded.clone();
    let file = named_handle(CBLOCK, "blpfix44.cfb");
    let merge = seeded
        .add_cfb_file(&file, Some(DIALECT))
        .expect("a readable CBlock folds");
    assert_eq!(merge.sources, 1);
    assert!(merge.dropped.is_empty(), "{:?}", merge.dropped);
    assert_eq!(merge.restated, 2);
    assert_eq!(
        seeded.field_by_tag(6).unwrap().dtype(),
        before.field_by_tag(6).unwrap().dtype()
    );
    assert_eq!(sources(seeded.field_by_tag(6).unwrap()), [DIALECT]);
    assert_eq!(seeded.dialects(), [DIALECT]);

    // A contradiction is one declaration, passed over and named with the
    // file it came from: PossDupFlag typed `integer` against the committed
    // `boolean` is a number where the dictionary holds a flag.
    let contradicting = r#"<?xml version="1.0"?>
<cplugin-configuration fix-version="4.4">
	<vocabulary>
		<vocabulary-tag name="43" alt="PossDupFlag" type="integer" />
		<vocabulary-tag name="58" alt="Text" type="string" />
	</vocabulary>
</cplugin-configuration>"#;
    let file = named_handle(contradicting, "blpfix44.cfb");
    let merge = seeded
        .add_cfb_file(&file, Some(DIALECT))
        .expect("a contradiction folds the rest of the file");
    let url = file.url().unwrap().to_string();
    assert_eq!(merge.dropped.len(), 1, "{:?}", merge.dropped);
    for drop in &merge.dropped {
        assert_eq!(drop.incoming.as_fix().tag().unwrap(), Some(43));
        assert_eq!(drop.source.as_deref(), Some(url.as_str()));
        assert!(drop.to_string().starts_with(&url), "{drop}");
        assert_eq!(sources(&drop.incoming), [DIALECT]);
    }
    assert_eq!(
        seeded.field_by_tag(43).unwrap(),
        before.field_by_tag(43).unwrap()
    );
    assert!(
        seeded
            .field_by_tag(58)
            .unwrap()
            .as_fix()
            .has_source(DIALECT)
    );

    // What leaves nothing to keep is still refused whole: a document that is
    // not XML folds nothing.
    let before = seeded.clone();
    let error = seeded
        .add_cfb_file(
            &handle("<cplugin-configuration><vocabulary>"),
            Some("other"),
        )
        .unwrap_err();
    assert!(matches!(error, Error::Parse { .. }), "{error}");
    assert_eq!(seeded, before, "neither a membership nor a field arrived");
}

#[test]
fn a_normalization_spells_a_tag_and_the_vocabulary_keeps_its_name() {
    let (registry, _) = parse(NORMALIZED);

    // The vocabulary keeps a name it gave: tag 605 is `LegSecurityAltID`
    // whatever a binding spells it, and a binding's spelling of it is at most
    // another way to reach it.
    assert_eq!(
        registry.field_by_tag(605).expect("LegSecurityAltID").name(),
        "legsecurityaltid"
    );

    // A tag its `vocabulary-tag` gave no `alt` takes the one name its
    // bindings agree on, where no other tag is called by it. Tag 22830 is
    // spelled `EXCLUDEDDEALERS`, and so is tag 22831, so the spelling names
    // neither: each keeps its own decimal, and the first claim still reaches
    // tag 22830 as an alias.
    let held = registry.field_by_tag(22830).expect("the unnamed tag");
    assert_eq!(
        held.name(),
        "22830",
        "a spelling two tags speak names neither"
    );
    assert_eq!(registry.field_by_tag(22831).unwrap().name(), "22831");
    assert_eq!(
        registry
            .get_field_by_name("ExcludedDealers")
            .map(Field::name),
        Some("22830"),
        "and the spelling reaches it as an alias",
    );

    // A separator-bearing spelling is a spelling the tag does not answer to
    // without one, so it is stored beside the first and resolves too.
    assert_eq!(
        registry
            .get_field_by_name("EXCLUDED_DEALERS")
            .map(Field::name),
        Some("22830"),
    );
    assert_eq!(
        held.as_fix().names().collect::<Vec<_>>(),
        ["EXCLUDEDDEALERS", "EXCLUDED_DEALERS"],
        "in the order the file spelled them",
    );
}

#[test]
fn a_name_a_tag_already_answers_to_is_not_stored_a_second_time() {
    let (registry, _) = parse(NORMALIZED);

    // Resolution folds ASCII case, so `LEGSECURITYID` already reaches the tag
    // the vocabulary spelled `LegSecurityID`. Most of a real binding is this.
    let held = registry.field_by_tag(602).expect("LegSecurityID");
    assert_eq!(held.name(), "legsecurityid");
    assert_eq!(held.as_fix().names().collect::<Vec<_>>(), [] as [&str; 0]);
    assert!(registry.get_field_by_name("LEGSECURITYID").is_some());

    // The same, spelled with the file's own casing inside a nested group.
    let alt = registry.field_by_tag(605).expect("LegSecurityAltID");
    assert_eq!(alt.as_fix().names().collect::<Vec<_>>(), [] as [&str; 0]);
}

#[test]
fn only_an_unconditional_reference_to_one_tag_is_a_name_for_it() {
    let (registry, _) = parse(NORMALIZED);

    // A condition makes the name conditional, and a name that means a tag
    // only when another tag holds a value is not another spelling of it.
    for conditional in ["LEGISINCODE", "LEGEXCHANGECODE"] {
        assert!(
            registry.get_field_by_name(conditional).is_none(),
            "{conditional} is $602 only under a condition",
        );
    }

    // A lookup decodes a value rather than naming a tag, and this layer holds
    // no evaluator to say what the decoded value would be. Tag 603 is reached
    // by the name its own `vocabulary-tag` gave it and by nothing this added.
    assert_eq!(
        registry
            .field_by_tag(603)
            .expect("LegSecurityIDSource")
            .as_fix()
            .names()
            .collect::<Vec<_>>(),
        [] as [&str; 0],
        "a lookup names nothing",
    );
    // Tag 609 was given no `alt`, so a lookup naming it leaves it unreachable
    // by any spelling but its own tag.
    assert!(registry.get_field_by_name("LEGSECURITYTYPE").is_none());
    assert_eq!(registry.field_by_tag(609).expect("609").name(), "609");

    // Two expressions under one mapping are a construction, not a mapping.
    assert!(registry.get_field_by_name("BUILT").is_none());

    // One expression that is a bare reference still is one, trailing space
    // and all: a CBlock leaves the space it wrapped an attribute with. Tag
    // 608 was given no `alt`, and `LEGCFICODE` is the one spelling every
    // binding of it agrees on and no other tag claims, so it names the tag,
    // the spelling kept as its display.
    let cfi = registry.field_by_tag(608).expect("608");
    assert_eq!(cfi.name(), "legcficode");
    assert_eq!(cfi.display(), Some("LEGCFICODE"));
    assert!(cfi.as_fix().names().next().is_none());
    assert_eq!(
        registry.get_field_by_name("LEGCFICODE").map(Field::name),
        Some("legcficode"),
    );
    // A tag no binding names alone keeps its decimal: `EXCLUDEDDEALERS` is
    // spoken for 22830 and 22831, `VENUESYM` is 22833's own `alt`, and
    // `LEGSECURITYID` is 602's.
    for tag in [22830, 22831, 22832, 22834] {
        assert_eq!(
            registry.field_by_tag(tag).unwrap().name(),
            tag.to_string(),
            "tag {tag}"
        );
    }

    // `rg-name` names a repeating group and never the counter beside it; the
    // binding spells that counter plainly in its own `tag-normalization`.
    assert_eq!(
        registry.field_by_tag(604).expect("the counter").name(),
        "nolegsecurityaltid",
    );
}

#[test]
fn a_spelling_that_cannot_be_answered_is_dropped_and_never_refused() {
    let (registry, roots) = parse(NORMALIZED);
    assert!(roots.is_empty(), "the file binds no grammar");

    // A spelling another tag already answers to would resolve to neither, so
    // the second claim goes exactly as a map entry's second claim does.
    assert_eq!(
        registry
            .get_field_by_name("EXCLUDEDDEALERS")
            .map(Field::name),
        Some("22830"),
        "the first claim keeps it",
    );
    assert_eq!(
        registry
            .field_by_tag(22831)
            .expect("the second claimant")
            .as_fix()
            .names()
            .collect::<Vec<_>>(),
        [] as [&str; 0],
    );

    // A spelling another tag in the same branch answers to canonically goes
    // too: a canonical name always wins a lookup, so the alias would be a
    // spelling stored where nothing could ever reach it.
    assert_eq!(
        registry
            .field_by_tag(22832)
            .expect("the tag spelled as another")
            .as_fix()
            .names()
            .collect::<Vec<_>>(),
        [] as [&str; 0],
    );
    assert_eq!(
        registry.get_field_by_name("VENUESYM").map(Field::name),
        Some("venuesym"),
    );

    // A dictionary is one namespace and a venue's tag lives in the same one
    // FIX's do, so a venue tag spelled with a name FIX already publishes is
    // spelled with a name another tag answers to canonically, and the
    // spelling goes exactly as it did above. The standard tag is still what
    // the name reaches.
    assert_eq!(
        registry
            .field_by_tag(22834)
            .expect("the venue tag spelled as a standard one")
            .as_fix()
            .names()
            .collect::<Vec<_>>(),
        [] as [&str; 0],
    );
    assert_eq!(
        registry.get_field_by_name("LEGSECURITYID").map(Field::name),
        Some("legsecurityid"),
        "the standard tag keeps the spelling",
    );

    // An empty `tag-name`, a comma the stored list is rendered with, a tag
    // the vocabulary never declared, and a mapping that maps nothing: each
    // drops its own name and none of them refuses the document.
    assert!(registry.get_field_by_name("COMMA,SPELLING").is_none());
    assert!(registry.get_field_by_name("UNDECLARED").is_none());
    assert!(registry.get_field_by_name("NOTHING").is_none());
    assert!(registry.get_field_by_tag(999).is_none());
}

#[test]
fn both_doors_carry_the_names_a_normalization_spelled() {
    let mut folded = FixRegistry::new();
    folded
        .add_cfb_file(&handle(NORMALIZED), Some("bloomberg"))
        .expect("a readable CBlock");
    let held = folded.field_by_tag(22830).expect("the unnamed tag");
    assert_eq!(
        held.as_fix().names().collect::<Vec<_>>(),
        ["EXCLUDEDDEALERS", "EXCLUDED_DEALERS"],
        "the folding door carries them too",
    );
}

/// The shape a real FX trade-capture dialect has where one spelling is two
/// tags: `HedgeCurrency` at the top of the message is the currency the hedge
/// settles in, and `HedgeCurrency` inside `NoHedgeGroups` is the one each
/// hedge leg is quoted in. Both are declared, both are bound, and the
/// grammar is the only place the file says which is which.
const HEDGED: &str = r#"<?xml version="1.0" encoding="US-ASCII"?>
<cplugin-configuration type="com.ullink.ulbridge2.toolkit.plugins.fix.model.state.cblock.BuySideFIXCPluginCBlock" version="1.2" fix-version="4.4" targetcompid="TRTNFX" sendercompid="OURDESK">
	<message-types>
		<message-type value="AE Inbound" description="Trade Capture Report" supported="true" />
	</message-types>
	<inbound-message-type-mappings>
		<entry key="tradecapturereport" value="AE Inbound" />
	</inbound-message-type-mappings>
	<vocabulary>
		<vocabulary-tag name="35" alt="MsgType" type="string" read-only="true" />
		<vocabulary-tag name="11020" alt="NoHedgeGroups" type="integer" read-only="false" />
		<vocabulary-tag name="11021" alt="HedgeSettlDate" type="utc-date" read-only="false" />
		<vocabulary-tag name="11022" alt="HedgeSide" type="string" read-only="false" />
		<vocabulary-tag name="11023" alt="HedgeQty" type="float" read-only="false" />
		<vocabulary-tag name="11024" alt="HedgeCurrency" type="string" read-only="false" />
		<vocabulary-tag name="11025" alt="HedgeCurrency" type="string" read-only="false" />
		<vocabulary-tag name="11026" alt="HedgePrice" type="float" read-only="false" />
		<vocabulary-tag name="11027" alt="HedgeVenueTransID" type="string" read-only="false" />
		<vocabulary-tag name="11033" alt="TR_FixingCenter" type="string" read-only="false" />
	</vocabulary>
	<grammar-binding type="AE Inbound">
		<grammar checkordering="false">
			<tag-constraint name="35" activated="true" read-only="true" part="header" required="true" />
			<tag-constraint name="11025" activated="true" read-only="false" part="body" required="false" />
			<tag-constraint name="11033" activated="true" read-only="false" part="body" required="false" />
			<grammar checkordering="false">
				<tag-constraint name="11020" activated="true" read-only="false" part="body" required="false" />
				<tag-constraint name="11024" activated="true" read-only="false" part="body" required="false" />
				<tag-constraint name="11026" activated="true" read-only="false" part="body" required="false" />
				<tag-constraint name="11023" activated="true" read-only="false" part="body" required="false" />
				<tag-constraint name="11021" activated="true" read-only="false" part="body" required="false" />
				<tag-constraint name="11022" activated="true" read-only="false" part="body" required="false" />
				<tag-constraint name="11027" activated="true" read-only="false" part="body" required="false" />
			</grammar>
		</grammar>
	</grammar-binding>
	<normalization-binding>
		<normalization type="inbound">
			<tag-normalization tag-name="HEDGE_CURRENCY" part="body">
				<mapping-expression>
					<expression value="$11025" />
				</mapping-expression>
			</tag-normalization>
			<tag-normalization tag-name="FIXINGCENTER" part="body">
				<mapping-expression>
					<expression value="$11033" />
				</mapping-expression>
			</tag-normalization>
		</normalization>
	</normalization-binding>
</cplugin-configuration>
"#;

#[test]
fn a_message_resolves_the_spelling_two_of_its_tags_share() {
    let (registry, roots) = parse(HEDGED);

    // Neither tag is named by the spelling both declared, and the tag that
    // declared one nothing contends keeps it.
    assert_eq!(registry.field_by_tag(11024).unwrap().name(), "11024");
    assert_eq!(registry.field_by_tag(11025).unwrap().name(), "11025");
    assert_eq!(
        registry.field_by_tag(11024).unwrap().display(),
        Some("HedgeCurrency")
    );
    assert_eq!(
        registry.field_by_tag(11033).unwrap().name(),
        "tr_fixingcenter"
    );
    assert!(
        registry.get_field_by_name("HedgeCurrency").is_none(),
        "a spelling two tags share names neither",
    );
    // Each carries the other's tag, so the pair the file made is recoverable
    // from either half of it.
    assert_eq!(
        registry
            .field_by_tag(11024)
            .unwrap()
            .as_fix()
            .tags()
            .unwrap(),
        vec![11025]
    );
    assert_eq!(
        registry
            .field_by_tag(11025)
            .unwrap()
            .as_fix()
            .tags()
            .unwrap(),
        vec![11024]
    );

    // A binding cannot spell a contended name back onto one of the two: the
    // fold `HEDGE_CURRENCY` reaches is the one both tags claim, so it is a
    // name for neither there too. A spelling one tag alone answers to is
    // still added, which is what the pass is read for.
    assert!(
        registry.get_field_by_name("HEDGE_CURRENCY").is_none(),
        "a normalization does not undo what the vocabulary left unnamed",
    );
    assert_eq!(
        registry
            .get_field_by_name("FixingCenter")
            .and_then(|held| held.as_fix().tag().ok().flatten()),
        Some(11033),
    );

    // The message says which is which: the file bound one tag per constraint,
    // so the spelling survives at each level it was bound at.
    let root = &roots[0];
    assert!(
        children(root).contains(&"hedgecurrency"),
        "{:?}",
        children(root)
    );
    let group = root
        .get_field_by_path("hedgegroups")
        .expect("the hedge group");
    let DataType::Serie(item) = group.dtype() else {
        panic!("a serie, got {}", group.dtype());
    };
    let members: Vec<&str> = item.fields().iter().map(Field::name).collect();
    assert!(members.contains(&"hedgecurrency"), "{members:?}");

    // And a bridge row of that message type reaches both tags: the flat key
    // through the message's own children, the packed one through the members
    // of the group it arrived in.
    let reader = super::fixed_codec(Arc::new(registry));
    let row: &[u8] = b"MSGTYPE=tradecapturereport|HEDGECURRENCY=USD|TR_FIXINGCENTER=LN\
|NOHEDGEGROUPS=1|NOHEDGEGROUPS[0]=HEDGESETTLDATE=20260818\x04\x03HEDGECURRENCY=XAU\x04\x03";
    let message =
        <FixCodec as super::SoleMessage>::sole_line(&reader, row).expect("the bridge row builds");
    assert_eq!(message.by_tag(11025).unwrap().as_str(), Some("USD"));
    assert_eq!(message.by_tag(11033).unwrap().as_str(), Some("LN"));
    let occurrences = message
        .by_name("hedgegroups")
        .expect("the hedge group")
        .as_sequence()
        .expect("its occurrences")
        .to_vec();
    assert_eq!(occurrences.len(), 1);
    // The row is what resolved both spellings; the arrival record keeps the
    // occurrence the bridge wrote, under the counter that heads it, because a
    // member key rendered out of a packed value names no range of the line.
    let held = message
        .entries()
        .iter()
        .find(|entry| entry.tag() == 11020)
        .expect("the counter pair");
    let occurrence = held.entries();
    assert_eq!(occurrence.len(), 1);
    assert_eq!(held.value(), Some("1"), "the counter counts the occurrence");
    assert_eq!(
        occurrence[0].value(),
        None,
        "an occurrence states nothing of its own"
    );
    // The currency is the group's own tag, and named by it: a spelling two
    // tags share names neither, so the member spells its tag. The date's
    // wire spelling is the entry's own fact, pinned by the equivalence
    // snapshot.
    assert_eq!(
        occurrence[0]
            .entries()
            .iter()
            .map(|member| (member.tag(), member.name()))
            .collect::<Vec<_>>(),
        [(11021, "hedgesettldate"), (11024, "11024")],
        "the members the bridge packed, each under its own field",
    );
    assert_eq!(occurrence[0].entries()[1].value(), Some("XAU"));
    // The member is reached by the name the dictionary gives its tag, which
    // is the tag itself where two tags share a spelling.
    assert_eq!(
        message
            .by_path(&path("hedgegroups[0].\"11024\""))
            .expect("the group's own currency")
            .as_str(),
        Some("XAU"),
        "the group's own HedgeCurrency, read out of that pair",
    );
}

#[test]
fn the_captures_trade_capture_frame_reads_against_the_dialect_that_declares_it() {
    // The line the bridge log fixture carries, read against the dialect whose
    // vocabulary spells `HedgeCurrency` twice. The frame says `35=UL`; the row
    // inside its `XmlData` says what it is, and that is the type its keys
    // resolve against - which is the whole of how a shared spelling reaches
    // the tag the file bound it at.
    let logged = include_str!("ulbridge.log")
        .lines()
        .find(|line| line.contains("MSGTYPE=tradecapturereport"))
        .expect("the trade capture frame");
    let (registry, _) = parse(HEDGED);
    let reader = super::fixed_codec(Arc::new(registry));
    let message = <FixCodec as super::SoleMessage>::sole_line(&reader, logged.as_bytes())
        .expect("the captured frame builds");

    // The row inside the frame says what the message is, and overrides the
    // envelope's `35=UL`: the bridge's own type is the transport's, and the
    // trade capture is what was carried.
    assert_eq!(message.by_tag(35).unwrap().as_str(), Some("AE"));
    assert_eq!(message.header().msgtype(), "AE");

    // The hedge the payload packs is this dialect's group, typed by it: the
    // currency is the group's own tag and not the one the top of the message
    // binds under the same spelling.
    let hedge = message
        .as_field()
        .get_field_by_path("hedgegroups")
        .expect("the hedge group");
    let DataType::Serie(item) = hedge.dtype() else {
        panic!("a serie, got {}", hedge.dtype());
    };
    let currency = item.fields().first().expect("its first member");
    // Named as the dictionary names the tag: a spelling two tags share
    // names neither, so the member spells its tag and displays the spelling.
    assert_eq!(currency.name(), "11024");
    assert_eq!(currency.as_fix().tag().unwrap(), Some(11024));
    let held = message
        .by_name("hedgegroups")
        .expect("the hedge group")
        .as_sequence()
        .expect("its occurrences")
        .to_vec();
    assert_eq!(
        held[0].as_sequence().expect("its members")[0].as_str(),
        Some("XAU")
    );
    // Typed by the vocabulary rather than kept as text, which is what says
    // the dialect and not the fallback answered.
    assert_eq!(
        item.field("hedgeqty").expect("HedgeQty").dtype(),
        &DataType::Float64
    );
}

#[test]
fn a_mapping_names_a_type_the_listing_declared_or_it_names_nothing() {
    // The two tables spell a type the way UlMessage does. They do not declare
    // one: a value the listing above them never stated is a mapping to
    // nothing, and taking it would answer `somethingelse` forever after with
    // whatever `ZZ` a typo produced.
    let body = r#"<?xml version="1.0" encoding="US-ASCII"?>
<cplugin-configuration fix-version="4.4">
	<message-types>
		<message-type value="J" description="Allocation" supported="true" />
		<message-type value="J Report" description="Allocation Report" supported="true" />
	</message-types>
	<outbound-message-type-mappings>
		<entry key="allocation" value="J" />
		<entry key="somethingelse" value="ZZ" />
	</outbound-message-type-mappings>
	<vocabulary><vocabulary-tag name="35" alt="MsgType" type="string" /></vocabulary>
</cplugin-configuration>"#;
    let (read, warnings) = super::warned::during(|| parse(body));
    let (registry, _) = read;
    let field = registry.field_by_tag(35).expect("MsgType");
    let codes = registry.codeset_of(field).expect("the set tag 35 reads by");

    // The key reaches the type the listing declared, and so does the whole
    // spelling the listing gave it.
    assert_eq!(codes.code_value("allocation"), Some("J"));
    assert_eq!(codes.code_value("J Report"), Some("J"));
    assert_eq!(codes.code_value("J"), Some("J"));

    // The entry naming nothing the listing declared names nothing at all.
    assert_eq!(codes.code_value("somethingelse"), None);
    assert_eq!(codes.code_value("ZZ"), None);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(
        warnings[0].contains("a message type this file's listing declares"),
        "{warnings:?}"
    );
    assert!(warnings[0].contains("\"ZZ\""), "{warnings:?}");

    // A `grammar-binding` still declares the type it binds, because binding
    // one is the file stating it and a table is the file spelling it.
    let bound = r#"<?xml version="1.0" encoding="US-ASCII"?>
<cplugin-configuration fix-version="4.4">
	<vocabulary><vocabulary-tag name="35" alt="MsgType" type="string" /></vocabulary>
	<grammar-binding type="AE Inbound"><grammar><tag-constraint name="35" part="header" /></grammar></grammar-binding>
</cplugin-configuration>"#;
    let (registry, _) = parse(bound);
    assert_eq!(
        registry
            .codeset_of(registry.field_by_tag(35).unwrap())
            .expect("the set tag 35 reads by")
            .code_value("AE Inbound"),
        Some("AE")
    );
}

#[test]
fn two_grammars_bound_under_one_wire_type_are_one_message_carrying_both() {
    // The dialect describes message type 6 in both directions, and the two
    // bindings do not carry the same body: inbound states tag 23, outbound
    // states tag 28. One wire type is one message, so the dictionary holds
    // one entry carrying every field either binding declared.
    let body = r#"<?xml version="1.0"?>
<cplugin-configuration fix-version="4.4">
	<message-types>
		<message-type value="6 Inbound" description="Indication of Interest" />
		<message-type value="6 Outbound" description="Indication of Interest" />
	</message-types>
	<vocabulary>
		<vocabulary-tag name="35" alt="MsgType" type="string" />
		<vocabulary-tag name="23" alt="IOIID" type="string" />
		<vocabulary-tag name="28" alt="IOITransType" type="char" />
	</vocabulary>
	<grammar-binding type="6 Inbound">
		<grammar><tag-constraint name="35" part="header" required="true" /><tag-constraint name="23" part="body" /></grammar>
	</grammar-binding>
	<grammar-binding type="6 Outbound">
		<grammar><tag-constraint name="35" part="header" required="true" /><tag-constraint name="28" part="body" /></grammar>
	</grammar-binding>
</cplugin-configuration>"#;
    let (registry, roots) = parse(body);

    // Tag 35 carries one code for the wire value, named after the value
    // rather than after either qualifier, answering to both spellings.
    let msgtype = registry.field_by_tag(35).expect("MsgType");
    let codes = registry
        .codeset_of(msgtype)
        .expect("the set tag 35 reads by");
    assert_eq!(codes.codes().count(), 1);
    assert_eq!(codes.code_name("6"), Some("6"));
    assert_eq!(codes.code_value("6 Inbound"), Some("6"));
    assert_eq!(codes.code_value("6 Outbound"), Some("6"));

    // One message, holding the union of what the two bindings declared, the
    // first binding's members first and in its order.
    assert_eq!(
        registry
            .iter()
            .filter(|field| field.as_fix().msgtype().is_some())
            .count(),
        1
    );
    let message = registry.msgtype("6").expect("one message under tag 35 `6`");
    assert_eq!(message.as_str(), "6");
    let members: Vec<&str> = message
        .as_field()
        .fields()
        .iter()
        .map(yggdryl::Field::name)
        .collect();
    assert_eq!(members, ["msgtype", "ioiid", "ioitranstype"]);

    // The roots are still one per binding: that is what the file bound.
    assert_eq!(roots.len(), 2);
    assert_eq!(children(&roots[0]), ["msgtype", "ioiid"]);
    assert_eq!(children(&roots[1]), ["msgtype", "ioitranstype"]);
}

/// Several CBlocks in one memory tree, for the glob the plural verb walks.
fn cblock_tree(files: &[(&str, &str)]) -> yggdryl::holder::Holder {
    let filesystem: Arc<dyn FileSystem> = Arc::new(MemoryFileSystem::new());
    filesystem
        .create_dir("cblocks", true)
        .expect("a container to write into");
    for (name, body) in files {
        let mut file = FsFile::from_path(Arc::clone(&filesystem), format!("cblocks/{name}"), None)
            .expect("a path under it");
        file.write_all_bytes(body.as_bytes()).expect("the document");
    }
    yggdryl::holder::Holder::from(
        yggdryl::fs::FsFolder::from_path(filesystem, "cblocks", None)
            .expect("the folder holding them"),
    )
}

/// One CBlock declaring one tag, for the ordering and membership cases.
fn one_tag(tag: i32, name: &str) -> String {
    format!(
        r#"<?xml version="1.0"?>
<cplugin-configuration fix-version="4.4">
	<vocabulary><vocabulary-tag name="{tag}" alt="{name}" type="string" /></vocabulary>
</cplugin-configuration>"#
    )
}

#[test]
fn a_glob_folds_every_cblock_it_selects_under_each_file_s_own_dialect() {
    let tree = cblock_tree(&[
        ("msfix44.cfb", &one_tag(9001, "MsVenueRef")),
        ("blpfix44.cfb", &one_tag(9002, "BlpVenueRef")),
        ("notes.txt", "not a dictionary"),
    ]);
    let mut registry = FixRegistry::new();
    let yggdryl::FixMerge {
        sources: files,
        added,
        merged,
        ..
    } = registry
        .add_cfb_files(std::slice::from_ref(&tree), None)
        .expect("two readable CBlocks");

    // The pattern is the filter: the text file is not selected, and the two
    // seeded clocks merge once per file, as one `add_cfb_file` merges them.
    assert_eq!((files, added), (2, 2));
    assert_eq!(merged, 4, "two clock seeds per file");

    // Each file named its own dialect from its own stem, which is what
    // globbing a folder of counterparty files is for.
    assert_eq!(sources(registry.field_by_tag(9001).unwrap()), ["msfix44"]);
    assert_eq!(sources(registry.field_by_tag(9002).unwrap()), ["blpfix44"]);

    // A name supplied here stamps every matched file with the one membership.
    let mut named = FixRegistry::new();
    named
        .add_cfb_files(std::slice::from_ref(&tree), Some("venues"))
        .expect("two readable CBlocks");
    assert_eq!(sources(named.field_by_tag(9001).unwrap()), ["venues"]);
    assert_eq!(sources(named.field_by_tag(9002).unwrap()), ["venues"]);

    // A pattern selecting nothing folds nothing rather than refusing.
    let mut empty = FixRegistry::new();
    assert_eq!(
        empty
            .add_cfb_files(&[tree.child_by_path("*.xml").expect("a glob")], None)
            .map(|merge| (merge.sources, merge.added, merge.merged))
            .unwrap(),
        (0, 0, 0)
    );
    assert_eq!(empty, FixRegistry::new());
}

#[test]
fn one_unreadable_cblock_among_many_is_left_out_and_the_rest_still_fold() {
    let tree = cblock_tree(&[
        ("aaa.cfb", &one_tag(9001, "GoodRef")),
        ("mmm.cfb", "<cplugin-configuration><vocabulary>"),
        ("zzz.cfb", &one_tag(9002, "LateRef")),
    ]);
    let mut registry = FixRegistry::new();
    let merge = registry
        .add_cfb_files(std::slice::from_ref(&tree), None)
        .expect("one bad file is one file");

    // The file that stops inside an element is left out and named, with what
    // the reader stopped on; the files around it fold as if it were not
    // there, and the count says how many did.
    assert_eq!(merge.sources, 2);
    assert_eq!(merge.failed.len(), 1, "{:?}", merge.failed);
    let failure = merge.failed[0].to_string();
    assert!(
        merge.failed[0]
            .source
            .as_deref()
            .is_some_and(|source| source.ends_with("mmm.cfb")),
        "{failure}"
    );
    assert!(failure.contains("a closed <vocabulary>"), "{failure}");
    assert!(!merge.is_clean());
    assert_eq!(sources(registry.field_by_tag(9001).unwrap()), ["aaa"]);
    assert_eq!(sources(registry.field_by_tag(9002).unwrap()), ["zzz"]);

    // Nothing of the left-out file is held: the dictionary is the one the
    // two good files make on their own.
    let good = cblock_tree(&[
        ("aaa.cfb", &one_tag(9001, "GoodRef")),
        ("zzz.cfb", &one_tag(9002, "LateRef")),
    ]);
    let mut alone = FixRegistry::new();
    alone
        .add_cfb_files(std::slice::from_ref(&good), None)
        .expect("two readable CBlocks");
    assert_eq!(registry, alone);
}

/// One CBlock declaring tag 532 as `type` and binding it in message `r`.
fn rejection(declared: &str) -> String {
    format!(
        r#"<?xml version="1.0"?>
<cplugin-configuration fix-version="4.4">
	<vocabulary>
		<vocabulary-tag name="35" alt="MsgType" type="string" />
		<vocabulary-tag name="532" alt="MassCancelRejectReason" type="{declared}" />
		<vocabulary-tag name="58" alt="Text" type="string" />
	</vocabulary>
	<grammar-binding type="r"><grammar>
		<tag-constraint name="532" />
		<tag-constraint name="58" />
	</grammar></grammar-binding>
</cplugin-configuration>"#
    )
}

#[test]
fn counterparties_typing_one_tag_two_ways_fold_whole_and_name_the_file_passed_over() {
    // Three dialects over one wire tag: a flag, a number and text. Every file
    // still folds; the first-sorting file's declaration is the one held, the
    // text says less than it and folds under it, and the number contradicts
    // it and is named with its file.
    let tree = cblock_tree(&[
        ("bloomberg_fix44_dropcopy.cfb", &rejection("integer")),
        ("axessiq_fix44.cfb", &rejection("boolean")),
        ("tradeweb_fix44.cfb", &rejection("string")),
    ]);
    let mut registry = FixRegistry::new();
    let merge = registry
        .add_cfb_files(std::slice::from_ref(&tree), None)
        .expect("every file folds");
    assert_eq!(merge.sources, 3);
    assert_eq!(merge.dropped.len(), 1, "{:?}", merge.dropped);
    assert_eq!(merge.restated, 1, "tradeweb's text, under the held flag");
    let drop = &merge.dropped[0];
    assert_eq!(drop.incoming.as_fix().tag().unwrap(), Some(532));
    assert_eq!(drop.incoming.dtype(), &DataType::Int32);
    assert_eq!(sources(&drop.incoming), ["bloomberg_fix44_dropcopy"]);
    assert!(
        drop.source
            .as_deref()
            .is_some_and(|source| source.ends_with("bloomberg_fix44_dropcopy.cfb")),
        "{drop}"
    );
    assert!(
        drop.reason.contains("boolean") && drop.reason.contains("int32"),
        "{drop}"
    );
    let held = registry.field_by_tag(532).unwrap();
    assert_eq!(held.dtype(), &DataType::Boolean);
    assert_eq!(sources(held), ["axessiq_fix44", "tradeweb_fix44"]);
    // The message the dropped declaration's file bound still arrived, and
    // reads tag 532 the way the dictionary holds it.
    let message = registry.msgtype("r").unwrap().as_field();
    assert!(
        sources(message).contains(&"bloomberg_fix44_dropcopy"),
        "{:?}",
        sources(message)
    );
    let member = message
        .fields()
        .iter()
        .find(|member| member.as_fix().tag().unwrap() == Some(532))
        .expect("tag 532 in the message");
    assert_eq!(member.dtype(), &DataType::Boolean);

    // The files fold in URL order whatever order the listing arrived in, and
    // parsing them side by side answers what one file at a time answers.
    let mut sequential = FixRegistry::new();
    for name in [
        "axessiq_fix44.cfb",
        "bloomberg_fix44_dropcopy.cfb",
        "tradeweb_fix44.cfb",
    ] {
        let file = tree.child_by_path(name).unwrap();
        sequential.add_cfb_file(file.as_io(), None).unwrap();
    }
    assert_eq!(sequential, registry);
}

#[test]
fn a_field_passed_over_by_its_name_takes_the_members_reading_it_along() {
    // The first dialect calls tag 5001 `VendorCode`, a flag; the second calls
    // tag 6001 the same, as an integer. A name reaching a field under another
    // tag is that field spelled another way, and here the datatype says it is
    // not - so the second declaration is passed over, and so is the member
    // of its message that reads it, which would otherwise read tag 6001 as
    // the held field under a tag that is not its own.
    let first = r#"<cplugin-configuration fix-version="4.4">
      <vocabulary><vocabulary-tag name="5001" alt="VendorCode" type="boolean" /></vocabulary>
    </cplugin-configuration>"#;
    let second = r#"<cplugin-configuration fix-version="4.4">
      <vocabulary>
        <vocabulary-tag name="6001" alt="VendorCode" type="integer" />
        <vocabulary-tag name="55" alt="Symbol" type="string" />
      </vocabulary>
      <grammar-binding type="D"><grammar>
        <tag-constraint name="55" />
        <tag-constraint name="6001" />
      </grammar></grammar-binding>
    </cplugin-configuration>"#;
    let tree = cblock_tree(&[("a.cfb", first), ("b.cfb", second)]);
    let mut registry = FixRegistry::new();
    let merge = registry
        .add_cfb_files(std::slice::from_ref(&tree), None)
        .expect("both files fold");
    let passed: Vec<(&str, Option<i32>)> = merge
        .dropped
        .iter()
        .map(|drop| (drop.incoming.name(), drop.incoming.as_fix().tag().unwrap()))
        .collect();
    assert_eq!(
        passed,
        [("vendorcode", Some(6001)), ("vendorcode", Some(6001))]
    );
    assert!(
        merge.dropped[1].reason.contains("passed over"),
        "{:?}",
        merge.dropped
    );
    assert_eq!(
        registry.field_by_name("VendorCode").unwrap().dtype(),
        &DataType::Boolean
    );
    assert!(registry.get_field_by_tag(6001).is_none());
    let message = registry.msgtype("D").unwrap().as_field();
    let tags: Vec<Option<i32>> = message
        .fields()
        .iter()
        .map(|member| member.as_fix().tag().unwrap())
        .collect();
    assert!(
        tags.contains(&Some(55)) && !tags.contains(&Some(6001)),
        "{tags:?}"
    );
}

/// One CBlock whose message `D` carries the `Underlyings` group with
/// `members`, beside message `8` carrying it with UnderlyingSymbol alone -
/// so a file whose `D` states more splits the group under `D`'s name.
fn underlyings(members: &str) -> String {
    format!(
        r#"<cplugin-configuration fix-version="4.4">
      <vocabulary>
        <vocabulary-tag name="35" alt="MsgType" type="string" />
        <vocabulary-tag name="711" alt="NoUnderlyings" type="integer" />
        <vocabulary-tag name="311" alt="UnderlyingSymbol" type="string" />
        <vocabulary-tag name="1044" alt="UnderlyingAdjustedQuantity" type="float" />
        <vocabulary-tag name="879" alt="UnderlyingQty" type="float" />
      </vocabulary>
      <grammar-binding type="8"><grammar>
        <grammar rg-name="Underlyings">
          <tag-constraint name="711" />
          <tag-constraint name="311" required="true" />
        </grammar>
      </grammar></grammar-binding>
      <grammar-binding type="D"><grammar>
        <grammar rg-name="Underlyings">
          <tag-constraint name="711" />
          <tag-constraint name="311" required="true" />
          {members}
        </grammar>
      </grammar></grammar-binding>
      <maps>
        <map name="MSGTYPE"><entries><entry key="NewOrderSingle" value="D" /></entries></map>
      </maps>
    </cplugin-configuration>"#
    )
}

#[test]
fn a_group_one_dialect_split_for_a_message_folds_into_the_group_that_message_reads() {
    use yggdryl::FixCategory::{Components, Groups};

    // The first dialect declares `Underlyings` one way for both messages; the
    // second declares one more member under `D`, so its `D` reads the split
    // `underlyings_newordersingle`. Folding the second into the first is one
    // message `D` reading one group: the split's members fold into the group
    // `D` already reads, and nothing is passed over - and once they have, the
    // split states the structure of the group it was split from, so it is
    // that group and no definition of its own.
    let tree = cblock_tree(&[
        ("a.cfb", &underlyings("")),
        ("b.cfb", &underlyings(r#"<tag-constraint name="1044" />"#)),
    ]);
    // A group two messages of one file declare alike is one group.
    let (alone, _) = parse(&underlyings(""));
    assert!(
        alone
            .get_definition(Groups, "underlyings_newordersingle")
            .is_none()
    );
    let mut registry = FixRegistry::new();
    let merge = registry
        .add_cfb_files(std::slice::from_ref(&tree), None)
        .expect("both dialects fold");
    assert!(merge.is_clean(), "{:?}", merge.dropped);
    let held = registry.definition(Components, "underlying").unwrap();
    assert!(
        held.get_field("underlyingadjustedquantity").is_some(),
        "the member the split added joins the group D reads"
    );
    // The split is the group its members folded into: one structure is one
    // definition, so nothing named for the message is held, and D reads the
    // group it always read.
    for (category, split) in [
        (Components, "underlying_newordersingle"),
        (Groups, "underlyings_newordersingle"),
    ] {
        assert!(
            registry.get_definition(category, split).is_none(),
            "{split} folded into the definition it restates"
        );
    }
    let message = registry.msgtype("D").unwrap().as_field();
    assert!(
        message
            .fields()
            .iter()
            .any(|member| member.as_fix().group() == Some("underlyings")),
        "D reads the held group"
    );

    // A group on another counter is another group, which is not folded: the
    // member D holds stays, and the other reading stands beside it as a
    // member of its own, named for its counter - two counters are two tags
    // on the wire, so the one message carries both groups.
    let recounted = underlyings("")
        .replace(
            r#"<vocabulary-tag name="879" alt="UnderlyingQty" type="float" />"#,
            r#"<vocabulary-tag name="712" alt="NoOtherUnderlyings" type="integer" />"#,
        )
        .replace(
            r#"<grammar-binding type="D"><grammar>
        <grammar rg-name="Underlyings">
          <tag-constraint name="711" />"#,
            r#"<grammar-binding type="D"><grammar>
        <grammar rg-name="Underlyings">
          <tag-constraint name="712" />"#,
        );
    let tree = cblock_tree(&[("a.cfb", &underlyings("")), ("c.cfb", &recounted)]);
    let mut registry = FixRegistry::new();
    let merge = registry
        .add_cfb_files(std::slice::from_ref(&tree), None)
        .expect("both dialects fold");
    assert!(merge.is_clean(), "{:?} {:?}", merge.dropped, merge.failed);
    let message = registry.msgtype("D").unwrap().as_field();
    let groups: Vec<(&str, Option<&str>)> = message
        .fields()
        .iter()
        .filter(|member| member.as_fix().group().is_some())
        .map(|member| (member.name(), member.as_fix().group()))
        .collect();
    assert_eq!(
        groups,
        [
            ("underlyings", Some("underlyings")),
            ("underlyings_712", Some("underlyings_newordersingle"))
        ],
        "D reads the group it held, and the other beside it"
    );
    // Folding the same files again places nothing beside twice: the member
    // already standing beside reads the same group, so it is that member.
    let once = registry.clone();
    let merge = registry
        .add_cfb_files(std::slice::from_ref(&tree), None)
        .expect("both dialects fold again");
    assert!(merge.is_clean(), "{:?} {:?}", merge.dropped, merge.failed);
    assert_eq!(
        registry, once,
        "a second fold of the same files changes nothing"
    );
    assert_eq!(
        registry
            .definition(Groups, "underlyings_newordersingle")
            .unwrap()
            .as_fix()
            .counter()
            .unwrap(),
        Some(712),
        "the other group arrives as one of its own"
    );
}

#[test]
fn a_field_merged_by_its_name_is_read_by_the_members_of_its_file_under_the_held_identity() {
    // Both dialects call a field `VenueRef` and agree it is text, under two
    // tags: the second is the first spelled with another number, and the
    // members of the second file's message read the one field that holds it.
    let first = r#"<cplugin-configuration fix-version="4.4">
      <vocabulary><vocabulary-tag name="9001" alt="VenueRef" type="string" /></vocabulary>
    </cplugin-configuration>"#;
    let second = r#"<cplugin-configuration fix-version="4.4">
      <vocabulary>
        <vocabulary-tag name="9002" alt="VenueRef" type="string" />
        <vocabulary-tag name="55" alt="Symbol" type="string" />
      </vocabulary>
      <grammar-binding type="D"><grammar>
        <tag-constraint name="55" />
        <tag-constraint name="9002" />
      </grammar></grammar-binding>
    </cplugin-configuration>"#;
    let tree = cblock_tree(&[("a.cfb", first), ("b.cfb", second)]);
    let mut registry = FixRegistry::new();
    let merge = registry
        .add_cfb_files(std::slice::from_ref(&tree), None)
        .expect("both files fold");
    assert!(merge.is_clean(), "{:?}", merge.dropped);
    let held = registry.field_by_tag(9002).unwrap();
    assert_eq!(held.as_fix().tag().unwrap(), Some(9001));
    let message = registry.msgtype("D").unwrap().as_field();
    let member = message
        .fields()
        .iter()
        .find(|member| member.as_fix().field_ref() == Some("venueref"))
        .expect("the member reading VenueRef");
    assert_eq!(member.as_fix().tag().unwrap(), Some(9001));
}

#[test]
fn a_spelling_another_field_holds_merges_into_that_field_and_the_members_reading_it_follow() {
    // The first dialect holds 9001 as `VenueOrderType` and `ClientRef` as
    // 9002; the second calls 9001 `ClientRef`. A name reaching a held field
    // is that field spelled with another number, before a held tag is a
    // field beside its holder: the second's `ClientRef` merges into the
    // first's, 9001 stays `VenueOrderType`'s and joins no alternate, and the
    // member of the second's message reads `ClientRef` under the identity
    // that holds it.
    let first = r#"<cplugin-configuration fix-version="4.4">
      <vocabulary>
        <vocabulary-tag name="9001" alt="VenueOrderType" type="string" />
        <vocabulary-tag name="9002" alt="ClientRef" type="string" />
      </vocabulary>
    </cplugin-configuration>"#;
    let second = r#"<cplugin-configuration fix-version="4.4">
      <vocabulary>
        <vocabulary-tag name="9001" alt="ClientRef" type="string" />
        <vocabulary-tag name="55" alt="Symbol" type="string" />
      </vocabulary>
      <grammar-binding type="D"><grammar>
        <tag-constraint name="55" />
        <tag-constraint name="9001" />
      </grammar></grammar-binding>
    </cplugin-configuration>"#;
    let tree = cblock_tree(&[("a.cfb", first), ("b.cfb", second)]);
    let mut registry = FixRegistry::new();
    let merge = registry
        .add_cfb_files(std::slice::from_ref(&tree), None)
        .expect("both files fold");
    assert!(merge.is_clean(), "{:?}", merge.dropped);
    assert_eq!(
        registry.field_by_tag(9001).unwrap().name(),
        "venueordertype"
    );
    let clientref = registry.field_by_name("ClientRef").unwrap();
    assert_eq!(clientref.as_fix().tag().unwrap(), Some(9002));
    assert!(
        clientref.as_fix().tags().unwrap().is_empty(),
        "9001 is VenueOrderType's and joins no alternate"
    );
    assert_eq!(sources(clientref), ["a", "b"]);
    let message = registry.msgtype("D").unwrap().as_field();
    let member = message
        .fields()
        .iter()
        .find(|member| member.as_fix().field_ref() == Some("clientref"))
        .expect("the member reading ClientRef");
    assert_eq!(member.as_fix().tag().unwrap(), Some(9002));
    assert!(
        message
            .fields()
            .iter()
            .all(|member| member.as_fix().tag().unwrap() != Some(9001)),
        "no member reads VenueOrderType's tag"
    );
}

#[test]
fn a_held_tag_redeclared_under_a_held_name_merges_into_the_name_s_holder_and_its_messages_fold() {
    // The base file declares 541 `MaturityDate` and 9999 `MaturityDate2`;
    // the second declares 541 as `MaturityDate2`, and every message it binds
    // reads it. The arrival is `MaturityDate2` spelled with `MaturityDate`'s
    // number: it merges into `MaturityDate2`, 541 stays `MaturityDate`'s,
    // and both messages fold with their member reading `MaturityDate2`
    // under the identity that holds it - nothing passed over, no file left
    // out.
    let base = r#"<cplugin-configuration fix-version="4.4">
      <vocabulary>
        <vocabulary-tag name="55" alt="Symbol" type="string" />
        <vocabulary-tag name="541" alt="MaturityDate" type="utc-date" />
        <vocabulary-tag name="9999" alt="MaturityDate2" type="utc-date" />
      </vocabulary>
      <grammar-binding type="D"><grammar>
        <tag-constraint name="55" />
        <tag-constraint name="541" />
        <tag-constraint name="9999" />
      </grammar></grammar-binding>
    </cplugin-configuration>"#;
    let collide = r#"<cplugin-configuration fix-version="4.4">
      <vocabulary><vocabulary-tag name="541" alt="MaturityDate2" type="utc-date" /></vocabulary>
      <grammar-binding type="D"><grammar>
        <tag-constraint name="541" />
      </grammar></grammar-binding>
      <grammar-binding type="8"><grammar>
        <tag-constraint name="541" />
      </grammar></grammar-binding>
    </cplugin-configuration>"#;
    let tree = cblock_tree(&[("a.cfb", base), ("b.cfb", collide)]);
    let mut registry = FixRegistry::new();
    let merge = registry
        .add_cfb_files(std::slice::from_ref(&tree), None)
        .expect("both files fold");
    assert!(merge.is_clean(), "{:?}", merge.dropped);
    assert!(merge.failed.is_empty(), "{:?}", merge.failed);
    assert_eq!(merge.sources, 2);
    assert_eq!(registry.field_by_tag(541).unwrap().name(), "maturitydate");
    let second = registry.field_by_name("MaturityDate2").unwrap();
    assert_eq!(second.as_fix().tag().unwrap(), Some(9999));
    assert!(second.as_fix().tags().unwrap().is_empty());
    assert_eq!(sources(second), ["a", "b"]);
    for msgtype in ["D", "8"] {
        let message = registry.msgtype(msgtype).unwrap().as_field();
        let member = message
            .fields()
            .iter()
            .find(|member| member.as_fix().field_ref() == Some("maturitydate2"))
            .unwrap_or_else(|| panic!("message {msgtype} reads MaturityDate2"));
        assert_eq!(member.as_fix().tag().unwrap(), Some(9999));
    }
    assert_eq!(
        children(registry.msgtype("D").unwrap().as_field()),
        ["symbol", "maturitydate", "maturitydate2"]
    );
}

#[test]
fn a_member_is_the_field_it_reads_before_the_name_it_carries_whichever_file_folds_first() {
    // One dialect spells `Urgency` over both 61 and 9252 - a contended
    // spelling, each tag named by its decimal and the message carrying the
    // spelling on both members - and another calls 9252 `Urgency`. A member
    // is the field it reads before the name it carries: the second's member
    // reading 9252 is the first's member reading 9252 whatever either is
    // called, and a member reading 61 under a name a held member reads
    // 9252 by is a member of its own beside it, two tags being two tags on
    // the wire. So nothing is passed over and the message reads both tags
    // whichever file folds first, where pairing by name alone passed over
    // the member reading 9252 in one order and the one reading 61 in the
    // other.
    let contended = r#"<cplugin-configuration fix-version="4.4">
      <vocabulary>
        <vocabulary-tag name="55" alt="Symbol" type="string" />
        <vocabulary-tag name="61" alt="Urgency" type="string" />
        <vocabulary-tag name="9252" alt="Urgency" type="string" />
      </vocabulary>
      <grammar-binding type="D"><grammar>
        <tag-constraint name="55" />
        <tag-constraint name="61" />
        <tag-constraint name="9252" />
      </grammar></grammar-binding>
    </cplugin-configuration>"#;
    let named = r#"<cplugin-configuration fix-version="4.4">
      <vocabulary>
        <vocabulary-tag name="55" alt="Symbol" type="string" />
        <vocabulary-tag name="9252" alt="Urgency" type="string" />
      </vocabulary>
      <grammar-binding type="D"><grammar>
        <tag-constraint name="55" />
        <tag-constraint name="9252" />
      </grammar></grammar-binding>
    </cplugin-configuration>"#;
    for (first, second) in [(contended, named), (named, contended)] {
        let tree = cblock_tree(&[("a.cfb", first), ("b.cfb", second)]);
        let mut registry = FixRegistry::new();
        let merge = registry
            .add_cfb_files(std::slice::from_ref(&tree), None)
            .expect("both files fold");
        assert!(merge.is_clean(), "{:?}", merge.dropped);
        assert_eq!(registry.field_by_tag(9252).unwrap().name(), "urgency");
        assert_eq!(registry.field_by_tag(61).unwrap().name(), "61");
        let message = registry.msgtype("D").unwrap().as_field();
        let mut read: Vec<(Option<i32>, Option<&str>)> = message
            .fields()
            .iter()
            .map(|member| (member.as_fix().tag().unwrap(), member.as_fix().field_ref()))
            .collect();
        read.sort_unstable();
        assert_eq!(
            read,
            [
                (Some(55), Some("symbol")),
                (Some(61), Some("61")),
                (Some(9252), Some("urgency"))
            ]
        );
    }
}

#[test]
fn a_group_counted_by_a_field_held_as_text_retypes_it_whichever_file_sorts_first() {
    // The first dialect binds 711 plainly and types it as text; the second
    // counts `Underlyings` by it. A group is counted by NumInGroup, an int32,
    // and text said less than the counter did: the field held is retyped,
    // the group stands and the message reads it - whichever of the two files
    // sorts first, so one dictionary answers both orders.
    let text = r#"<cplugin-configuration fix-version="4.4">
      <vocabulary><vocabulary-tag name="711" alt="NoUnderlyings" type="string" /></vocabulary>
    </cplugin-configuration>"#;
    let group = underlyings("");
    let mut answers = Vec::new();
    for files in [
        [("a.cfb", text), ("b.cfb", group.as_str())],
        [("b.cfb", text), ("a.cfb", group.as_str())],
    ] {
        let tree = cblock_tree(&files);
        let mut registry = FixRegistry::new();
        let (merge, warnings) = super::warned::during(|| {
            registry.add_cfb_files(std::slice::from_ref(&tree), Some("venue"))
        });
        let merge = merge.expect("both files fold");
        assert!(merge.is_clean(), "{:?} {:?}", merge.dropped, merge.failed);
        assert_eq!(
            registry.field_by_tag(711).unwrap().dtype(),
            &DataType::Int32
        );
        assert!(
            registry
                .get_definition(yggdryl::FixCategory::Groups, "underlyings")
                .is_some()
        );
        let message = registry.msgtype("D").unwrap().as_field();
        assert!(
            message
                .fields()
                .iter()
                .any(|member| member.as_fix().group() == Some("underlyings")),
            "D reads the group"
        );
        // Retyping a field another dialect declared is said, not done
        // silently - only where the text file folded first, since a counter
        // already held as one has nothing to retype.
        if files.contains(&("a.cfb", text)) {
            assert!(
                warnings.iter().any(|warning| warning.contains(
                    "tag 711 counts a repeating group, so nounderlyings is retyped int32 from utf8"
                )),
                "{warnings:?}"
            );
        }
        answers.push(registry);
    }
    assert_eq!(
        answers[0], answers[1],
        "the order the files sort in decides nothing"
    );

    // A field held as a decimal is no coarser count but a quantity: the group
    // a source counts by it is passed over and named, and so is the member
    // of its message reading it.
    let mut held = DataType::decimal(38, 18)
        .unwrap()
        .nullable_field("nounderlyings");
    held.as_fix_mut().set_tag(711).unwrap();
    let mut registry = FixRegistry::from_fields([held]).unwrap();
    let tree = cblock_tree(&[("b.cfb", &underlyings(""))]);
    let merge = registry
        .add_cfb_files(std::slice::from_ref(&tree), None)
        .expect("the file folds");
    let passed: Vec<String> = merge.dropped.iter().map(ToString::to_string).collect();
    assert!(
        passed
            .iter()
            .any(|drop| drop.contains("int32 repeating-group counter")),
        "{passed:?}"
    );
    assert!(
        passed
            .iter()
            .any(|drop| drop.contains("which the merge passed over")),
        "{passed:?}"
    );
    assert_eq!(
        registry.field_by_tag(711).unwrap().dtype(),
        &DataType::decimal(38, 18).unwrap()
    );
    assert!(
        registry
            .get_definition(yggdryl::FixCategory::Groups, "underlyings")
            .is_none()
    );
}

#[test]
fn a_message_name_another_wire_code_holds_is_named_for_its_own_code() {
    // Two dialects name two custom codes `CustomReport`. They are two
    // messages, so the second takes the name its own wire value derives.
    let custom = |wire: &str| {
        format!(
            r#"<cplugin-configuration fix-version="4.4">
      <vocabulary>
        <vocabulary-tag name="35" alt="MsgType" type="string" />
        <vocabulary-tag name="55" alt="Symbol" type="string" />
      </vocabulary>
      <grammar-binding type="{wire}"><grammar><tag-constraint name="55" /></grammar></grammar-binding>
      <maps><map name="MSGTYPE"><entries><entry key="CustomReport" value="{wire}" /></entries></map></maps>
    </cplugin-configuration>"#
        )
    };
    let tree = cblock_tree(&[("a.cfb", &custom("U7")), ("b.cfb", &custom("U8"))]);
    let mut registry = FixRegistry::new();
    let merge = registry
        .add_cfb_files(std::slice::from_ref(&tree), None)
        .expect("both files fold");
    assert!(merge.is_clean(), "{:?}", merge.dropped);
    assert_eq!(registry.msgtype("U7").unwrap().name(), "customreport");
    assert_eq!(registry.msgtype("U8").unwrap().name(), "message5538");
}

#[test]
fn a_definition_another_dictionary_derived_a_held_tag_for_takes_a_free_one() {
    // A definition arrives carrying the tag its own file derived for it, and
    // in a dictionary that already holds definitions that slot can be taken.
    // Whatever it collides with, every definition ends on a tag of its own.
    let mut seeded = super::committed_registry().as_ref().clone();
    let venue = r#"<cplugin-configuration fix-version="4.4">
      <vocabulary>
        <vocabulary-tag name="9100" alt="NoVenueLegs" type="integer" />
        <vocabulary-tag name="9101" alt="VenueLegRef" type="string" />
      </vocabulary>
      <grammar-binding type="UV"><grammar>
        <grammar rg-name="VenueGrp263">
          <tag-constraint name="9100" />
          <tag-constraint name="9101" required="true" />
        </grammar>
      </grammar></grammar-binding>
    </cplugin-configuration>"#;
    let merge = seeded
        .add_cfb_file(&handle(venue), Some("venue"))
        .expect("the venue folds");
    assert!(merge.is_clean(), "{:?}", merge.dropped);
    let mut tags = std::collections::HashMap::new();
    for category in [
        yggdryl::FixCategory::Components,
        yggdryl::FixCategory::Groups,
    ] {
        for definition in seeded.definitions(category) {
            let tag = definition.as_fix().tag().unwrap().unwrap();
            if let Some(held) = tags.insert(tag, definition.name().to_owned()) {
                panic!("{held:?} and {:?} both hold {tag}", definition.name());
            }
        }
    }
    assert!(
        seeded
            .definitions(yggdryl::FixCategory::Groups)
            .any(|group| group.as_fix().counter().unwrap() == Some(9100)),
        "the venue's group arrived"
    );
}

/// A CBlock declaring both cases of one letter declares two message types.
///
/// FIX tag 35 is case-bearing - `b` is MassQuoteAcknowledgement and `B` is
/// News, `c` is SecurityDefinitionRequest and `C` is Email - so a dialect
/// stating both states two messages. The reader used to fold the two into
/// one and drop the second, which took a counterparty's whole lower-case
/// half of the protocol with it and then failed every mapping entry that
/// named one.
#[test]
fn a_cblock_declaring_both_cases_of_one_letter_keeps_two_message_types() {
    let body = r#"<?xml version="1.0"?>
<cplugin-configuration fix-version="4.4">
	<message-types>
		<message-type value="B" description="News" />
		<message-type value="b Inbound" description="Mass Quote Acknowledgement" />
		<message-type value="c SDR" description="Security Definition Request" />
		<message-type value="C" description="Email" />
		<message-type value="j Outbound" description="Business Message Reject" />
		<message-type value="J" description="Allocation" />
	</message-types>
	<outbound-message-type-mappings>
		<entry key="massquoteacknowledgement" value="b Inbound" />
		<entry key="businessmessagereject" value="j Outbound" />
	</outbound-message-type-mappings>
	<vocabulary><vocabulary-tag name="35" alt="MsgType" type="string" /></vocabulary>
	<grammar-binding type="b Inbound"><grammar><tag-constraint name="35" part="body" /></grammar></grammar-binding>
	<grammar-binding type="B"><grammar><tag-constraint name="35" part="body" /></grammar></grammar-binding>
</cplugin-configuration>"#;
    let (registry, _) = parse(body);
    let set = registry
        .codeset_of(registry.field_by_tag(35).expect("tag 35"))
        .expect("tag 35's vocabulary");
    let held: Vec<&str> = set.codes().map(|code| code.unwrap().value()).collect();
    assert_eq!(held, ["B", "b", "c", "C", "j", "J"]);
    // Every value answers itself and never its other case.
    for value in held {
        assert_eq!(set.code_value(value), Some(value), "{value:?}");
    }
    // A qualified spelling is still a spelling of the value it qualifies, and
    // the mapping tables' UlMessage keys still reach their own type.
    for (spelling, value) in [
        ("b Inbound", "b"),
        ("c SDR", "c"),
        ("j Outbound", "j"),
        ("massquoteacknowledgement", "b"),
        ("businessmessagereject", "j"),
    ] {
        assert_eq!(set.code_value(spelling), Some(value), "{spelling:?}");
    }
    // The described wording follows the type it was written beside.
    assert_eq!(
        set.code("b").and_then(|code| code.parse_doc().unwrap()),
        Some("Mass Quote Acknowledgement".to_owned())
    );
    assert_eq!(
        set.code("B").and_then(|code| code.parse_doc().unwrap()),
        Some("News".to_owned())
    );
    // The two bound grammars are two messages in the catalog, not one.
    let (lower, upper) = (
        registry.msgtype("b").expect("tag 35 b"),
        registry.msgtype("B").expect("tag 35 B"),
    );
    assert_eq!(lower.as_str(), "b");
    assert_eq!(upper.as_str(), "B");
    assert_ne!(lower.name(), upper.name());
}

/// One wire type is one message, however many grammars are bound under it,
/// and the members only a later binding declares are appended to the first.
#[test]
fn every_grammar_bound_under_one_wire_type_folds_into_one_message() {
    let body = r#"<?xml version="1.0"?>
<cplugin-configuration fix-version="4.4">
	<message-types>
		<message-type value="s" description="New Order Cross" />
		<message-type value="S" description="Quote" />
	</message-types>
	<vocabulary>
		<vocabulary-tag name="35" alt="MsgType" type="string" />
		<vocabulary-tag name="11" alt="ClOrdID" type="string" />
		<vocabulary-tag name="38" alt="OrderQty" type="float" />
		<vocabulary-tag name="44" alt="Price" type="float" />
		<vocabulary-tag name="55" alt="Symbol" type="string" />
	</vocabulary>
	<grammar-binding type="s Inbound"><grammar>
		<tag-constraint name="35" part="body" /><tag-constraint name="11" part="body" />
	</grammar></grammar-binding>
	<grammar-binding type="s Outbound"><grammar>
		<tag-constraint name="35" part="body" /><tag-constraint name="38" part="body" /><tag-constraint name="44" part="body" />
	</grammar></grammar-binding>
	<grammar-binding type="S"><grammar>
		<tag-constraint name="35" part="body" /><tag-constraint name="55" part="body" />
	</grammar></grammar-binding>
</cplugin-configuration>"#;
    let (registry, roots) = parse(body);
    // The caller is handed one root per binding, because that is what the
    // file bound.
    assert_eq!(
        roots.iter().map(Field::name).collect::<Vec<_>>(),
        ["s Inbound", "s Outbound", "S"]
    );
    // The dictionary holds the union: the first binding's members in their
    // order, then every member only the second declares.
    let cross = registry.msgtype("s").expect("tag 35 s");
    assert_eq!(
        children(cross.as_field()),
        ["msgtype", "clordid", "orderqty", "price"]
    );
    // And the other case is its own message, untouched by that fold.
    let quote = registry.msgtype("S").expect("tag 35 S");
    assert_eq!(children(quote.as_field()), ["msgtype", "symbol"]);
}

/// The same fold across two files: a second CBlock adds the members its own
/// grammar declares to the message the first one already stated.
#[test]
fn a_second_cblock_adds_the_members_only_it_declares_to_a_held_message() {
    let first = r#"<?xml version="1.0"?>
<cplugin-configuration fix-version="4.4">
	<message-types><message-type value="b" description="Mass Quote Acknowledgement" /></message-types>
	<vocabulary>
		<vocabulary-tag name="35" alt="MsgType" type="string" />
		<vocabulary-tag name="11" alt="ClOrdID" type="string" />
	</vocabulary>
	<grammar-binding type="b"><grammar>
		<tag-constraint name="35" part="body" /><tag-constraint name="11" part="body" />
	</grammar></grammar-binding>
</cplugin-configuration>"#;
    let second = r#"<?xml version="1.0"?>
<cplugin-configuration fix-version="4.4">
	<message-types>
		<message-type value="b" description="Mass Quote Acknowledgement" />
		<message-type value="B" description="News" />
	</message-types>
	<vocabulary>
		<vocabulary-tag name="35" alt="MsgType" type="string" />
		<vocabulary-tag name="44" alt="Price" type="float" />
		<vocabulary-tag name="55" alt="Symbol" type="string" />
	</vocabulary>
	<grammar-binding type="b"><grammar>
		<tag-constraint name="35" part="body" /><tag-constraint name="44" part="body" />
	</grammar></grammar-binding>
	<grammar-binding type="B"><grammar>
		<tag-constraint name="35" part="body" /><tag-constraint name="55" part="body" />
	</grammar></grammar-binding>
</cplugin-configuration>"#;
    let (mut registry, _) = parse(first);
    assert_eq!(
        children(registry.msgtype("b").expect("tag 35 b").as_field()),
        ["msgtype", "clordid"]
    );
    registry
        .add_cfb_file(&handle(second), Some(DIALECT))
        .expect("the second file folds");
    // The delta the second file declared is appended, in its own order, and
    // the members the first file declared keep theirs.
    assert_eq!(
        children(registry.msgtype("b").expect("tag 35 b").as_field()),
        ["msgtype", "clordid", "price"]
    );
    // The type the second file added alone arrives whole, and is not the
    // message the first file's lower-case type already named.
    assert_eq!(
        children(registry.msgtype("B").expect("tag 35 B").as_field()),
        ["msgtype", "symbol"]
    );
    let set = registry
        .codeset_of(registry.field_by_tag(35).expect("tag 35"))
        .expect("tag 35's vocabulary");
    assert_eq!(set.code_value("b"), Some("b"));
    assert_eq!(set.code_value("B"), Some("B"));
}

/// A dialect stating every case-bearing FIX 4.4 letter reads without one
/// warning, and holds each letter twice.
///
/// This is the whole of the reported defect: a real counterparty CBlock
/// declares `b`, `c`, `d`, `g`, `h`, `j`, `q`, `r` and `s` beside `B`, `C`,
/// `D`, `G`, `H`, `J`, `Q`, `R` and `S`, and the reader used to fold each
/// pair, warn once per lower-case type it dropped, and then warn again for
/// every mapping entry that named one of them. Nothing is dropped and
/// nothing is warned about: eighteen declared types are eighteen codes.
#[test]
fn a_dialect_stating_both_cases_of_every_letter_reads_without_a_warning() {
    const PAIRS: [(&str, &str); 9] = [
        ("b", "B"),
        ("c", "C"),
        ("d", "D"),
        ("g", "G"),
        ("h", "H"),
        ("j", "J"),
        ("q", "Q"),
        ("r", "R"),
        ("s", "S"),
    ];
    let mut body = String::from(
        "<?xml version=\"1.0\"?>\n<cplugin-configuration fix-version=\"4.4\">\n\t<message-types>\n",
    );
    for (lower, upper) in PAIRS {
        body.push_str(&format!(
            "\t\t<message-type value=\"{lower} Inbound\" description=\"Lower {lower}\" />\n\
             \t\t<message-type value=\"{upper}\" description=\"Upper {upper}\" />\n"
        ));
    }
    body.push_str("\t</message-types>\n\t<outbound-message-type-mappings>\n");
    for (lower, upper) in PAIRS {
        body.push_str(&format!(
            "\t\t<entry key=\"lower{lower}\" value=\"{lower} Inbound\" />\n\
             \t\t<entry key=\"upper{upper}\" value=\"{upper}\" />\n"
        ));
    }
    body.push_str(
        "\t</outbound-message-type-mappings>\n\
         \t<vocabulary><vocabulary-tag name=\"35\" alt=\"MsgType\" type=\"string\" /></vocabulary>\n",
    );
    for (lower, upper) in PAIRS {
        body.push_str(&format!(
            "\t<grammar-binding type=\"{lower} Inbound\"><grammar><tag-constraint name=\"35\" part=\"body\" /></grammar></grammar-binding>\n\
             \t<grammar-binding type=\"{upper}\"><grammar><tag-constraint name=\"35\" part=\"body\" /></grammar></grammar-binding>\n"
        ));
    }
    body.push_str("</cplugin-configuration>");

    let (read, warnings) =
        super::warned::during(|| FixRegistry::from_cfb_file(&handle(&body), Some(DIALECT)));
    assert!(warnings.is_empty(), "{warnings:#?}");
    let (registry, roots) = read.expect("a readable CBlock");
    assert_eq!(roots.len(), 18);
    let set = registry
        .codeset_of(registry.field_by_tag(35).expect("tag 35"))
        .expect("tag 35's vocabulary");
    assert_eq!(set.codes().count(), 18);
    for (lower, upper) in PAIRS {
        // Each letter answers itself on both sides of the case.
        assert_eq!(set.code_value(lower), Some(lower));
        assert_eq!(set.code_value(upper), Some(upper));
        // The wording stayed with the type it was written beside.
        assert_eq!(
            set.code(lower).and_then(|code| code.parse_doc().unwrap()),
            Some(format!("Lower {lower}"))
        );
        assert_eq!(
            set.code(upper).and_then(|code| code.parse_doc().unwrap()),
            Some(format!("Upper {upper}"))
        );
        // The qualifier and the UlMessage key both reach their own type.
        assert_eq!(set.code_value(&format!("{lower} Inbound")), Some(lower));
        assert_eq!(set.code_value(&format!("lower{lower}")), Some(lower));
        assert_eq!(set.code_value(&format!("upper{upper}")), Some(upper));
        // And each is its own message in the catalog.
        assert_eq!(registry.msgtype(lower).expect(lower).as_str(), lower);
        assert_eq!(registry.msgtype(upper).expect(upper).as_str(), upper);
    }
}

/// A document as ISO-8859-1 writes it: each character one byte.
fn latin1(text: &str) -> Vec<u8> {
    text.chars()
        .map(|character| u8::try_from(u32::from(character)).expect("an ISO-8859-1 character"))
        .collect()
}

/// A CBlock whose wording carries a character outside US-ASCII twice: in a
/// `message-type`'s `description` attribute and in a tag's `description`.
const ACCENTED: &str = r#"<?xml version="1.0" encoding="ISO-8859-1"?>
<cplugin-configuration fix-version="4.4">
	<message-types>
		<message-type value="D" description="Ordre créé" supported="true" />
	</message-types>
	<vocabulary>
		<vocabulary-tag name="35" alt="MsgType" type="string" />
		<vocabulary-tag name="58" alt="Text" type="string">
			<description>Texte libre, accentué</description>
		</vocabulary-tag>
	</vocabulary>
</cplugin-configuration>"#;

/// Reads one document's bytes, answering the dictionary and the warnings.
fn read_bytes(bytes: Vec<u8>) -> (FixRegistry, Vec<String>) {
    let (read, warnings) = super::warned::during(|| {
        FixRegistry::from_cfb_file(&Buffer::from_bytes(bytes), Some(DIALECT))
    });
    (read.expect("a readable CBlock").0, warnings)
}

/// What the accented document says, read back: the tag's description and
/// the wording of message type `D`.
fn accented_wording(registry: &FixRegistry) -> (Option<&str>, Option<String>) {
    let described = registry.field_by_tag(58).unwrap().description();
    let msgtype = registry
        .codeset_of(registry.field_by_tag(35).unwrap())
        .expect("tag 35's set");
    let wording = msgtype.code("D").expect("D").parse_doc().unwrap();
    (described, wording)
}

#[test]
fn a_document_crosses_the_charset_it_declares_once() {
    // ISO-8859-1 is what real exports declare, and a byte above 0x7F in an
    // attribute or a description is that charset's character - never a
    // refusal of the whole file, and never a replacement character.
    let bytes = latin1(ACCENTED);
    assert!(bytes.contains(&0xE9) && std::str::from_utf8(&bytes).is_err());
    let (registry, warnings) = read_bytes(bytes);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(
        accented_wording(&registry),
        (Some("Texte libre, accentué"), Some("Ordre créé".to_owned()))
    );

    // A byte order mark states the charset before the declaration can, so a
    // UTF-16LE export reads to the same dictionary.
    let mut sixteen = vec![0xFF, 0xFE];
    sixteen.extend(
        ACCENTED
            .replace("ISO-8859-1", "UTF-16")
            .encode_utf16()
            .flat_map(u16::to_le_bytes),
    );
    let (read, warnings) = read_bytes(sixteen);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(read, registry);

    // A document declaring UTF-8 with one byte a Windows editor left in it is
    // read whole: every valid run kept, the stray byte read as the
    // Windows-1252 character it is, and the one transcription named once.
    let declared = ACCENTED.replace("ISO-8859-1", "UTF-8");
    let at = declared.find("accentué").unwrap() + "accentu".len();
    let mut stray = declared.into_bytes();
    stray.splice(at..at + 'é'.len_utf8(), [0xE9]);
    let (read, warnings) = read_bytes(stray);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    for held in ["the document's utf-8 text", "read as Windows-1252"] {
        assert!(
            warnings[0].contains(held),
            "{held} missing from {warnings:?}"
        );
    }
    assert_eq!(accented_wording(&read), accented_wording(&registry));

    // A charset the crate has no table for is named once and the document
    // read under UTF-8, which is what a CBlock's grammar is written in.
    let unknown = ACCENTED.replace("ISO-8859-1", "X-KLINGON");
    let (read, warnings) = read_bytes(unknown.into_bytes());
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    for held in ["the document's charset", "X-KLINGON", "; read as utf-8"] {
        assert!(
            warnings[0].contains(held),
            "{held} missing from {warnings:?}"
        );
    }
    assert_eq!(accented_wording(&read), accented_wording(&registry));
}

#[test]
fn an_attribute_the_reader_cannot_unescape_is_kept_as_the_file_spelled_it() {
    // What a hand-edited export holds: a bare `&` in an attribute, an HTML
    // entity XML does not define, and an attribute stated twice. None of
    // them refuses the document.
    let body = r#"<?xml version="1.0"?>
<cplugin-configuration fix-version="4.4">
	<message-types>
		<message-type value="D" description="S&P order" supported="true" />
		<message-type value="F" description="Annul&eacute;" supported="true" />
	</message-types>
	<vocabulary>
		<vocabulary-tag name="35" alt="MsgType" type="string" />
		<vocabulary-tag name="58" alt="Text" alt="Other" type="string">
			<description>S&P 500 constituent</description>
		</vocabulary-tag>
	</vocabulary>
</cplugin-configuration>"#;
    let (read, warnings) =
        super::warned::during(|| FixRegistry::from_cfb_file(&handle(body), None));
    let (registry, _) = read.expect("a hand-edited CBlock reads whole");

    let text = registry.field_by_tag(58).unwrap();
    assert_eq!(text.description(), Some("S&P 500 constituent"));
    // An attribute stated twice is read once, the first statement standing.
    assert_eq!(text.name(), "text");
    assert!(registry.get_field_by_name("Other").is_none());

    // An attribute value that will not unescape keeps the file's spelling,
    // and each is named with the element that states it.
    let msgtype = registry
        .codeset_of(registry.field_by_tag(35).unwrap())
        .expect("tag 35's set");
    for (value, spelled) in [("D", "S&P order"), ("F", "Annul&eacute;")] {
        assert_eq!(
            msgtype
                .code(value)
                .expect(value)
                .parse_doc()
                .unwrap()
                .as_deref(),
            Some(spelled)
        );
    }
    assert_eq!(warnings.len(), 2, "{warnings:?}");
    for (warning, value) in warnings.iter().zip(["D", "F"]) {
        assert_located(body, warning);
        for held in [
            "expected an escaped description attribute".to_owned(),
            format!("in \"<message-type value=\\\"{value}\\\""),
            "; the value is kept as the file spelled it".to_owned(),
        ] {
            assert!(warning.contains(&held), "{held} missing from {warning}");
        }
    }
}

#[test]
fn a_type_word_reads_as_the_datatype_it_names_and_one_nothing_reads_is_text() {
    let body = r#"<?xml version="1.0"?>
<cplugin-configuration fix-version="4.4">
	<vocabulary>
		<vocabulary-tag name="44" alt="Price" type="float" />
		<vocabulary-tag name="75" alt="TradeDate" type="local-mkt-date" />
		<vocabulary-tag name="200" alt="MaturityMonthYear" type="month-year" />
		<vocabulary-tag name="96" alt="RawData" type="data" />
		<vocabulary-tag name="34" alt="MsgSeqNum" type="int64" />
		<vocabulary-tag name="5001" alt="VendorWidget" type="widget" />
	</vocabulary>
	<grammar-binding type="D"><grammar>
		<tag-constraint name="44" />
		<tag-constraint name="5001" required="true" />
	</grammar></grammar-binding>
</cplugin-configuration>"#;
    let (read, warnings) =
        super::warned::during(|| FixRegistry::from_cfb_file(&handle(body), None));
    let (registry, roots) = read.expect("a readable CBlock");

    // `float` is FIX's float family, float64.
    assert_eq!(
        registry.field_by_tag(44).unwrap().dtype(),
        &DataType::Float64
    );
    // A word outside the eight reads through the grammar and the FIX logical
    // names behind it, exactly as a caller's datatype expression would.
    for (tag, word) in [
        (75, "local-mkt-date"),
        (200, "month-year"),
        (96, "data"),
        (34, "int64"),
    ] {
        let named: DataType = word.parse().expect("a word the grammar reads");
        assert_ne!(named, DataType::utf8(), "{word} names more than text");
        assert_eq!(
            registry.field_by_tag(tag).unwrap().dtype(),
            &named,
            "{word}"
        );
    }
    assert_eq!(registry.field_by_tag(34).unwrap().dtype(), &DataType::Int64);

    // A word nothing reads types the tag as text, which every FIX datatype
    // is on the wire: the tag is kept, and so is every constraint naming it.
    let widget = registry.field_by_tag(5001).expect("the tag is kept");
    assert_eq!(widget.dtype(), &DataType::utf8());
    assert_eq!(children(&roots[0]), ["price", "vendorwidget"]);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert_located(body, &warnings[0]);
    for held in [
        "\"widget\"",
        "or a datatype name",
        "; the tag is typed string",
    ] {
        assert!(
            warnings[0].contains(held),
            "{held} missing from {warnings:?}"
        );
    }
}

#[test]
fn every_coarser_cblock_word_restates_under_the_committed_datatype_and_a_contradiction_is_named() {
    // The eight words are coarse, and each of these says less than the
    // dictionary rather than something else: a price as a float, a date as
    // a UTC midnight, a currency, a venue, a raw payload and a side as
    // text, a sequence number as an int32.
    let body = r#"<?xml version="1.0"?>
<cplugin-configuration fix-version="4.4">
	<vocabulary>
		<vocabulary-tag name="44" alt="Price" type="float" />
		<vocabulary-tag name="75" alt="TradeDate" type="utc-date" />
		<vocabulary-tag name="15" alt="Currency" type="string" />
		<vocabulary-tag name="207" alt="SecurityExchange" type="string" />
		<vocabulary-tag name="34" alt="MsgSeqNum" type="integer" />
		<vocabulary-tag name="96" alt="RawData" type="string" />
		<vocabulary-tag name="200" alt="MaturityMonthYear" type="string" />
		<vocabulary-tag name="231" alt="ContractMultiplier" type="float" />
		<vocabulary-tag name="54" alt="Side" type="char" />
	</vocabulary>
</cplugin-configuration>"#;
    let committed = super::committed_registry();
    let mut seeded = committed.as_ref().clone();
    let merge = seeded
        .add_cfb_file(&named_handle(body, "blpfix44.cfb"), Some(DIALECT))
        .expect("a coarser vocabulary folds");
    assert!(merge.dropped.is_empty(), "{:?}", merge.dropped);
    // Eight restated; ContractMultiplier is stored float64, which is what a
    // CBlock `float` reads as, so it merges as stated.
    assert_eq!(merge.restated, 8);
    for tag in [44, 75, 15, 207, 34, 96, 200, 231, 54] {
        let held = seeded.field_by_tag(tag).unwrap();
        assert_eq!(
            held.dtype(),
            committed.field_by_tag(tag).unwrap().dtype(),
            "tag {tag} keeps its stored datatype"
        );
        assert_eq!(sources(held), [DIALECT], "tag {tag}");
    }

    // A contradiction is still one: PossDupFlag stated as a number against
    // the committed flag is passed over, named with both datatypes and the
    // file it came from, and the rest of that file folds.
    let contradicting = r#"<?xml version="1.0"?>
<cplugin-configuration fix-version="4.4">
	<vocabulary>
		<vocabulary-tag name="43" alt="PossDupFlag" type="integer" />
		<vocabulary-tag name="44" alt="Price" type="float" />
	</vocabulary>
</cplugin-configuration>"#;
    let file = named_handle(contradicting, "tradeweb.cfb");
    let merge = seeded
        .add_cfb_file(&file, None)
        .expect("a contradiction folds the rest of the file");
    assert_eq!(merge.dropped.len(), 1, "{:?}", merge.dropped);
    assert_eq!(merge.restated, 1, "Price, again");
    let drop = &merge.dropped[0];
    assert_eq!(drop.incoming.as_fix().tag().unwrap(), Some(43));
    assert_eq!(drop.incoming.dtype(), &DataType::Int32);
    assert!(
        drop.reason.contains("boolean") && drop.reason.contains("int32"),
        "{drop}"
    );
    assert_eq!(
        drop.source.as_deref(),
        Some(file.url().unwrap().to_string().as_str())
    );
    assert_eq!(
        seeded.field_by_tag(43).unwrap(),
        committed.field_by_tag(43).unwrap()
    );
    assert_eq!(
        sources(seeded.field_by_tag(44).unwrap()),
        [DIALECT, "tradeweb"]
    );
}

/// Tag 39 decoded by two maps, the UlMessage way round and the FIX way
/// round, disagreeing on what code 8 is called; `last` is the map the file
/// states second.
fn two_maps(last: &str) -> String {
    let ulmessage = r#"<map name="ORDSTATUS"><entries>
			<entry key="new" value="0" />
			<entry key="none" value="8" />
		</entries></map>"#;
    let fix = r#"<map name="OrdStatus"><entries>
			<entry key="0" value="new" />
			<entry key="8" value="rejected" />
		</entries></map>"#;
    let (first, second) = if last == "ORDSTATUS" {
        (fix, ulmessage)
    } else {
        (ulmessage, fix)
    };
    format!(
        r#"<?xml version="1.0"?>
<cplugin-configuration fix-version="4.4">
	<vocabulary><vocabulary-tag name="39" alt="OrdStatus" type="char" /></vocabulary>
	<maps>
		{first}
		{second}
	</maps>
</cplugin-configuration>"#
    )
}

#[test]
fn two_maps_naming_one_code_two_ways_fold_into_the_committed_set() {
    // A real export decodes OrdStatus twice - once the UlMessage way, where
    // code 8 is `none`, and once the FIX way, where it is `rejected` - and
    // the dictionary already names code 8 `Rejected`. Folding it is never a
    // refusal of the file: the value is the committed code's, and the name
    // the file's maps give it is one more spelling of that code - but `none`
    // is a venue writing that it has no name to give, so it is no spelling
    // of anything, whichever map is last.
    let committed = super::committed_registry();
    for last in ["ORDSTATUS", "OrdStatus"] {
        let mut seeded = committed.as_ref().clone();
        let (merge, warnings) = super::warned::during(|| {
            seeded.add_cfb_file(&named_handle(&two_maps(last), "blpfix44.cfb"), None)
        });
        let merge = merge.expect("two maps of one tag never refuse the file");
        assert!(
            warnings.iter().any(|warning| warning
                .contains(r#"code set ordstatuscodeset: 1 map entry spelling none, null or nothing: "ORDSTATUS" key "none" value "8""#)),
            "{warnings:?}"
        );
        assert!(merge.dropped.is_empty(), "{:?}", merge.dropped);
        let status = seeded.field_by_tag(39).unwrap();
        let set = seeded.codeset_of(status).expect("the committed set");
        assert_eq!(set.name(), "ordstatuscodeset");
        assert_eq!(
            set.codes().count(),
            committed
                .codeset_of(committed.field_by_tag(39).unwrap())
                .unwrap()
                .codes()
                .count(),
            "the file states no value the dictionary lacks"
        );
        assert_eq!(set.code_name("8"), Some("Rejected"));
        assert_eq!(set.code_name("0"), Some("New"));
        assert_eq!(set.code_value("none"), None, "last map {last}");
        assert_eq!(set.code_value("rejected"), Some("8"));
    }
}

#[test]
fn an_unnamed_tag_in_one_file_is_the_field_another_file_names() {
    // One dialect declares tag 541 with no `alt`, another names it; both bind
    // it in message D. Whichever file sorts first, the dictionary holds one
    // field on the tag, named by the file that named it, carrying both
    // memberships - and the message reads that field.
    let unnamed = r#"<cplugin-configuration fix-version="4.4">
      <vocabulary><vocabulary-tag name="541" type="utc-date" /></vocabulary>
      <grammar-binding type="D"><grammar><tag-constraint name="541" /></grammar></grammar-binding>
    </cplugin-configuration>"#;
    let named = r#"<cplugin-configuration fix-version="4.4">
      <vocabulary><vocabulary-tag name="541" alt="MaturityDate" type="utc-date" /></vocabulary>
      <grammar-binding type="D"><grammar><tag-constraint name="541" /></grammar></grammar-binding>
    </cplugin-configuration>"#;
    for files in [
        [("a.cfb", unnamed), ("b.cfb", named)],
        [("a.cfb", named), ("b.cfb", unnamed)],
    ] {
        let tree = cblock_tree(&files);
        let mut registry = FixRegistry::new();
        let merge = registry
            .add_cfb_files(std::slice::from_ref(&tree), None)
            .expect("both files fold");
        assert!(merge.dropped.is_empty(), "{:?}", merge.dropped);
        let held: Vec<&Field> = registry
            .iter()
            .filter(|field| field.as_fix().tag().unwrap() == Some(541))
            .collect();
        assert_eq!(held.len(), 1, "one field on the tag: {held:?}");
        assert_eq!(held[0].name(), "maturitydate");
        assert_eq!(held[0].display(), Some("MaturityDate"));
        assert_eq!(sources(held[0]), ["a", "b"]);
        let message = registry.msgtype("D").unwrap().as_field();
        let members: Vec<(&str, Option<&str>)> = message
            .fields()
            .iter()
            .filter(|member| member.as_fix().tag().unwrap() == Some(541))
            .map(|member| (member.name(), member.as_fix().field_ref()))
            .collect();
        // One member, named as the field is: the member the file that named
        // the tag declared and the one the file that did not declared are
        // the same member once the field is one.
        assert_eq!(members, [("maturitydate", Some("maturitydate"))]);
    }

    // Within one file, a tag whose `alt` repeats another tag's is named by
    // the one spelling its normalization gives it, and the spelling is then
    // the other tag's alone: neither is left numbered, so neither links the
    // other.
    let restated = r#"<?xml version="1.0"?>
<cplugin-configuration fix-version="4.4">
	<vocabulary>
		<vocabulary-tag name="5190" alt="LegLastPx" type="float" />
		<vocabulary-tag name="637" alt="LegLastPx" type="float" />
	</vocabulary>
	<normalization-binding>
		<normalization type="inbound">
			<tag-normalization tag-name="LEGLASTSPOTRATE" part="body">
				<mapping-expression><expression value="$5190" /></mapping-expression>
			</tag-normalization>
		</normalization>
	</normalization-binding>
</cplugin-configuration>"#;
    let (registry, _) = parse(restated);
    let spot = registry.field_by_tag(5190).unwrap();
    assert_eq!(spot.name(), "leglastspotrate");
    assert_eq!(spot.display(), Some("LEGLASTSPOTRATE"));
    let last = registry.field_by_tag(637).unwrap();
    assert_eq!(last.name(), "leglastpx");
    assert_eq!(last.display(), Some("LegLastPx"));
    for field in [spot, last] {
        assert!(
            field.as_fix().tags().unwrap().is_empty(),
            "{}",
            field.name()
        );
    }
}

#[test]
fn a_field_arriving_on_a_held_tag_lends_the_holder_no_name() {
    // Two dialects put two unrelated fields on tag 1132. Each is a field of
    // its own, reached by its own name, and neither answers to the other's.
    let tree = cblock_tree(&[
        (
            "a.cfb",
            &one_tag(1132, "TZTransactTime").replace("string", "utc-timestamp"),
        ),
        ("b.cfb", &one_tag(1132, "BidPx2").replace("string", "float")),
    ]);
    let mut registry = FixRegistry::new();
    let merge = registry
        .add_cfb_files(std::slice::from_ref(&tree), None)
        .expect("both files fold");
    assert!(merge.dropped.is_empty(), "{:?}", merge.dropped);
    let held: Vec<&Field> = registry
        .iter()
        .filter(|field| field.as_fix().tag().unwrap() == Some(1132))
        .collect();
    assert_eq!(held.len(), 2, "{held:?}");
    for field in held {
        assert!(field.as_fix().names().next().is_none(), "{}", field.name());
    }
    assert_eq!(
        registry.field_by_tag(1132).unwrap().name(),
        "tztransacttime"
    );
    let price = registry.get_field_by_name("BidPx2").expect("the price");
    assert_eq!(price.name(), "bidpx2");
    assert_eq!(price.dtype(), &DataType::Float64);
    assert_eq!(
        registry
            .get_field_by_name("TZTransactTime")
            .map(Field::name),
        Some("tztransacttime")
    );
}

#[test]
fn a_stem_is_read_as_the_file_is_named() {
    // A stem stands in as the file is named rather than as its URL spells
    // it, so a space is a space and not its percent escape - by either door.
    let body = one_tag(9001, "VenueRef");
    let mut folded = FixRegistry::new();
    folded
        .add_cfb_file(&named_handle(&body, "Morgan Stanley.cfb"), None)
        .expect("a readable CBlock");
    assert_eq!(
        sources(folded.field_by_tag(9001).unwrap()),
        ["morgan stanley"]
    );
    let tree = cblock_tree(&[("Morgan Stanley.cfb", &body)]);
    let mut globbed = FixRegistry::new();
    globbed
        .add_cfb_files(std::slice::from_ref(&tree), None)
        .expect("a readable CBlock");
    assert_eq!(globbed, folded);

    // A stem carrying a comma is a name like any other: the ids are stored
    // as an array, so nothing a stem holds keeps it from standing in.
    let mut comma = FixRegistry::new();
    comma
        .add_cfb_file(&named_handle(&body, "ms,bloomberg.cfb"), None)
        .expect("a stem holding a comma is a name");
    assert_eq!(sources(comma.field_by_tag(9001).unwrap()), ["ms,bloomberg"]);
    assert_eq!(comma.dialects(), ["ms,bloomberg"]);
    assert_eq!(
        comma.get_source("ms,bloomberg").unwrap().file(),
        Some("ms,bloomberg.cfb")
    );
}

/// A venue's CBlock folded into the committed dictionary under its own stem,
/// with the warnings the fold logged.
fn fold_committed(
    body: &str,
    name: &str,
) -> (yggdryl::Result<yggdryl::FixMerge>, FixRegistry, Vec<String>) {
    let mut seeded = super::committed_registry().as_ref().clone();
    let (merge, warnings) =
        super::warned::during(|| seeded.add_cfb_file(&named_handle(body, name), None));
    (merge, seeded, warnings)
}

/// One vocabulary tag and the map a venue decodes it by, in the shape the
/// Avaloq and FrontArena exports state `DateRollConvention`.
fn date_roll(entries: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="US-ASCII"?>
<cplugin-configuration fix-version="4.4">
	<vocabulary>
		<vocabulary-tag name="40922" alt="DateRollConvention" type="string" />
	</vocabulary>
	<maps>
		<map name="DateRollConvention">
			{entries}
		</map>
	</maps>
</cplugin-configuration>"#
    )
}

#[test]
fn a_map_entry_spelling_nothing_states_no_code_and_is_named_once_per_code_set() {
    // `<entry key="0" value="none"/>` is a venue writing that it has no code
    // to state. The dictionary already holds a code named after its own
    // value `NONE`, and folding `none` beside it used to refuse the whole
    // batch over a name the writer counts twice. It states no code: the file
    // folds clean and the committed set is untouched.
    let committed = super::committed_registry();
    let held = committed
        .codeset("daterollconventioncodeset")
        .unwrap()
        .document()
        .to_owned();
    let (merge, seeded, warnings) = fold_committed(
        &date_roll(r#"<entry key="0" value="none" />"#),
        "Avaloq_FIX44_BuySide_FX.cfb",
    );
    let merge = merge.expect("a sentinel entry refuses nothing");
    assert!(merge.is_clean(), "{:?}", merge.dropped);
    let set = seeded.codeset("daterollconventioncodeset").unwrap();
    assert_eq!(set.document(), held, "the committed set is untouched");
    assert_eq!(set.code("0"), None);
    assert_eq!(set.code_value("NONE"), Some("NONE"));
    let named: Vec<&String> = warnings
        .iter()
        .filter(|warning| warning.contains("spelling none, null or nothing"))
        .collect();
    assert_eq!(named.len(), 1, "{warnings:?}");
    assert!(
        named[0].starts_with("Avaloq_FIX44_BuySide_FX.cfb ")
            && named[0].contains(
                r#"tag 40922 "DateRollConvention", code set daterollconventioncodeset: 1 map entry"#
            )
            && named[0].contains(r#""DateRollConvention" key "0" value "none""#),
        "{named:?}"
    );

    // Every spelling of nothing - any case, a separator, blank, or an
    // attribute the entry never states - on either side of an entry, and in
    // two maps of one tag: one warning for the one code set, counting them
    // all, and the codes the maps do state still fold.
    let (merge, seeded, warnings) = fold_committed(
        &date_roll(
            r#"<entry key="0" value="NULL" />
			<entry key="  " value="FirstDay" />
			<entry key="31" value="N_o-N e" />
			<entry value="SecondDay" />
			<entry key="EOM" value="EndOfMonth" />
		</map>
		<map name="DATEROLLCONVENTION">
			<entry key="none" value="5" />"#,
        ),
        "venue.cfb",
    );
    assert!(merge.expect("sentinels refuse nothing").is_clean());
    let named: Vec<&String> = warnings
        .iter()
        .filter(|warning| warning.contains("spelling none, null or nothing"))
        .collect();
    assert_eq!(named.len(), 1, "{warnings:?}");
    assert!(named[0].contains(": 5 map entries spelling"), "{named:?}");
    let set = seeded.codeset("daterollconventioncodeset").unwrap();
    assert_eq!(set.code_value("EndOfMonth"), Some("EOM"));
    assert_eq!(set.code("31"), None);
}

#[test]
fn a_name_folding_onto_a_held_placeholder_keeps_its_code_unnamed_rather_than_refusing() {
    // The committed set names `EOM` after its own value. A venue naming a
    // new value `eom` states a name a lookup of `EOM` would reach two ways,
    // which the writer refuses: the fold keeps the value under no name and
    // says so, rather than refusing the file - or the files beside it.
    let tree = cblock_tree(&[
        ("a.cfb", &date_roll(r#"<entry key="99" value="eom" />"#)),
        ("b.cfb", &one_tag(9001, "GoodRef")),
    ]);
    let mut seeded = super::committed_registry().as_ref().clone();
    let (merge, warnings) =
        super::warned::during(|| seeded.add_cfb_files(std::slice::from_ref(&tree), None));
    let merge = merge.expect("one name refuses nothing");
    assert_eq!(merge.sources, 2);
    assert!(merge.is_clean(), "{:?} {:?}", merge.dropped, merge.failed);
    let set = seeded.codeset("daterollconventioncodeset").unwrap();
    assert_eq!(set.code_name("99"), Some("99"));
    assert_eq!(set.code_value("EOM"), Some("EOM"));
    assert!(seeded.get_field_by_tag(9001).is_some());
    assert!(
        warnings
            .iter()
            .any(|warning| warning.contains(r#"value "99" keeps no name"#)),
        "{warnings:?}"
    );
}

#[test]
fn two_fields_a_fold_lands_on_one_held_field_are_two_members_of_one_message() {
    // Predator binds tag 37, which it leaves unnamed, and a custom 9037 it
    // calls `OrderID`. The fold lands both on the committed `orderid`: the
    // name reaches it, and the bare tag is its own. The message reads the one
    // field twice, in wire order, as a duplicate constraint does - never a
    // refusal of the whole file over a struct naming a member twice. The
    // file names type 8 nothing readable, so its message is filed under the
    // name the wire value derives.
    let body = r#"<?xml version="1.0" encoding="US-ASCII"?>
<cplugin-configuration fix-version="4.4">
  <message-types><message-type value="8" description="Execution Report" supported="true" /></message-types>
  <vocabulary>
    <vocabulary-tag name="35" alt="MsgType" type="string" />
    <vocabulary-tag name="37" type="string" />
    <vocabulary-tag name="9037" alt="OrderID" type="string" />
  </vocabulary>
  <grammar-binding type="8">
    <grammar checkordering="false">
      <tag-constraint name="35" part="header" required="true" />
      <tag-constraint name="37" part="body" required="true" />
      <tag-constraint name="9037" part="body" required="false" />
    </grammar>
  </grammar-binding>
</cplugin-configuration>"#;
    let (merge, seeded, _) = fold_committed(body, "Predator_FIX_44_BuySide0101.cfb");
    let merge = merge.expect("one message, one field twice");
    assert!(merge.is_clean(), "{:?}", merge.dropped);
    let held = seeded.field_by_tag(9037).unwrap();
    assert_eq!(held.name(), "orderid");
    assert_eq!(held.as_fix().tag().unwrap(), Some(37));
    let message = seeded
        .definition(yggdryl::FixCategory::Components, "message38")
        .unwrap();
    let reading: Vec<(&str, Option<&str>, Option<i32>)> = message
        .fields()
        .iter()
        .filter(|member| member.as_fix().field_ref() == Some("orderid"))
        .map(|member| {
            (
                member.name(),
                member.as_fix().field_ref(),
                member.as_fix().tag().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        reading,
        [
            ("orderid", Some("orderid"), Some(37)),
            ("orderid2", Some("orderid"), Some(37))
        ]
    );
}

#[test]
fn a_field_passed_over_on_a_held_tag_leaves_its_members_reading_the_held_field() {
    // One venue types DealerID(9691) as an integer in its dealers group; the
    // next leaves 9691 unnamed and types it a flag. The flag contradicts the
    // integer and is passed over - and the next venue's group member reads
    // the field the dictionary keeps, rather than a field named `9691` that
    // nothing holds, which used to refuse the whole file.
    let first = r#"<cplugin-configuration fix-version="4.4">
      <vocabulary>
        <vocabulary-tag name="35" alt="MsgType" type="string" />
        <vocabulary-tag name="9690" alt="NoDealers" type="integer" />
        <vocabulary-tag name="9691" alt="DealerID" type="integer" />
      </vocabulary>
      <grammar-binding type="8"><grammar>
        <grammar rg-name="Dealers"><tag-constraint name="9690" /><tag-constraint name="9691" /></grammar>
      </grammar></grammar-binding>
    </cplugin-configuration>"#;
    let second = first.replace(
        r#"<vocabulary-tag name="9691" alt="DealerID" type="integer" />"#,
        r#"<vocabulary-tag name="9691" type="boolean" />"#,
    );
    let tree = cblock_tree(&[("a.cfb", first), ("b.cfb", &second)]);
    let mut registry = FixRegistry::new();
    let merge = registry
        .add_cfb_files(std::slice::from_ref(&tree), None)
        .expect("a contradiction is one declaration");
    assert!(merge.failed.is_empty(), "{:?}", merge.failed);
    let passed: Vec<String> = merge.dropped.iter().map(ToString::to_string).collect();
    assert_eq!(passed.len(), 1, "{passed:?}");
    assert!(
        passed[0].contains("b.cfb: ")
            && passed[0].contains("int32 stored for dealerid (9691), got boolean"),
        "{passed:?}"
    );
    let dealer = registry
        .definition(yggdryl::FixCategory::Components, "dealer")
        .unwrap();
    assert_eq!(
        dealer
            .fields()
            .iter()
            .map(|member| (member.name(), member.as_fix().field_ref()))
            .collect::<Vec<_>>(),
        [("dealerid", Some("dealerid"))]
    );
}

/// A trade capture report counting its regulatory trade identifiers by
/// `counter`, spelled `spelling`, under `rg-name` where one is given.
fn regulatory(counter: i32, spelling: &str, declared: Option<&str>) -> String {
    let declared = declared.map_or_else(String::new, |name| format!(r#" rg-name="{name}""#));
    let spelling = if spelling.is_empty() {
        String::new()
    } else {
        format!(r#" alt="{spelling}""#)
    };
    format!(
        r#"<?xml version="1.0" encoding="US-ASCII"?>
<cplugin-configuration fix-version="4.4">
  <message-types><message-type value="AE" description="Trade Capture Report" supported="true" /></message-types>
  <vocabulary>
    <vocabulary-tag name="35" alt="MsgType" type="string" />
    <vocabulary-tag name="571" alt="TradeReportID" type="string" />
    <vocabulary-tag name="{counter}"{spelling} type="integer" />
    <vocabulary-tag name="1903" alt="RegulatoryTradeID" type="string" />
    <vocabulary-tag name="1905" alt="RegulatoryTradeIDSource" type="string" />
  </vocabulary>
  <grammar-binding type="AE">
    <grammar checkordering="false">
      <tag-constraint name="35" part="header" required="true" />
      <tag-constraint name="571" part="body" required="true" />
      <grammar{declared}>
        <tag-constraint name="{counter}" required="false" />
        <tag-constraint name="1903" required="false" />
        <tag-constraint name="1905" required="false" />
      </grammar>
    </grammar>
  </grammar-binding>
</cplugin-configuration>"#
    )
}

#[test]
fn a_group_counted_by_a_field_merged_onto_another_tag_counts_the_held_group() {
    // A venue counts RegulatoryTradeIDGrp by NoRegulatoryTradeIDs on its own
    // tag 20001. The name reaches the committed counter on 1907, so 20001 is
    // that field spelled with another number - and the venue's group, read
    // under the counter that holds it now, is the committed group rather
    // than an unknown one passed over with every member reading it.
    let (merge, seeded, _) = fold_committed(
        &regulatory(20001, "NoRegulatoryTradeIDs", None),
        "venue.cfb",
    );
    let merge = merge.expect("one group, one counter");
    assert!(merge.is_clean(), "{:?}", merge.dropped);
    assert_eq!(
        seeded.field_by_tag(20001).unwrap().as_fix().tag().unwrap(),
        Some(1907)
    );
    let message = seeded
        .definition(yggdryl::FixCategory::Components, "message4145")
        .unwrap();
    assert!(
        message
            .fields()
            .iter()
            .any(|member| member.as_fix().group() == Some("regulatorytradeids")),
        "the venue's AE reads the committed group"
    );
}

#[test]
fn a_group_counted_by_a_count_s_alternate_tag_counts_the_held_group() {
    // One venue spells NoRegulatoryTradeIDs with its own 20001, so 20001 is
    // that count spelled with another number. Another counts the same group
    // by an unnamed 20001: a count on a count's alternate is that count, so
    // the group folds into the held one as the first venue's did.
    let tree = cblock_tree(&[
        ("a.cfb", &regulatory(20001, "NoRegulatoryTradeIDs", None)),
        ("b.cfb", &regulatory(20001, "", None)),
    ]);
    let mut seeded = super::committed_registry().as_ref().clone();
    let merge = seeded
        .add_cfb_files(std::slice::from_ref(&tree), None)
        .expect("one group, one counter");
    assert!(merge.is_clean(), "{:?} {:?}", merge.dropped, merge.failed);
    assert_eq!(merge.sources, 2);
    let message = seeded
        .definition(yggdryl::FixCategory::Components, "message4145")
        .unwrap();
    assert!(
        message
            .fields()
            .iter()
            .any(|member| member.as_fix().group() == Some("regulatorytradeids")),
        "both venues' AE read the committed group"
    );
}

#[test]
fn a_group_held_under_its_name_on_another_counter_arrives_under_its_counter() {
    // A venue names its own NoRegTradeIDs(20001) group RegulatoryTradeIDGrp,
    // which the dictionary holds on 1907. Two counters are two groups: the
    // venue's arrives named for its counter, and the message member reading
    // it stands beside the one AE already reads, rather than being passed
    // over. A second venue's AE reading the committed group folds into the
    // first venue's message, and the two members stand side by side there.
    let tree = cblock_tree(&[
        ("a.cfb", &regulatory(1907, "NoRegulatoryTradeIDs", None)),
        (
            "b.cfb",
            &regulatory(20001, "NoRegTradeIDs", Some("RegulatoryTradeIDGrp")),
        ),
    ]);
    let mut seeded = super::committed_registry().as_ref().clone();
    let merge = seeded.add_cfb_files(std::slice::from_ref(&tree), None);
    let merge = merge.expect("two groups, two counters");
    assert!(merge.is_clean(), "{:?}", merge.dropped);
    let group = seeded
        .definition(yggdryl::FixCategory::Groups, "regulatorytradeids_20001")
        .expect("the venue's group under its counter");
    assert_eq!(group.as_fix().counter().unwrap(), Some(20001));
    let message = seeded
        .definition(yggdryl::FixCategory::Components, "message4145")
        .unwrap();
    let groups: Vec<(&str, Option<&str>)> = message
        .fields()
        .iter()
        .filter(|member| {
            member
                .as_fix()
                .group()
                .is_some_and(|name| name.starts_with("regulatorytradeids"))
        })
        .map(|member| (member.name(), member.as_fix().group()))
        .collect();
    assert_eq!(
        groups,
        [
            ("regulatorytradeids", Some("regulatorytradeids")),
            ("regulatorytradeids_20001", Some("regulatorytradeids_20001"))
        ]
    );
}

#[test]
fn a_group_counted_by_another_field_s_alternate_tag_is_passed_over_and_the_field_kept() {
    // One venue spells Text with its own 9001, so the dictionary reads 9001
    // as Text's value. Another counts a group by an unnamed 9001: that
    // contradicts the dictionary, so the counter is passed over and named -
    // never Text retyped int32, nor the group moved onto 58, either of which
    // would read every Text on the wire as a count.
    let tree = cblock_tree(&[
        ("a.cfb", &one_tag(9001, "Text")),
        ("b.cfb", &regulatory(9001, "", None)),
    ]);
    let committed = super::committed_registry();
    let mut seeded = committed.as_ref().clone();
    let merge = seeded
        .add_cfb_files(std::slice::from_ref(&tree), None)
        .expect("one contradiction refuses nothing");
    assert_eq!(merge.sources, 2);
    assert!(merge.failed.is_empty(), "{:?}", merge.failed);
    let text = seeded.field_by_tag(58).unwrap();
    assert_eq!(text.dtype(), committed.field_by_tag(58).unwrap().dtype());
    assert!(text.as_fix().tags().unwrap().contains(&9001));
    let passed: Vec<String> = merge.dropped.iter().map(ToString::to_string).collect();
    assert!(
        passed
            .iter()
            .any(|drop| drop.contains("the alternate tag of text (58), held as utf8")),
        "{passed:?}"
    );
    assert!(
        seeded
            .definitions(yggdryl::FixCategory::Groups)
            .all(|group| group.as_fix().counter().unwrap() != Some(58)),
        "no group is counted by Text"
    );
}

#[test]
fn a_time_of_day_against_a_held_instant_is_passed_over_saying_why() {
    // MaturityTime(1079) is a TZTimeOnly, which the dictionary reads as an
    // instant on the epoch day - and a bare clock as null. A CBlock has no
    // word for it but `utc-time-only`, a time of day: the two read disjoint
    // spellings, so the declaration stays a contradiction, and the reason
    // says what each reading accepts.
    let body = r#"<cplugin-configuration fix-version="4.4">
      <vocabulary><vocabulary-tag name="1079" alt="MaturityTime" type="utc-time-only" /></vocabulary>
    </cplugin-configuration>"#;
    let (merge, seeded, _) = fold_committed(body, "SmartTrade_EventFeed.cfb");
    let merge = merge.expect("one contradiction refuses nothing");
    let passed: Vec<String> = merge.dropped.iter().map(ToString::to_string).collect();
    assert_eq!(passed.len(), 1, "{passed:?}");
    assert!(
        passed[0].contains(r#"expected the datatype datetime64(ns,"UTC") stored for maturitytime (1079), got time64(ns): the stored instant reads a clock only with its offset, a bare time of day as null"#),
        "{passed:?}"
    );
    assert_eq!(
        seeded.field_by_tag(1079).unwrap().dtype(),
        super::committed_registry()
            .field_by_tag(1079)
            .unwrap()
            .dtype()
    );
}

#[test]
fn a_spelling_no_catalog_name_holds_is_folded_and_kept_as_the_display() {
    // Bloomberg and Fidessa spell fields, normalizations and groups with
    // spaces and brackets. A reference names a catalog name, so each folds
    // to one - lower case, every run of anything else one `_` - and keeps
    // the spelling as its display: no message is dropped over a name.
    let body = r#"<?xml version="1.0"?>
<cplugin-configuration fix-version="4.4">
	<vocabulary>
		<vocabulary-tag name="35" alt="MsgType" type="string" />
		<vocabulary-tag name="9701" alt="OTC Trade Flags" type="string" />
		<vocabulary-tag name="9705" alt="(BloombergCustomTag05)" type="string" />
		<vocabulary-tag name="9720" alt="No Fidessa Legs" type="integer" />
		<vocabulary-tag name="9721" alt="FidessaLegRef" type="string" />
		<vocabulary-tag name="9740" type="string" />
		<vocabulary-tag name="9750" alt="???" type="string" />
	</vocabulary>
	<maps>
		<map name="(BloombergCustomTag05)"><entry key="A" value="alpha" /></map>
	</maps>
	<grammar-binding type="8"><grammar>
		<tag-constraint name="9701" />
		<tag-constraint name="9705" />
		<tag-constraint name="9740" />
		<tag-constraint name="9750" />
		<grammar rg-name="(Fidessa Legs)">
			<tag-constraint name="9720" />
			<tag-constraint name="9721" />
		</grammar>
	</grammar></grammar-binding>
	<normalization-binding>
		<normalization type="inbound">
			<tag-normalization tag-name="Fidessa Msg Type">
				<mapping-expression><expression value="$9740" /></mapping-expression>
			</tag-normalization>
		</normalization>
	</normalization-binding>
</cplugin-configuration>"#;
    let (read, warnings) =
        super::warned::during(|| FixRegistry::from_cfb_file(&handle(body), Some(DIALECT)));
    let (registry, roots) = read.expect("a readable CBlock");
    assert_eq!(roots.len(), 1, "{warnings:?}");
    assert!(warnings.is_empty(), "{warnings:?}");
    for (tag, name, display) in [
        (9701, "otc_trade_flags", Some("OTC Trade Flags")),
        (9705, "bloombergcustomtag05", Some("(BloombergCustomTag05)")),
        (9720, "no_fidessa_legs", Some("No Fidessa Legs")),
        (9740, "fidessa_msg_type", Some("Fidessa Msg Type")),
        (9750, "9750", Some("???")),
    ] {
        let field = registry.field_by_tag(tag).unwrap();
        assert_eq!(
            (field.name(), field.display()),
            (name, display),
            "tag {tag}"
        );
    }
    // The map decodes the field its spelling names, filed under the name.
    let set = registry
        .codeset_of(registry.field_by_tag(9705).unwrap())
        .expect("the set the map decodes");
    assert_eq!(set.name(), "bloombergcustomtag05codeset");
    assert_eq!(set.code_value("alpha"), Some("A"));
    let group = registry
        .definition(yggdryl::FixCategory::Groups, "fidessa_legs")
        .expect("the group the rg-name names");
    assert_eq!(group.display(), Some("(Fidessa Legs)"));
    let message = registry.msgtype("8").unwrap().as_field();
    assert!(
        message
            .fields()
            .iter()
            .any(|member| member.as_fix().field_ref() == Some("otc_trade_flags"))
    );
}

#[test]
fn a_message_declaring_one_group_in_several_shapes_takes_a_split_per_shape() {
    use yggdryl::FixCategory::{Components, Groups};

    // Message 8 declares Parties one way. Z declares them a second way at
    // its root and a third inside its legs, and `b` twice more beside each
    // other: every shape is a definition of its own, split for the message
    // in the order it declares them, and no message is dropped.
    let parties = |members: &str| {
        format!(
            r#"<grammar rg-name="Parties"><tag-constraint name="453" /><tag-constraint name="448" />{members}</grammar>"#
        )
    };
    let body = format!(
        r#"<?xml version="1.0"?>
<cplugin-configuration fix-version="4.4">
	<vocabulary>
		<vocabulary-tag name="35" alt="MsgType" type="string" />
		<vocabulary-tag name="453" alt="NoPartyIDs" type="integer" />
		<vocabulary-tag name="448" alt="PartyID" type="string" />
		<vocabulary-tag name="447" alt="PartyIDSource" type="char" />
		<vocabulary-tag name="452" alt="PartyRole" type="integer" />
		<vocabulary-tag name="555" alt="NoLegs" type="integer" />
		<vocabulary-tag name="600" alt="LegSymbol" type="string" />
	</vocabulary>
	<grammar-binding type="8"><grammar>{plain}</grammar></grammar-binding>
	<grammar-binding type="Z"><grammar>{source}
		<grammar rg-name="Legs"><tag-constraint name="555" /><tag-constraint name="600" />{role}</grammar>
	</grammar></grammar-binding>
</cplugin-configuration>"#,
        plain = parties(""),
        source = parties(r#"<tag-constraint name="447" />"#),
        role = parties(r#"<tag-constraint name="452" />"#),
    );
    let (read, warnings) =
        super::warned::during(|| FixRegistry::from_cfb_file(&handle(&body), Some(DIALECT)));
    let (registry, roots) = read.expect("a readable CBlock");
    assert_eq!(roots.len(), 2, "{warnings:?}");
    assert!(warnings.is_empty(), "{warnings:?}");
    for (name, member) in [
        ("party", None),
        ("party_message5a", Some("partyidsource")),
        ("party_message5a_2", Some("partyrole")),
    ] {
        let component = registry.definition(Components, name).expect(name);
        if let Some(member) = member {
            assert!(component.get_field(member).is_some(), "{name} {member}");
        }
    }
    assert!(
        registry
            .get_definition(Groups, "parties_message5a_2")
            .is_some()
    );
}

/// One CBlock whose messages `8` and `D` each declare the parties group over
/// `453`, `448` and `447`, both requiring `448`: message `8` spells it
/// `Parties` with `447` required, message `D` spells it `parties` with `447`
/// optional, and `members` is appended to `D`'s declaration.
fn parties_twice(members: &str) -> String {
    format!(
        r#"<?xml version="1.0"?>
<cplugin-configuration fix-version="4.4">
	<vocabulary>
		<vocabulary-tag name="35" alt="MsgType" type="string" />
		<vocabulary-tag name="453" alt="NoPartyIDs" type="integer" />
		<vocabulary-tag name="448" alt="PartyID" type="string" />
		<vocabulary-tag name="447" alt="PartyIDSource" type="char" />
		<vocabulary-tag name="452" alt="PartyRole" type="integer" />
	</vocabulary>
	<grammar-binding type="8"><grammar>
		<grammar rg-name="Parties"><tag-constraint name="453" /><tag-constraint name="448" required="true" /><tag-constraint name="447" required="true" /></grammar>
	</grammar></grammar-binding>
	<grammar-binding type="D"><grammar>
		<grammar rg-name="parties"><tag-constraint name="453" /><tag-constraint name="448" required="true" /><tag-constraint name="447" />{members}</grammar>
	</grammar></grammar-binding>
	<maps>
		<map name="MSGTYPE"><entries><entry key="NewOrderSingle" value="D" /></entries></map>
	</maps>
</cplugin-configuration>"#
    )
}

#[test]
fn a_message_declaring_a_group_another_declared_alike_reads_that_group_however_strictly_spelled() {
    use yggdryl::FixCategory::{Components, Groups};

    // The same tags on the same counter in the same order are one group,
    // whatever `D` spells it and however strictly it states a member: the
    // definition `8` wrote holds, its member relaxed to what `D` states, and
    // `D` reads it rather than a split of its own.
    let (read, warnings) = super::warned::during(|| parse(&parties_twice("")));
    let (registry, roots) = read;
    assert_eq!(roots.len(), 2, "{warnings:?}");
    assert!(warnings.is_empty(), "{warnings:?}");
    for (category, split) in [
        (Components, "party_newordersingle"),
        (Groups, "parties_newordersingle"),
    ] {
        assert!(
            registry.get_definition(category, split).is_none(),
            "{split} is no definition: one structure is one definition"
        );
    }
    let party = registry.definition(Components, "party").unwrap();
    assert_eq!(
        party.display(),
        Some("Party"),
        "the first declaration's spelling holds"
    );
    assert!(
        party.get_field("partyidsource").unwrap().is_nullable(),
        "a member one message requires and another does not is nullable"
    );
    assert!(!party.get_field("partyid").unwrap().is_nullable());
    for wire in ["8", "D"] {
        let message = registry.msgtype(wire).unwrap().as_field();
        let member = message
            .fields()
            .iter()
            .find(|member| member.as_fix().group().is_some())
            .unwrap_or_else(|| panic!("message {wire} holds the group"));
        assert_eq!(member.as_fix().group(), Some("parties"), "message {wire}");
        assert_eq!(member.display(), Some("Parties"), "message {wire}");
        let item = super::item_of(member);
        assert!(
            item.get_field("partyidsource").unwrap().is_nullable(),
            "message {wire} holds the relaxed occurrence"
        );
    }

    // Another member is another structure, and is still the split it was.
    let (registry, _) = parse(&parties_twice(r#"<tag-constraint name="452" />"#));
    assert!(
        registry
            .get_definition(Components, "party_newordersingle")
            .is_some()
    );
    assert!(
        !registry
            .definition(Components, "party")
            .unwrap()
            .get_field("partyidsource")
            .unwrap()
            .is_nullable(),
        "a definition no alike declaration met keeps what it stated"
    );
}

/// One CBlock whose message `8` declares the parties group over `453`,
/// `448` and `447`, spelled `name`, `448` required and `447` as `strictness`
/// states, and whose message `D` is `members`.
fn parties_file(name: &str, strictness: &str, members: &str) -> String {
    format!(
        r#"<cplugin-configuration fix-version="4.4">
      <vocabulary>
        <vocabulary-tag name="35" alt="MsgType" type="string" />
        <vocabulary-tag name="453" alt="NoPartyIDs" type="integer" />
        <vocabulary-tag name="448" alt="PartyID" type="string" />
        <vocabulary-tag name="447" alt="PartyIDSource" type="char" />
        <vocabulary-tag name="55" alt="Symbol" type="string" />
      </vocabulary>
      <grammar-binding type="8"><grammar>
        <grammar rg-name="{name}"><tag-constraint name="453" /><tag-constraint name="448" required="true" /><tag-constraint name="447" {strictness}/></grammar>
      </grammar></grammar-binding>
      <grammar-binding type="D"><grammar>{members}</grammar></grammar-binding>
    </cplugin-configuration>"#
    )
}

#[test]
fn two_cblocks_declaring_one_component_differing_only_in_nullability_and_sources_fold_into_one() {
    use yggdryl::FixCategory::{Components, Groups};

    // Two dialects declare the parties group alike - the same tags on the
    // same counter in the same order - and differ in what no structure is:
    // one requires PartyIDSource where the other does not, and each is a
    // source of its own. Folded, they are one component and one group, of
    // both sources, the member nullable because the dictionary serves both -
    // whichever dialect folds first.
    for (strict, lax) in [("a.cfb", "b.cfb"), ("b.cfb", "a.cfb")] {
        let tree = cblock_tree(&[
            (
                strict,
                &parties_file(
                    "Parties",
                    r#"required="true" "#,
                    r#"<tag-constraint name="55" />"#,
                ),
            ),
            (
                lax,
                &parties_file("Parties", "", r#"<tag-constraint name="55" />"#),
            ),
        ]);
        let mut registry = FixRegistry::new();
        let merge = registry
            .add_cfb_files(std::slice::from_ref(&tree), None)
            .expect("both dialects fold");
        assert!(merge.is_clean(), "{:?} {:?}", merge.dropped, merge.failed);
        let components: Vec<&str> = super::definitions(&registry, Components)
            .map(Field::name)
            .filter(|name| name.starts_with("part"))
            .collect();
        assert_eq!(components, ["party"], "{strict} first");
        let groups: Vec<&str> = super::definitions(&registry, Groups)
            .map(Field::name)
            .filter(|name| name.starts_with("part"))
            .collect();
        assert_eq!(groups, ["parties"], "{strict} first");
        let party = registry.definition(Components, "party").unwrap();
        assert_eq!(sources(party), ["a", "b"], "{strict} first");
        assert!(
            party.get_field("partyidsource").unwrap().is_nullable(),
            "{strict} first: a member one dialect requires and the other does not is nullable"
        );
        assert!(
            !party.get_field("partyid").unwrap().is_nullable(),
            "{strict} first: a member both require is required"
        );
        let parties = registry.definition(Groups, "parties").unwrap();
        assert_eq!(sources(parties), ["a", "b"], "{strict} first");
        assert_eq!(parties.as_fix().component(), Some("party"));
        assert!(
            super::item_of(parties)
                .get_field("partyidsource")
                .unwrap()
                .is_nullable(),
            "{strict} first: the group holds the relaxed occurrence"
        );
        // Both dialects' message 8 is one message reading the one group.
        let message = registry.msgtype("8").unwrap().as_field();
        let members: Vec<(&str, Option<&str>)> = message
            .fields()
            .iter()
            .filter(|member| member.as_fix().group().is_some())
            .map(|member| (member.name(), member.as_fix().group()))
            .collect();
        assert_eq!(members, [("parties", Some("parties"))], "{strict} first");
    }
}

#[test]
fn a_dialect_declaring_a_held_structure_under_another_name_reads_the_held_definition() {
    use yggdryl::FixCategory::{Components, Groups};

    // `a` declares the parties group as `Parties`; `c` declares the same
    // structure as `Dealers`, in its message 8 and in a message D of its own.
    // Folded, the structure is the one definition `a` named: no `dealer` or
    // `dealers` is held, `c`'s message D reads `parties` under the member
    // name its grammar gave it, and message 8 - one message in both
    // dialects - reads the group once, not under each dialect's name.
    let tree = cblock_tree(&[
        (
            "a.cfb",
            &parties_file("Parties", "", r#"<tag-constraint name="55" />"#),
        ),
        (
            "c.cfb",
            &parties_file(
                "Dealers",
                "",
                r#"<grammar rg-name="Dealers"><tag-constraint name="453" /><tag-constraint name="448" required="true" /><tag-constraint name="447" /></grammar>"#,
            ),
        ),
    ]);
    let mut registry = FixRegistry::new();
    let merge = registry
        .add_cfb_files(std::slice::from_ref(&tree), None)
        .expect("both dialects fold");
    assert!(merge.is_clean(), "{:?} {:?}", merge.dropped, merge.failed);
    for (category, name) in [(Components, "dealer"), (Groups, "dealers")] {
        assert!(
            registry.get_definition(category, name).is_none(),
            "{name} is the structure `a` named"
        );
    }
    let party = registry.definition(Components, "party").unwrap();
    assert_eq!(sources(party), ["a", "c"]);
    assert_eq!(
        party.display(),
        Some("Party"),
        "the first declaration's spelling holds"
    );
    let parties = registry.definition(Groups, "parties").unwrap();
    assert_eq!(sources(parties), ["a", "c"]);
    for (wire, member) in [("8", "parties"), ("D", "dealers")] {
        let message = registry.msgtype(wire).unwrap().as_field();
        let members: Vec<(&str, Option<&str>)> = message
            .fields()
            .iter()
            .filter(|member| member.as_fix().group().is_some())
            .map(|member| (member.name(), member.as_fix().group()))
            .collect();
        assert_eq!(members, [(member, Some("parties"))], "message {wire}");
    }
    // Folded again, the same files change nothing.
    let once = registry.clone();
    let merge = registry
        .add_cfb_files(std::slice::from_ref(&tree), None)
        .expect("both dialects fold again");
    assert!(merge.is_clean(), "{:?} {:?}", merge.dropped, merge.failed);
    assert_eq!(registry, once);
}

#[test]
fn a_message_declaring_a_held_structure_under_another_name_reads_the_held_definition() {
    use yggdryl::FixCategory::{Components, Groups};

    // Message 8 declares the parties group as `Parties`; message D of the
    // same file declares the structure as `Dealers`, one member stated
    // otherwise. The structure is the definition 8 named: no `dealer` or
    // `dealers` is held, D's member reads `parties` under the name its own
    // grammar gave it with the held spelling, and the member D states
    // otherwise is relaxed.
    let (read, warnings) = super::warned::during(|| {
        parse(&parties_file(
            "Parties",
            r#"required="true" "#,
            r#"<grammar rg-name="Dealers"><tag-constraint name="453" /><tag-constraint name="448" required="true" /><tag-constraint name="447" /></grammar>"#,
        ))
    });
    let (registry, roots) = read;
    assert_eq!(roots.len(), 2, "{warnings:?}");
    assert!(warnings.is_empty(), "{warnings:?}");
    for (category, name) in [(Components, "dealer"), (Groups, "dealers")] {
        assert!(
            registry.get_definition(category, name).is_none(),
            "{name} is the structure 8 named"
        );
    }
    let party = registry.definition(Components, "party").unwrap();
    assert!(!party.get_field("partyid").unwrap().is_nullable());
    assert!(
        party.get_field("partyidsource").unwrap().is_nullable(),
        "a member one message requires and the other does not is nullable"
    );
    let message = registry.msgtype("D").unwrap().as_field();
    let members: Vec<(&str, Option<&str>, Option<&str>)> = message
        .fields()
        .iter()
        .filter(|member| member.as_fix().group().is_some())
        .map(|member| (member.name(), member.as_fix().group(), member.display()))
        .collect();
    assert_eq!(members, [("dealers", Some("parties"), Some("Parties"))]);
}

/// One CBlock whose message `D` is `grammar`, over the party, allocation and
/// leg fields.
fn nested_parties_file(grammar: &str) -> String {
    format!(
        r#"<cplugin-configuration fix-version="4.4">
      <vocabulary>
        <vocabulary-tag name="35" alt="MsgType" type="string" />
        <vocabulary-tag name="453" alt="NoPartyIDs" type="integer" />
        <vocabulary-tag name="448" alt="PartyID" type="string" />
        <vocabulary-tag name="447" alt="PartyIDSource" type="char" />
        <vocabulary-tag name="78" alt="NoAllocs" type="integer" />
        <vocabulary-tag name="79" alt="AllocAccount" type="string" />
        <vocabulary-tag name="555" alt="NoLegs" type="integer" />
        <vocabulary-tag name="600" alt="LegSymbol" type="string" />
      </vocabulary>
      <grammar-binding type="D"><grammar>{grammar}</grammar></grammar-binding>
    </cplugin-configuration>"#
    )
}

/// The parties group as a grammar declares it, `448` as `strictness` states.
fn parties_grammar(strictness: &str) -> String {
    format!(
        r#"<grammar rg-name="Parties"><tag-constraint name="453" /><tag-constraint name="448" {strictness}/><tag-constraint name="447" /></grammar>"#
    )
}

/// The nullability of every `partyid` a record holds, at any depth, in
/// document order.
fn partyids(field: &Field) -> Vec<bool> {
    match field.dtype() {
        DataType::Struct(children) => children.iter().flat_map(partyids).collect(),
        DataType::Serie(item) => partyids(item),
        _ if field.name() == "partyid" => vec![field.is_nullable()],
        _ => Vec::new(),
    }
}

/// The vocabulary the nested-parties cases below share, and `8` declaring
/// `Parties` on 453 beside `D` declaring `NestedParties` on 539.
fn two_parties_file(more: &str) -> String {
    format!(
        r#"<cplugin-configuration fix-version="4.4">
  <vocabulary>
    <vocabulary-tag name="35" alt="MsgType" type="string" />
    <vocabulary-tag name="453" alt="NoPartyIDs" type="integer" />
    <vocabulary-tag name="448" alt="PartyID" type="string" />
    <vocabulary-tag name="447" alt="PartyIDSource" type="char" />
    <vocabulary-tag name="539" alt="NoNestedPartyIDs" type="integer" />
    <vocabulary-tag name="524" alt="NestedPartyID" type="string" />
    <vocabulary-tag name="525" alt="NestedPartyIDSource" type="char" />
    <vocabulary-tag name="55" alt="Symbol" type="string" />
  </vocabulary>
  <grammar-binding type="8"><grammar>
    <tag-constraint name="55" />
    <grammar rg-name="Parties"><tag-constraint name="453" /><tag-constraint name="448" required="true" /><tag-constraint name="447" /></grammar>
  </grammar></grammar-binding>
  <grammar-binding type="D"><grammar>
    <tag-constraint name="55" />
    <grammar rg-name="NestedParties"><tag-constraint name="539" /><tag-constraint name="524" required="true" /><tag-constraint name="525" /></grammar>
  </grammar></grammar-binding>{more}
</cplugin-configuration>"#
    )
}

/// A message `G` declaring `NestedParties` on 453 over 448 and 447: the
/// structure `Parties` holds under the name `NestedParties` holds for
/// another.
const NESTED_PARTIES_AS_PARTIES: &str = r#"
  <grammar-binding type="G"><grammar>
    <tag-constraint name="55" />
    <grammar rg-name="NestedParties"><tag-constraint name="453" /><tag-constraint name="448" required="true" /><tag-constraint name="447" /></grammar>
  </grammar></grammar-binding>"#;

#[test]
fn a_held_structure_under_a_name_held_for_another_is_the_held_definition_in_one_file_and_merges_by_name_across_two()
 {
    use yggdryl::FixCategory::{Components, Groups};

    // `a` declares `Parties` on 453 in message 8 and `NestedParties` on 539
    // in message D; `c` declares `NestedParties` on 453 over 448 and 447 in
    // a message G - the structure `party` and `parties` hold, under a name
    // `a` holds for another structure.
    //
    // In one file the grammar's group is the held group of its structure:
    // G's member reads `parties`, and `nestedparty` keeps the two members D
    // declared. Across two files every definition folds by name first, as a
    // name two dialects spell always has: `c`'s `nestedparty` widens the
    // held one, its group on 453 stands beside the one on 539 as
    // `nestedparties_453`, and G reads it - a structure `parties` does not
    // state, so `party` keeps `a` alone. Folding `c` again changes nothing.
    let c = format!(
        r#"<cplugin-configuration fix-version="4.4">
  <vocabulary>
    <vocabulary-tag name="35" alt="MsgType" type="string" />
    <vocabulary-tag name="453" alt="NoPartyIDs" type="integer" />
    <vocabulary-tag name="448" alt="PartyID" type="string" />
    <vocabulary-tag name="447" alt="PartyIDSource" type="char" />
    <vocabulary-tag name="55" alt="Symbol" type="string" />
  </vocabulary>{NESTED_PARTIES_AS_PARTIES}
</cplugin-configuration>"#
    );
    let group_members = |registry: &FixRegistry| -> Vec<(String, Option<String>)> {
        registry
            .msgtype("G")
            .unwrap()
            .as_field()
            .fields()
            .iter()
            .filter(|member| member.as_fix().group().is_some())
            .map(|member| {
                (
                    member.name().to_owned(),
                    member.as_fix().group().map(str::to_owned),
                )
            })
            .collect()
    };
    let groups = |registry: &FixRegistry| -> Vec<(String, Option<i32>)> {
        super::definitions(registry, Groups)
            .filter(|group| {
                group.name().starts_with("parties") || group.name().starts_with("nested")
            })
            .map(|group| (group.name().to_owned(), group.as_fix().counter().unwrap()))
            .collect()
    };

    let (one_file, _) = parse(&two_parties_file(NESTED_PARTIES_AS_PARTIES));
    assert_eq!(
        children(one_file.definition(Components, "nestedparty").unwrap()),
        ["nestedpartyid", "nestedpartyidsource"],
        "one file: the structure D declared under the name is not widened into"
    );
    assert_eq!(
        groups(&one_file),
        [
            ("nestedparties".to_owned(), Some(539)),
            ("parties".to_owned(), Some(453))
        ],
        "one file: one group per structure"
    );
    assert_eq!(
        group_members(&one_file),
        [("nestedparties".to_owned(), Some("parties".to_owned()))],
        "one file: G reads the held group under its own grammar's name"
    );

    let two = cblock_tree(&[("a.cfb", &two_parties_file("")), ("c.cfb", &c)]);
    let mut two_files = FixRegistry::new();
    let merge = two_files
        .add_cfb_files(std::slice::from_ref(&two), None)
        .expect("both dialects fold");
    assert!(merge.is_clean(), "{:?} {:?}", merge.dropped, merge.failed);
    let nestedparty = two_files.definition(Components, "nestedparty").unwrap();
    assert_eq!(
        children(nestedparty),
        [
            "nestedpartyid",
            "nestedpartyidsource",
            "partyid",
            "partyidsource"
        ],
        "two files: a name both dialects spell merges by name"
    );
    assert_eq!(sources(nestedparty), ["a", "c"]);
    assert_eq!(
        groups(&two_files),
        [
            ("nestedparties".to_owned(), Some(539)),
            ("nestedparties_453".to_owned(), Some(453)),
            ("parties".to_owned(), Some(453))
        ],
        "two files: the group on 453 stands beside the one on 539"
    );
    assert_eq!(
        group_members(&two_files),
        [(
            "nestedparties".to_owned(),
            Some("nestedparties_453".to_owned())
        )],
        "two files: G reads the group on its own counter"
    );
    let party = two_files.definition(Components, "party").unwrap();
    assert_eq!(children(party), ["partyid", "partyidsource"]);
    assert_eq!(
        sources(party),
        ["a"],
        "no structure the fold made is party's"
    );

    let once = two_files.clone();
    let again = two_files
        .add_cfb_files(std::slice::from_ref(&two), None)
        .expect("both dialects fold again");
    assert!(again.is_clean(), "{:?} {:?}", again.dropped, again.failed);
    assert_eq!(
        two_files, once,
        "folding the same files twice changes nothing"
    );
}

#[test]
fn a_message_relaxing_a_definition_it_already_read_reads_it_relaxed_throughout() {
    use yggdryl::FixCategory::{Components, Groups};

    // One message states the parties group twice, strictly first: at its
    // root and then inside its legs, or inside its allocations and then
    // inside its legs. The second statement relaxes the definition the first
    // wrote, and the members built on the first - the root's parties, the
    // allocations and the occurrence they hold - read the relaxed definition
    // too, so the message restates exactly what the catalog holds and is
    // kept whole rather than dropped for a copy it no longer holds. Stated
    // laxly first, the second statement relaxes nothing and the message is
    // the same.
    let strict = parties_grammar(r#"required="true" "#);
    let lax = parties_grammar("");
    let legs = |parties: &str| {
        format!(
            r#"<grammar rg-name="Legs"><tag-constraint name="555" /><tag-constraint name="600" />{parties}</grammar>"#
        )
    };
    let allocs = |parties: &str| {
        format!(
            r#"<grammar rg-name="Allocs"><tag-constraint name="78" /><tag-constraint name="79" />{parties}</grammar>"#
        )
    };
    for (case, grammar) in [
        ("the root, then the legs", format!("{strict}{}", legs(&lax))),
        (
            "the allocations, then the legs",
            format!("{}{}", allocs(&strict), legs(&lax)),
        ),
        (
            "laxly at the root, then the legs",
            format!("{lax}{}", legs(&strict)),
        ),
    ] {
        let (read, warnings) = super::warned::during(|| {
            FixRegistry::from_cfb_file(&handle(&nested_parties_file(&grammar)), Some(DIALECT))
        });
        let (registry, roots) = read.unwrap_or_else(|error| panic!("{case}: {error}"));
        assert!(warnings.is_empty(), "{case}: {warnings:?}");
        assert_eq!(roots.len(), 1, "{case}");
        for (category, split) in [
            (Components, "party_message44"),
            (Groups, "parties_message44"),
        ] {
            assert!(
                registry.get_definition(category, split).is_none(),
                "{case}: {split} is no definition, one structure being one"
            );
        }
        let party = registry.definition(Components, "party").unwrap();
        assert!(
            party.get_field("partyid").unwrap().is_nullable(),
            "{case}: a member one statement requires and the other does not is nullable"
        );
        let message = registry.msgtype("D").unwrap().as_field();
        assert_eq!(
            partyids(message),
            [true, true],
            "{case}: every occurrence the message holds is the relaxed one"
        );
        assert_eq!(
            FixRegistry::from_json(&registry.into_json().unwrap()).unwrap(),
            registry,
            "{case}"
        );
    }
}

#[test]
fn a_dialect_naming_a_held_group_s_counter_after_its_own_wider_group_widens_the_held_one() {
    use yggdryl::FixCategory::{Components, Groups};

    // `a` reads the parties group on 453 as `Parties`; `c` declares a wider
    // group on the same counter as `Dealers`, with a role no `a` member
    // states. Two groups on one counter are one member of message 8 read
    // two ways, so `c`'s folds into the member `a` declared rather than
    // standing beside it, widening the held component by the role - and
    // once widened, `c`'s definitions are of its structure and are it.
    let dealers = r#"<cplugin-configuration fix-version="4.4">
      <vocabulary>
        <vocabulary-tag name="35" alt="MsgType" type="string" />
        <vocabulary-tag name="453" alt="NoPartyIDs" type="integer" />
        <vocabulary-tag name="448" alt="PartyID" type="string" />
        <vocabulary-tag name="447" alt="PartyIDSource" type="char" />
        <vocabulary-tag name="452" alt="PartyRole" type="integer" />
        <vocabulary-tag name="55" alt="Symbol" type="string" />
      </vocabulary>
      <grammar-binding type="8"><grammar>
        <grammar rg-name="Dealers"><tag-constraint name="453" /><tag-constraint name="448" required="true" /><tag-constraint name="447" /><tag-constraint name="452" /></grammar>
      </grammar></grammar-binding>
    </cplugin-configuration>"#;
    let tree = cblock_tree(&[
        (
            "a.cfb",
            &parties_file("Parties", "", r#"<tag-constraint name="55" />"#),
        ),
        ("c.cfb", dealers),
    ]);
    let mut registry = FixRegistry::new();
    let merge = registry
        .add_cfb_files(std::slice::from_ref(&tree), None)
        .expect("both dialects fold");
    assert!(merge.is_clean(), "{:?} {:?}", merge.dropped, merge.failed);
    let message = registry.msgtype("8").unwrap().as_field();
    let members: Vec<(&str, Option<&str>)> = message
        .fields()
        .iter()
        .filter(|member| member.as_fix().group().is_some())
        .map(|member| (member.name(), member.as_fix().group()))
        .collect();
    assert_eq!(members, [("parties", Some("parties"))]);
    let party = registry.definition(Components, "party").unwrap();
    assert_eq!(
        party.fields().iter().map(Field::name).collect::<Vec<_>>(),
        ["partyid", "partyidsource", "partyrole"]
    );
    assert_eq!(sources(party), ["a", "c"]);
    for (category, name) in [(Components, "dealer"), (Groups, "dealers")] {
        assert!(
            registry.get_definition(category, name).is_none(),
            "{name} is the structure `parties` now has"
        );
    }
}

#[test]
fn a_second_binding_of_one_wire_type_folds_into_the_first_member_by_member() {
    // `S Inbound` and `S Outbound` are one message, S, bound twice with legs
    // of two shapes on one counter. The second folds into the first: the
    // legs S reads gain what only the second declared, and the message is
    // one message rather than one binding dropped.
    let body = r#"<?xml version="1.0"?>
<cplugin-configuration fix-version="4.4">
	<vocabulary>
		<vocabulary-tag name="35" alt="MsgType" type="string" />
		<vocabulary-tag name="555" alt="NoLegs" type="integer" />
		<vocabulary-tag name="600" alt="LegSymbol" type="string" />
		<vocabulary-tag name="624" alt="LegSide" type="char" />
		<vocabulary-tag name="687" alt="LegQty" type="float" />
		<vocabulary-tag name="686" alt="LegPriceType" type="integer" />
	</vocabulary>
	<grammar-binding type="8"><grammar>
		<grammar rg-name="Legs"><tag-constraint name="555" /><tag-constraint name="600" /><tag-constraint name="624" /></grammar>
	</grammar></grammar-binding>
	<grammar-binding type="S Inbound"><grammar>
		<grammar rg-name="Legs"><tag-constraint name="555" /><tag-constraint name="600" /><tag-constraint name="687" /></grammar>
	</grammar></grammar-binding>
	<grammar-binding type="S Outbound"><grammar>
		<grammar rg-name="Legs"><tag-constraint name="555" /><tag-constraint name="600" /><tag-constraint name="686" /></grammar>
	</grammar></grammar-binding>
</cplugin-configuration>"#;
    let (read, warnings) =
        super::warned::during(|| FixRegistry::from_cfb_file(&handle(body), Some(DIALECT)));
    let (registry, roots) = read.expect("a readable CBlock");
    assert_eq!(roots.len(), 3, "one root per binding: {warnings:?}");
    assert!(warnings.is_empty(), "{warnings:?}");
    let message = registry.msgtype("S").unwrap().as_field();
    let legs = message
        .fields()
        .iter()
        .find(|member| member.name() == "legs")
        .expect("S reads its legs");
    let group = registry
        .definition(yggdryl::FixCategory::Groups, legs.as_fix().group().unwrap())
        .unwrap();
    let component = registry
        .definition(
            yggdryl::FixCategory::Components,
            group.as_fix().component().unwrap(),
        )
        .unwrap();
    for member in ["legsymbol", "legqty", "legpricetype"] {
        assert!(component.get_field(member).is_some(), "{member}");
    }
    assert!(
        component.get_field("legside").is_none(),
        "8's legs stay 8's"
    );
}

#[test]
fn a_location_is_read_for_what_it_holds() {
    // A folder holds the .cfb files directly inside it, a glob what it
    // matches, a file itself - and nothing at all, nothing. Several
    // locations are one fold, and a file reached twice folds once.
    let root = yggdryl::local::LocalFolder::temporary()
        .unwrap()
        .path()
        .unwrap()
        .join(format!("yggdryl-cfb-locations-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("cblocks/nested")).unwrap();
    std::fs::write(root.join("cblocks/MSFIX44.cfb"), one_tag(9001, "MsRef")).unwrap();
    std::fs::write(root.join("cblocks/blpfix44.CFB"), one_tag(9002, "BlpRef")).unwrap();
    std::fs::write(root.join("cblocks/notes.txt"), "not a dictionary").unwrap();
    std::fs::write(
        root.join("cblocks/nested/deep.cfb"),
        one_tag(9003, "DeepRef"),
    )
    .unwrap();
    std::fs::write(root.join("venue.xml"), one_tag(9004, "XmlRef")).unwrap();
    let held = |path: &std::path::Path| yggdryl::holder::Holder::local(path).unwrap();
    let tags = |registry: &FixRegistry| {
        [9001, 9002, 9003, 9004].map(|tag| registry.get_field_by_tag(tag).is_some())
    };
    for (locations, sources, expected) in [
        (
            vec![held(&root.join("cblocks"))],
            2,
            [true, true, false, false],
        ),
        (
            vec![held(&root.join("cblocks/*.cfb"))],
            1,
            [true, false, false, false],
        ),
        (
            vec![held(&root.join("cblocks/**/*.cfb"))],
            2,
            [true, false, true, false],
        ),
        (
            vec![held(&root.join("venue.xml"))],
            1,
            [false, false, false, true],
        ),
        (
            vec![held(&root.join("nothing.cfb"))],
            0,
            [false, false, false, false],
        ),
        (
            vec![
                held(&root.join("cblocks")),
                held(&root.join("cblocks/MSFIX44.cfb")),
            ],
            2,
            [true, true, false, false],
        ),
    ] {
        let mut registry = FixRegistry::new();
        let merge = registry
            .add_cfb_files(&locations, None)
            .expect("a readable location");
        assert_eq!(
            (merge.sources, tags(&registry)),
            (sources, expected),
            "{locations:?}"
        );
    }
    // A location holding no file beside one that holds some adds nothing,
    // and says so rather than folding the rest without a word.
    let (merge, warnings) = super::warned::during(|| {
        FixRegistry::new().add_cfb_files(
            &[
                held(&root.join("cblocks")),
                held(&root.join("cblocks/*.fix")),
            ],
            None,
        )
    });
    assert_eq!(merge.expect("a readable location").sources, 2);
    assert!(
        warnings
            .iter()
            .any(|warning| warning.contains("*.fix: holds no .cfb file")),
        "{warnings:?}"
    );
    std::fs::remove_dir_all(&root).unwrap();
}

#[cfg(feature = "internals")]
mod internal {
    use yggdryl::FixRegistry;
    use yggdryl::internals::fix_codes::create_codeset;

    #[test]
    fn a_stored_set_stating_one_name_twice_heals_when_a_cblock_folds_into_it() {
        // What an earlier loader filed: one set naming two codes `pending`,
        // which answers the spelling for neither.
        let mut registry = FixRegistry::new();
        create_codeset(
            &mut registry,
            "ordstatuscodeset",
            r#"[{"value":"8","name":"pending"},{"value":"9","name":"pending"}]"#.to_owned(),
        )
        .expect("a loader files what it is handed");
        let stored = registry.codeset("ordstatuscodeset").unwrap();
        assert_eq!(
            stored.code_value("pending"),
            None,
            "two codes, one spelling"
        );

        // Folding a file that decodes the tag by that set is not refused: the
        // first code keeps the name, the second keeps its value and no name,
        // and the file's own codes join them.
        let (merge, warnings) = super::super::warned::during(|| {
            registry.add_cfb_file(
                &super::named_handle(&super::two_maps("ORDSTATUS"), "blpfix44.cfb"),
                None,
            )
        });
        let merge = merge.expect("the fold heals the stored set");
        assert!(merge.dropped.is_empty(), "{:?}", merge.dropped);
        let set = registry
            .codeset_of(registry.field_by_tag(39).unwrap())
            .expect("the set tag 39 reads by");
        assert_eq!(set.name(), "ordstatuscodeset");
        assert_eq!(set.code_name("8"), Some("pending"));
        assert_eq!(set.code_name("9"), Some("9"), "the second keeps its value");
        assert_eq!(set.code_value("pending"), Some("8"));
        assert_eq!(set.code_value("new"), Some("0"));
        assert!(
            warnings
                .iter()
                .any(|warning| warning.contains("value \"9\" keeps no name")
                    && warning.contains("\"pending\" already names value \"8\"")),
            "{warnings:?}"
        );
    }
}

#[test]
fn required_reads_every_spelling_a_flag_is_read_in_and_a_condition_stays_not_required() {
    let condition = r#"required="$59 = '6' and empty($126)""#;
    assert_eq!(CBLOCK.matches(condition).count(), 1);
    for (spelled, nullable) in [
        ("true", false),
        ("TRUE", false),
        ("yes", false),
        ("Y", false),
        ("on", false),
        ("1", false),
        ("false", true),
        ("no", true),
        ("0", true),
        // A condition is no flag: it is read as not-required, never refused.
        ("$59 = '6'", true),
        ("", true),
    ] {
        let body = CBLOCK.replacen(condition, &format!(r#"required="{spelled}""#), 1);
        let (_, roots) = parse(&body);
        let fields = roots[0].dtype().as_fields().unwrap();
        assert_eq!(fields[3].is_nullable(), nullable, "required={spelled:?}");
    }
}

#[test]
fn an_ingest_holds_the_dialect_in_the_sources_catalog_under_the_file_it_read() {
    // The stem names the dialect and the entry records the file as it is
    // named - the percent escape decoded - so the dictionary says where each
    // id came from without a field carrying it.
    let mut registry = FixRegistry::new();
    registry
        .add_cfb_file(&named_handle(CBLOCK, "Venue FIX44.cfb"), None)
        .expect("a readable CBlock");
    let entry = registry
        .get_source("venue fix44")
        .expect("the stem's entry");
    assert_eq!(entry.file(), Some("Venue FIX44.cfb"));
    assert_eq!(registry.dialects(), ["venue fix44"]);
    assert!(
        registry
            .field_by_name("exludeddealers")
            .unwrap()
            .as_fix()
            .has_source("Venue FIX44")
    );
    // A supplied name is the id, and the file is still the handle's.
    let mut supplied = FixRegistry::new();
    supplied
        .add_cfb_file(&named_handle(CBLOCK, "one.cfb"), Some("Desk"))
        .expect("a readable CBlock");
    assert_eq!(supplied.get_source("desk").unwrap().file(), Some("one.cfb"));
    assert!(supplied.get_source("one").is_none());
    // A buffer names no file, and no dialect holds no entry.
    let mut buffered = FixRegistry::new();
    buffered
        .add_cfb_file(
            &Buffer::from_bytes(CBLOCK.as_bytes().to_vec()),
            Some("desk"),
        )
        .expect("a readable CBlock");
    assert_eq!(buffered.get_source("desk").unwrap().file(), None);
    let (bare, _) = FixRegistry::from_cfb_file(&handle(CBLOCK), None).unwrap();
    assert_eq!(bare.sources().len(), 0);
    // Through a glob, one entry per file, each under its own stem, and a
    // dialect ingested again keeps its one entry.
    let first = one_tag(9001, "First");
    let second = one_tag(9002, "Second");
    let tree = cblock_tree(&[("a.cfb", &first), ("b.cfb", &second)]);
    let mut globbed = FixRegistry::new();
    globbed
        .add_cfb_files(std::slice::from_ref(&tree), None)
        .expect("readable CBlocks");
    assert_eq!(
        globbed
            .sources()
            .map(|source| (source.id(), source.file()))
            .collect::<Vec<_>>(),
        [("a", Some("a.cfb")), ("b", Some("b.cfb"))]
    );
    let third = one_tag(9003, "Third");
    globbed
        .add_cfb_file(&named_handle(&third, "a.cfb"), None)
        .expect("a readable CBlock");
    assert_eq!(globbed.sources().len(), 2);
    assert_eq!(sources(globbed.field_by_tag(9003).unwrap()), ["a"]);
}

/// The root's `type` names the plugin's class, and its role - `BuySide`
/// or `SellSide` in the class's own name - is the one fact read off the
/// root: the dialect's catalog entry states it, `UKNW` where the root
/// names neither or nothing, and a file read under no dialect drops it with
/// the entry it would have stated it on.
#[test]
fn the_root_type_names_the_plugins_role_which_the_dialects_entry_states() {
    let (buy, _) = parse(CBLOCK);
    assert_eq!(
        buy.get_source(DIALECT).unwrap().pluginside(),
        PluginSide::BuySide
    );
    let (sell, _) = FixRegistry::from_cfb_file(&handle(SELLSIDE), Some("ms")).unwrap();
    assert_eq!(
        sell.get_source("ms").unwrap().pluginside(),
        PluginSide::SellSide
    );
    let (overlay, _) = FixRegistry::from_cfb_file(&handle(OVERLAY), Some("overlay")).unwrap();
    assert_eq!(
        overlay.get_source("overlay").unwrap().pluginside(),
        PluginSide::SellSide
    );
    // No type, a type naming neither role, and a role in the package rather
    // than the class: none stated.
    for (body, why) in [
        (one_tag(9001, "Bare"), "no type attribute"),
        (
            one_tag(9001, "Bare").replace(
                "<cplugin-configuration fix-version=\"4.4\">",
                "<cplugin-configuration type=\"com.x.cblock.FIXCPluginCBlock\" fix-version=\"4.4\">",
            ),
            "a class naming neither role",
        ),
        (
            one_tag(9001, "Bare").replace(
                "<cplugin-configuration fix-version=\"4.4\">",
                "<cplugin-configuration type=\"com.buyside.FIXCPluginCBlock\" fix-version=\"4.4\">",
            ),
            "a package naming a role names none",
        ),
    ] {
        let (bare, _) = FixRegistry::from_cfb_file(&handle(&body), Some("bare")).unwrap();
        assert_eq!(
            bare.get_source("bare").unwrap().pluginside(),
            PluginSide::Unknown,
            "{why}"
        );
    }
    // No dialect, no entry, so nowhere for the role to be stated.
    let (none, _) = FixRegistry::from_cfb_file(&handle(CBLOCK), None).unwrap();
    assert_eq!(none.sources().len(), 0);

    // Ingested under their dialects into one dictionary, each entry states
    // its own file's role - the two sell-side fixtures `SELL`, the buy-side
    // one `BUYS` - and a later file of a dialect stating no role leaves the
    // stated one standing.
    let mut dictionary = FixRegistry::new();
    dictionary
        .add_cfb_file(&named_handle(CBLOCK, "buy.cfb"), Some("buy"))
        .expect("a readable CBlock");
    dictionary
        .add_cfb_file(&named_handle(SELLSIDE, "sell.cfb"), Some("sell"))
        .expect("a readable CBlock");
    dictionary
        .add_cfb_file(&named_handle(OVERLAY, "overlay.cfb"), None)
        .expect("a readable CBlock");
    dictionary
        .add_cfb_file(&named_handle(&one_tag(9001, "Bare"), "buy.cfb"), None)
        .expect("a readable CBlock");
    assert_eq!(
        dictionary
            .sources()
            .map(|source| (source.id(), source.file(), source.pluginside()))
            .collect::<Vec<_>>(),
        [
            ("buy", Some("buy.cfb"), PluginSide::BuySide),
            ("overlay", Some("overlay.cfb"), PluginSide::SellSide),
            ("sell", Some("sell.cfb"), PluginSide::SellSide),
        ]
    );
    // The role is the entry's, never a field's: the file's own fields carry
    // their membership and nothing else of the root.
    let avgpx = dictionary.field_by_tag(6).unwrap().as_fix();
    assert_eq!(avgpx.sources().collect::<Vec<_>>(), ["buy", "overlay"]);
    assert!(avgpx.codeset().is_none());
}
