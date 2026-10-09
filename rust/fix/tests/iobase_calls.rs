//! The `IOBase` call counts over what `yggdryl-fix` owns, beside the
//! core's own in `rust/tests/iobase_calls.rs`.
//!
//! What every derived surface costs in calls to the handle underneath it.
//!
//! `IOBase` is the one boundary a layer crosses to reach storage, and on a
//! store each crossing is a round trip - so the number of them an operation
//! makes is the property this crate is built around, and these are the tests
//! that hold it. Each states its count exactly rather than as a bound: a read
//! that quietly became two calls is the regression worth catching, and so is
//! an operation that stopped talking to storage at all.
//!
//! [`Counted`] is the instrument. It wraps the byte handle, forwards every
//! call unchanged, and tallies it, so the stack built on top of it is measured
//! rather than argued about. The counts here are what a *layer* asks of
//! storage; how many requests a backend then makes of the network is the object
//! client's own `Stats`, asserted in `rust/tests/s3/`.

#[path = "support/install.rs"]
mod install;
use std::sync::Arc;

use yggdryl::holder::Buffer;
use yggdryl::holder::counted::{Calls, Counted};
use yggdryl::{IOBase, Url};
use yggdryl_fix::FixFieldMut;

/// A handle over `bytes`, named so its media type is what `url` says.
fn source(bytes: &[u8], url: &str) -> Counted<Buffer> {
    let url = Url::from_str(url).expect("a location");
    let mut buffer = Buffer::from_bytes(bytes.to_vec());
    buffer.set_media_type(url.media_type());
    Counted::new(buffer)
}

/// Run `operation` and assert it cost exactly `expected` in calls.
///
/// The expectation is the tally's own rendering - `read_all_bytes=1`, or
/// `none` - so a failure names the call that appeared as well as the count.
fn costs(what: &str, calls: &Arc<Calls>, expected: &str, operation: impl FnOnce()) {
    calls.reset();
    operation();
    assert_eq!(calls.snapshot().to_string(), expected, "{what}");
}

#[test]
fn fix_catalog_storage_resolves_each_root_path_once() {
    crate::install::installed();
    use yggdryl::DataType;
    use yggdryl::local::LocalFolder;
    use yggdryl_fix::FixRegistry;

    let path = LocalFolder::temporary()
        .unwrap()
        .path()
        .unwrap()
        .join(format!("yggdryl-fix-root-calls-{}", std::process::id()));
    let mut folder = Counted::new(LocalFolder::new(&path).unwrap());
    let calls = Arc::clone(folder.calls());
    let mut field = DataType::utf8().nullable_field("Symbol");
    FixFieldMut::new(&mut field).set_tag(55).unwrap();
    let registry = FixRegistry::from_fields([field]).unwrap();
    // Counted measures navigation at this root; child handles own the
    // document reads and writes and are outside this tally. No manifest:
    // a dictionary is one namespace, and what each dialect contributed
    // travels on the field it contributed to.
    // Eight documents and four roots: the store's own field shard, the
    // crate's block on its own shard, its `metadata` group and its `fixmsg`
    // component - a store states the whole row, so the crate's three
    // documents are written beside the store's one - plus the built-in
    // market data kind, market data type, plugin side and state vocabularies. The three category roots and `codesets/` are each reached
    // once for pruning. Each intrinsic set adds one document lookup and no
    // root lookup. The sources catalog, `sources.json` at the root, is
    // one more resolution each way whatever the registry holds: a write
    // publishes it where the registry holds an entry and otherwise reads
    // its digest once to know whether a stale one is there to remove, and
    // a read resolves it because an absent document is no source. So a
    // write resolves thirteen and a read five.
    assert_eq!(
        registry
            .codesets()
            .map(|set| set.name())
            .collect::<Vec<_>>(),
        [
            "marketdatakindcodeset",
            "marketdatatypecodeset",
            "msgpluginsidecodeset",
            "statecodeset"
        ],
    );
    costs(
        "eight documents, four roots, the sources catalog",
        &calls,
        "child_by_path=13",
        || {
            registry.commit(&mut folder).unwrap();
        },
    );
    // And five on the way back: the code sets and the sources catalog are
    // read before the fields, because a field naming a set the dictionary
    // does not hold is refused.
    costs(
        "four roots, the sources catalog",
        &calls,
        "child_by_path=5",
        || {
            assert_eq!(FixRegistry::from_handle(&folder).unwrap(), registry);
        },
    );
    folder.remove(true).unwrap();
}

/// The bridge's own capture, the same bytes the FIX suite reads it from.
const CAPTURE: &[u8] = include_bytes!("../../tests/support/ulbridge.log");

