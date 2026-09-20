//! The corpus every market benchmark reads: the bridge's own capture,
//! parsed once under the shipped dictionary, so a door is timed over
//! messages already built and never over the parse in front of it.

use std::path::PathBuf;
use std::sync::Arc;

use yggdryl::holder::Buffer;
use yggdryl::local::Folder;
use yggdryl::media::RecordOptions;
use yggdryl::text::{TextOptions, read_text_lines};
use yggdryl::{FixCodec, FixMsg, FixRegistry, Timezone, Url};

/// The capture, exactly as the bridge wrote it; it ends in a newline, so
/// repeating it repeats whole lines.
const LOG: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fix/ulbridge.log"
));

/// How many times the capture is repeated in one measured run.
///
/// A repeated capture is the same second logged again, so the doors fold
/// every copy of a message into the statement it restates: the release
/// corpus measures the fold as much as the reading, which is what a
/// capture of a bridge behind several hops costs.
pub(crate) const REPEATS: usize = crate::bench_profile::corpus(8, 1);

/// How many messages one copy of the capture reads as under the codec's
/// own defaults: every frame, row and document less the session traffic.
pub(crate) const MESSAGES: usize = 79;

/// The tracked seed dictionary, relative to the crate manifest.
fn seed_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("config")
        .join("fix")
}

/// The shipped dictionary, loaded once and shared.
pub(crate) fn seed() -> Arc<FixRegistry> {
    static REGISTRY: std::sync::OnceLock<Arc<FixRegistry>> = std::sync::OnceLock::new();
    Arc::clone(REGISTRY.get_or_init(|| {
        let folder = Folder::new(seed_root()).expect("the seed folder is a local path");
        Arc::new(FixRegistry::from_handle(&folder).expect("the tracked seed loads"))
    }))
}

/// The text options a bridge log is read under: the bridge's own row
/// header framed, its clock read in UTC, each line numbered.
fn text() -> TextOptions {
    let mut options = TextOptions::new()
        .try_with_rowheader(yggdryl::ULBRIDGE_ROWHEADER)
        .expect("the row header compiles")
        .with_timezone(Timezone::UTC);
    options.start_rownum = Some(1);
    options.parse_mimetype = true;
    options
}

/// The codec every door reads under: the shipped dictionary, the bridge's
/// captures named, and one explicit intake clock so nothing consults now.
pub(crate) fn codec() -> FixCodec {
    let options = text();
    let RecordOptions::Text(read) = RecordOptions::from(options) else {
        panic!("a text read")
    };
    FixCodec::new(seed())
        .with_capture_names(read.capture_names().map(ToOwned::to_owned))
        .try_with_default_sending_time(Some(
            yggdryl::Scalar::datetime64(
                1_704_190_530_000_000_000,
                yggdryl::TimeUnit::Nanosecond,
                Timezone::UTC,
            )
            .expect("a clock"),
        ))
        .expect("a codec")
}

/// Every message the repeated capture reads as, parsed once, in line
/// order and before any walk: what every door is handed.
pub(crate) fn messages(codec: &FixCodec) -> Vec<FixMsg> {
    let source = Buffer::from_bytes(LOG.repeat(REPEATS)).with_media_type(
        Url::from_str("file:///ulbridge.log")
            .expect("a URL")
            .media_type(),
    );
    let options = text();
    let lines = read_text_lines(&source, &options)
        .expect("a decoded line stream")
        .map(|line| line.expect("a line"));
    let messages: Vec<FixMsg> = codec
        .parse_text_lines(lines)
        .collect::<yggdryl::Result<_>>()
        .expect("every line reads");
    assert_eq!(messages.len(), MESSAGES * REPEATS, "the corpus");
    messages
}
