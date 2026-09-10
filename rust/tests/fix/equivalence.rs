//! What the codec answers today, over every capture this branch holds.
//!
//! The FIX layer is about to be rewritten onto the text reader that landed in
//! `main`: one pair scanner instead of two, entries over ranges of a page
//! instead of owned copies, a line entry point instead of a record one. Each
//! of those steps is meant to move *how* a message is read and nothing about
//! *what* it reads, and the only way to say that with a straight face is to
//! write down the answer first and keep it byte for byte afterwards.
//!
//! So this is a golden file and nothing else. It asserts no meaning - every
//! other suite in this directory does that, and each of them says what its
//! own rule is for. What this holds is the whole answer, per message, in the
//! four forms that between them catch every way the reading could move:
//!
//! - the **digest**, which walks the entries in arrival order, length-prefixes
//!   every value and separates a parent of two from two flat siblings by its
//!   child count. A value cut one byte short, a pair that appeared or went, a
//!   tree that flattened: all of them move it, and nothing else does.
//! - the **entries**, keyed by their position in the tree, so a difference
//!   names the entry rather than the message. The digest says *that* the
//!   arrival record moved; these say where.
//! - the **row** under the fixed schema, every column the message filled. The
//!   entries are what arrived; the row is what the dictionary made of it, and
//!   the two move independently.
//! - the **wire** [`FixMsg::into_bytes`] re-emits, which is the round trip.
//!
//! Around those four sit the facts that say *which* reading answered at all:
//! the line that went in, the message type the frame named, how many messages
//! one line yielded, and the refusal where there was one - because a line that
//! stops being a message, or starts being two, is a change nobody would see in
//! a digest that is no longer taken.
//!
//! The two entries columns are left out of the row on purpose: they are the
//! entries again, in the shape a row materializes them, and pinning one fact
//! twice would mean a deliberate change to the entry column has to be
//! re-blessed in two places. The entries above are the owner.
//!
//! The file is one `key<TAB>value` a line. Values are rendered by the crate's
//! own canonical spellings - [`into_json_scalar`] for a column, the wire bytes
//! for a frame - with every byte outside printable ASCII escaped, so a control
//! separator survives a diff and a terminal instead of being guessed at.
//!
//! The corpus is this file's own, drawn from the lines the codec, batch,
//! capture, pipeline, lift, enrich and latest suites are written against, plus
//! `ulbridge.log` whole. Those suites keep asserting what their lines *mean*;
//! copying the bytes here is deliberate, so that editing one of them cannot
//! quietly shrink what equivalence covers.
//!
//! Regenerating is deliberate and never automatic:
//!
//! ```bash
//! YGGDRYL_FIX_EQUIVALENCE_WRITE=1 cargo test --locked -p yggdryl --test fix equivalence
//! ```
//!
//! A regeneration is a claim that the reading changed on purpose, so the diff
//! belongs in the commit that changed it, beside the decision that says why.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::PathBuf;

use yggdryl::fix::{ENTRIES_COLUMN, UNMAPPED_COLUMN};
use yggdryl::holder::Buffer;
use yggdryl::media::RecordOptions;
use yggdryl::media::text::TextOptions;
use yggdryl::{
    Field, FixBranch, FixCodec, FixEntry, FixMsg, IOMedia, Scalar, Timezone, Url, fix_schema,
    into_json_scalar,
};

/// The environment variable that turns the comparison into a write.
const WRITE: &str = "YGGDRYL_FIX_EQUIVALENCE_WRITE";

/// How many differences a failure spells out before it counts the rest.
const REPORTED: usize = 24;

/// The capture, exactly as the bridge wrote it - the same bytes the dataset
/// suite reads, and the only fixture here that is a file rather than a line.
const LOG: &[u8] = include_bytes!("ulbridge.log");

/// Where the committed answer lives.
fn snapshot_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fix")
        .join("equivalence.snapshot")
}

