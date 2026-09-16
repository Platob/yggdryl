//! Smoke dump: parse ulbridge.log and print messages before and after the lifecycle walk.
use std::sync::Arc;

use yggdryl::graph::{Element, Event, MarketElement};
use yggdryl::holder::Buffer;
use yggdryl::media::text::{TextOptions, read_text_lines};
use yggdryl::{FixCodec, FixMsg, FixRegistry, Timezone, Url};

fn describe(msg: &FixMsg) -> String {
    let header = msg.header();
    let names: Vec<String> = msg
        .entries()
        .iter()
        .map(|entry| {
            if entry.entries().is_empty() {
                format!("{}={}", entry.name(), entry.value().unwrap_or("-"))
            } else {
                format!(
                    "{}={} [{} nested]",
                    entry.name(),
                    entry.value().unwrap_or("-"),
                    entry.entries().len()
                )
            }
        })
        .collect();
    format!(
        "msgtype={} sender={:?} target={:?} seq={:?} unix={} state={} side={} px={} qty={} crosscode={:?} curruuid={} crossuuid={} prevuuid={:?} seqnum={} creatunix={:?} parents={} ids={:?} isin={:?} cur={} plugin={:?} session={:?} text={:?} metadata={:?}\n    entries: {}",
        header.msgtype(),
        header.sendercompid(),
        header.targetcompid(),
        header.msgseqnum(),
        msg.get_unix(),
        msg.get_state().as_str(),
        msg.get_side().as_str(),
        msg.get_px(),
        msg.get_qty(),
        msg.get_crosscode(),
        msg.get_curruuid(),
        msg.get_crossuuid(),
        msg.get_prevuuid(),
        msg.get_seqnum(),
        msg.get_creatunix(),
        msg.get_parentuuids().len(),
        msg.get_identifiers(),
        msg.get_isincode().map(|held| held.as_str().to_owned()),
        msg.get_currency().as_str(),
        msg.capture().pluginid(),
        msg.capture().msgsessionid(),
        msg.text(),
        msg.metadata(),
        names.join(" | "),
    )
}

