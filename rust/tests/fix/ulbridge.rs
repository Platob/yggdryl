//! `rust/src/fix/ulbridge.rs`: a bridge's own log line - the whole capture as
//! one dataset, read a line at a time and a batch at a time, and required to
//! agree.

use super::SoleMessage;
use super::allocations;
use super::batch;
use super::committed_registry;
use super::decimal;
use super::fixed_codec;
use super::format_target;
use super::path;
use super::tag_index;

mod dataset {
    use std::sync::Arc;

    use arrow_array::RecordBatch;
    use yggdryl::graph::{Element, Event, Market};
    use yggdryl::holder::Buffer;
    use yggdryl::media::RecordOptions;
    use yggdryl::text::{TextLine, TextOptions, read_text_lines};
    use yggdryl::{FixCodec, FixMsg, FixRegistry, IOMedia, Scalar, Timezone, Url};

    use super::{SoleMessage, path};

    /// The capture, exactly as the bridge wrote it.
    const LOG: &[u8] = include_bytes!("ulbridge.log");

    /// How many lines the capture holds.
    const LINES: usize = 144;

    /// How many messages the capture reads as under the codec's own defaults:
    /// every frame, bridge row and document the log carries, less the session
    /// traffic `DEFAULT_REFUSED_MSGTYPES` names.
    const ROWS: usize = 79;

    /// How many it reads as when nothing is refused: the same lines plus the
    /// heartbeats, the test request and the rows that state no type at all.
    const EVERY_ROW: usize = 94;

    fn registry() -> Arc<FixRegistry> {
        super::committed_registry()
    }