/// One byte string as one snapshot line: printable ASCII as itself, and
/// everything else spelled, so a control separator or a UTF-8 byte survives a
/// diff and a terminal without either being guessed at.
fn escaped(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    for &byte in bytes {
        match byte {
            b'\\' => out.push_str("\\\\"),
            b'\t' => out.push_str("\\t"),
            b'\n' => out.push_str("\\n"),
            b'\r' => out.push_str("\\r"),
            0x20..=0x7e => out.push(char::from(byte)),
            _ => {
                let _ = write!(out, "\\x{byte:02x}");
            }
        }
    }
    out
}

/// Everything one reading answered, in the order it was read.
#[derive(Default)]
struct Pinned {
    records: Vec<(String, String)>,
}

impl Pinned {
    fn push(&mut self, key: String, value: String) {
        self.records.push((key, value));
    }

    /// Every pair one level of entries carries, keyed by its place in the
    /// tree: `1.0` is the first child of the second entry, so a child that
    /// moved up a level is a key that went and a key that appeared rather
    /// than a value that changed under a stable name.
    fn entries(&mut self, at: &str, path: &str, entries: &[FixEntry]) {
        for (index, entry) in entries.iter().enumerate() {
            let here = if path.is_empty() {
                index.to_string()
            } else {
                format!("{path}.{index}")
            };
            self.push(
                format!("{at}.entry[{here}]"),
                format!(
                    "t{} {}={}",
                    entry.tag(),
                    escaped(entry.key().as_bytes()),
                    escaped(entry.value().as_bytes())
                ),
            );
            self.entries(at, &here, entry.children());
        }
    }

    /// One message, whole: what it is, what arrived, what the dictionary made
    /// of it, and what it re-emits.
    fn message(&mut self, at: &str, message: &FixMsg, schema: &Field) {
        self.push(format!("{at}.type"), message.as_field().name().to_owned());
        self.push(format!("{at}.digest"), format!("{:032x}", message.digest()));
        self.push(format!("{at}.wire"), escaped(&message.into_bytes(b'|')));
        self.entries(at, "", message.entries());
        let row = message.into_row(schema).expect("a message fills its row");
        let values = row.as_sequence().expect("a row is a sequence");
        for (column, value) in schema.fields().iter().zip(values) {
            let name = column.name();
            if name == ENTRIES_COLUMN || name == UNMAPPED_COLUMN || value.is_null() {
                continue;
            }
            self.push(
                format!("{at}.field.{name}"),
                escaped(
                    into_json_scalar(value)
                        .expect("a column value renders")
                        .as_bytes(),
                ),
            );
        }
    }

    /// One line read through the door every caller uses, filled where the
    /// group it belongs to is a filled one.
    ///
    /// A line the codec refuses is pinned too: a refusal that turns into a
    /// message, or a message that turns into a refusal, is exactly the kind
    /// of movement this file exists to catch.
    fn line(&mut self, at: &str, codec: &FixCodec, schema: &Field, line: &[u8], enrich: bool) {
        self.push(format!("{at}.line"), escaped(line));
        let held = match codec.parse_line(line) {
            Ok(held) => held,
            Err(refused) => {
                self.push(format!("{at}.refused"), refused.to_string());
                return;
            }
        };
        let mut answered = 0;
        for (index, message) in held.enumerate() {
            answered += 1;
            let at = format!("{at}:{index}");
            match message.and_then(|message| {
                if enrich {
                    codec.enrich_message(message)
                } else {
                    Ok(message)
                }
            }) {
                Ok(message) => self.message(&at, &message, schema),
                Err(refused) => self.push(format!("{at}.refused"), refused.to_string()),
            }
        }
        self.push(format!("{at}.messages"), answered.to_string());
    }

    /// Every line of one corpus, under one codec.
    fn lines(&mut self, group: &str, codec: &FixCodec, lines: &[Vec<u8>], enrich: bool) {
        let schema = fix_schema(codec.registry(), "fix").expect("the fixed schema");
        for (index, line) in lines.iter().enumerate() {
            self.line(
                &format!("{group}[{index:03}]"),
                codec,
                &schema,
                line,
                enrich,
            );
        }
    }