/// Reading a capture as text and then as FIX asks storage for it once.
///
/// The two readers compose - the text reader frames and classifies the lines,
/// the codec reads each framed body into a message - and the composition is
/// where a second decode would hide, because each half is correct on its own
/// while the pair reads the file twice. So the count is taken over both at
/// once: one bounded stream, and the codec never reaching past it.
#[test]
fn a_capture_read_as_text_and_then_as_fix_is_one_decode() {
    crate::install::installed();
    use yggdryl::media::RecordOptions;
    use yggdryl::text::{TextOptions, read_text_lines};
    use yggdryl::{IOMedia, Timezone};
    use yggdryl_fix::{FixCodec, FixRegistry};

    /// How many lines the capture holds, which is how many rows the text
    /// reader answers.
    const LINES: usize = 144;
    /// How many FIX rows they read as: a row for every message the capture
    /// carries that the codec reads - none for the bridge's own prose, which
    /// carries no message at all, and none for the session traffic and the
    /// documents `DEFAULT_REFUSED_MSGTYPES` keeps out of a live read, which
    /// is what this codec is. What a row costs is the same either way; the
    /// count is here so that a reader knows what was drained. It is 84 where
    /// it was 79 since the parse splits each fill off its report (A12): five
    /// filling reports each bring their execution with them.
    const ROWS: usize = 84;
    /// How many messages the lifecycle walk answers for those rows: a
    /// message arriving under the identity the live one arrived under is the
    /// same message logged at another hop, so it restates that one rather
    /// than joining the chain behind it. This codec reads through a bare
    /// registry, which types almost nothing, so most of the capture's rows
    /// state the same little and collapse onto each other. It was 21 where
    /// it was 16 since the parse splits each fill off its report (A12): the
    /// five executions are each a delivery of their own, `FILLED` ending
    /// each chain, so a later fill under one `ExecID` starts afresh. It is
    /// 55 since the walk reads its input as already cleaned: the rows whose
    /// type the bare registry does not define, which the walk used to
    /// refuse on its own, are walked as the parse handed them over, and
    /// only the rows that collapse onto a live identity fold.
    const WALKED: usize = 55;
    /// What one bounded stream over the capture costs, before a message is
    /// built from any of it - through the record dispatcher and through the
    /// text door alike: both ask the one container question, because a
    /// folder or a glob is read leaf by leaf through either, and this capture
    /// is one leaf.
    const DECODE: &str =
        "pstream_bytes=1 url=1 bound_location=3 mtime=1 media_type=1 is_container=1 parent=1";

    let handle = source(CAPTURE, "file:///bridge.log");
    let calls = Arc::clone(handle.calls());
    let mut options = TextOptions::new()
        .try_with_rowheader(yggdryl_fix::ULBRIDGE_ROWHEADER)
        .expect("the bridge's row header compiles")
        .with_timezone(Timezone::UTC);
    options.parse_mimetype = true;
    let options: RecordOptions = options.into();
    let codec = FixCodec::new(Arc::new(FixRegistry::new()));

    // Explicit text options own the encoding, so the generic record read
    // skips document inference and costs the direct FIX text intake.
    costs("the capture read as text alone", &calls, DECODE, || {
        let read: usize = handle
            .read_arrow_reader(&options)
            .expect("a text reader")
            .map(|batch| batch.expect("a batch").num_rows())
            .sum();
        assert_eq!(read, LINES);
    });
    // The same count with the codec on top: the messages are built out of the
    // bytes that stream already returned, so the composition costs the decode
    // and nothing besides it.
    costs(
        "the capture read as text and then as FIX",
        &calls,
        DECODE,
        || {
            let text = handle.read_arrow_reader(&options).expect("a text reader");
            let read: usize = codec
                .parse_text_arrow_reader(text)
                .expect("a FIX reader")
                .map(|batch| batch.expect("a batch").num_rows())
                .sum();
            assert_eq!(read, ROWS);
        },
    );
    costs(
        "the decoded capture composed through the FIX lifecycle",
        &calls,
        DECODE,
        || {
            let RecordOptions::Text(options) = &options else {
                panic!("text options")
            };
            let lines = read_text_lines(&handle, options).expect("a text reader");
            let read = codec
                .lifecycle(codec.parse_text_lines(lines))
                .try_fold(
                    0_usize,
                    |read, message: yggdryl::Result<yggdryl_fix::FixMsg>| message.map(|_| read + 1),
                )
                .expect("a walked message");
            assert_eq!(read, WALKED);
        },
    );
}