    /// The log as the `.log` handle a reader opens.
    fn source() -> &'static Buffer {
        static SOURCE: std::sync::OnceLock<Buffer> = std::sync::OnceLock::new();
        SOURCE.get_or_init(|| {
            Buffer::from_bytes(LOG.to_vec()).with_media_type(
                Url::from_str("file:///ulbridge.log")
                    .expect("a URL")
                    .media_type(),
            )
        })
    }

    /// The text options a bridge log is read under: its own row header, each
    /// line numbered and classified.
    fn reading() -> RecordOptions {
        let mut options = TextOptions::new()
            .try_with_rowheader(yggdryl::ULBRIDGE_ROWHEADER)
            .expect("the bridge's row header compiles")
            .with_timezone(Timezone::UTC);
        options.start_rownum = Some(1);
        options.parse_mimetype = true;
        options.into()
    }

    /// What the bridge's row header captures, in the order it declares them.
    fn header_captures() -> Vec<String> {
        let RecordOptions::Text(options) = reading() else {
            panic!("a text read")
        };
        options.capture_names().map(ToOwned::to_owned).collect()
    }

    /// The codec every read uses: the bridge's dictionary, and what the run's
    /// captures are called, because a line answers them by position and only
    /// this boundary knows what each position means.
    fn codec() -> FixCodec {
        super::fixed_codec(registry()).with_capture_names(header_captures())
    }

    /// Every line the capture holds, as the text reader decodes them.
    fn text_lines() -> Vec<TextLine> {
        let RecordOptions::Text(options) = reading() else {
            panic!("a text read")
        };
        read_text_lines(source(), &options)
            .expect("a line reader")
            .map(|line| line.expect("a line"))
            .collect()
    }

    /// Every message the line door answers for the capture, in line order.
    fn line_messages(codec: &FixCodec) -> Vec<FixMsg> {
        codec
            .parse_text_lines(text_lines())
            .collect::<yggdryl::Result<Vec<_>>>()
            .expect("every line reads")
    }

    #[test]
    fn ulbridge_lifecycle_preserves_deliveries_and_identities_through_arrow() {
        let codec = codec().with_exclude_msgtypes::<[&str; 0], &str>([]);
        let messages = line_messages(&codec);
        assert_eq!(messages.len(), EVERY_ROW);
        let schema = super::format_target(&registry());
        let reader = codec.arrow_reader(schema, messages.clone()).unwrap();
        let direct = codec
            .lifecycle(messages)
            .collect::<yggdryl::Result<Vec<_>>>()
            .unwrap();
        // Complete four-part capture keys identify the bridge's repeated
        // observations before the older content-key dedup runs: 27 deliveries
        // and one expiry.
        //
        // It was 32 while the row header's clock admitted three fractional
        // digits and no more. The capture's last fifteen lines write grouped
        // microseconds, so the header matched 129 of its 144 lines; the
        // fifteen it missed arrived carrying no session, context or sequence,
        // built no `msgsesseventid`, and so could not be folded onto the
        // deliveries they are repeats of. Reading them is what folds them.
        assert_eq!(direct.len(), 28);
        assert_eq!(
            direct
                .iter()
                .filter(|message| message.get_state().as_str() == "95EXPIRED")
                .count(),
            1
        );
        let arrow = codec
            .messages(codec.lifecycle_arrow_reader(reader).unwrap())
            .collect::<yggdryl::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(arrow.len(), direct.len(), "the same distinct deliveries");
        for (index, (before, after)) in direct.iter().zip(&arrow).enumerate() {
            assert_eq!(after.get_curruuid(), before.get_curruuid(), "row {index}");
            assert_eq!(
                after.get_currhashcode(),
                before.get_currhashcode(),
                "row {index}"
            );
            assert_eq!(after.get_currunix(), before.get_currunix(), "row {index}");
            assert_eq!(after.get_prevuuid(), before.get_prevuuid(), "row {index}");
            assert_eq!(after.get_srcuuids(), before.get_srcuuids(), "row {index}");
            assert_eq!(after.get_seqnum(), before.get_seqnum(), "row {index}");
            assert_eq!(after.get_state(), before.get_state(), "row {index}");
            assert_eq!(
                after.capture().msgsesseventid(),
                before.capture().msgsesseventid(),
                "row {index}"
            );
        }
    }

    /// The batch door's answer for the whole capture: a capture row in, one FIX
    /// row per message out.
    fn batches(codec: &FixCodec) -> Vec<RecordBatch> {
        codec
            .parse_text_arrow_reader(source().read_arrow_reader(&reading()).expect("a reader"))
            .expect("the batch reader opens")
            .map(|batch| batch.expect("a batch"))
            .collect()
    }

    /// One read's rows, by value, beside the schema they landed under.
    fn rows_of(batches: &[RecordBatch]) -> (yggdryl::Field, Vec<Vec<Scalar>>) {
        let schema = yggdryl::Field::from_arrow_schema("row", &batches[0].schema())
            .expect("the batch schema reads");
        let mut rows = Vec::new();
        for batch in batches {
            let held =
                yggdryl::Serie::from_arrow_batch(None, batch, yggdryl::ArrowCastOptions::default())
                    .expect("the batch reads");
            for row in held.rows().iter() {
                rows.push(row.as_sequence().expect("a row").to_vec());
            }
        }
        (schema, rows)
    }

    /// Measure twice so the printed pair exposes any first-use cache noise. The
    /// second result is retained by the caller and checked by this test.
    fn profiled<T>(stage: &str, body: impl Fn() -> T) -> T {
        let (first, first_counts) = super::allocations::measure(&body);
        let (second, second_counts) = super::allocations::measure(body);
        eprintln!(
            "{stage}: first allocations={} reallocations={} requested_bytes={}; \
         second allocations={} reallocations={} requested_bytes={}",
            first_counts.allocations,
            first_counts.reallocations,
            first_counts.requested_bytes,
            second_counts.allocations,
            second_counts.reallocations,
            second_counts.requested_bytes,
        );
        drop(first);
        second
    }

    #[test]
    fn ulbridge_dataset_allocation_profile_is_sequential_and_staged() {
        // Settle the shared dictionary, schema, and source before the measured
        // sections. The codec then stays on this thread for every read below.
        let registry = registry();
        let target = super::format_target(&registry);
        let _ = source();
        let default = codec().with_threads(1);
        let every = default
            .clone()
            .with_exclude_msgtypes::<[&str; 0], &str>([])
            .with_threads(1);
        assert_eq!(default.threads(), 1);
        assert_eq!(every.threads(), 1);

        let lines = profiled("ulbridge text framing", text_lines);
        assert_eq!(lines.len(), LINES);

        let messages = profiled("ulbridge codec parse default", || {
            default
                .parse_text_lines(lines.iter().cloned())
                .collect::<yggdryl::Result<Vec<_>>>()
                .expect("the prepared lines parse")
        });
        assert_eq!(messages.len(), ROWS);
        let all_messages = profiled("ulbridge codec parse all", || {
            every
                .parse_text_lines(lines.iter().cloned())
                .collect::<yggdryl::Result<Vec<_>>>()
                .expect("the prepared lines parse with no refusals")
        });
        assert_eq!(all_messages.len(), EVERY_ROW);

        let canonical_rows = profiled("ulbridge message into_row format_target", || {
            messages
                .iter()
                .map(|message| message.into_row(&target).expect("the fixed row"))
                .collect::<Vec<_>>()
        });
        assert_eq!(canonical_rows.len(), ROWS);

        let residual_column = target.index_of("fixentries").expect("the residual column");
        let residual_count = target.index_of("nofixentries").expect("the residual count");
        let original_entries: usize = messages.iter().map(|message| message.entries().len()).sum();
        let mut residual_entries = 0;
        for row in &canonical_rows {
            let cells = row.as_sequence().expect("a row");
            let count = cells[residual_column]
                .as_sequence()
                .expect("residual entries")
                .len();
            assert_eq!(cells[residual_count].as_i128(), Some(count as i128));
            residual_entries += count;
        }
        assert!(
            residual_entries < original_entries,
            "represented columns remove duplicate entries"
        );
        eprintln!(
            "ulbridge residual entries: {residual_entries} of {original_entries} content entries"
        );

        let records = profiled("ulbridge FieldRecord::new", || {
            canonical_rows
                .iter()
                .cloned()
                .map(|row| yggdryl::FieldRecord::new(&target, row).expect("a canonical record"))
                .collect::<Vec<_>>()
        });
        assert_eq!(records.len(), canonical_rows.len());

        let round_trip = profiled("ulbridge FieldRecord into_scalar", || {
            records
                .iter()
                .cloned()
                .map(yggdryl::FieldRecord::into_scalar)
                .collect::<Vec<_>>()
        });
        assert_eq!(round_trip, canonical_rows);

        let root = Arc::new(target.clone());
        let record_batches = profiled("ulbridge FieldRecord one-row Serie batch", || {
            records
                .iter()
                .cloned()
                .map(|record| {
                    yggdryl::Serie::from_scalars(Arc::clone(&root), [record.into_scalar()])
                        .expect("a canonical record")
                        .into_arrow_batch()
                        .expect("a one-row batch")
                })
                .collect::<Vec<_>>()
        });
        assert_eq!(record_batches.len(), ROWS);
        assert!(record_batches.iter().all(|batch| batch.num_rows() == 1));

        let parsed_batches = profiled("ulbridge batch door", || batches(&default));
        assert_eq!(
            parsed_batches
                .iter()
                .map(RecordBatch::num_rows)
                .sum::<usize>(),
            ROWS
        );
    }

    #[test]
    fn the_codec_refuses_the_session_traffic_and_reads_every_other_line() {
        let codec = codec();
        let lines = text_lines();
        assert_eq!(lines.len(), LINES);

        let read = line_messages(&codec);
        assert_eq!(read.len(), ROWS);
        for message in &read {
            let msgtype = message.header().msgtype();
            assert!(
                codec.reads_msgtype(msgtype),
                "the codec refuses {msgtype} and still read one"
            );
            assert!(!yggdryl::DEFAULT_REFUSED_MSGTYPES.contains(&msgtype));
        }

        // A caller that says it wants everything gets the session traffic too,
        // and that is the whole of the difference: the fifteen more lines are
        // heartbeats, one test request, and the rows that state no type.
        let every = codec.clone().with_exclude_msgtypes::<[&str; 0], &str>([]);
        let read_all = every
            .parse_text_lines(text_lines())
            .collect::<yggdryl::Result<Vec<_>>>()
            .expect("every line reads");
        assert_eq!(read_all.len(), EVERY_ROW);
        let mut refused: Vec<&str> = read_all
            .iter()
            .map(|message| message.header().msgtype())
            .filter(|msgtype| !codec.reads_msgtype(msgtype))
            .collect();
        refused.sort_unstable();
        assert_eq!(refused.len(), EVERY_ROW - ROWS);
        assert_eq!(refused[..4], ["", "", "", ""], "the rows stating no type");
        assert_eq!(refused[10..], ["0", "0", "0", "0", "1"]);
    }

    #[test]
    fn the_line_door_and_the_batch_door_land_the_same_rows() {
        let codec = codec();
        let held = super::format_target(codec.registry());
        let expected = line_messages(&codec)
            .iter()
            .map(|message| message.into_row(&held).expect("the fixed row"))
            .collect::<Vec<_>>();

        let batches = batches(&codec);
        let (schema, rows) = rows_of(&batches);
        assert_eq!(rows.len(), expected.len());
        // The batch door keeps the capture's own columns in front of the fixed
        // ones, so the two rows are lined up by the tag each column carries
        // rather than by position.
        for tag in yggdryl::fix_schema_tags() {
            // Tag 385 is the one column the two doors are allowed to disagree
            // on: an unmarked line takes the codec's own direction on the batch
            // door and states nothing on the line door.
            // Two columns the two doors are allowed to disagree on, both
            // capture facts rather than the message's: an unmarked line takes
            // the codec's own direction on the batch door and states none on
            // the line door, and only a read through a handle knows what object
            // the line came off.
            if tag == yggdryl::MSGDIRECTION_TAG_NAME.0 || tag == yggdryl::SOURCEURL_TAG_NAME.0 {
                continue;
            }
            let Some(mine) = yggdryl::fix_column_of(&held, tag) else {
                continue;
            };
            let theirs =
                yggdryl::fix_column_of(&schema, tag).expect("the batch keeps every column");
            for (at, row) in rows.iter().enumerate() {
                let want = expected[at].as_sequence().expect("a row")[mine].clone();
                assert_eq!(row[theirs], want, "row {at}, tag {tag}");
            }
        }
    }

    #[test]
    fn a_batch_bound_changes_the_batches_and_never_the_rows() {
        let codec = codec();
        let one_batch = batches(&codec);
        let (_, expected) = rows_of(&one_batch);
        assert_eq!(one_batch.len(), 1, "the whole capture fits one batch");
        assert_eq!(expected.len(), ROWS);

        // Either bound closes a batch, whichever it reaches first, and neither
        // changes what the rows hold.
        for split in [
            codec.clone().with_batch_byte_size(4 * 1024),
            codec.clone().with_batch_row_size(8),
        ] {
            let batches = batches(&split);
            assert!(batches.len() > 1, "{} batches", batches.len());
            let (_, rows) = rows_of(&batches);
            assert_eq!(rows, expected);
            for batch in &batches {
                assert_eq!(batch.schema(), batches[0].schema());
            }
        }
    }

    #[test]
    fn a_source_error_moves_through_the_line_stream_which_then_fuses() {
        const FAILED_AFTER: usize = 5;
        let codec = codec();
        let lines = text_lines();
        let before = codec.parse_text_lines(&lines[..FAILED_AFTER]).count();
        let marker = Arc::new(());
        let mut items = lines
            .into_iter()
            .map(Ok)
            .collect::<Vec<yggdryl::Result<TextLine>>>();
        items.insert(FAILED_AFTER, Err(super::batch::source_failure(&marker)));
        let mut items = items.into_iter();
        let pulls = std::rc::Rc::new(std::cell::Cell::new(0_usize));
        let counted = std::rc::Rc::clone(&pulls);
        let source = std::iter::from_fn(move || {
            counted.set(counted.get() + 1);
            items.next()
        });
        let mut stream = codec.parse_text_lines(source);
        // Nothing is pulled before the first message is asked for.
        assert_eq!(pulls.get(), 0);
        for _ in 0..before {
            stream.next().unwrap().unwrap();
        }
        super::batch::same_source_failure(stream.next().unwrap().unwrap_err(), &marker);
        assert_eq!(pulls.get(), FAILED_AFTER + 1);
        // The source's own error moves through and never advances the walk: the
        // lines behind it are read as if it had not been there.
        let after = stream
            .by_ref()
            .try_fold(0_usize, |read, message| message.map(|_| read + 1))
            .unwrap();
        assert_eq!(before + after, ROWS);
        // Exhaustion is what fuses it, and a fused stream pulls nothing more.
        let exhausted = pulls.get();
        assert_eq!(exhausted, LINES + 2);
        assert!(stream.next().is_none());
        assert_eq!(pulls.get(), exhausted);
    }

    #[test]
    fn a_bridge_frame_carrying_a_row_is_the_type_that_row_states() {
        let codec = codec();
        // `35=UL` is the envelope the bridge sent and `MSGTYPE=` inside its
        // `XmlData` is what the row it carried calls itself. The row's type is
        // the message's, which is also the type the codec filters on.
        let message = line_messages(&codec)
            .into_iter()
            .find(|message| {
                message
                    .get_by_tag(213)
                    .and_then(|held| held.as_bytes().map(<[u8]>::to_vec))
                    .is_some_and(|held| {
                        String::from_utf8_lossy(&held).contains("MSGTYPE=tradecapturereport")
                    })
            })
            .expect("the trade capture frame");
        assert_eq!(message.by_tag(35).unwrap().as_str(), Some("AE"));
        // The envelope's own version still says what the session speaks.
        assert_eq!(message.by_tag(8).unwrap().as_str(), Some("FIX.4.2"));

        // The bridge packs an occurrence's members behind the two glyphs a log
        // viewer prints for its control bytes, and a group packed inside an
        // occurrence nests inside it rather than beside it, at every depth:
        // the leg carries its own allocations, the side its parties, and a
        // party its sub-identifiers.
        assert_eq!(message.by_tag(555).unwrap().as_i64(), Some(1), "NoLegs");
        assert_eq!(message.by_tag(552).unwrap().as_i64(), Some(1), "NoSides");
        assert_eq!(
            message
                .by_path(&path("TrdInstrmtLegGrp[0].LegAllocs[0].LegAllocQty"))
                .unwrap(),
            super::decimal("600")
        );
        assert_eq!(
            message
                .by_path(&path("TrdCapRptSideGrp[0].Parties"))
                .unwrap()
                .as_sequence()
                .map(<[Scalar]>::len),
            Some(7)
        );
        // One of the seven packs two sub-identifiers of its own, and they are
        // the party's rather than the side's.
        let subs: Vec<usize> = (0..7)
            .map(|index| {
                message
                    .get_by_path(&path(&format!(
                        "TrdCapRptSideGrp[0].Parties[{index}].PartySubIDs"
                    )))
                    .and_then(|held| held.as_sequence().map(<[Scalar]>::len))
                    .unwrap_or_default()
            })
            .collect();
        assert_eq!(
            subs,
            [1, 2, 0, 1, 0, 1, 0],
            "the sub-identifiers each party packs"
        );
        // A key the venue spelled under a namespace of its own is the bridge's
        // metadata rather than a child of the row, dot and all.
        assert_eq!(
            message
                .metadata()
                .get("metal.loco")
                .map(smol_str::SmolStr::as_str),
            Some("LN")
        );
    }

    #[test]
    fn a_parse_fills_the_crate_columns_the_line_only_implied() {
        let codec = codec();
        // The first fill the bridge received: 21 shares at 83.08.
        let fill = line_messages(&codec)
            .into_iter()
            .find(|message| {
                message.header().msgtype() == "8"
                    && message.get_by_tag(32) == Some(super::decimal("21"))
            })
            .expect("the fill of 21 shares");

        // A parse enriches, so the derived facts are on the message the line
        // door answered and no second pass adds them: the amount the fill comes
        // to, the currency it settles in, the instrument's ISIN under its stated
        // source and the country that ISIN opens with, the product the
        // dictionary files the security type under, the market the line names
        // first, and the ranked state it reports.
        // Exact, because a quantity times a price is an exact number and no
        // longer a float that has to be compared within a tolerance.
        assert_eq!(fill.by_tag(381).unwrap(), super::decimal("1744.68"));
        assert_eq!(fill.by_tag(120).unwrap().as_str(), Some("CHF"));
        assert_eq!(
            fill.by_tag(44).unwrap().as_decimal(),
            Some((yggdryl::i256::from_i128(83_080_000_000_000_000_000), 18)),
            "the price the line stated, exact"
        );
        assert_eq!(
            fill.get_price().to_string(),
            "83.08",
            "and the price the message is about, read off it"
        );
        assert_eq!(fill.get_securityids().get("ISIN"), Some("CH0012221716"));
        assert_eq!(fill.by_tag(470).unwrap().as_str(), Some("CH"));
        assert_eq!(fill.by_tag(460).unwrap().as_i128(), Some(5), "Product");
        assert_eq!(fill.get_miccode().map(|held| held.as_str()), Some("XSWX"));
        assert_eq!(
            Some(fill.get_state().clone()),
            yggdryl::State::from_spelling("1"),
        );
        // A stated value is never a derived one: the line said 260 remain.
        assert_eq!(fill.by_tag(151).unwrap(), super::decimal("260"));

        // An identifier the check digit does not close is no identifier: the
        // anonymized line names one, and nothing is read off it.
        let masked = line_messages(&codec)
            .into_iter()
            .find(|message| {
                message
                    .get_by_tag(48)
                    .and_then(|held| held.as_str().map(ToOwned::to_owned))
                    == Some("XX0000000001".to_owned())
            })
            .expect("the anonymized line");
        assert_eq!(
            masked.get_securityids().get("ISIN"),
            None,
            "no ISIN off a masked one"
        );
        assert!(
            masked.get_by_tag(470).is_none_or(|held| held.is_null()),
            "and no country either"
        );
    }

    fn wire_tokens(wire: &str) -> Vec<&str> {
        let mut tokens = wire
            .split('|')
            .filter(|token| !token.is_empty())
            .collect::<Vec<_>>();
        tokens.sort_unstable();
        tokens
    }

    fn unresolved_occurrences(
        message: &FixMsg,
    ) -> std::collections::BTreeMap<&str, Vec<&yggdryl::FixEntry>> {
        let mut occurrences = std::collections::BTreeMap::<_, Vec<_>>::new();
        for entry in message.entries().iter().filter(|entry| entry.tag() == 0) {
            occurrences.entry(entry.name()).or_default().push(entry);
        }
        occurrences
    }

    #[test]
    fn the_writer_re_emits_each_rows_own_wire_and_none_of_the_captures_columns() {
        let codec = codec();
        let source_messages = line_messages(&codec);
        assert_eq!(source_messages.len(), ROWS);
        let target = super::format_target(codec.registry());
        let protocol_columns = target
            .fields()
            .iter()
            .enumerate()
            .filter_map(|(at, field)| {
                let fix = field.as_fix();
                fix.counter()
                    .expect("a valid group counter")
                    .or(fix.tag().expect("a valid FIX tag"))
                    // Crate facts and capture-derived MsgDirection are outside
                    // the protocol content written to the wire.
                    .filter(|tag| {
                        !(yggdryl::CRATE_TAG_MIN..=yggdryl::CRATE_TAG_MAX).contains(tag)
                            && *tag != yggdryl::MSGDIRECTION_TAG_NAME.0
                    })
                    .map(|tag| (at, field.name().to_owned(), tag))
            })
            .collect::<Vec<_>>();
        assert!(
            !protocol_columns.is_empty(),
            "the fixed schema has FIX columns"
        );

        let mut written: Vec<u8> = Vec::new();
        let filled = codec
            .parse_text_arrow_reader(source().read_arrow_reader(&reading()).expect("a reader"))
            .expect("the batch reader opens");
        let rows = codec
            .clone()
            .with_separator(b'|')
            .write_arrow_reader(filled, &mut written)
            .expect("the capture writes");
        assert_eq!(rows, ROWS as u64);

        // A row in is a line out: root entries may move between residual and
        // projected schema order, but every tag=value token remains exactly once.
        let wires = source_messages
            .iter()
            .map(|message| String::from_utf8_lossy(&message.into_bytes(b'|')).into_owned())
            .collect::<Vec<_>>();
        let written = std::str::from_utf8(&written)
            .expect("the wire is text here")
            .lines()
            .collect::<Vec<_>>();
        assert_eq!(wires.len(), ROWS);
        assert_eq!(written.len(), ROWS);
        for (at, (message, wire)) in source_messages.iter().zip(&wires).enumerate() {
            // The capture's columns are the capture's: the body the line was
            // read from, what the reader classified it as and the bridge's row
            // header are not content, so none of them reaches a counterparty.
            for carried in [
                "|body=",
                "|mimetype=",
                "|timestamp=",
                "|level=",
                "|msgthreadid=",
            ] {
                assert!(!written[at].contains(carried), "row {at}: {}", written[at]);
            }
            let source_tokens = wire_tokens(wire);
            let stated_tags = source_tokens
                .iter()
                .filter_map(|token| token.split_once('=')?.0.parse::<i32>().ok())
                .collect::<Vec<_>>();
            assert_eq!(
                source_tokens,
                wire_tokens(written[at]),
                "row {at} lost or duplicated a scalar token"
            );

            // Root ordering is intentionally not a row contract. Reparse the
            // emitted frame and compare protocol columns under the fixed schema,
            // keeping nested scopes and repeated scalar/group order exact.
            let reparsed = codec
                .sole_line(written[at].as_bytes())
                .unwrap_or_else(|error| panic!("row {at} did not parse: {error}"));
            let reparsed_unknowns = unresolved_occurrences(&reparsed);
            for (name, occurrences) in unresolved_occurrences(message) {
                assert_eq!(
                    reparsed_unknowns.get(name),
                    Some(&occurrences),
                    "row {at} changed unresolved {name} occurrence order or content"
                );
            }
            // Pipes inside this length-delimited value are data, not root
            // boundaries; the token multiset alone cannot prove their order.
            assert_eq!(
                reparsed.get_by_tag(213),
                message.get_by_tag(213),
                "row {at} changed XmlData bytes"
            );
            let source_row = message
                .into_row(&target)
                .unwrap_or_else(|error| panic!("row {at} did not project: {error}"));
            let reparsed_row = reparsed
                .into_row(&target)
                .unwrap_or_else(|error| panic!("row {at} did not reproject: {error}"));
            let source_cells = source_row.as_sequence().expect("a fixed row");
            let reparsed_cells = reparsed_row.as_sequence().expect("a fixed row");
            for (column, name, tag) in &protocol_columns {
                // A fresh wire parse can infer a previously absent scalar from
                // residual content. Conversely, capture fills such as Text can
                // have a protocol tag without being emitted in this frame.
                if source_cells[*column].is_null() || !stated_tags.contains(tag) {
                    continue;
                }
                let source = &source_cells[*column];
                let reparsed = &reparsed_cells[*column];
                // A bridge spelling such as `day` and the wire code `0` name
                // one value in the registry's shared vocabulary.
                let same_code = match (
                    source.as_str(),
                    reparsed.as_str(),
                    codec.registry().codeset_of(&target.fields()[*column]),
                ) {
                    (Some(source), Some(reparsed), Some(codes)) => {
                        matches!((codes.code_value(source), codes.code_value(reparsed)),
                            (Some(source), Some(reparsed)) if source == reparsed)
                    }
                    _ => false,
                };
                assert!(
                    source == reparsed || same_code,
                    "row {at} changed FIX column {name} from {source:?} to {reparsed:?}: {}",
                    written[at]
                );
            }
        }

        // Every frame the bridge wrote with `|` comes back with all of its tokens,
        // including frames whose nested groups changed root entry order.
        let framed = wires.iter().filter(|wire| wire.contains("|10=")).count();
        assert!(framed >= 11, "{framed} frames checked");
    }
}