    /// The snapshot as the committed file spells it.
    fn rendered(&self) -> String {
        let mut out = String::new();
        out.push_str(
            "# What the FIX codec answers over this branch's captures. Generated; do not edit.\n\
             # Regenerate deliberately, and only beside the decision that changed the reading:\n\
             #   YGGDRYL_FIX_EQUIVALENCE_WRITE=1 cargo test --locked -p yggdryl --test fix equivalence\n",
        );
        for (key, value) in &self.records {
            let _ = writeln!(out, "{key}\t{value}");
        }
        out
    }
}

/// The committed answer, keyed the way the reading is.
fn committed(text: &str) -> BTreeMap<&str, &str> {
    text.lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| {
            line.split_once('\t')
                .unwrap_or_else(|| panic!("a snapshot line is a key and a value: {line}"))
        })
        .collect()
}

/// The committed dictionary, and the codec every generic frame is read under.
fn committed_codec() -> FixCodec {
    FixCodec::new(super::committed_registry())
}

/// The bridge's own dialect, pinned for the whole run, exactly as the dataset
/// and pipeline suites pin it.
fn bridge_codec() -> FixCodec {
    FixCodec::new(super::ulbridge_registry())
        .with_branch(&FixBranch::from_str(yggdryl::ULBRIDGE_BRANCH).expect("the bridge's branch"))
}

fn owned(lines: &[&str]) -> Vec<Vec<u8>> {
    lines.iter().map(|line| line.as_bytes().to_vec()).collect()
}

/// Every shape a real capture holds - the catalogue the batch and codec
/// suites are written against, prose and framing and marks alike.
fn shapes() -> Vec<Vec<u8>> {
    owned(&[
        "sending >> 8=FIX.4.2|9=176|35=D|11=ORDER-1|55=AAPL|54=1|10=203| << queued seq=1092",
        "raw 8=FIX.4.4|9=224|35=8|17=E1|37=O9|31=12.75|32=50|10=118|",
        "8=FIX.4.4|35=8|58=quoting #A=1 and #B=2|10=1|",
        "sending >> 8=FIX.4.2|35=UL|#SYMBOL=TTF|#SIDE=1|10=044|",
        "8=FIX.4.4|35=D|11=ORDER-1|SYMBOL=AAPL|SIDE=1|10=000",
        "toBridge #ISINCODE=XX|#SYMBOL=TTF|#SIDE=1",
        "ACCOUNT=A1|MSGTYPE=D|CLORDID=ORDER-1|SYMBOL=AAPL|SIDE=1",
        "After Enrichment -> ACCOUNT=ACCT-000117 CLIENTID=MCFP2 VENUE=XPAR",
        "Referential(dbi|equity|dbi;GB00BN7SWP63_XLON_GBX|[quantity-type=])",
        "<Order ClOrdID='XML-1'>body</Order>",
        "Receiving XmlApi: <Execution ExecID='E1'></Execution>",
        "Message rejected because : ignoring OMSSales expiry message",
        "no level printed by this plugin",
        "heartbeat emitted seq=7",
    ])
}

