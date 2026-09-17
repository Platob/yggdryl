//! Scratch probe.

use std::sync::Arc;

use yggdryl::holder::Buffer;
use yggdryl::media::RecordOptions;
use yggdryl::media::text::{TextOptions, read_text_lines};
use yggdryl::{FixCodec, FixRegistry, Timezone, Url};

const LOG: &[u8] = include_bytes!("ulbridge.log");

fn registry() -> Arc<FixRegistry> {
    super::committed_registry()
}

fn source() -> Buffer {
    Buffer::from_bytes(LOG.to_vec()).with_media_type(
        Url::from_str("file:///ulbridge.log")
            .expect("a URL")
            .media_type(),
    )
}

fn reading() -> RecordOptions {
    let mut options = TextOptions::new()
        .try_with_rowheader(yggdryl::ULBRIDGE_ROWHEADER)
        .expect("the bridge's row header compiles")
        .with_timezone(Timezone::UTC);
    options.start_rownum = Some(1);
    options.parse_mimetype = true;
    options.into()
}

fn codec() -> FixCodec {
    let RecordOptions::Text(options) = reading() else {
        panic!("a text read")
    };
    let names: Vec<String> = options.capture_names().map(ToOwned::to_owned).collect();
    super::fixed_codec(registry()).with_capture_names(names)
}

fn messages() -> Vec<yggdryl::FixMsg> {
    let RecordOptions::Text(options) = reading() else {
        panic!("a text read")
    };
    let held = source();
    let lines: Vec<_> = read_text_lines(&held, &options)
        .expect("a line reader")
        .map(|line| line.expect("a line"))
        .collect();
    codec()
        .parse_text_lines(lines)
        .collect::<yggdryl::Result<Vec<_>>>()
        .expect("messages")
}

#[test]
fn probe() {
    let all = messages();
    // the trade capture frame
    let capture = all
        .iter()
        .find(|message| {
            message
                .get_by_tag(213)
                .and_then(|held| held.as_bytes().map(<[u8]>::to_vec))
                .is_some_and(|held| {
                    String::from_utf8_lossy(&held).contains("MSGTYPE=tradecapturereport")
                })
        })
        .expect("the trade capture frame");
    for index in 0..7 {
        let id = capture
            .get_by_path(&super::path(&format!(
                "TrdCapRptSideGrp[0].Parties[{index}].PartyID"
            )))
            .and_then(|held| held.as_str().map(ToOwned::to_owned));
        let role = capture
            .get_by_path(&super::path(&format!(
                "TrdCapRptSideGrp[0].Parties[{index}].PartyRole"
            )))
            .and_then(|held| held.as_i64());
        let subs = capture
            .get_by_path(&super::path(&format!(
                "TrdCapRptSideGrp[0].Parties[{index}].PtysSubGrp"
            )))
            .and_then(|held| held.as_sequence().map(<[yggdryl::Scalar]>::len));
        let sub0 = capture
            .get_by_path(&super::path(&format!(
                "TrdCapRptSideGrp[0].Parties[{index}].PtysSubGrp[0].PartySubID"
            )))
            .and_then(|held| held.as_str().map(ToOwned::to_owned));
        println!("PARTY {index} id={id:?} role={role:?} subs={subs:?} sub0={sub0:?}");
    }
    println!("META {:?}", capture.metadata());
}