mod pipeline {
    use super::SoleMessage;

    use std::sync::Arc;

    use arrow_array::Array as _;
    use arrow_array::RecordBatch;
    use arrow_array::cast::AsArray;
    use yggdryl::holder::Buffer;
    use yggdryl::media::RecordOptions;
    use yggdryl::text::TextOptions;
    use yggdryl::{
        FixCodec, FixRegistry, IOMedia, Scalar, TimeUnit, Timezone, Url, fix_schema,
        fix_schema_carrying,
    };

    /// The committed dictionary, in which every line of the bridge's resolves.
    fn registry() -> Arc<FixRegistry> {
        super::committed_registry()
    }

    /// The Jolokia answer, which is the one line that is a document: a body the
    /// codec does not read, and so one `unknown` row with no entries.
    const RESPONSE: &str = concat!(
        r#"2026-08-14 06:46:22.255 [23] [Jolokia] (DEBUG) Response: {"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:name=Router_TradeCapture,plugin-type=FIX,type=Plugin","type":"read"},"#,
        r#""value":{"SenderCompID":"CLIENTFIS","TargetCompID":"VENUEADC","BeginString":"FIX.4.2","Category":"Fix TradeCapture","PrimaryHost":"10.20.30.40","PrimaryPort":9726,"BackupHost":null,"BackupPort":-1,"#,
        r#""CurrentHost":"10.20.30.40","CurrentPort":9726,"IncomingMsgSeqNum":4507,"OutgoingMsgSeqNum":571,"LogLevel":-1,"PriorityLevel":5,"NotificationsStatus":false,"NeedReload":false,"#,
        r#""Name":"Router_TradeCapture","Version":"4.7.0","State":"logged","Type":"I","ExtendedActions":[{"name":"send-test-request","enabled":true}],"Enrichments":[]},"timestamp":1755153982,"status":200}"#,
    );