/// The frames whose reading the four disagreements turn on: a value holding
/// a byte the generic scanner ends on, a printed SOH in each of its
/// spellings, a length-prefixed data field carrying the frame separator, a
/// pair after the checksum, a mark beside its bare twin, and the packed
/// occurrences a bridge writes.
fn frames() -> Vec<Vec<u8>> {
    let nested = "MSGTYPE=D|ORDERID=9|#ORDERID=9|#SIDE=1";
    let document = concat!(
        r#"<FIXML v="5.0 SP2"><ExecRpt ExecID="E1" ClOrdID="ORDER-1" LastQty="21" "#,
        r#"LastPx="83.08"><Instrmt Symbol="HOLN" /></ExecRpt></FIXML>"#
    );
    let mut lines = owned(&[
        // Repeating groups by tag, nested, and the same group by name.
        "8=FIX.4.4|35=D|453=2|448=A|447=D|452=1|802=2|523=DESK|803=1|523=CLIENT|803=2|448=B|447=D|452=3|802=1|523=OTHER|803=3|55=AAPL|10=0|",
        "MSGTYPE=D|453=2|453[0]=448=BUYSIDE|453[1]=448=VENUE",
        "MSGTYPE=D|NOPARTYIDS=2|NOPARTYIDS[0]=PARTYID=BUYSIDE|NOPARTYIDS[1]=PARTYID=VENUE",
        "MSGTYPE=B|NOLINESOFTEXT=2|NOLINESOFTEXT[0]=TEXT=a|NOLINESOFTEXT[1]=TEXT=b",
        "8=FIX.4.4|35=J|70=A1|78=1|79=ACC|80=5|10=0|",
        "8=FIX.4.4|35=AE|571=T1|552=1|54=1|453=1|448=P1|452=1|802=1|523=S1|803=1|10=0|",
        // Keys by name, by unknown name, and by a tag no dictionary holds.
        "8=FIX.4.4|MsgType=D|Symbol=AAPL|Side=1|10=0|",
        "8=FIX.4.4|35=D|VenueOwnThing=x|9999=y|9=abc|10=0|",
        "8=FIX.4.4|55=AAPL|35=D|9=100|10=000|",
        "MSGTYPE=D|SYMBOL=AAPL",
        "MSGTYPE=D|PartyID[2]=third|PartyID[0]=first",
        // A stated absence, under a spelling and under a word.
        "8=FIX.4.4|35=D|58=nullable|10=0|",
        "8=FIX.4.4|35=D|58=null|10=0|",
        // A mark beside its bare twin, and every row of the truth table.
        "MSGTYPE=D|ORDERID=123|#ORDERID=345",
        "MSGTYPE=D|#ORDERID=345|ORDERID=123",
        "MSGTYPE=D|#ORDERID=345",
        "MSGTYPE=D|ORDERID=123|#ORDERID=123",
        "MSGTYPE=D|OrderId=123|#ORDERID=123",
        "MSGTYPE=D|ORDERID=123 |#ORDERID= 123",
        "MSGTYPE=D|ORDERID=abc|#ORDERID=ABC",
        "MSGTYPE=D|#ORDERID=123|##ORDERID=123",
        "MSGTYPE=D|#ORDERID=123|##ORDERID=345",
        "MSGTYPE=D|##ORDERID=345",
        "8=FIX.4.2|35=UL|ORDERID=123|#ORDERID=345|10=0|",
        "8=FIX.4.2|35=UL|ORDERID=123|#ORDERID=123|10=0|",
        // A frame a space separates, which is the separator inference the
        // codec and the generic scanner disagree about.
        "MSGTYPE=D ORDERID=123 #ORDERID=345",
        "MSGTYPE=ZMIN|#453=1|#453[0]=PARTYID=BUYSIDEPARTYROLE=1",
        "toBridge #NOPARTYIDS=1|#NOPARTYIDS[0]=PARTYID=BUYSIDE",
    ]);
    // The packed occurrences a bridge writes, whose members a control byte
    // separates and whose runs close on an empty segment.
    lines.push(
        b"MSGTYPE=D|NOPARTYIDS[0]=whole|#NOPARTYIDS[0]=PARTYID=A\x04\x03PARTYROLE=1".to_vec(),
    );
    lines.push(
        b"MSGTYPE=D|NOPARTYIDS=1|NOPARTYIDS[0]=PARTYID=BARE\x04\x03PARTYROLE=1\
|#NOPARTYIDS=2|#NOPARTYIDS[0]=PARTYID=A\x04\x03PARTYROLE=1|#NOPARTYIDS[1]=PARTYID=B\x04\x03PARTYROLE=3"
            .to_vec(),
    );
    lines.push(
        b"MSGTYPE=D|NOPARTYIDS=1|NOPARTYIDS[0]=PARTYID=B\x04\x03PARTYROLE=1\
|#NOPARTYIDS=1|#NOPARTYIDS[0]=PARTYID=A\x04\x03PARTYROLE=1"
            .to_vec(),
    );
    lines.push(
        b"MSGTYPE=D|NOPARTYIDS=2\
|NOPARTYIDS[0]=NOPARTYSUBIDS=1\x04\x03NOPARTYSUBIDS[0]=PARTYSUBID=a\x04\x03PARTYSUBIDTYPE=1\x04\x03VENUE_SEQ=7\x04\x03\x04\x03PARTYID=X\x04\x03PARTYROLE=1\x04\x03\
|NOPARTYIDS[1]=PARTYID=Y\x04\x03PARTYROLE=3\x04\x03"
            .to_vec(),
    );
    lines.push(
        b"MSGTYPE=D|NOPARTYIDS=1\
|NOPARTYIDS[0]=NOPARTYSUBIDS=1\x04\x03NOPARTYSUBIDS[0]=PARTYSUBID=a\x04\x03PARTYSUBIDTYPE=1\x04\x03VENUE_SEQ=7\x04\x03PARTYID=X\x04\x03PARTYROLE=1\x04\x03"
            .to_vec(),
    );
    lines.push(
        b"MSGTYPE=AE|NOSIDES=1\
|NOSIDES[0]=NOPARTYIDS=2\x04\x03NOPARTYIDS[0]=NOPARTYSUBIDS=1\x04\x03NOPARTYSUBIDS[0]=PARTYSUBID=a\x04\x03PARTYSUBIDTYPE=1\x04\x03PARTYID=X\x04\x03PARTYROLE=1\x04\x03NOPARTYIDS[1]=PARTYID=Y\x04\x03PARTYROLE=3\x04\x03"
            .to_vec(),
    );
    // A frame written with the byte, and the four spellings a log prints for
    // it, each carrying prose after the checksum.
    lines.push(b"recv 8=FIX.4.4\x019=61\x0135=0\x0149=XPAR\x0110=017\x01".to_vec());
    for spelling in ["^A", "\\x01", "<SOH>", "{SOH}"] {
        lines.push(
            format!(
                "recv 8=FIX.4.4{spelling}9=61{spelling}35=0{spelling}49=XPAR{spelling}10=017{spelling} on session 3"
            )
            .into_bytes(),
        );
    }
    // A length-prefixed data field whose value carries the frame separator,
    // one that states no length the trailer cannot correct, and one holding
    // a whole document.
    lines.push(
        format!(
            "8=FIX.4.2|35=UL|#SYMBOL=TTF|212={}|213={nested}|10=0|",
            nested.len()
        )
        .into_bytes(),
    );
    lines.push(b"8=FIX.4.2|9=0|35=UL|212=17|213=EXECTYPE=Restated|10=0|".to_vec());
    lines.push(
        format!(
            "8=FIX.4.2|9=0|35=n|212={}|213={document}|10=0|",
            document.len()
        )
        .into_bytes(),
    );
    lines
}

