//! One Ullink CBlock configuration, read into a dictionary and message roots.

use super::path;

use std::path::PathBuf;

use std::sync::Arc;
use yggdryl::holder::local::Folder;

use yggdryl::holder::Buffer;
use yggdryl::holder::fs::{File, FileSystem, MemoryFileSystem};
use yggdryl::{
    DataType, Error, Field, FixCategory, FixCodec, FixField, FixId, FixRegistry, IOBase,
};

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
        File::from_path(filesystem, format!("cblock/{name}"), None).expect("a path under it");
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
fn branches(field: &Field) -> Vec<&str> {
    field.as_fix().branches().collect()
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
    assert_eq!(registry.len(), 15 + super::crated());

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
    assert_eq!(branches(field), [DIALECT]);
    assert_eq!(branches(registry.field_by_tag(10001).unwrap()), [DIALECT]);
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
        (35, DataType::Utf8),
        (59, DataType::Utf8),
        (9, DataType::Int32),
        (6, DataType::Float32),
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
            "nolegs",
            "legs",
            "beginstring2"
        ],
    );

    // The duplicate keeps the tag, which is what recovers it.
    let fields = root.dtype().as_fields().unwrap();
    assert_eq!(fields[0].as_fix().tag().unwrap(), Some(8));
    assert_eq!(fields[7].as_fix().tag().unwrap(), Some(8));
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
fn a_nested_grammar_keeps_its_counter_and_names_its_group_separately() {
    let (registry, roots) = parse(CBLOCK);
    let fields = roots[0].dtype().as_fields().unwrap();
    let count = fields.iter().find(|held| held.name() == "nolegs").unwrap();
    assert_eq!(count.dtype(), &DataType::Int32);
    assert_eq!(count.as_fix().tag().unwrap(), Some(555));
    assert_eq!(
        registry.field_by_tag(555).unwrap().dtype(),
        &DataType::Int32
    );
    let group = fields.iter().find(|held| held.name() == "legs").unwrap();
    assert_eq!(group.as_fix().tag().unwrap(), None);
    assert_eq!(group.as_fix().counter().unwrap(), Some(555));
    let DataType::List(item) = group.dtype() else {
        panic!("a list, got {}", group.dtype());
    };
    assert_eq!(item.name(), "leg");
    assert!(!item.is_nullable());
    let members = item.dtype().as_fields().expect("an item struct");
    assert_eq!(
        members.iter().map(yggdryl::Field::name).collect::<Vec<_>>(),
        ["legcurrency", "nolegsecurityaltid", "legsecurityaltidgrp"]
    );
    assert!(!members[0].is_nullable(), "556 is required");
    assert_eq!(members[1].dtype(), &DataType::Int32);
    assert_eq!(members[1].as_fix().tag().unwrap(), Some(604));
    let DataType::List(inner) = members[2].dtype() else {
        panic!("a nested list, got {}", members[2].dtype());
    };
    assert_eq!(members[2].as_fix().tag().unwrap(), None);
    assert_eq!(members[2].as_fix().counter().unwrap(), Some(604));
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
        // The crate's own fields are every registry's and no file's.
        if field
            .as_fix()
            .tag()
            .unwrap()
            .is_some_and(yggdryl::is_crate_tag)
        {
            assert!(!field.as_fix().has_branch(DIALECT), "{}", field.name());
            continue;
        }
        assert!(field.as_fix().has_branch(DIALECT), "{}", field.name());
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
    assert!(!bare.field_by_tag(6).unwrap().as_fix().has_branch(DIALECT));
}