    /// A heartbeat the bridge sent, under a bracket holding the thread alone.
    const HEARTBEAT: &str = "2026-08-14 06:46:30.416 [15261] [OMS_X1_TradeCapture] (DEBUG) Sending : 8=FIX.4.4|9=68|35=0|49=CLIAUDITX1|56=OMSAUDITX1|34=696|52=20260814-04:46:30.415655|10=159|";

    /// A fill the bridge received, wide enough to fill the body columns.
    const FILL: &str = "2026-08-14 06:46:36.887 [653] [Spot_FX_TradeCapture] (INFO) Receiving : 8=FIX.4.2|9=0322|35=8|34=4507|49=VENUEADC|56=CLIENTFIS|52=20260814-04:46:36|1=client|6=547.771791547861|11=20260814_TP1_CLIENT_1003|14=982|15=INR|17=E-20260814-4507|31=547.77|32=982|37=O-20260814-1003|38=982|39=2|40=1|44=547.771791547861|48=XX0000000001|54=1|55=EXAMPLECO|58=Filled|59=0|60=20260814-04:46:36|75=20260814|150=2|151=0|10=197|";

    /// The same fill as the bridge routes it, keyed by name, under a bracket
    /// stating the session, the message context and the sequence number.
    const ROUTED: &str = "2026-08-14 06:46:37.153 [15333-e7254b22:9f015ee861:4507] [Broker_DarkPool_TradeCapture] (DEBUG) RouteMessage : ACCOUNT=client|AVGPX=547.771791547861|CLORDID=20260814_TP1_CLIENT_1003|CUMQTY=982|CURRENCY=INR|EXECBROKER=BRKR|EXECTYPE=2|LASTPX=547.77|LASTQTY=982|LEAVESQTY=0|MSGTYPE=8|ORDERQTY=982|ORDSTATUS=2|ORDTYPE=1|SIDE=1|SYMBOL=EXAMPLECO|TRANSACTTIME=20260814-04:46:36|";

    /// Every line the log interleaves, in the order it writes them.
    const CAPTURE: [&str; 11] = [
        "2026-08-14 06:46:22.255 [23] [Jolokia] (DEBUG) URI: /jolokia/read/com.ullink.ulbridge.sessioninterfaces.plugins:name=Router_TradeCapture,plugin-type=FIX,type=Plugin",
        "2026-08-14 06:46:22.255 [23] [Jolokia] (DEBUG) Request: JmxReadRequest[attribute=null, objectName = com.ullink.ulbridge.sessioninterfaces.plugins:name=Router_TradeCapture,plugin-type=FIX,type=Plugin]",
        "2026-08-14 06:46:22.255 [23] [Jolokia] (DEBUG) Execution time: 0 ms",
        RESPONSE,
        HEARTBEAT,
        "2026-08-14 06:46:30.947 [402-e7254b20:9f015ed023:935] [ULMSG_BROKER_TO_DMZ] (DEBUG) Receiving : 8=FIX.4.2|9=55|35=0|49=ULB_DMZ|56=ULB_BRK|34=935|52=20260814-04:46:30.967|10=186|",
        FILL,
        ROUTED,
        "2026-08-14 06:46:37.153 [15333-e7254b22:9f015ee861:4507] [Broker_DarkPool_TradeCapture] (INFO) Filtering - Message for RiskMonitor",
        "2026-08-14 06:46:37.153 [15333-e7254b22:9f015ee861:4507] [EnrichmentManager] (INFO) Enrichment execution[&SetEnv, &Broker_DarkPool_TradeCapture]",
        "2026-08-14 06:46:37.153 [15333-e7254b22:9f015ee861:4507] [ULBridge] (INFO) Execution report (ClOrderID : 20260814_TP1_CLIENT_1003) without any route so using not persisted route: [UNDEFINED] --> [Broker_DarkPool_TradeCapture]",
    ];

    /// Which capture lines carry a message, in the order they carry them.
    /// Six of the eleven carry none, and each for the same
    /// reason - the line opens no frame, states no bridge pair and carries no
    /// document. Lines 1 and 2 hold runs of named pairs -
    /// `name=Router_TradeCapture,plugin-type=FIX,type=Plugin` and
    /// `attribute=null, objectName = ...` - but a comma and a space are not a
    /// separator a line names and the bridge marked no key, so they are prose
    /// carrying an `=`; lines 3, 9, 10 and 11 are sentences with no pair in them
    /// at all. Every one of the six used to be a row holding an entry-less
    /// `unknown`.
    const CARRYING: [usize; 5] = [3, 4, 5, 6, 7];

    /// How many rows the batch door answers for this capture: one per message,
    /// never one per line.
    const MESSAGES: usize = CARRYING.len();

    /// Where each shape sits among the messages, which is where its row sits in
    /// every batch the codec answers.
    const RESPONSE_ROW: usize = 0;
    const HEARTBEAT_ROW: usize = 1;
    const RELAY_ROW: usize = 2;
    const FILL_ROW: usize = 3;
    const ROUTED_ROW: usize = 4;

    /// The log as the bytes a `.log` file holds.
    fn corpus(lines: &[&str]) -> Buffer {
        let mut bytes = Vec::new();
        for line in lines {
            bytes.extend_from_slice(line.as_bytes());
            bytes.push(b'\n');
        }
        Buffer::from_bytes(bytes).with_media_type(
            Url::from_str("file:///bridge.log")
                .expect("a URL")
                .media_type(),
        )
    }

    /// The text options a bridge log is read under: the bridge's own row header,
    /// its clock read in UTC, and each line numbered, classified and read for
    /// its direction.
    fn text_options() -> TextOptions {
        let mut options = TextOptions::new()
            .try_with_rowheader(yggdryl::ULBRIDGE_ROWHEADER)
            .expect("the row header compiles")
            .with_timezone(Timezone::UTC);
        options.start_rownum = Some(1);
        options.parse_mimetype = true;
        options
    }

    /// The same, as the record options a read takes.
    fn text() -> RecordOptions {
        text_options().into()
    }

    /// The codec: over the committed dictionary, with nothing pinned and
    /// nothing refused.
    ///
    /// Four of this capture's five messages are session traffic or a document -
    /// two heartbeats and a Jolokia answer - which is what a live read refuses
    /// through `DEFAULT_REFUSED_MSGTYPES`. A capture written to interleave every
    /// shape a bridge writes is asking for all of them, and says so here; what
    /// the default leaves out is pinned by
    /// `the_default_read_answers_only_the_two_business_messages`.
    fn codec() -> FixCodec {
        super::fixed_codec(registry()).with_exclude_msgtypes::<[&str; 0], &str>([])
    }

    /// The first stage alone, as one batch: what the text reader hands the codec.
    fn text_stage(lines: &[&str]) -> RecordBatch {
        let batches: Vec<RecordBatch> = corpus(lines)
            .read_arrow_reader(&text())
            .expect("a reader")
            .map(|batch| batch.expect("a batch"))
            .collect();
        assert_eq!(batches.len(), 1, "one batch, under the byte target");
        batches.into_iter().next().expect("the batch")
    }

    /// The whole path, as the batches it answers - which is none where the lines
    /// carry no message at all.
    fn read_batches(lines: &[&str]) -> Vec<RecordBatch> {
        codec()
            .parse_text_arrow_reader(corpus(lines).read_arrow_reader(&text()).expect("a reader"))
            .expect("the batch reader opens")
            .map(|batch| batch.expect("a batch"))
            .collect()
    }

    /// The whole path, as one batch: the capture is far under the byte target.
    fn read(lines: &[&str]) -> RecordBatch {
        let batches = read_batches(lines);
        assert_eq!(batches.len(), 1, "one batch, under the byte target");
        batches.into_iter().next().expect("the batch")
    }

