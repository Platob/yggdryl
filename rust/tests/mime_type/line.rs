//! `rust/src/mime_type/line.rs`: the line classifier no caller can name.
//!
//! The walk that decides what a line *is* - which byte framed its fields,
//! where each value ends, which pairs it stated at all - answers a caller
//! nothing but a [`MimeType`](yggdryl::MimeType), so what it read is reached
//! through `yggdryl::internals`. Everything a caller can observe is in
//! `rust/tests/media/` and `rust/tests/text/`.

use yggdryl::MimeType;
use yggdryl::internals::mime_type_line::{PairSpan, classify, entry_spans};

/// Every pair a line declares, each rendered as the line wrote it - the
/// mark put back in front, so a fixture reads the way the capture does.
fn read(line: &[u8]) -> Vec<String> {
    entry_spans(line)
        .map(|span| rendered(line, &span))
        .collect()
}

fn rendered(line: &[u8], span: &PairSpan) -> String {
    format!(
        "{}{}={}",
        if span.marked { "#" } else { "" },
        String::from_utf8_lossy(&line[span.key.clone()]),
        String::from_utf8_lossy(&line[span.value.clone()])
    )
}

#[test]
fn a_value_inside_a_frame_ends_at_the_frames_separator() {
    // Each of these closes early under the loose rule: at the space, at
    // the space again, and at the space a third time - leaving `A=1` and
    // `B=2` standing as pairs of their own.
    assert_eq!(
        read(b"8=FIX.4.4|35=D|18=G L|48=ABBN SW|58=quoting #A=1 and #B=2|10=0|"),
        [
            "8=FIX.4.4",
            "35=D",
            "18=G L",
            "48=ABBN SW",
            "58=quoting #A=1 and #B=2",
            "10=0",
        ]
    );
}

#[test]
fn every_byte_that_ends_a_loose_value_is_ordinary_inside_a_frame() {
    // Ten of the eleven bytes the loose walk closes a value at, inside one
    // SOH-framed value; the eleventh is SOH itself, which the pipe-framed
    // line below carries raw.
    let framed = b"8=FIX.4.4\x0158=a|b c\td\re\n f]g)h}i,j;k\x0110=0\x01";
    assert_eq!(
        read(framed),
        ["8=FIX.4.4", "58=a|b c\td\re\n f]g)h}i,j;k", "10=0"]
    );
    assert_eq!(
        read(b"8=FIX.4.4|35=D|58=x\x01y|10=123|"),
        ["8=FIX.4.4", "35=D", "58=x\u{1}y", "10=123"],
        "the frame opened on a pipe, so a raw SOH is a byte of the value"
    );
}

#[test]
fn a_frame_separates_on_what_it_opened_with_and_not_on_what_it_could_have() {
    assert_eq!(
        read(b"8=FIX.4.4 35=D 58=a|b 10=0"),
        ["8=FIX.4.4", "35=D", "58=a", "10=0"],
        "the pipe stands inside a value of a line that ran its fields \
         together with spaces, so it separates nothing"
    );
    assert_eq!(
        read(b"8=FIX.4.4|35=D|58=a b|10=0"),
        ["8=FIX.4.4", "35=D", "58=a b", "10=0"],
        "and the space stands inside a value of a line that named the pipe"
    );
}

#[test]
fn a_line_that_named_no_separator_is_read_the_way_prose_is() {
    // Whitespace is not a naming: every line that runs words together
    // holds some. These read exactly as a sentence does, which is what
    // they are until something says otherwise.
    assert_eq!(
        read(b"host=srv1, port=8080, mode=fast"),
        ["host=srv1", "port=8080", "mode=fast"],
        "a comma ends a value nothing bounded"
    );
    assert_eq!(read(b"user=bob,role=admin"), ["user=bob"]);
    assert_eq!(read(b"a=(1) b=[2] c={3}"), ["a=(1", "b=[2", "c={3"]);
    assert_eq!(
        read(b"8=FIX.4.4 35=D 11=A 10=123"),
        ["8=FIX.4.4", "35=D", "11=A", "10=123"],
        "a frame that named nothing is read by the same rule, and these \
         values hold none of the bytes the rule ends one at"
    );
    assert_eq!(
        read(b"8=FIX.4.4;35=D;11=A;10=123"),
        ["8=FIX.4.4"],
        "a semicolon is not a separator this crate reads, so the pairs \
         behind one open no field: the line named nothing"
    );
    assert_eq!(
        read(b"MSGTYPE=D|SYMBOL=AAPL|SIDE=1"),
        ["MSGTYPE=D", "SYMBOL=AAPL", "SIDE=1"],
        "the same bridge row, under a separator the line did name"
    );
}