/// The bridge's own lines: a frame behind the prose its process printed, a
/// row keyed by name with a group packed into it, a document, and the row
/// header a real capture writes in front of all of them.
fn bridge() -> Vec<Vec<u8>> {
    owned(&[
        "sending >> 8=FIX.4.4|9=176|35=D|49=BUYSIDE|56=VENUE|11=ORDER-1|55=AAPL|54=1|38=100|44=10.5|59=0|60=20240102-10:15:30.000|10=203| << queued seq=1092",
        "8=FIX.4.4|9=224|35=8|49=VENUE|56=BUYSIDE|37=O-9|17=E-1|39=1|150=F|55=AAPL|54=1|38=100|14=40|32=40|31=10.5|64=20240104|10=118|",
        "8=FIX.4.4|9=224|35=8|49=VENUE|56=BUYSIDE|37=O-9|17=E-2|39=2|150=F|55=AAPL|54=1|38=100|14=100|32=60|31=10.5|15=EUR|155=1.1|10=119|",
        "recv |MSGTYPE=D|SYMBOL=TTF|SIDE=1|ORDERQTY=1200|#NOPARTYIDS=1|#NOPARTYIDS[0]=PARTYID=BUYSIDE\u{4}\u{3}PARTYIDSOURCE=D\u{4}\u{3}PARTYROLE=1|",
        r#"<FIXML><Order ClOrdID="ORDER-2" Side="1" OrdQty="50"/></FIXML>"#,
        concat!(
            r#"{"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:name=ULMSG_BROKER_TO_DMZ,"#,
            r#"plugin-type=FIX,type=Plugin","type":"read"},"value":{"SenderCompID":"ULB_BKRBDG","#,
            r#""TargetCompID":"ULB_PTBDG","BeginString":"FIX.4.2","State":"logged"},"status":200}"#,
        ),
        "no level printed by this plugin, and no pairs either",
        "2026-08-14 06:46:30.416 [15261] [OMS_X1_TradeCapture] (DEBUG) Sending : 8=FIX.4.4|9=68|35=0|49=CLIAUDITX1|56=OMSAUDITX1|34=696|52=20260814-04:46:30.415655|10=159|",
        "2026-08-14 06:46:36.887 [653] [Spot_FX_TradeCapture] (INFO) Receiving : 8=FIX.4.2|9=0322|35=8|34=4507|49=VENUEADC|56=CLIENTFIS|52=20260814-04:46:36|1=client|6=547.771791547861|11=20260814_TP1_CLIENT_1003|14=982|15=INR|17=E-20260814-4507|31=547.77|32=982|37=O-20260814-1003|38=982|39=2|40=1|44=547.771791547861|48=XX0000000001|54=1|55=EXAMPLECO|58=Filled|59=0|60=20260814-04:46:36|75=20260814|150=2|151=0|10=197|",
        "2026-08-14 06:46:37.153 [15333-e7254b22:9f015ee861:4507] [Broker_DarkPool_TradeCapture] (DEBUG) RouteMessage : ACCOUNT=client|AVGPX=547.771791547861|CLORDID=20260814_TP1_CLIENT_1003|CUMQTY=982|CURRENCY=INR|EXECBROKER=BRKR|EXECTYPE=2|LASTPX=547.77|LASTQTY=982|LEAVESQTY=0|MSGTYPE=8|ORDERQTY=982|ORDSTATUS=2|ORDTYPE=1|SIDE=1|SYMBOL=EXAMPLECO|TRANSACTTIME=20260814-04:46:36|",
        "2026-08-14 06:46:37.153 [15333-e7254b22:9f015ee861:4507] [ULBridge] (INFO) Execution report (ClOrderID : 20260814_TP1_CLIENT_1003) without any route so using not persisted route: [UNDEFINED] --> [Broker_DarkPool_TradeCapture]",
    ])
}