    /// Both readings of one object: what the text reader hands the codec, and
    /// what the codec answers - over the same buffer, because a line's cross
    /// code is the object it was read from and its identity follows from that,
    /// so two buffers over the same bytes are two chains.
    fn staged_and_read(lines: &[&str]) -> (RecordBatch, RecordBatch) {
        let source = corpus(lines);
        let staged: Vec<RecordBatch> = source
            .read_arrow_reader(&text())
            .expect("a reader")
            .map(|batch| batch.expect("a batch"))
            .collect();
        let held: Vec<RecordBatch> = codec()
            .parse_text_arrow_reader(source.read_arrow_reader(&text()).expect("a reader"))
            .expect("the batch reader opens")
            .map(|batch| batch.expect("a batch"))
            .collect();
        assert_eq!(staged.len(), 1, "one staged batch, under the byte target");
        assert_eq!(held.len(), 1, "one batch, under the byte target");
        (
            staged.into_iter().next().expect("the staged batch"),
            held.into_iter().next().expect("the batch"),
        )
    }

    /// How many messages each capture line carries, read by the codec over the
    /// very bodies the text reader hands it.
    fn messages_per_line(lines: &[&str]) -> Vec<usize> {
        let codec = codec();
        column(&text_stage(lines), "body")
            .iter()
            .map(|body| {
                codec
                    .parse_line(body.as_str().expect("a body").as_bytes())
                    .expect("the line reads")
                    .count()
            })
            .collect()
    }

    /// One column of one batch, by position, as the values it holds.
    fn column_at(batch: &RecordBatch, at: usize) -> Vec<Scalar> {
        let held =
            yggdryl::Serie::from_arrow_batch(None, batch, yggdryl::ArrowCastOptions::default())
                .expect("the batch reads");
        held.rows()
            .iter()
            .map(|row| row.as_sequence().expect("a row")[at].clone())
            .collect()
    }

    /// One column of one batch, by name.
    fn column(batch: &RecordBatch, name: &str) -> Vec<Scalar> {
        let at = batch
            .schema()
            .index_of(name)
            .unwrap_or_else(|_| panic!("a {name} column"));
        column_at(batch, at)
    }

    /// One column of one batch, by the tag its field carries.
    fn tag_column(batch: &RecordBatch, tag: i32) -> Vec<Scalar> {
        column_at(batch, super::tag_index(batch, tag))
    }

    /// Each value's text, null where it states none.
    fn texts(held: &[Scalar]) -> Vec<Option<String>> {
        held.iter()
            .map(|held| held.as_str().map(ToOwned::to_owned))
            .collect()
    }

    /// One column's text per row, by name.
    fn text_column(batch: &RecordBatch, name: &str) -> Vec<Option<String>> {
        texts(&column(batch, name))
    }

    /// One column's text per row, by the tag its field carries.
    fn tag_text(batch: &RecordBatch, tag: i32) -> Vec<Option<String>> {
        texts(&tag_column(batch, tag))
    }

    #[test]
    fn the_schema_is_the_captures_columns_then_the_fixed_ones_and_never_depends_on_the_data() {
        let registry = registry();
        let held = read(&CAPTURE);
        let schema = held.schema();
        let names: Vec<&str> = schema
            .fields()
            .iter()
            .map(|held| held.name().as_str())
            .collect();

        // The text reader's own columns lead the row - what the line was
        // classified as, the line itself and the header's captures - and the
        // fixed columns follow. A capture whose folded name a fixed column
        // takes is not carried in front, it fills that column: the reader's
        // `msgtype`, and the header's `bridgesessionid`, `msgctxid` and
        // `msgseqnum`, each named for the field it fills. What is left in
        // front is what no column is spelled for - the clock the bridge
        // printed, which is the line's own text and not the message's
        // `SendingTime`, the thread that wrote the line and its level. Where
        // the line came out of, which line it was and when it was written lead
        // nothing any more: they are `crosscode`, `seqnum` and `currunix`, the
        // event columns both halves already open with, so they stand in the
        // fixed band with the rest of them.
        assert_eq!(
            &names[..5],
            ["mimetype", "body", "timestamp", "msgthreadid", "level"],
            "{names:?}"
        );
        // The crate's own clocks open the fixed columns; the standard header
        // follows them.
        let at = |name: &str| {
            names
                .iter()
                .position(|held| *held == name)
                .unwrap_or_else(|| panic!("a {name} column in {names:?}"))
        };
        for pair in ["level", "currunix", "creaunix", "prevunix"].windows(2) {
            assert!(at(pair[0]) < at(pair[1]), "{pair:?} in {names:?}");
        }
        let header = names
            .iter()
            .position(|held| *held == "beginstring")
            .expect("the header opens");
        assert_eq!(
            &names[header..header + 4],
            ["beginstring", "msgtype", "msgcat", "msgseqnum"],
            "{names:?}"
        );
        for once in [
            "msgtype",
            "crosscode",
            "seqnum",
            "currunix",
            "timestamp",
            "msgsessionid",
            "msgctxid",
            "msgpluginid",
            "msgseqnum",
        ] {
            assert_eq!(
                names.iter().filter(|held| **held == once).count(),
                1,
                "{once} is one column"
            );
        }
        // Which way a line moved is FIX's own `msgdirection`.
        assert!(names.contains(&"msgdirection"), "{names:?}");
        assert!(!names.contains(&"direction"), "{names:?}");
        assert_eq!(names.last(), Some(&"fixentries"));

        // The timestamp capture was typed from its pattern before a byte was
        // read and took the zone the options declared; updatedat is independently
        // settled as an exact nanosecond UTC instant on every row.
        let stage = text_stage(&CAPTURE);
        let captured = stage.schema();
        let clock = captured
            .field_with_name("timestamp")
            .expect("the timestamp capture");
        assert!(
            matches!(
                clock.data_type(),
                arrow_schema::DataType::Timestamp(_, Some(_))
            ),
            "{clock:?}"
        );
        let stamp = schema
            .field_with_name("currunix")
            .expect("the clock column");
        assert!(
            matches!(
                stamp.data_type(),
                arrow_schema::DataType::Timestamp(_, Some(_))
            ),
            "{stamp:?}"
        );
        assert!(!stamp.is_nullable(), "every row is stamped");

        // A capture whose lines carry no message is no rows and so no batch at
        // all: the first two lines state runs of named pairs whose
        // only separators are a comma and a space - never a separator a line
        // names - and open no frame, so the batch door answers nothing for
        // either. They used to be two rows holding an entry-less `unknown`.
        assert_eq!(messages_per_line(&CAPTURE[..2]), [0, 0]);
        assert!(
            read_batches(&CAPTURE[..2]).is_empty(),
            "no message, no row, and with no row no batch"
        );

        // The shape is a function of the options and the dictionary alone: a
        // capture of two lines and one of eleven answer the same schema, and it
        // is exactly the composition the two halves publish.
        let two = read(&CAPTURE[CARRYING[RESPONSE_ROW]..=CARRYING[HEARTBEAT_ROW]]);
        assert_eq!(two.num_rows(), 2, "the document and the heartbeat");
        assert_eq!(two.schema(), held.schema());
        let composed = fix_schema_carrying(
            &text_options().source_field().expect("the text root"),
            &fix_schema(&registry, "fix").expect("the fixed root"),
        )
        .expect("the composition");
        let composed: Vec<&str> = composed.fields().iter().map(yggdryl::Field::name).collect();
        assert_eq!(composed, names, "the composition the two halves publish");
    }