#[test]
fn replacing_a_referenced_cblock_field_is_atomic_and_unreferenced_fields_replace() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("config")
        .join("fix");
    let mut seeded = FixRegistry::from_handle(&Folder::new(root).unwrap()).unwrap();
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
        &DataType::Float32
    );
    // The same tag under another name is another field: it is registered
    // beside the holder under its own identity, the holder gains the name as
    // an alias, and the bare tag keeps answering the holder.
    let mut renamed = vocabulary.field_by_tag(6).unwrap().clone();
    renamed.set_name("somethingelse");
    let before = standalone.len();
    assert_eq!(standalone.insert(renamed).unwrap(), None);
    assert_eq!(standalone.len(), before + 1);
    assert_eq!(standalone.field_by_tag(6).unwrap().name(), "avgpx");
    assert!(
        standalone
            .field_by_tag(6)
            .unwrap()
            .as_fix()
            .aliases()
            .any(|alias| alias == "somethingelse")
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
        assert_eq!(
            occurrence
                .as_fix()
                .codes()
                .collect::<Result<Vec<_>, _>>()
                .unwrap(),
            canonical
                .as_fix()
                .codes()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        );
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
        let group = registry.definition(FixCategory::Groups, name).unwrap();
        let component = registry
            .definition(FixCategory::Components, component)
            .unwrap();
        assert_eq!(branches(group), ["venue"]);
        assert_eq!(branches(component), ["venue"]);
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
                .get_group_by_counter(counter)
                .is_some()
        );
        assert!(registry.get_group_by_counter(counter).is_some());
    }
    let DataType::List(item) = roots[0].get_field("vendorentries").unwrap().dtype() else {
        panic!("a list group");
    };
    assert_eq!(branches(item), ["venue"]);
    // A standard tag the file speaks is this dictionary's member too:
    // membership means "this dictionary speaks it", not "this dictionary
    // invented it".
    assert_eq!(branches(registry.field_by_tag(55).unwrap()), ["venue"]);
    assert_eq!(
        FixRegistry::from_json(&registry.into_json().unwrap()).unwrap(),
        registry
    );
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

    // A constraint naming a tag the vocabulary does not have dangles, so the
    // constraint goes and the message keeps its other children.
    let dangling = r#"<?xml version="1.0"?>
<cplugin-configuration fix-version="4.4">
	<vocabulary><vocabulary-tag name="35" alt="MsgType" type="string" /></vocabulary>
	<grammar-binding type="0"><grammar><tag-constraint name="35" part="body" /><tag-constraint name="99" part="body" /></grammar></grammar-binding>
</cplugin-configuration>"#;
    let (read, warnings) =
        super::warned::during(|| FixRegistry::from_cfb_file(&handle(dangling), None));
    let (_, roots) = read.expect("a readable CBlock");
    assert!(warnings[0].contains("99"), "{warnings:?}");
    assert_eq!(children(&roots[0]), ["msgtype"]);
}