#[test]
fn a_frames_value_ends_where_the_classifier_reads_it_ending() {
    // A log wrapping a row in its own punctuation closes the last value
    // with it. One owner for where a value ends, or one column would hold
    // two spellings of one value depending on how the line was decorated.
    let wrapped = b"(8=FIX.4.4|35=D)";
    assert_eq!(read(wrapped), ["8=FIX.4.4", "35=D"]);
    assert_eq!(classify(wrapped), (MimeType::FIX, Some(&b"D"[..])));
    assert_eq!(
        read(b"8=FIX.4.4|35=D|58=trailing space |10=0|"),
        ["8=FIX.4.4", "35=D", "58=trailing space", "10=0"]
    );
}

#[test]
fn a_balanced_value_tail_is_not_the_transport_closing_the_line() {
    assert_eq!(
        read(b"MSGTYPE=D|ORDERID=[N/A]|TEXT=(none)|ACCOUNT={n/a}"),
        ["MSGTYPE=D", "ORDERID=[N/A]", "TEXT=(none)", "ACCOUNT={n/a}",]
    );
    assert_eq!(
        read(b"(MSGTYPE=D|TEXT=[N/A])"),
        ["MSGTYPE=D", "TEXT=[N/A]"],
        "the outer close is transport decoration and the inner one is data"
    );
    assert_eq!(
        read(b"MSGTYPE=D|TEXT=[outer({inner})]"),
        ["MSGTYPE=D", "TEXT=[outer({inner})]"],
        "nested balanced bracket kinds remain literal bytes"
    );
}

#[test]
fn an_unmatched_trailing_closer_is_transport_decoration() {
    assert_eq!(
        read(b"MSGTYPE=D|ORDERID=[N/A])"),
        ["MSGTYPE=D", "ORDERID=[N/A]"]
    );
    assert_eq!(
        read(b"MSGTYPE=D|ORDERID=[N/A]]"),
        ["MSGTYPE=D", "ORDERID=[N/A]"]
    );
    assert_eq!(
        read(b"MSGTYPE=D|ORDERID=plain])"),
        ["MSGTYPE=D", "ORDERID=plain"]
    );
}

#[test]
fn a_long_bracket_tail_is_scanned_once_and_keeps_no_null_policy() {
    let payload = "x".repeat(16 * 1024);
    let line = format!("MSGTYPE=D|ORDERID=[{payload}]]|SIDE=none|TEXT=[n/a]");
    assert_eq!(
        read(line.as_bytes()),
        vec![
            "MSGTYPE=D".to_owned(),
            format!("ORDERID=[{payload}]"),
            "SIDE=none".to_owned(),
            "TEXT=[n/a]".to_owned(),
        ],
        "the scanner preserves bytes; default absence is the codec's policy"
    );
}

#[test]
fn a_segment_that_states_no_field_is_prose_and_reads_as_prose() {
    // A key is a name or a tag, indexed where the writer indexed it and
    // spaced where a renderer spaced it - the fold that reads `Msg Type`
    // as `MsgType` ignores a space exactly as it ignores `_`, and a frame
    // that spelled the field that way spelled a field.
    assert_eq!(
        read(b"8=FIX.4.4\x01Msg Type=D\x0110=0\x01"),
        ["8=FIX.4.4", "Msg Type=D", "10=0"]
    );
    // The frame widens which bytes a key may hold, never whether a key is
    // a name, so a remark carrying bytes no name carries states no field
    // and the pair it does state is still read.
    assert_eq!(
        read(b"8=FIX.4.4\x0135=D\x0158=hello world\x0110=0\x01 sent >> seq=7"),
        ["8=FIX.4.4", "35=D", "58=hello world", "10=0", "seq=7"]
    );
    // And this is what the space costs, asserted rather than avoided: a
    // remark spelled in nothing but words is a run of name bytes in front
    // of an `=`, so the frame's own segment reading claims it whole. No
    // rule available here separates it from the renderer's key above -
    // this walk holds no dictionary - and a reader that holds one answers
    // nothing for `trailing note`.
    assert_eq!(
        read(b"8=FIX.4.4\x0135=D\x0158=hello world\x0110=0\x01 trailing note=x"),
        [
            "8=FIX.4.4",
            "35=D",
            "58=hello world",
            "10=0",
            "trailing note=x"
        ]
    );
}