    #[test]
    fn a_message_in_is_a_row_out_and_the_captures_own_columns_ride_in_front() {
        let (stage, read) = staged_and_read(&CAPTURE);

        // Per line, what the line carries: the Jolokia answer's
        // document, three framed messages and the bridge row, and nothing at all
        // for the other six. The `URI:` and `Request:` lines name no separator
        // for their runs of pairs - a comma and a space are never one, and the
        // bridge marked no key - and the four sentences hold no pair and no
        // frame; none of the six opens a frame or carries a document, so none of
        // them states a message.
        assert_eq!(
            messages_per_line(&CAPTURE),
            [0, 0, 0, 1, 1, 1, 1, 1, 0, 0, 0]
        );

        // The text reader is what answers one row per line; the batch door
        // answers one row per message, so eleven lines are five rows.
        assert_eq!(stage.num_rows(), CAPTURE.len());
        assert_eq!(read.num_rows(), MESSAGES);

        // Which line a message came out of is no longer a number riding in
        // front of the row. A line's place is its own `seqnum` - the text
        // stage numbers every line from one, as the options said - and the
        // fixed row's `seqnum` is the *message's* place, which none of these
        // five states, so the column is null on every row rather than
        // repeating the line's. What ties a row to its line is the line's
        // identity: the row names it under `srcuuids`, and the six silent
        // lines are simply missing from that.
        let stage_seqnum = stage
            .column(stage.schema().index_of("seqnum").expect("seqnum"))
            .as_primitive::<arrow_array::types::UInt64Type>();
        assert_eq!(stage_seqnum.null_count(), 0, "the count opened at one");
        assert_eq!(
            stage_seqnum.values().iter().copied().collect::<Vec<_>>(),
            (1..=CAPTURE.len() as u64).collect::<Vec<u64>>()
        );
        assert!(
            column(&read, "seqnum").iter().all(Scalar::is_null),
            "a message the bridge logged states no place of its own"
        );
        let lines = column(&stage, "curruuid");
        let sources = column(&read, "srcuuids");
        assert_eq!(sources.len(), MESSAGES);
        for (row, line) in CARRYING.iter().enumerate() {
            assert_eq!(
                sources[row]
                    .as_sequence()
                    .expect("the line the row was read from"),
                [lines[*line].clone()],
                "row {row} names line {line}"
            );
        }

        // The row header's captures survive the codec untouched: the thread that
        // wrote the line and the level, and the bracket's sequence number - null
        // where the bracket held only the thread.
        assert_eq!(
            text_column(&read, "level")[FILL_ROW].as_deref(),
            Some("INFO")
        );
        let thread = column(&read, "msgthreadid");
        assert_eq!(thread[ROUTED_ROW].as_i64(), Some(15_333));
        assert_eq!(thread[HEARTBEAT_ROW].as_i64(), Some(15_261));
        // The bracket's sequence number is FIX's own `MsgSeqNum(34)`, so it fills
        // that column rather than riding in front of the row - and only where the
        // message states none. The routed line is keyed by name and spells no
        // `34`, so the bracket's is what it reads; the heartbeat spells its own
        // and keeps it; the Jolokia response has neither.
        let seq = tag_column(&read, 34);
        assert_eq!(seq[ROUTED_ROW].as_i64(), Some(4_507));
        assert_eq!(seq[HEARTBEAT_ROW].as_i64(), Some(696));
        assert!(seq[RESPONSE_ROW].is_null(), "no session, no sequence");

        // All three parts of the bracket are captures named after the fields they
        // fill, so all three land in those columns rather than in front: the
        // routed row's bracket stated them, the heartbeat's did not.
        let session = tag_text(&read, yggdryl::MSGSESSIONID_TAG_NAME.0);
        let context = tag_text(&read, yggdryl::MSGCTXID_TAG_NAME.0);
        assert_eq!(session[ROUTED_ROW].as_deref(), Some("e7254b22"));
        assert_eq!(context[ROUTED_ROW].as_deref(), Some("9f015ee861"));
        assert_eq!(session[HEARTBEAT_ROW], None);
        assert_eq!(context[HEARTBEAT_ROW], None);

        // The plugin that logged a line is a capture named after the crate's own
        // column, so it lands there rather than in front - on every framed line,
        // as the bracket spells it - and it is never anything else: the session
        // names a line moved between are what the line itself spells, and no
        // line here spells one, nor which plugin the message came through
        // before.
        let plugin = tag_text(&read, yggdryl::MSGPLUGINID_TAG_NAME.0);
        assert_eq!(
            plugin[HEARTBEAT_ROW].as_deref(),
            Some("OMS_X1_TradeCapture")
        );
        assert_eq!(plugin[FILL_ROW].as_deref(), Some("Spot_FX_TradeCapture"));
        assert_eq!(
            plugin[ROUTED_ROW].as_deref(),
            Some("Broker_DarkPool_TradeCapture")
        );
        assert_eq!(plugin[RESPONSE_ROW].as_deref(), Some("Jolokia"));
        // The session instance the bridge handled a line on is the bracket's
        // own, and no line here spells one.
        let session = tag_text(&read, yggdryl::MSGSESSIONID_TAG_NAME.0);
        assert_eq!(session[HEARTBEAT_ROW], None);

        // The bracket's sequence number fills `MsgSeqNum` where the line stated
        // none - the routed row is keyed by name and carries no 34 - and never
        // where it did: the heartbeat keeps its own.
        let msgseqnum = tag_column(&read, 34);
        assert_eq!(msgseqnum[ROUTED_ROW].as_i64(), Some(4_507));
        assert_eq!(msgseqnum[HEARTBEAT_ROW].as_i64(), Some(696));

        // What each line was, read once by the text reader and carried through.
        let mimetype = text_column(&read, "mimetype");
        // A Jolokia answer is JSON, which is what it is: the classifier locates
        // the document behind the prose and names nothing about what it is for.
        assert_eq!(mimetype[RESPONSE_ROW].as_deref(), Some("application/json"));
        assert_eq!(mimetype[HEARTBEAT_ROW].as_deref(), Some("text/fix"));
        assert_eq!(mimetype[FILL_ROW].as_deref(), Some("text/fix"));
        assert_eq!(mimetype[RELAY_ROW].as_deref(), Some("text/fix"));
        assert_eq!(mimetype[ROUTED_ROW].as_deref(), Some("text/ullink"));
        // What a line is classified as and whether it carries a message are two
        // different answers. The text reader still calls line 1
        // `application/octet-stream` and line 2 `text/key-value` - a run of named
        // pairs is what a classifier can see without parsing - and neither line
        // carries a message, so neither reaches the batch: the five rows here are
        // the document, the three frames and the bridge row, and nothing else.
        let classified = text_column(&stage, "mimetype");
        assert_eq!(classified[0].as_deref(), Some("application/octet-stream"));
        assert_eq!(classified[1].as_deref(), Some("text/key-value"));
        assert_eq!(mimetype.len(), MESSAGES);

        // Which way each message moved is FIX's own tag 385: the
        // verb in front of the frame, and the `Response:` Jolokia wrote in front
        // of the document. The codec's pin for a line that states
        // no direction has nothing left to fill on this capture - the lines that
        // stated none were the sentences, and a sentence is no row
        // - so every row here states the direction its own line spelled.
        let fix_direction = tag_text(&read, yggdryl::MSGDIRECTION_TAG_NAME.0);
        assert_eq!(fix_direction[HEARTBEAT_ROW].as_deref(), Some("S"));
        assert_eq!(fix_direction[FILL_ROW].as_deref(), Some("R"));
        assert_eq!(fix_direction[RESPONSE_ROW].as_deref(), Some("R"));
        assert_eq!(fix_direction[RELAY_ROW].as_deref(), Some("R"));
        assert_eq!(fix_direction[ROUTED_ROW].as_deref(), Some("S"));
    }

    #[test]
    fn every_framed_line_fills_its_tag_columns_typed() {
        let read = read(&CAPTURE);

        let msgtype = tag_text(&read, 35);
        assert_eq!(msgtype[HEARTBEAT_ROW].as_deref(), Some("0"));
        assert_eq!(msgtype[FILL_ROW].as_deref(), Some("8"));
        assert_eq!(
            msgtype[ROUTED_ROW].as_deref(),
            Some("8"),
            "a bridge row names its type"
        );
        // `unknown` names a frame, a bridge row or a document that states no
        // type - never a line that states no frame. The capture's
        // sentences used to be `unknown` rows and are now no rows at all, which
        // is what the five-row count says. The Jolokia answer is one: a
        // document is a body the codec does not read, so nothing states a type
        // for it - not the document, and not the crate - and tag 35 is null.
        assert_eq!(
            msgtype[RESPONSE_ROW], None,
            "a document states no type, and nothing states one for it"
        );
        assert_eq!(msgtype.len(), MESSAGES, "no sentence is a row");

        // Header facts, by tag.
        let sender = tag_text(&read, 49);
        assert_eq!(sender[HEARTBEAT_ROW].as_deref(), Some("CLIAUDITX1"));
        assert_eq!(sender[FILL_ROW].as_deref(), Some("VENUEADC"));
        let seq = tag_column(&read, 34);
        assert_eq!(seq[HEARTBEAT_ROW].as_i64(), Some(696));
        assert_eq!(seq[FILL_ROW].as_i64(), Some(4507));

        // A sending time is an instant, not the text it arrived as.
        let sent = tag_column(&read, 52);
        assert!(
            sent[HEARTBEAT_ROW].is_temporal(),
            "{:?}",
            sent[HEARTBEAT_ROW]
        );
        assert!(sent[RELAY_ROW].is_temporal(), "{:?}", sent[RELAY_ROW]);
        // A row states tag 52 only where the message did: the explicit codec
        // default is intake's stand-in for a line that named no clock, and it
        // lands in the event's own instant rather than in the header's column.
        assert!(sent[RESPONSE_ROW].is_null(), "{:?}", sent[RESPONSE_ROW]);
        assert_eq!(
            tag_column(&read, yggdryl::CURRUNIX_TAG_NAME.0)[RESPONSE_ROW]
                .temporal_count_at(TimeUnit::Nanosecond),
            Some(1_704_190_530_000_000_000),
            "an unstated sending time stands in as the event's instant"
        );

        // The fill's body: symbol, side, quantities and prices, typed.
        assert_eq!(tag_text(&read, 55)[FILL_ROW].as_deref(), Some("EXAMPLECO"));
        assert_eq!(tag_text(&read, 54)[FILL_ROW].as_deref(), Some("BUY"));
        // `OrderQty(38)` and `Price(44)` are columns of their own, exact at the
        // one width this crate keeps a number at.
        assert_eq!(
            tag_column(&read, 38)[FILL_ROW].as_decimal(),
            Some((yggdryl::i256::from_i128(982_000_000_000_000_000_000), 18))
        );
        assert_eq!(
            tag_column(&read, 44)[FILL_ROW].as_decimal(),
            Some((yggdryl::i256::from_i128(547_771_791_547_861_000_000), 18))
        );
        assert_eq!(tag_column(&read, 151)[FILL_ROW], super::decimal("0"));
        // The row carries the code the wire wrote, and the ranked state the
        // traits answer is read off it rather than columned beside it.
        assert_eq!(tag_text(&read, 39)[FILL_ROW].as_deref(), Some("2"));

        // The routed row states the same trade under names, and lands on the
        // same tags.
        assert_eq!(
            tag_text(&read, 55)[ROUTED_ROW].as_deref(),
            Some("EXAMPLECO")
        );
        assert_eq!(
            tag_text(&read, 11)[ROUTED_ROW].as_deref(),
            Some("20260814_TP1_CLIENT_1003")
        );
        assert_eq!(
            tag_column(&read, 38)[ROUTED_ROW],
            tag_column(&read, 38)[FILL_ROW]
        );
        assert_eq!(tag_column(&read, 31)[ROUTED_ROW], super::decimal("547.77"));

        // Every projected row carries the code it settled on its content.
        // Distinct real messages remain distinct, independently of the separate
        // arrival digest.
        let identities = tag_column(&read, yggdryl::CURRHASHCODE_TAG_NAME.0);
        for (row, held) in identities.iter().enumerate() {
            assert!(held.as_u64().is_some(), "row {row} states a content code");
        }
        assert_ne!(identities[FILL_ROW], identities[ROUTED_ROW]);
        let stamp = tag_column(&read, yggdryl::CURRUNIX_TAG_NAME.0);
        assert!(stamp[FILL_ROW].is_temporal());
        assert!(stamp[ROUTED_ROW].is_temporal());
    }