#[test]
fn a_warning_quotes_the_element_and_the_content_it_read() {
    // Each case: the document, and every span the warning has to carry for a
    // reader to find the declaration in a file that is megabytes of them.
    for (body, wanted) in [
        (
            r#"<?xml version="1.0"?>
<cplugin-configuration fix-version="4.4">
	<vocabulary><vocabulary-tag name="35" alt="MsgType" type="decimal" /></vocabulary>
</cplugin-configuration>"#,
            // The eight are named, the ninth is quoted, and the element that
            // declared it is quoted whole.
            vec![
                "utc-time-only",
                "\"decimal\"",
                "<vocabulary-tag name=\\\"35\\\"",
            ],
        ),
        (
            r#"<?xml version="1.0"?>
<cplugin-configuration fix-version="4.4">
	<vocabulary><vocabulary-tag name="MsgType" type="string" /></vocabulary>
</cplugin-configuration>"#,
            vec!["a decimal tag", "\"MsgType\""],
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
        ),
    ] {
        let (read, warnings) =
            super::warned::during(|| FixRegistry::from_cfb_file(&handle(body), None));
        read.expect("a readable CBlock");
        let rendered = warnings.first().expect("one warning").clone();
        for held in wanted {
            assert!(rendered.contains(held), "{held} missing from {rendered}");
        }
        // Bounded: a warning never grows with the document it read.
        assert!(rendered.len() < 400, "{rendered}");
        // Both doors read the same documents, and warn with the same sentence.
        let (also, spelled) =
            super::warned::during(|| FixField::from_cfb_file(&handle(body), Some("bloomberg")));
        also.expect("a readable CBlock");
        assert_eq!(spelled.first().map(String::as_str), Some(rendered.as_str()));
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
    let also = FixField::from_cfb_file(&handle(malformed), Some("bloomberg")).unwrap_err();
    assert_eq!(also.to_string(), rendered);

    // An element longer than the budget is quoted up to it and elided, so a
    // vocabulary tag carrying a paragraph of attributes still names itself.
    let wide = format!(
        r#"<?xml version="1.0"?>
<cplugin-configuration fix-version="4.4">
	<vocabulary><vocabulary-tag name="35" alt="MsgType" type="decimal" note="{}" /></vocabulary>
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
    // its own identity beside the first, which keeps the bare tag and gains
    // the second name as an alias. Nothing is dropped, so nothing is warned.
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
    assert_eq!(registry.len(), 2 + super::crated());
    // The first declaration is the one the bare tag answers.
    let holder = registry.field_by_tag(35).unwrap();
    assert_eq!(holder.name(), "msgtype");
    assert_eq!(
        holder.as_fix().aliases().collect::<Vec<_>>(),
        ["somethingelse"]
    );
    let second = registry
        .field_by_id(FixId::of(35, "SomethingElse").unwrap())
        .unwrap();
    assert_eq!(second.name(), "somethingelse");
    assert_eq!(second.as_fix().tag().unwrap(), Some(35));
    assert_eq!(
        registry.get_field_by_name("somethingelse").map(Field::name),
        Some("somethingelse"),
        "a canonical name answers before an alias",
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

    // The vocabulary door reads the same file the same way.
    let fields = FixField::from_cfb_file(&handle(&body), Some("bloomberg")).unwrap();
    assert_eq!(
        fields[0].as_fix().description(),
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
    assert_eq!((registry.len(), roots.len()), (2 + super::crated(), 1));

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
    assert_eq!((registry.len(), roots.len()), (2 + super::crated(), 0));
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
    // The guard stops the reader descending; it does not throw the file away.
    // This message goes with it, because thirty grammars over one tag are
    // thirty definitions of one name and the catalog holds one - which is its
    // own warning, named as one.
    assert!(roots.is_empty());
    assert!(
        warnings
            .iter()
            .any(|held| held.contains("one CBlock definition per context")),
        "{warnings:?}"
    );
    // The vocabulary the file declared is still a dictionary.
    assert_eq!(registry.field_by_tag(555).unwrap().name(), "nolegs");
}

#[test]
fn the_message_types_a_file_declares_become_the_code_set_of_tag_35() {
    let (registry, _) = parse(CBLOCK);
    let msgtype = registry.field_by_tag(35).expect("MsgType");
    let view = msgtype.as_fix();

    // A CBlock spells a type as the wire value and a qualifier, and the wire
    // value is what tag 35 carries: `P Report Ack` is `P` used as a report
    // ack, under the wording the listing gave it.
    assert_eq!(view.code_value("P Report Ack"), Some("P"));
    assert_eq!(view.code_name("P"), Some("P Report Ack"));
    assert_eq!(
        view.code_by_name("P Report Ack")
            .and_then(|code| code.parse_doc().ok().flatten()),
        Some("Allocation Report ACK".to_owned()),
    );

    // The qualifier is the direction as often as a role, and both directions
    // are one type on the wire: one code, answering to both spellings.
    assert_eq!(view.code_value("AR Inbound"), Some("AR"));
    assert_eq!(view.code_value("AR Outbound"), Some("AR"));
    assert_eq!(view.code_name("AR"), Some("AR Inbound"));
    assert_eq!(
        view.codes().count(),
        4,
        "AR was declared twice and is one code"
    );

    // Two roles of one wire type are one code too: `c SDR` and `c SLR` are
    // both tag 35 `c`, and a code set keys on the wire.
    assert_eq!(view.code_value("c SLR"), Some("c"));

    // The mapping table spells the same type the way UlMessage does, so that
    // spelling reaches the value rather than declaring a second type.
    assert_eq!(view.code_value("allocationreportack"), Some("P"));

    // A bound type the listing never mentioned is still a type this dialect
    // carries: the binding declares `7`.
    assert_eq!(view.code_value("7"), Some("7"));

    // The vocabulary door reads the same file the same way.
    let fields = FixField::from_cfb_file(&handle(CBLOCK), Some("bloomberg")).unwrap();
    let held = fields
        .iter()
        .find(|field| field.as_fix().tag().ok() == Some(Some(35)))
        .expect("MsgType");
    assert_eq!(held.as_fix().code_value("AR Outbound"), Some("AR"));
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
    assert_eq!(registry.len(), 1 + super::crated());
    assert!(registry.get_field_by_tag(35).is_none());
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
    assert_eq!(registry.get_field_by_name("notafield"), None);
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

    // Every field is keyed, so it enters a dictionary as it stands, and every
    // one is a member of the dialect - a standard tag the file speaks as
    // much as a venue's own.
    let avgpx = &fields[3];
    assert_eq!(avgpx.as_fix().tag().unwrap(), Some(6));
    assert_eq!(branches(avgpx), [DIALECT]);
    assert_eq!(branches(&fields[4]), [DIALECT]);
    assert!(
        fields
            .iter()
            .all(|field| field.as_fix().has_branch(DIALECT))
    );

    // The maps sit past the grammar bindings, so a code set proves the whole
    // document was read and not just its first pass.
    assert_eq!(fields[14].as_fix().code_value("buy"), Some("B"));

    // The roots are what a registry holds instead.
    let (registry, roots) = FixRegistry::from_cfb_file(&handle(CBLOCK), Some(DIALECT)).unwrap();
    assert_eq!(registry.len(), fields.len() + super::crated());
    assert_eq!(roots.len(), 1);
    assert_eq!(registry.dialects(), [DIALECT]);
}

#[test]
fn an_unnamed_file_takes_its_dialect_from_its_own_stem() {
    // A CBlock never names itself, so the file standing in for the caller is
    // the stem and nothing else of the path, folded by case as every
    // membership is.
    let fields = FixField::from_cfb_file(&named_handle(CBLOCK, "MSFIX44.cfb"), None)
        .expect("a readable CBlock");
    assert_eq!(branches(&fields[4]), ["msfix44"]);
    assert_eq!(branches(&fields[3]), ["msfix44"]);

    // An explicit name still wins over the stem.
    let fields = FixField::from_cfb_file(&named_handle(CBLOCK, "MSFIX44.cfb"), Some(DIALECT))
        .expect("a readable CBlock");
    assert_eq!(branches(&fields[4]), [DIALECT]);

    // Bytes held in memory are named by the caller: a buffer's URL is an
    // identity and not a location, and a name the caller states is the
    // dialect whatever the handle answers.
    let mut buffer = Buffer::new();
    buffer.write_all_bytes(CBLOCK.as_bytes()).unwrap();
    let fields = FixField::from_cfb_file(&buffer, Some(DIALECT)).expect("a readable CBlock");
    assert_eq!(branches(&fields[4]), [DIALECT]);
    assert!(
        fields
            .iter()
            .all(|field| field.as_fix().has_branch(DIALECT))
    );
}

#[test]
fn a_stem_that_cannot_be_a_membership_is_refused_rather_than_folded_into_one() {
    // A supplied membership is held to the alias grammar - non-empty, and
    // free of the comma the stored list is rendered with - and nothing else:
    // a leading digit or a long name is a name. A stem stands in for a name
    // the caller did not supply, so it is taken only where it reads as one,
    // opening with a letter: `4.4-ms.cfb` names nothing on its own and
    // stamps nothing, while the same spelling supplied is a membership. A
    // dictionary keyed on a guess is worse than a refusal, so the two that
    // cannot be one are refused rather than repaired, and by every door
    // before a byte of the file is read.
    let fields = FixField::from_cfb_file(&named_handle(CBLOCK, "4.4-ms.cfb"), None)
        .expect("a stem that is not a name stands in for nothing");
    assert!(branches(&fields[4]).is_empty());
    let fields = FixField::from_cfb_file(&named_handle(CBLOCK, "4.4-ms.cfb"), Some("4.4-ms"))
        .expect("a supplied name beginning with a digit is a name");
    assert_eq!(branches(&fields[4]), ["4.4-ms"]);
    let fields = FixField::from_cfb_file(
        &named_handle(CBLOCK, "a-name-well-past-the-old-inline-cap.cfb"),
        None,
    )
    .expect("a long stem is a name");
    assert_eq!(
        branches(&fields[4]),
        ["a-name-well-past-the-old-inline-cap"]
    );
    // A buffer is identified by an address, which is a stem and not a name.
    let fields = FixField::from_cfb_file(&Buffer::from_bytes(CBLOCK.as_bytes().to_vec()), None)
        .expect("bytes held in memory are named by the caller or not at all");
    assert!(branches(&fields[4]).is_empty());

    let refused = |error: &Error| matches!(error, Error::InvalidMetadataValue { key, .. } if key == "fix:branches");
    let error = FixField::from_cfb_file(&handle(CBLOCK), Some("ms,bloomberg")).unwrap_err();
    assert!(refused(&error), "{error}");
    assert!(error.to_string().contains("','"), "{error}");
    assert!(error.to_string().contains("\"ms,bloomberg\""), "{error}");
    let error = FixField::from_cfb_file(&handle(CBLOCK), Some("")).unwrap_err();
    assert!(refused(&error), "{error}");
    assert!(error.to_string().contains("non-empty"), "{error}");
    let error = FixRegistry::from_cfb_file(&handle(CBLOCK), Some("ms,bloomberg")).unwrap_err();
    assert!(refused(&error), "{error}");
    let error = FixRegistry::from_cfb_file(&handle(CBLOCK), Some("")).unwrap_err();
    assert!(refused(&error), "{error}");
    let mut dictionary = FixRegistry::new();
    let error = dictionary
        .add_cfb_file(&handle(CBLOCK), Some("ms,bloomberg"))
        .unwrap_err();
    assert!(refused(&error), "{error}");
    assert_eq!(
        dictionary.len(),
        super::crated(),
        "a refused read writes nothing"
    );
    assert!(dictionary.dialects().is_empty());
}

#[test]
fn a_cblock_vocabulary_folds_into_a_dictionary_that_already_exists() {
    // Two counterparties' files meet in the one namespace a dictionary is:
    // a tag both declare under one name is one field, and each file's name
    // is recorded on it - which is the case the fold exists for.
    let mut dictionary =
        FixRegistry::from_fields(FixField::from_cfb_file(&handle(CBLOCK), Some(DIALECT)).unwrap())
            .expect("one file's vocabulary");
    let before = dictionary.len();

    let (added, merged) = dictionary
        .add_fields(FixField::from_cfb_file(&handle(OVERLAY), Some("morgan")).unwrap())
        .expect("the second file folds into the first");
    assert_eq!((added, merged), (1, 1));
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
    assert_eq!(branches(avgpx), [DIALECT, "morgan"]);
    // And a tag only the second file declares arrives, as its member alone.
    let price = dictionary.field_by_name("price").unwrap();
    assert_eq!(price.name(), "price");
    assert_eq!(branches(price), ["morgan"]);
    assert_eq!(dictionary.dialects(), [DIALECT, "morgan"]);

    // The same vocabulary read again under a second dialect is the same
    // fields: nothing is added, every one merges, and every one now names
    // both dictionaries - the user range included, because a tag is what
    // identifies a field on the wire and no dictionary owns a range of them.
    let (added, merged) = dictionary
        .add_fields(FixField::from_cfb_file(&named_handle(CBLOCK, "morgan.cfb"), None).unwrap())
        .expect("the same vocabulary under a second dialect");
    assert_eq!(
        (added, merged),
        (0, 15),
        "one vocabulary, whichever file spoke it"
    );
    assert_eq!(
        branches(dictionary.field_by_tag(10001).unwrap()),
        [DIALECT, "morgan"]
    );
    assert_eq!(dictionary.dialects(), [DIALECT, "morgan"]);
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

    // The imported AvgPx datatype disagrees with its committed physical width.
    let fields = FixField::from_cfb_file(&handle(CBLOCK), Some("bloomberg")).unwrap();
    let error = seeded.add_fields(fields.clone()).unwrap_err();
    assert!(matches!(error, Error::InvalidRecord { .. }), "{error}");
    assert!(error.to_string().contains("float32"), "{error}");
    assert_eq!(
        seeded, before,
        "a conflicting scalar datatype does not mutate the catalog"
    );
    let avgpx = fields
        .into_iter()
        .find(|field| field.as_fix().tag().unwrap() == Some(6))
        .unwrap();
    let error = seeded.add_fields([avgpx]).unwrap_err();
    assert!(matches!(error, Error::InvalidRecord { .. }), "{error}");
    let message = error.to_string();
    assert!(
        message.contains("avgpx") && message.contains("float32"),
        "{message}"
    );
    assert_eq!(seeded.field_by_tag(35).unwrap().dtype(), &DataType::Utf8);
    assert_eq!(seeded.field_by_tag(6).unwrap().dtype(), &DataType::Float64);

    assert_eq!(seeded, before, "a refused fold writes nothing");
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

    // The vocabulary door reads the same file the same way.
    let fields =
        FixField::from_cfb_file(&handle(clashing), Some("bloomberg")).expect("a readable CBlock");
    let named: Vec<&str> = fields.iter().map(Field::name).collect();
    assert_eq!(named, ["44", "3044"]);

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
            registry.field_by_tag(tag).unwrap().as_fix().codes().count(),
            0,
            "tag {tag} keeps no code set from an ambiguous map",
        );
    }
    assert_eq!(
        registry.field_by_tag(4).unwrap().as_fix().codes().count(),
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
    // the bare tag and gains the second's name as an alias; the second is
    // reached by its name or its identity.
    let doubled = r#"<?xml version="1.0"?>
<cplugin-configuration fix-version="4.4">
	<vocabulary>
		<vocabulary-tag name="44" alt="Price" type="float" />
		<vocabulary-tag name="44" alt="LastPx" type="float" />
	</vocabulary>
</cplugin-configuration>"#;
    let (read, warnings) =
        super::warned::during(|| FixField::from_cfb_file(&handle(doubled), None));
    // The vocabulary door answers declaration order.
    let fields = read.expect("a readable CBlock");
    assert_eq!(fields.len(), 2);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(
        fields.iter().map(Field::name).collect::<Vec<_>>(),
        ["price", "lastpx"]
    );
    let (registry, _) = FixRegistry::from_cfb_file(&handle(doubled), None).unwrap();
    assert_eq!(registry.len(), 2 + super::crated());
    let holder = registry.field_by_tag(44).unwrap();
    assert_eq!(holder.name(), "price");
    assert_eq!(holder.as_fix().aliases().collect::<Vec<_>>(), ["lastpx"]);
    let lastpx = FixId::of(44, "LastPx").unwrap();
    assert_eq!(registry.field_by_id(lastpx).unwrap().name(), "lastpx");
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
    assert_eq!(held, [44, 44], "tag-major, the pair adjacent");
    let price = FixId::of(44, "Price").unwrap();
    assert_eq!(
        registry.next_field_after(None).map(|field| field.name()),
        Some(if price.digest() < lastpx.digest() {
            "price"
        } else {
            "lastpx"
        })
    );

    // And the one difference that is not a loss: a dictionary keeps one entry
    // per identity, so a tag declared twice identically arrives twice here.
    let repeated = r#"<?xml version="1.0"?>
<cplugin-configuration fix-version="4.4">
	<vocabulary>
		<vocabulary-tag name="44" alt="Price" type="float" />
		<vocabulary-tag name="44" alt="Price" type="float" />
	</vocabulary>
</cplugin-configuration>"#;
    let (read, warnings) =
        super::warned::during(|| FixField::from_cfb_file(&handle(repeated), None));
    let fields = read.expect("a readable CBlock");
    assert_eq!(fields.len(), 2);
    assert!(warnings.is_empty(), "{warnings:?}");
    let (registry, _) = FixRegistry::from_cfb_file(&handle(repeated), None).unwrap();
    assert_eq!(registry.len(), 1 + super::crated());
}

#[test]
fn a_cblock_reads_in_whole_with_its_dialect_and_the_file_it_arrived_as() {
    let mut dictionary = FixRegistry::new();
    let (added, merged) = dictionary
        .add_cfb_file(&named_handle(CBLOCK, "MSFIX44.cfb"), Some("morgan"))
        .expect("a readable CBlock");
    // Fifteen of the file's added, and nothing merged: the parsed dictionary
    // holds the crate's own fields as every registry does, and a fold never
    // counts them.
    assert_eq!((added, merged), (15, 0));
    assert_eq!(dictionary.len(), 15 + super::crated());

    // The dialect the caller named is what every field the file produced is
    // a member of - the name wins over the stem - and the version the root
    // declared is read past: which version a run reads at is the codec's pin.
    assert_eq!(dictionary.dialects(), ["morgan"]);
    for field in dictionary.iter() {
        if field
            .as_fix()
            .tag()
            .unwrap()
            .is_some_and(yggdryl::is_crate_tag)
        {
            continue;
        }
        assert_eq!(branches(field), ["morgan"], "{}", field.name());
    }
    assert_eq!(
        branches(dictionary.msgtype("7").unwrap().as_field()),
        ["morgan"],
        "a message the file bound is a member too",
    );

    // Reading a second file is not a statement that the first one's names were
    // wrong: a field both speak is one field naming both dictionaries, and
    // with no dialect named the stem is the name.
    let (added, merged) = dictionary
        .add_cfb_file(&named_handle(SELLSIDE, "morgan-2024.cfb"), None)
        .expect("the same dialect, read again");
    assert_eq!(
        (added, merged),
        (0, 1),
        "SELLSIDE declares only tag 35; the crate's own fields are never folded"
    );
    assert_eq!(
        branches(dictionary.field_by_tag(35).unwrap()),
        ["morgan", "morgan-2024"]
    );
    assert_eq!(branches(dictionary.field_by_tag(6).unwrap()), ["morgan"]);
    assert_eq!(dictionary.dialects(), ["morgan", "morgan-2024"]);

    // With no dialect named, the stem is the name, folded by case.
    let mut standalone = FixRegistry::new();
    standalone
        .add_cfb_file(&handle(CBLOCK), None)
        .expect("a readable CBlock");
    assert_eq!(standalone.dialects(), ["one"]);
    assert_eq!(branches(standalone.field_by_tag(6).unwrap()), ["one"]);
}

#[test]
fn a_cblock_merged_under_a_dialect_stamps_what_it_touched_and_unions_onto_the_standard_field() {
    // The shipped dictionary declares no membership: what the specification
    // alone defines is nobody's dialect. A CBlock folded into it under a name
    // stamps that name on every field, group, component and message it
    // produced, and a standard field the file speaks gains the membership
    // without losing what the specification said about it.
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("config")
        .join("fix");
    let mut seeded =
        FixRegistry::from_handle(&Folder::new(root).expect("the seed folder")).expect("the seed");
    assert!(seeded.dialects().is_empty());
    let symbol = seeded.field_by_tag(55).unwrap().clone();
    assert!(symbol.description().is_some());
    assert!(branches(&symbol).is_empty());
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
    let (added, merged) = seeded
        .add_cfb_file(&named_handle(body, "VENUE.cfb"), None)
        .expect("a compatible vocabulary folds");
    assert_eq!(
        (added, merged),
        (2, 1),
        "two venue tags added, Symbol merged"
    );
    assert_eq!(seeded.dialects(), ["venue"]);

    // The standard field: the same tag under the same folded name, so the
    // membership unions onto it and the specification's words survive.
    let merged = seeded.field_by_tag(55).unwrap();
    assert_eq!(merged.name(), symbol.name());
    assert_eq!(merged.description(), symbol.description());
    assert_eq!(merged.dtype(), symbol.dtype());
    assert_eq!(branches(merged), ["venue"]);
    assert_eq!(
        seeded.get_field_by_name("Symbol").map(Field::name),
        Some(symbol.name())
    );

    // The venue's own tags, the group, its component and the message it
    // bound: every one a member.
    for tag in [5000, 5001] {
        assert_eq!(
            branches(seeded.field_by_tag(tag).unwrap()),
            ["venue"],
            "tag {tag}"
        );
    }
    assert_eq!(
        branches(
            seeded
                .definition(FixCategory::Groups, "VendorEntries")
                .unwrap()
        ),
        ["venue"]
    );
    assert_eq!(
        branches(
            seeded
                .definition(FixCategory::Components, "VendorEntry")
                .unwrap()
        ),
        ["venue"]
    );
    assert!(seeded.get_group_by_counter(5000).is_some());
    // A field no file touched states nothing still.
    assert!(branches(seeded.field_by_tag(35).unwrap()).is_empty());

    // Message codes live in one namespace under the same rule as fields: the
    // file bound `D` under a name of its own, so it is a second message whose
    // bare code keeps answering the first holder, and the newcomer is reached
    // by its name and carries the membership.
    assert_eq!(seeded.msgtype("D").unwrap().name(), first_holder);
    let bound = seeded.msgtype("message44").expect("the file's own message");
    assert_eq!(bound.as_field().as_fix().msgtype(), Some("D"));
    assert_eq!(branches(bound.as_field()), ["venue"]);
    assert!(branches(seeded.msgtype("D").unwrap().as_field()).is_empty());

    // The same file under a second name unions, and the list is sorted and
    // folded whichever order the dictionaries arrived in.
    let (added, merged) = seeded
        .add_cfb_file(&handle(body), Some("Other"))
        .expect("the same vocabulary again");
    assert_eq!((added, merged), (0, 3));
    assert_eq!(
        branches(seeded.field_by_tag(55).unwrap()),
        ["other", "venue"]
    );
    assert_eq!(
        branches(seeded.field_by_tag(5001).unwrap()),
        ["other", "venue"]
    );
    assert_eq!(
        branches(seeded.msgtype("message44").unwrap().as_field()),
        ["other", "venue"],
        "a definition re-declared under the same name folds into the stored one",
    );
    assert_eq!(seeded.dialects(), ["other", "venue"]);
}

#[test]
fn reading_a_cblock_in_whole_is_one_mutation() {
    // A changed scalar width refuses the whole imported document.
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("config")
        .join("fix");
    let mut seeded =
        FixRegistry::from_handle(&Folder::new(root).expect("the seed folder")).expect("the seed");
    let before = seeded.clone();

    let error = seeded
        .add_cfb_file(&handle(CBLOCK), Some(DIALECT))
        .unwrap_err();
    assert!(matches!(error, Error::InvalidRecord { .. }), "{error}");
    assert!(error.to_string().contains("float32"), "{error}");
    assert_eq!(seeded, before, "neither a membership nor a field arrived");
    assert!(seeded.dialects().is_empty());
}

#[test]
fn a_normalization_spells_a_tag_and_the_vocabulary_keeps_its_name() {
    let (registry, _) = parse(NORMALIZED);

    // A tag its `vocabulary-tag` gave no `alt` is named by its own decimal
    // tag, and the normalization is the only place the file says what it is
    // called - which is the whole of what reading the binding is worth.
    let held = registry.field_by_tag(22830).expect("the unnamed tag");
    assert_eq!(held.name(), "22830", "the vocabulary keeps the name");
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
        held.as_fix().aliases().collect::<Vec<_>>(),
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
    assert_eq!(held.as_fix().aliases().collect::<Vec<_>>(), [] as [&str; 0]);
    assert!(registry.get_field_by_name("LEGSECURITYID").is_some());

    // The same, spelled with the file's own casing inside a nested group.
    let alt = registry.field_by_tag(605).expect("LegSecurityAltID");
    assert_eq!(alt.as_fix().aliases().collect::<Vec<_>>(), [] as [&str; 0]);
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
            .aliases()
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
    // 608 is FIX's own, so the spelling lands in the standard branch with it.
    assert_eq!(
        registry.get_field_by_name("LEGCFICODE").map(Field::name),
        Some("608"),
    );

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
            .aliases()
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
            .aliases()
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
            .aliases()
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
    let fields =
        FixField::from_cfb_file(&handle(NORMALIZED), Some("bloomberg")).expect("a readable CBlock");
    let held = fields
        .iter()
        .find(|field| field.name() == "22830")
        .expect("the unnamed tag");
    assert_eq!(
        held.as_fix().aliases().collect::<Vec<_>>(),
        ["EXCLUDEDDEALERS", "EXCLUDED_DEALERS"],
        "the vocabulary door carries them too",
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
    let DataType::List(item) = group.dtype() else {
        panic!("a list, got {}", group.dtype());
    };
    let members: Vec<&str> = item.fields().iter().map(Field::name).collect();
    assert!(members.contains(&"hedgecurrency"), "{members:?}");

    // And a bridge row of that message type reaches both tags: the flat key
    // through the message's own children, the packed one through the members
    // of the group it arrived in.
    let reader = FixCodec::new(Arc::new(registry));
    let row: &[u8] = b"MSGTYPE=tradecapturereport|HEDGECURRENCY=USD|TR_FIXINGCENTER=LN\
|NOHEDGEGROUPS=1|NOHEDGEGROUPS[0]=HEDGESETTLDATE=20260818\x04\x03HEDGECURRENCY=XAU\x04\x03";
    let message = <FixCodec as super::OneMessage>::one_line(&reader, row, false)
        .expect("the bridge row builds");
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
    let occurrence = held.children();
    assert_eq!(occurrence.len(), 1);
    assert_eq!(
        occurrence[0].key().as_str(),
        Some("NOHEDGEGROUPS[0]"),
        "the pair the bridge wrote",
    );
    assert_eq!(
        occurrence[0].value().as_str(),
        Some("HEDGESETTLDATE=20260818\u{4}\u{3}HEDGECURRENCY=XAU\u{4}\u{3}"),
    );
    assert_eq!(
        message
            .by_path(&path("hedgegroups[0].hedgecurrency"))
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
    let reader = FixCodec::new(Arc::new(registry));
    let message = <FixCodec as super::OneMessage>::one_line(&reader, logged.as_bytes(), false)
        .expect("the captured frame builds");

    // The frame's own type stays the frame's.
    assert_eq!(message.by_tag(35).unwrap().as_str(), Some("UL"));

    // The hedge the payload packs is this dialect's group, typed by it: the
    // currency is the group's own tag and not the one the top of the message
    // binds under the same spelling.
    let hedge = message
        .as_field()
        .get_field_by_path("hedgegroups")
        .expect("the hedge group");
    let DataType::List(item) = hedge.dtype() else {
        panic!("a list, got {}", hedge.dtype());
    };
    let currency = item.fields().first().expect("its first member");
    assert_eq!(currency.name(), "hedgecurrency");
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
        &DataType::Float32
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
    let codes = field.as_fix();

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
            .field_by_tag(35)
            .unwrap()
            .as_fix()
            .code_value("AE Inbound"),
        Some("AE")
    );
}