/// The lines the facet table is written against: what an order, a fill and a
/// quote answer for who, what, how much and when.
fn lifts() -> Vec<Vec<u8>> {
    owned(&[
        "8=FIX.4.4|35=D|49=SENDER|56=TARGET|34=7|11=ORDER-1|55=AAPL|54=1|38=100|44=12.5|60=20240102-10:15:30.000|10=0|",
        "8=FIX.4.4|35=8|17=EXEC-1|37=ORD-9|31=12.75|44=12.5|32=50|38=100|10=0|",
        "8=FIX.4.4|35=8|17=E|54=1|31=12.75|44=12.5|32=50|10=0|",
        "8=FIX.4.4|35=D|11=A|54=1|44=12.5|38=100|10=0|",
        "8=FIX.4.4|35=D|11=A|54=2|44=12.5|38=100|10=0|",
        "8=FIX.4.4|35=D|11=A|54=8|44=12.5|38=100|10=0|",
        "8=FIX.4.4|35=D|11=A|53=1000000|10=0|",
        "8=FIX.4.4|35=D|11=A|53=1000000|854=5|15=USD|10=0|",
        "8=FIX.4.4|35=D|11=A|38=100|465=1|854=2|10=0|",
        "8=FIX.4.4|35=D|11=A|132=12.4|10=0|",
        "8=FIX.4.4|35=S|117=Q1|132=12.4|133=12.6|10=0|",
        "8=FIX.4.4|35=D|11=A|60=20240102-09:00:00.000|52=20240102-10:15:30.000|10=0|",
        "8=FIX.4.4|35=D|11=A|Symbol[0]=AAPL|Symbol[1]=MSFT|10=0|",
        "8=FIX.4.4|35=D|9=abc|11=A|10=0|",
        "49=SENDER|56=TARGET|34=1092|43=Y|52=20240102-10:15:30.000",
        "MSGTYPE=D|#NOPARTYIDS=3|#NOPARTYIDS[0]=PARTYID=ONE",
    ])
}