    #[test]
    fn every_row_keeps_its_event_clock_capture_clock_and_fix_version() {
        let read = read(&CAPTURE);
        let stage = text_stage(&CAPTURE);

        // A capture instant is ordinary context. The message's own clocks date
        // the event independently of when the bridge logged it, and a row
        // stating no `SendingTime` - the routed fill, keyed by name - takes the
        // codec's clock, its `TransactTime` standing far outside the delay that
        // would let it date the message instead.
        let clock = column(&stage, "timestamp");
        let carried = column(&read, "timestamp");
        let stamp = tag_column(&read, yggdryl::CURRUNIX_TAG_NAME.0);
        let snapshot = tag_column(&read, yggdryl::SNAPUNIX_TAG_NAME.0);
        let created = tag_column(&read, yggdryl::CREAUNIX_TAG_NAME.0);
        assert_eq!(stamp.len(), MESSAGES);
        assert_eq!(clock.len(), CAPTURE.len());
        for (row, line) in CARRYING.into_iter().enumerate() {
            assert_eq!(carried[row], clock[line], "capture context for row {row}");
            // No snapshot was taken of any of these, so the column stays empty
            // and the event instant is readable as `createdat`.
            assert!(snapshot[row].is_null());
            assert_eq!(created[row], stamp[row]);
            assert_ne!(stamp[row], clock[line]);
        }
        let millis = |row: usize| stamp[row].temporal_count_at(TimeUnit::Millisecond);
        assert_eq!(millis(RESPONSE_ROW), Some(1_704_190_530_000));
        assert_eq!(
            stamp[HEARTBEAT_ROW].temporal_count_at(TimeUnit::Nanosecond),
            Some(1_786_682_790_415_655_000)
        );
        assert_eq!(millis(FILL_ROW), Some(1_786_682_796_000));
        assert_eq!(millis(ROUTED_ROW), Some(1_704_190_530_000));

        // Every row says which FIX it was read as: the wire's own `BeginString`
        // where the frame stated one, and `FIX.` and the version the row was read
        // at where it did not - the routed row, keyed by name, and the Jolokia
        // document. A sentence answers no version because it answers no row.
        let version = tag_text(&read, 8);
        for (row, held) in version.iter().enumerate() {
            assert!(held.is_some(), "row {row} states a version");
        }
        assert_eq!(version[HEARTBEAT_ROW].as_deref(), Some("FIX.4.4"));
        assert_eq!(version[FILL_ROW].as_deref(), Some("FIX.4.2"));
        assert!(
            version[ROUTED_ROW]
                .as_deref()
                .is_some_and(|held| held.starts_with("FIX.")),
            "{:?}",
            version[ROUTED_ROW]
        );
    }

    #[test]
    fn a_json_document_is_one_unknown_row_carrying_only_what_the_row_stated() {
        let codec = codec();

        // The line as the text reader hands it to the codec: the row header
        // gone, the `Response:` prose still in front of the document.
        let body = &RESPONSE[ROWHEADER_WIDTH..];
        assert!(body.starts_with("Response: {"), "{body}");
        let message = codec
            .sole_line(body.as_bytes())
            .expect("the line carries one document, and so one message");

        // A JSON document is a body this codec does not read: the row said
        // something, and what it said is one message named `unknown` with no
        // entries - nothing the document spelled reaches a field, a tag or the
        // wire - carrying only what the row stated around it: the half the
        // `Response:` prose names.
        assert_eq!(message.as_field().name(), "unknown");
        assert!(message.entries().is_empty());
        // The version the row was read at is the whole of what it re-emits.
        assert_eq!(
            String::from_utf8(message.into_bytes(b'|')).unwrap(),
            "8=FIX.4.4|"
        );
        assert!(message.get_by_tag(35).is_none_or(|held| held.is_null()));
        for spelled in ["SenderCompID", "TargetCompID", "Name", "CurrentPort"] {
            assert!(
                message
                    .get_by_name(spelled)
                    .is_none_or(|held| held.is_null()),
                "{spelled}: nothing the document spelled reaches the message"
            );
        }
        // Tag 8 is filled from the version the row was read at, as on every
        // built message - the crate's default here, since the row states none
        // - and never from the `FIX.4.2` the document spelled.
        assert_eq!(message.by_tag(8).unwrap().as_str(), Some("FIX.4.4"));
        assert_eq!(message.by_tag(385).unwrap().as_str(), Some("R"));

        // In the batch the same document is the same row: no type, an empty
        // arrival record, and the capture's own columns filled - the clock the
        // header stated, the plugin that logged it, the direction the prose
        // named.
        let read = read(&CAPTURE);
        let stage = text_stage(&CAPTURE);
        assert_eq!(tag_text(&read, 35)[RESPONSE_ROW], None);
        let entries = column(&read, "fixentries");
        assert_eq!(
            entries[RESPONSE_ROW]
                .as_sequence()
                .map(<[Scalar]>::len)
                .unwrap_or_default(),
            0,
            "{:?}",
            entries[RESPONSE_ROW]
        );
        assert_eq!(
            column(&read, "timestamp")[RESPONSE_ROW],
            column(&stage, "timestamp")[CARRYING[RESPONSE_ROW]]
        );
        assert_eq!(
            tag_text(&read, yggdryl::MSGPLUGINID_TAG_NAME.0)[RESPONSE_ROW].as_deref(),
            Some("Jolokia")
        );
        assert_eq!(
            tag_text(&read, yggdryl::MSGDIRECTION_TAG_NAME.0)[RESPONSE_ROW].as_deref(),
            Some("R")
        );
        assert_eq!(tag_text(&read, 49)[RESPONSE_ROW], None);
    }

    /// How many bytes the row header takes off the front of every line here.
    const ROWHEADER_WIDTH: usize = "2026-08-14 06:46:22.255 [23] [Jolokia] (DEBUG) ".len();

    #[test]
    fn the_batched_read_agrees_with_the_line_read_and_re_emits_the_wire() {
        let codec = codec();
        let read = read(&CAPTURE);

        // The text reader's bodies are what the codec reads, so the line read
        // runs over them rather than over the raw lines. Every row of the batch
        // came from a line carrying exactly one message here, so `sole_line` is
        // the line read for all five - and it is the line the row came from,
        // `CARRYING[row]`, whose columns the batch filled from. Where the line
        // alone answers nothing, what the batch answers is what that row's own
        // columns stated - here the header's `msgseqnum`. The reader states no
        // message type of its own: a line's type is what its frame says, read by
        // the codec.
        let stage = text_stage(&CAPTURE);
        let sequenced = column(&stage, "msgseqnum");
        let bodies = column(&read, "body");
        let rendered = |value: &Scalar| match value {
            Scalar::Null => None,
            held => Some(
                held.as_str()
                    .map_or_else(|| format!("{held:?}"), ToString::to_string),
            ),
        };
        for tag in [8, 35, 49, 56, 34, 11, 55, 54, 150, 151, 60] {
            let held = tag_column(&read, tag);
            for (row, body) in bodies.iter().enumerate() {
                let body = body.as_str().expect("a body").as_bytes();
                let message = codec
                    .sole_line(body)
                    .unwrap_or_else(|error| panic!("row {row}: {error}"));
                let alone = message.get_by_tag(tag).unwrap_or(Scalar::Null);
                let expected = match (tag, &alone) {
                    // The header's own column is an `int64` in the capture and a
                    // `uint64` on the message, so the count is what is compared.
                    (34, Scalar::Null) => sequenced[CARRYING[row]]
                        .as_i128()
                        .map(|count| count.to_string()),
                    (34, held) => held.as_i128().map(|count| count.to_string()),
                    _ => rendered(&alone),
                };
                let held = match tag {
                    34 => &held[row].as_i128().map_or(Scalar::Null, Scalar::from),
                    _ => &held[row],
                };
                let held = match tag {
                    34 => held.as_i128().map(|count| count.to_string()),
                    _ => rendered(held),
                };
                assert_eq!(
                    expected, held,
                    "tag {tag} on row {row} differs between the line read and the batch",
                );
            }
        }

        // The tags the batch answers and the line read cannot are fills from the
        // row's own columns: the sequence number the header stated for the routed
        // row, which carried none. The residual record excludes those fills and
        // every content field successfully projected into a typed column.
        let routed = bodies[ROUTED_ROW].as_str().expect("a body").as_bytes();
        let alone = codec.sole_line(routed).expect("the routed row");
        assert!(alone.get_by_tag(34).is_none());
        assert_eq!(tag_column(&read, 34)[ROUTED_ROW].as_i64(), Some(4_507));
        let entries = column(&read, "fixentries");
        let recorded: Vec<i64> = entries[ROUTED_ROW]
            .as_sequence()
            .expect("the entries")
            .iter()
            .map(|entry| {
                entry.as_sequence().expect("an entry")[0]
                    .as_i64()
                    .unwrap_or_default()
            })
            .collect();
        // This projection leaves ExecBroker, GrossTradeAmt and CurrencyCodeSource
        // in the residual record; its other content fields have typed columns.
        assert_eq!(recorded, [76, 381, 2897]);
        for filled in [
            34,
            yggdryl::MSGCTXID_TAG_NAME.0,
            yggdryl::MSGPLUGINID_TAG_NAME.0,
            yggdryl::CURRUNIX_TAG_NAME.0,
        ] {
            assert!(
                !recorded.contains(&i64::from(filled)),
                "tag {filled} is a fill, never an entry"
            );
        }

        // Written back out, the wire is rebuilt from each row's arrival record
        // and the facts it holds typed: what the row filled from its header is
        // not an entry, and the capture's own columns are the capture's, so
        // neither is re-emitted. One line is written per message, not per
        // source line, so the six lines that carried none write nothing and
        // eleven lines come back as five.
        let mut written: Vec<u8> = Vec::new();
        let emitting = codec.clone().with_separator(b'|');
        let source = emitting
            .parse_text_arrow_reader(
                corpus(&CAPTURE)
                    .read_arrow_reader(&text())
                    .expect("a reader"),
            )
            .expect("the batch reader opens");
        let rows = emitting
            .write_arrow_reader(source, &mut written)
            .expect("the capture writes");
        assert_eq!(rows, MESSAGES as u64);
        let lines: Vec<&str> = std::str::from_utf8(&written)
            .expect("text")
            .lines()
            .collect();
        assert_eq!(lines.len(), MESSAGES, "{lines:?}");
        // The document's row arrived with no entries and stated no type, so it
        // re-emits nothing but the version every built message states.
        assert_eq!(lines[RESPONSE_ROW], "8=FIX.4.4|");
        // The prose the text reader framed the line in is gone, and so are the
        // columns the capture put in front of the row.
        for line in &lines {
            for carried in [
                "|body=",
                "|mimetype=",
                "|timestamp=",
                "|level=",
                "Receiving :",
            ] {
                assert!(!line.contains(carried), "{line}");
            }
        }
        // Every message states its own header band first, then its entries: a
        // `BodyLength(9)` the frame opened with is an entry like any other, so
        // it re-emits behind the header rather than where the frame wrote it.
        assert_eq!(
            lines[HEARTBEAT_ROW],
            "8=FIX.4.4|35=0|49=CLIAUDITX1|56=OMSAUDITX1|34=696|52=20260814-04:46:30.415655|9=68|10=159|"
        );
        assert!(
            lines[ROUTED_ROW].starts_with("8=FIX.4.4|35=8|"),
            "{}",
            lines[ROUTED_ROW]
        );
        assert!(lines[FILL_ROW].contains("|11=20260814_TP1_CLIENT_1003|"));
    }