#[test]
fn a_frame_separates_on_a_byte_it_used_and_not_on_one_it_merely_holds() {
    // Position alone cannot tell a separator from a byte inside a value:
    // both of these hold a space before their first pipe, or a pipe before
    // their first space, and each is a frame on the other one's candidate.
    // What separates two fields has a field after it.
    assert_eq!(
        read(b"MSGTYPE=P Report Ack|SYMBOL=AAPL|"),
        ["MSGTYPE=P Report Ack", "SYMBOL=AAPL"],
        "the space stands inside a value and separates nothing"
    );
    assert_eq!(
        read(b"8=FIX.4.4 35=D 58=a|b 10=0"),
        ["8=FIX.4.4", "35=D", "58=a", "10=0"],
        "a space stands in front of the pipe and names no frame, so \
         nothing was named and the loose rule reads the line"
    );
    // A wire message ends with its separator, so a line closing on one
    // named it as plainly as a field after one would.
    assert_eq!(read(b"35=U|"), ["35=U"]);
    assert_eq!(classify(b"35=U|"), (MimeType::FIX, Some(&b"U"[..])));
}

#[test]
fn a_key_a_bridge_marked_twice_keeps_the_mark_it_wrote() {
    // The walk strips the one mark it reads, so a second is part of the
    // key the frame wrote: what two marks mean belongs to whoever holds
    // the dictionary, and that the frame stated a field here is this
    // walk's answer.
    assert_eq!(
        read(b"MSGTYPE=D|#ORDERID=123|##ORDERID=345"),
        ["MSGTYPE=D", "#ORDERID=123", "##ORDERID=345"]
    );
}

#[test]
fn a_field_the_locator_cannot_read_is_a_field_wherever_it_sits() {
    // The frame is located at the first pair a locator can read, and a
    // locator requiring a name and a value reads neither of these. The
    // frame opens in front of them all the same, or the same field would
    // be a pair second and no pair first.
    assert_eq!(
        read(b"MSGTYPE=D|SYMBOL=|SIDE=1"),
        ["MSGTYPE=D", "SYMBOL=", "SIDE=1"]
    );
    assert_eq!(
        read(b"SYMBOL=|MSGTYPE=D|SIDE=1"),
        ["SYMBOL=", "MSGTYPE=D", "SIDE=1"]
    );
    assert_eq!(
        read(b"SYMBOL=\"A B\"|MSGTYPE=D|SIDE=1"),
        ["SYMBOL=\"A B\"", "MSGTYPE=D", "SIDE=1"],
        "a value opening on a quote is a value like any other inside a \
         frame, wherever the frame states it"
    );
    assert_eq!(
        read(b"x=1|58=|8=FIX.4.4|35=D|10=0|"),
        ["x=1", "58=", "8=FIX.4.4", "35=D", "10=0"],
        "and the walk back stops at a segment stating a pair of its own, \
         because a pair in front of a frame is the transport's"
    );
}

#[test]
fn a_frame_mixing_soh_spellings_is_still_one_frame() {
    // Three spellings of one separator on one line, which is what a relay
    // rewriting a frame it was handed produces.
    let mixed = br"8=FIX.4.4^A35=D<SOH>11=A\x0110=123";
    assert_eq!(read(mixed), ["8=FIX.4.4", "35=D", "11=A", "10=123"]);
    // And the classifier reads the same frame, so tag 35 is the message
    // type rather than everything up to the next `^A`.
    assert_eq!(classify(mixed), (MimeType::FIX, Some(&b"D"[..])));
}

#[test]
fn an_indexed_or_dotted_key_is_a_key_inside_a_frame() {
    // The loose walk finds neither: it runs a key backwards over key bytes
    // into the `]`, and `]` opens no field.
    assert_eq!(
        read(b"8=FIX.4.4|35=D|NoAllocs[0].79=ACCT|Symbol[0]=AAPL|10=0|"),
        [
            "8=FIX.4.4",
            "35=D",
            "NoAllocs[0].79=ACCT",
            "Symbol[0]=AAPL",
            "10=0",
        ]
    );
    assert_eq!(
        read(b"MSGTYPE=D|#NOPARTYIDS=3|#NOPARTYIDS[0]=PARTYID=ONE"),
        ["MSGTYPE=D", "#NOPARTYIDS=3", "#NOPARTYIDS[0]=PARTYID=ONE"]
    );
    assert!(
        read(b"Symbol[0]=AAPL").is_empty(),
        "and standing alone it is still no pair: what a frame widens is \
         the key inside it, not the walk that finds a frame"
    );
}

#[test]
fn the_same_bytes_read_loosely_in_front_of_the_frame_they_precede() {
    assert_eq!(
        read(b"58=quoting #A=1 and #B=2 : 8=FIX.4.4|35=D|10=0|"),
        ["58=quoting", "#A=1", "#B=2", "8=FIX.4.4", "35=D", "10=0"],
        "nothing bounds a field in front of the frame, so every byte that \
         could end one does"
    );
    assert_eq!(
        read(b"8=FIX.4.4|35=D|58=quoting #A=1 and #B=2|10=0|"),
        ["8=FIX.4.4", "35=D", "58=quoting #A=1 and #B=2", "10=0"],
        "the same bytes inside a frame are one Text field"
    );
}

