//! Reads the bundled bridge capture and prints its messages, before the
//! lifecycle walk and after it.
//!
//! `rust/tests/fix/ulbridge.log` is a real Ullink bridge log: every line is a
//! row header the capture wrote - [`ULBRIDGE_ROWHEADER`](yggdryl::ULBRIDGE_ROWHEADER)
//! names its columns - and then the message, so one message is logged again at
//! every hop it passes. Parsing lands each line in a [`FixMsg`] already filled
//! with what it implies; [`FixCodec::lifecycle`] then reads them as the chains
//! they belong to, so a message states what it follows and a message logged
//! twice folds into one event rather than growing its chain.
//!
//! ```text
//! cargo run --example fix_capture            # the first six of each
//! cargo run --example fix_capture 94         # all of them
//! ```

use std::sync::Arc;

use yggdryl::graph::{Element, Event, MarketElement};
use yggdryl::holder::Buffer;
use yggdryl::text::{TextOptions, read_text_lines};
use yggdryl::{FixCodec, FixMsg, FixRegistry, Timezone, Url};

fn main() -> yggdryl::Result<()> {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let registry = Arc::new(FixRegistry::from_handle(&yggdryl::local::Folder::new(
        manifest.join("../config/fix"),
    )?)?);
    let capture = Buffer::from_bytes(std::fs::read(manifest.join("tests/fix/ulbridge.log"))?)
        .with_media_type(Url::from_str("file:///ulbridge.log")?.media_type());
    // The row header is the capture's own columns, and the codec is told their
    // names so a message carries where it was read from.
    let mut options = TextOptions::new()
        .try_with_rowheader(yggdryl::ULBRIDGE_ROWHEADER)?
        .with_timezone(Timezone::UTC);
    options.start_rownum = Some(1);
    options.parse_mimetype = true;
    let names: Vec<String> = options.capture_names().map(ToOwned::to_owned).collect();
    let codec = FixCodec::new(Arc::clone(&registry)).with_capture_names(names);

    let mut parsed: Vec<FixMsg> = Vec::new();
    for line in read_text_lines(&capture, &options)? {
        for message in codec.parse_text_line(&line?)? {
            parsed.push(message?);
        }
    }
    let limit: usize = std::env::args()
        .nth(1)
        .and_then(|held| held.parse().ok())
        .unwrap_or(6);
    println!("parsed {} messages", parsed.len());
    println!("\n=== before the walk: what each message says of itself");
    for (index, message) in parsed.iter().enumerate().take(limit) {
        println!("[{index}] {}", describe(message));
    }

    let walked: Vec<FixMsg> = codec
        .lifecycle(parsed)
        .collect::<yggdryl::Result<Vec<_>>>()?;
    println!("\n=== after the walk: what each message follows");
    for (index, message) in walked.iter().enumerate().take(limit) {
        println!("[{index}] {}", describe(message));
    }

    println!("\n=== the chains the walk read");
    let mut chains: std::collections::BTreeMap<&str, (usize, std::collections::HashSet<_>, u64)> =
        std::collections::BTreeMap::new();
    for message in &walked {
        let held = chains.entry(message.get_crosscode()).or_default();
        held.0 += 1;
        held.1.insert(message.get_curruuid());
        held.2 = held.2.max(message.get_seqnum());
    }
    for (code, (messages, events, seqnum)) in &chains {
        // Fewer events than messages is the capture logging one message at
        // several hops: those statements are one event and share one UUID.
        let code = if code.is_empty() { "<no chain>" } else { code };
        println!(
            "{code:<26} {messages:>3} messages  {:>3} events  last seqnum {seqnum}",
            events.len()
        );
    }
    Ok(())
}

/// One message as the facts a reader of this capture wants: what it is, when
/// it happened, what it stands for in its market, the identity it settled on
/// and the chain it belongs to.
fn describe(message: &FixMsg) -> String {
    let header = message.header();
    format!(
        "{} {} px={} qty={} {} | currunix={} state={} | curruuid={} chain={:?} seqnum={} prev={:?} | {}",
        header.msgtype(),
        message.get_side().as_str(),
        message.get_px(),
        message.get_qty(),
        message.get_currency().as_str(),
        message.get_currunix(),
        message.get_state().as_str(),
        message.get_curruuid(),
        message.get_crosscode(),
        message.get_seqnum(),
        message.get_prevuuid(),
        message.capture().pluginid().unwrap_or("<no plugin stated>"),
    )
}