fn main() -> yggdryl::Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = Arc::new(FixRegistry::from_handle(
        &yggdryl::holder::local::Folder::new(root)?,
    )?);
    let log = std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fix/ulbridge.log"),
    )?;
    let source = Buffer::from_bytes(log)
        .with_media_type(Url::from_str("file:///ulbridge.log")?.media_type());
    let mut options = TextOptions::new()
        .try_with_rowheader(yggdryl::ULBRIDGE_ROWHEADER)?
        .with_timezone(Timezone::UTC);
    options.start_rownum = Some(1);
    options.parse_mimetype = true;
    let names: Vec<String> = options.capture_names().map(ToOwned::to_owned).collect();
    let codec = FixCodec::new(Arc::clone(&registry)).with_capture_names(names);
    let mut parsed: Vec<FixMsg> = Vec::new();
    let mut errors = 0;
    for line in read_text_lines(&source, &options)? {
        let line = line?;
        for message in codec.parse_text_line(&line)? {
            match message {
                Ok(message) => parsed.push(message),
                Err(error) => {
                    errors += 1;
                    eprintln!("error: {error}");
                }
            }
        }
    }
    println!("parsed {} messages, {errors} errors", parsed.len());
    let limit: usize = std::env::args()
        .nth(1)
        .and_then(|held| held.parse().ok())
        .unwrap_or(6);
    println!("=== before lifecycle");
    for (index, message) in parsed.iter().enumerate().take(limit) {
        println!("[{index}] {}", describe(message));
    }
    let walked: Vec<FixMsg> = codec
        .lifecycle(parsed.clone())
        .collect::<yggdryl::Result<Vec<_>>>()?;
    println!("=== after lifecycle");
    for (index, message) in walked.iter().enumerate().take(limit) {
        println!("[{index}] {}", describe(message));
    }
    let chained = walked
        .iter()
        .filter(|held| held.get_prevuuid().is_some())
        .count();
    println!("chained: {chained} of {}", walked.len());
    let mut by_cross: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for message in &walked {
        *by_cross
            .entry(message.get_crosscode().to_owned())
            .or_default() += 1;
    }
    let mut chains: Vec<(String, usize)> = by_cross.into_iter().collect();
    chains.sort_by(|left, right| right.1.cmp(&left.1));
    println!("chains: {:?}", &chains[..chains.len().min(8)]);
    if std::env::args().nth(2).as_deref() == Some("dedup") {
        let mut by_digest: std::collections::HashMap<u128, Vec<usize>> = Default::default();
        let mut by_hash: std::collections::HashMap<u64, Vec<usize>> = Default::default();
        let mut by_uuid: std::collections::HashMap<yggdryl::types::Uuid, Vec<usize>> =
            Default::default();
        let mut by_line: std::collections::HashMap<Vec<u8>, Vec<usize>> = Default::default();
        for (index, message) in parsed.iter().enumerate() {
            by_digest.entry(message.digest()).or_default().push(index);
            by_hash
                .entry(message.get_hashcode())
                .or_default()
                .push(index);
            by_uuid
                .entry(message.get_curruuid())
                .or_default()
                .push(index);
            by_line
                .entry(message.into_bytes(b'|'))
                .or_default()
                .push(index);
        }
        fn dupes<K>(held: &std::collections::HashMap<K, Vec<usize>>) -> Vec<Vec<usize>> {
            let mut groups: Vec<Vec<usize>> = held
                .values()
                .filter(|held| held.len() > 1)
                .cloned()
                .collect();
            groups.sort();
            groups
        }
        println!("duplicate wire digests: {:?}", dupes(&by_digest));
        println!("duplicate hashcodes: {:?}", dupes(&by_hash));
        println!("duplicate curruuids: {:?}", dupes(&by_uuid));
        println!("duplicate wire bytes: {:?}", dupes(&by_line));
        let mut dedup = yggdryl::FixDedup::new(parsed.iter().cloned());
        let kept = dedup.by_ref().count();
        println!(
            "FixDedup (adjacent): kept {kept}, dropped {}",
            dedup.dropped()
        );
        for group in dupes(&by_digest) {
            for index in group {
                let message = &parsed[index];
                println!(
                    "  [{index}] unix={} seq={:?} possdup={:?} 35={} 11={:?} 37={:?} 17={:?} text={}",
                    message.get_unix(),
                    message.header().msgseqnum(),
                    message.header().possdupflag(),
                    message.header().msgtype(),
                    message
                        .get_by_tag(11)
                        .and_then(|held| held.as_str().map(str::to_owned)),
                    message
                        .get_by_tag(37)
                        .and_then(|held| held.as_str().map(str::to_owned)),
                    message
                        .get_by_tag(17)
                        .and_then(|held| held.as_str().map(str::to_owned)),
                    &message.into_text('|')?[..120.min(message.into_text('|')?.len())]
                );
            }
        }
        let walked_dupes = {
            let mut by_uuid: std::collections::HashMap<yggdryl::types::Uuid, usize> =
                Default::default();
            for message in &walked {
                *by_uuid.entry(message.get_curruuid()).or_default() += 1;
            }
            by_uuid.values().filter(|held| **held > 1).count()
        };
        println!(
            "after lifecycle: {} messages, {} curruuids held twice",
            walked.len(),
            walked_dupes
        );
        let mut per_chain: std::collections::BTreeMap<
            String,
            (usize, std::collections::HashSet<yggdryl::types::Uuid>, u64),
        > = Default::default();
        for message in &walked {
            let held = per_chain
                .entry(message.get_crosscode().to_owned())
                .or_default();
            held.0 += 1;
            held.1.insert(message.get_curruuid());
            held.2 = held.2.max(message.get_seqnum());
        }
        for (code, (count, uuids, seqnum)) in &per_chain {
            println!(
                "  chain {code:?}: {count} messages, {} distinct events, last seqnum {seqnum}",
                uuids.len()
            );
        }
    }
    if let Some(target) = std::env::args().nth(3) {
        let mut folder = yggdryl::holder::local::Folder::new(std::path::PathBuf::from(target))?;
        registry.write_into(&mut folder)?;
        println!("dumped the store");
    }
    if std::env::args().nth(2).as_deref() == Some("row") {
        let schema = yggdryl::fix_schema(&registry, "fix")?;
        let row = walked[1].into_row(&schema)?;
        println!("row[1]: {}", yggdryl::into_json_scalar(&row)?);
        println!("text[1]: {}", walked[1].into_text('|')?);
        let back = yggdryl::FixMsg::from_row(std::sync::Arc::clone(&registry), &schema, &row)?;
        println!("back[1]: {}", back.into_text('|')?);
        println!("same entries: {}", back.entries() == walked[1].entries());
        let again = back.into_row(&schema)?;
        println!("same row: {}", again == row);
        for ((column, left), right) in schema
            .fields()
            .iter()
            .zip(row.as_sequence().unwrap_or_default())
            .zip(again.as_sequence().unwrap_or_default())
        {
            if left != right {
                println!("  differs {}: {left:?} -> {right:?}", column.name());
            }
        }
        println!(
            "same hash: {}",
            back.get_hashcode() == walked[1].get_hashcode()
        );
        let (left, right) = (&walked[1], &back);
        if left.event() != right.event() {
            println!(
                "event differs:\n  {:?}\n  {:?}",
                left.event(),
                right.event()
            );
        }
        if left.header() != right.header() {
            println!(
                "header differs:\n  {:?}\n  {:?}",
                left.header(),
                right.header()
            );
        }
        if left.text() != right.text() || left.metadata() != right.metadata() {
            println!("text/metadata differ");
        }
        let lv = left.as_value().as_sequence().unwrap_or_default();
        let rv = right.as_value().as_sequence().unwrap_or_default();
        println!("children: {} vs {}", lv.len(), rv.len());
        for (index, (lf, lval)) in left.as_field().fields().iter().zip(lv).enumerate() {
            let found = right
                .as_field()
                .fields()
                .iter()
                .position(|rf| rf.name() == lf.name());
            match found {
                None => println!("  only left: {} = {lval:?}", lf.name()),
                Some(at) => {
                    let rf = &right.as_field().fields()[at];
                    let rval = &rv[at];
                    if lval != rval
                        || lf.dtype() != rf.dtype()
                        || lf.as_metadata() != rf.as_metadata()
                    {
                        println!(
                            "  differs {} [{index}->{at}]:\n    {:?} {:?} {lval:?}\n    {:?} {:?} {rval:?}",
                            lf.name(),
                            lf.dtype(),
                            lf.as_metadata(),
                            rf.dtype(),
                            rf.as_metadata()
                        );
                    }
                }
            }
        }
        for rf in right.as_field().fields() {
            if !left
                .as_field()
                .fields()
                .iter()
                .any(|lf| lf.name() == rf.name())
            {
                println!("  only right: {}", rf.name());
            }
        }
    }
    Ok(())
}