/// The lines the specification's tables are read against, each pinned with
/// the fill applied - the composition, not the read alone.
fn enrichments() -> Vec<Vec<u8>> {
    owned(&[
        "8=FIX.4.4|35=8|150=0|38=100|14=0|10=0|",
        "8=FIX.4.4|35=8|150=F|151=60|14=40|10=0|",
        "8=FIX.4.4|35=8|150=F|151=0|14=100|10=0|",
        "8=FIX.4.4|35=8|150=G|151=0|10=0|",
        "8=FIX.4.4|35=8|39=1|150=F|10=0|",
        "8=FIX.4.4|35=8|39=2|150=F|151=60|14=40|10=0|",
        "8=FIX.4.4|35=8|150=A|10=0|",
        "8=FIX.4.4|35=8|150=D|10=0|",
        "8=FIX.4.4|35=8|15=EUR|120=USD|10=0|",
        "8=FIX.4.4|35=D|11=A|48=US0378331005|10=0|",
        "8=FIX.4.4|35=D|11=A|48=us0378331005|10=0|",
        "8=FIX.4.4|35=D|11=A|48=CH0012221716|22=4|10=0|",
        "8=FIX.4.4|35=D|11=A|48=037833100|10=0|",
        "8=FIX.4.4|35=D|11=A|48=B0YBKJ7|10=0|",
        "8=FIX.4.4|35=D|11=A|48=ABBN SW|22=A|10=0|",
        "8=FIX.4.4|35=D|11=A|55=NOVN|48=ABBN|22=8|10=0|",
        "8=FIX.4.4|35=D|11=A|461=DBFUFR|10=0|",
        "8=FIX.4.4|35=D|11=A|461=OPEICS|10=0|",
        "8=FIX.4.4|35=D|11=A|461=esvtfr|10=0|",
        "8=FIX.4.4|35=D|11=A|167=OPT|10=0|",
        "8=FIX.4.4|35=D|11=A|167=NOSUCH|10=0|",
        "8=FIX.4.4|35=F|11=B|41=A|10=0|",
        "MSGTYPE=D|CLORDID=A|ISINCODE=GB0002634946",
    ])
}

/// The execution report a 4.2 session sends, which the newest dictionary
/// restates, and an order carrying a retired date.
fn restatements() -> Vec<Vec<u8>> {
    owned(&[
        "8=FIX.4.2|35=8|37=O1|17=E1|20=1|150=1|39=1|55=AAPL|54=1|32=100|31=10.5|14=100|151=0|47=A|109=CLIENT1|76=BRKR|10=0|",
        "8=FIX.4.4|35=D|11=A|541=20240605|10=0|",
    ])
}

/// The text options the bridge's own log is read under, exactly as the
/// dataset suite reads it: its own row header, in UTC, every line numbered,
/// classified and read for its direction.
fn reading() -> RecordOptions {
    let mut options = TextOptions::new()
        .try_with_rowheader(yggdryl::ULBRIDGE_ROWHEADER)
        .expect("the bridge's row header compiles")
        .with_timezone(Timezone::UTC);
    options.start_rownum = Some(1);
    options.parse_direction = true;
    options.parse_mimetype = true;
    options.into()
}

/// The capture as the rows a text reader hands the codec, one record a row.
fn capture_records() -> Vec<Scalar> {
    let source = Buffer::from_bytes(LOG.to_vec()).with_media_type(
        Url::from_str("file:///ulbridge.log")
            .expect("a URL")
            .media_type(),
    );
    let mut records = Vec::new();
    let mut names: Vec<String> = Vec::new();
    for batch in source.read_arrow_reader(&reading()).expect("a reader") {
        let batch = batch.expect("a batch");
        if names.is_empty() {
            names = batch
                .schema()
                .fields()
                .iter()
                .map(|field| field.name().clone())
                .collect();
        }
        let held = yggdryl::arrow::batch_to_value(&batch).expect("the batch reads");
        for row in held.as_sequence().expect("rows") {
            let values = row.as_sequence().expect("a row");
            records.push(
                Scalar::from_record(names.iter().cloned().zip(values.iter().cloned()))
                    .expect("a record"),
            );
        }
    }
    records
}