    /// A relay that batched two frames into one log write, and a sentence the
    /// bridge wrote after it: two lines carrying two messages between them.
    const BATCHED: [&str; 2] = [
        "2026-08-14 06:46:30.947 [402-e7254b20:9f015ed023:935] [ULMSG_BROKER_TO_DMZ] (DEBUG) Receiving : 8=FIX.4.4|9=68|35=0|49=CLIAUDITX1|56=OMSAUDITX1|34=696|52=20260814-04:46:30.415655|10=159|8=FIX.4.2|9=55|35=0|49=ULB_DMZ|56=ULB_BRK|34=935|52=20260814-04:46:30.967|10=186|",
        "2026-08-14 06:46:37.153 [15333-e7254b22:9f015ee861:4507] [ULBridge] (INFO) Filtering - Message for RiskMonitor",
    ];

    #[test]
    fn a_line_of_two_frames_is_two_rows_and_a_sentence_is_none() {
        let (stage, read) = staged_and_read(&BATCHED);

        // A row yields none, one or many. The first line holds two
        // frames - the second opens at the `8=` behind the first's `10=` checksum
        // - and the second line is a sentence with no frame and no pair in it, so
        // it holds none. Two lines in, two rows out, and neither count is the
        // other's: the text reader still reads both lines.
        assert_eq!(messages_per_line(&BATCHED), [2, 0]);
        assert_eq!(stage.num_rows(), BATCHED.len());
        assert_eq!(read.num_rows(), 2);

        // Both rows came from line 1, because line 2 contributed none, and
        // both say which line by naming its identity rather than its number:
        // `seqnum` on a fixed row is the message's own place, and neither of
        // these two frames states one.
        let first = column(&stage, "curruuid")[0].clone();
        let sources = column(&read, "srcuuids");
        assert_eq!(sources.len(), 2);
        for named in &sources {
            assert_eq!(
                named.as_sequence().expect("the line the row was read from"),
                std::slice::from_ref(&first)
            );
        }
        assert!(
            column(&read, "seqnum").iter().all(Scalar::is_null),
            "a frame the bridge relayed states no place of its own"
        );

        // Each message owns the entries of its own frame and none of its
        // neighbour's: two versions, two senders, two sequence numbers.
        assert_eq!(
            tag_text(&read, 8),
            [Some("FIX.4.4".to_owned()), Some("FIX.4.2".to_owned())]
        );
        assert_eq!(
            tag_text(&read, 49),
            [Some("CLIAUDITX1".to_owned()), Some("ULB_DMZ".to_owned())]
        );
        assert_eq!(tag_column(&read, 34)[0].as_i64(), Some(696));
        assert_eq!(tag_column(&read, 34)[1].as_i64(), Some(935));

        // Capture context is shared, while each frame keeps its own event clock.
        let stamp = tag_column(&read, yggdryl::CURRUNIX_TAG_NAME.0);
        assert_ne!(stamp[0], stamp[1]);
        assert_eq!(stamp, tag_column(&read, 52));
        let captured = column(&read, "timestamp");
        assert_eq!(captured[0], captured[1]);
        assert_eq!(
            captured[0].temporal_count_at(TimeUnit::Millisecond),
            Some(1_786_689_990_947)
        );
        assert_eq!(
            tag_text(&read, yggdryl::MSGPLUGINID_TAG_NAME.0),
            vec![Some("ULMSG_BROKER_TO_DMZ".to_owned()); 2]
        );
        assert_eq!(
            tag_text(&read, yggdryl::MSGDIRECTION_TAG_NAME.0),
            vec![Some("R".to_owned()); 2]
        );

        // The classifier describes the line's first frame, which is what it can
        // know without parsing, and both rows of that line carry its answer.
        assert_eq!(
            text_column(&read, "mimetype"),
            vec![Some("text/fix".to_owned()); 2]
        );

        // Written back out, each message re-emits only its own bytes, and the
        // sentence writes nothing.
        let emitting = codec().with_separator(b'|');
        let mut written: Vec<u8> = Vec::new();
        let source = emitting
            .parse_text_arrow_reader(
                corpus(&BATCHED)
                    .read_arrow_reader(&text())
                    .expect("a reader"),
            )
            .expect("the batch reader opens");
        let rows = emitting
            .write_arrow_reader(source, &mut written)
            .expect("the capture writes");
        assert_eq!(rows, 2);
        let lines: Vec<&str> = std::str::from_utf8(&written)
            .expect("text")
            .lines()
            .collect();
        // Each frame re-emits its own bytes with its header band in front: the
        // `BodyLength(9)` the frame opened with is an entry, so it follows the
        // header rather than leading it.
        assert_eq!(
            lines,
            [
                "8=FIX.4.4|35=0|49=CLIAUDITX1|56=OMSAUDITX1|34=696|52=20260814-04:46:30.415655|9=68|10=159|",
                "8=FIX.4.2|35=0|49=ULB_DMZ|56=ULB_BRK|34=935|52=20260814-04:46:30.967|9=55|10=186|"
            ]
        );

        // The single-frame door refuses a body holding a second: a caller
        // holding two frames has a row, not a frame.
        let body = column(&text_stage(&BATCHED), "body");
        let held = body[0].as_str().expect("a body");
        let refused = codec()
            .parse_fix_line(held.as_bytes())
            .expect_err("the door refuses a second frame");
        assert!(
            refused
                .to_string()
                .contains("expected one frame, got a second"),
            "{refused}"
        );
    }

    #[test]
    fn the_default_read_answers_only_the_two_business_messages() {
        // A live read of this capture answers two rows, not five: the two
        // heartbeats are `Heartbeat`, the Jolokia answer states no type, and
        // those are three of the types `DEFAULT_REFUSED_MSGTYPES` names.
        let default = super::fixed_codec(registry());
        let batches: Vec<RecordBatch> = default
            .parse_text_arrow_reader(
                corpus(&CAPTURE)
                    .read_arrow_reader(&text())
                    .expect("a reader"),
            )
            .expect("the batch reader opens")
            .map(|batch| batch.expect("a batch"))
            .collect();
        let rows: usize = batches.iter().map(RecordBatch::num_rows).sum();
        assert_eq!(rows, 2);
        assert_eq!(
            tag_text(&batches[0], 35),
            vec![Some("8".to_owned()); 2],
            "the fill and the row it was routed as"
        );
        // The line door refuses the same lines the batch door did.
        assert_eq!(
            column(&text_stage(&CAPTURE), "body")
                .iter()
                .filter(|body| default
                    .parse_line(body.as_str().expect("a body").as_bytes())
                    .expect("the line reads")
                    .next()
                    .is_some())
                .count(),
            rows
        );
    }
}