#[test]
fn an_empty_value_is_a_pair_inside_a_frame_and_punctuation_outside_one() {
    assert_eq!(
        read(b"MSGTYPE=D|SYMBOL=|SIDE=null|PRICE=<null>|ACCOUNT=A"),
        [
            "MSGTYPE=D",
            "SYMBOL=",
            "SIDE=null",
            "PRICE=<null>",
            "ACCOUNT=A",
        ],
        "each spelling of absence is a pair; which of them means absent is \
         a dialect's reading"
    );
    assert_eq!(
        read(b"58= 8=FIX.4.4|35=D|10=0|"),
        ["8=FIX.4.4", "35=D", "10=0"],
        "in front of the frame an `=` with nothing after it states nothing"
    );
}

#[test]
fn the_mark_the_line_wrote_survives_the_key_being_stripped() {
    assert_eq!(
        read(b"MSGTYPE=D|ORDERID=123|#ORDERID=123|#SIDE=1"),
        ["MSGTYPE=D", "ORDERID=123", "#ORDERID=123", "#SIDE=1"]
    );
    let line = b"MSGTYPE=D|ORDERID=123|#ORDERID=123";
    let spans: Vec<PairSpan> = entry_spans(line).collect();
    assert_eq!(
        &line[spans[1].key.clone()],
        &line[spans[2].key.clone()],
        "the two keys are the same bytes once the mark is off"
    );
    assert!(!spans[1].marked && spans[2].marked);
    assert_eq!(
        read(b"MSGTYPE=D|ORDERID=123|#ORDERID=345"),
        ["MSGTYPE=D", "ORDERID=123", "#ORDERID=345"],
        "a restatement that differs is still marked"
    );
}

#[test]
fn a_line_with_no_pair_states_none() {
    assert!(read(b"no pairs here at all").is_empty());
    assert!(read(b"x = 5").is_empty(), "an `=` alone is punctuation");
}

#[test]
fn a_byte_that_opens_a_marker_is_a_marker_only_where_the_spelling_follows() {
    // The lines three reviewers could not trace by hand when the walk
    // that picks a separator was rewritten, pinned to what it answered
    // before and after: an opener with no spelling behind it is a byte
    // of the value, a spelling is a separator wherever it stands, and a
    // candidate ranks where it first stood, not where it first separated.
    assert_eq!(
        read(b"8=FIX.4.4<x|35=D|58=a^A10=123"),
        ["8=FIX.4.4<x", "35=D", "58=a^A10=123"]
    );
    assert_eq!(
        read(br"8=FIX.4.4^A\x0135=D<SOH>{SOH}10=1^A"),
        ["8=FIX.4.4", "35=D", "10=1"]
    );
    assert_eq!(
        read(br"8=FIX.4.4\x0\x0135=D\x01"),
        [r"8=FIX.4.4\x0", "35=D"]
    );
    assert_eq!(
        read(br"58=a\x01 8=FIX.4.4\x0135=D\x0110=1\x01"),
        ["58=a", "8=FIX.4.4", "35=D", "10=1"]
    );
    assert_eq!(read(b"8=FIX.4.4{^A35=D^A"), ["8=FIX.4.4{", "35=D"]);
    assert_eq!(read(b"8=FIX.4.4^{SOH}35=D{SOH}"), ["8=FIX.4.4^", "35=D"]);
    assert_eq!(read(b"8=FIX.4.4{SOH}35=D^A"), ["8=FIX.4.4", "35=D"]);
    assert_eq!(
        read(br"8=FIX.4.4\\x0135=D"),
        [r"8=FIX.4.4\", "35=D"],
        "an escaped backslash in front of the spelling is a byte of the value"
    );
    // A space in front of the first pipe: whitespace stood first and
    // never separated, so the pipe names the frame - and the reverse.
    assert_eq!(read(b"8=FIX.4.4 x|35=D|"), ["8=FIX.4.4 x", "35=D"]);
    assert_eq!(
        read(b"8=FIX.4.4 x|35=D y=1"),
        ["8=FIX.4.4", "35=D", "y=1"],
        "here the pipe never separated a field, and whitespace did"
    );
    assert_eq!(classify(b"8=FIX.4.4 x|35=D y=1"), (MimeType::FIXUL, None));
    // A raw SOH between two pipes: the pipe stood first and closes the
    // line, so it names the frame, and the SOH is a byte of a segment
    // that states no field under the pipe - read loosely instead.
    assert_eq!(read(b"8=FIX.4.4|\x0135=D\x01|"), ["8=FIX.4.4", "35=D"]);
    assert_eq!(classify(b"8=FIX.4.4|\x0135=D\x01|"), (MimeType::FIX, None));
    assert_eq!(read(b"35=U \t\n"), ["35=U"]);
}