/// The bridge's own capture, read the way a dataset read reads it: framed by
/// the text reader, then each row through the codec's record door.
fn capture(pinned: &mut Pinned) {
    let codec = bridge_codec();
    let schema = fix_schema(codec.registry(), "fix").expect("the fixed schema");
    for (index, record) in capture_records().iter().enumerate() {
        let at = format!("ulbridge[{index:03}]");
        let held = match codec.parse_text_record(record) {
            Ok(held) => held,
            Err(refused) => {
                pinned.push(format!("{at}.refused"), refused.to_string());
                continue;
            }
        };
        let mut answered = 0;
        for (ordinal, message) in held.enumerate() {
            answered += 1;
            let at = format!("{at}:{ordinal}");
            match message {
                Ok(message) => pinned.message(&at, &message, &schema),
                Err(refused) => pinned.push(format!("{at}.refused"), refused.to_string()),
            }
        }
        pinned.push(format!("{at}.messages"), answered.to_string());
    }
}

/// Everything this branch answers, in one reading.
fn read() -> Pinned {
    let mut pinned = Pinned::default();
    capture(&mut pinned);
    pinned.lines("shapes", &committed_codec(), &shapes(), false);
    pinned.lines("frames", &committed_codec(), &frames(), false);
    pinned.lines("bridge", &bridge_codec(), &bridge(), false);
    pinned.lines("lift", &committed_codec(), &lifts(), false);
    pinned.lines("enrich", &committed_codec(), &enrichments(), true);
    pinned.lines("latest", &committed_codec(), &restatements(), false);
    // The absence convention is deliberately not byte-preserving, so the same
    // marked and null-spelled lines are read once more with it turned off and
    // the separator stated: a difference the convention would have swallowed
    // shows here instead.
    let verbatim = FixCodec::new(super::committed_registry())
        .with_separator(b'|')
        .with_null_values::<[&str; 0], _>([]);
    pinned.lines("verbatim", &verbatim, &frames(), false);
    pinned
}

/// The codec's answer over every capture this branch holds, byte for byte.
///
/// This is the equivalence gate the adaptation is judged against: it asserts
/// no rule of its own, and a difference here means the reading moved. Where
/// that was the point, the snapshot is regenerated in the commit that moved
/// it; where it was not, it is a defect.
#[test]
fn the_codec_answers_what_it_answered() {
    let pinned = read();
    let path = snapshot_path();
    if std::env::var_os(WRITE).is_some() {
        std::fs::write(&path, pinned.rendered()).expect("the snapshot is writable");
        return;
    }
    let text = std::fs::read_to_string(&path).unwrap_or_else(|absent| {
        panic!(
            "{}: {absent}. Write it with {WRITE}=1 cargo test --locked -p yggdryl --test fix equivalence",
            path.display()
        )
    });
    let expected = committed(&text);
    let answered: BTreeMap<&str, &str> = pinned
        .records
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect();
    assert_eq!(
        answered.len(),
        pinned.records.len(),
        "the reading keyed two records the same way"
    );

    let mut moved = Vec::new();
    for (key, value) in &expected {
        match answered.get(key) {
            Some(held) if held == value => {}
            Some(held) => moved.push(format!("{key}\n    was {value}\n    now {held}")),
            None => moved.push(format!("{key}\n    was {value}\n    now nothing")),
        }
    }
    for (key, value) in &answered {
        if !expected.contains_key(key) {
            moved.push(format!("{key}\n    was nothing\n    now {value}"));
        }
    }
    assert!(
        moved.is_empty(),
        "the codec's answer moved over {} of the committed captures:\n  {}{}\n\
         Regenerate only beside the decision that changed the reading:\n  \
         {WRITE}=1 cargo test --locked -p yggdryl --test fix equivalence",
        moved.len(),
        moved
            .iter()
            .take(REPORTED)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n  "),
        if moved.len() > REPORTED {
            format!("\n  ... and {} more", moved.len() - REPORTED)
        } else {
            String::new()
        }
    );
}
