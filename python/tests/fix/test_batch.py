"""The codec's streams and Arrow twins, and the fixed row both ends of it.

Every rule here is the core's; what these check is the crossing - that a
Python iterable is pulled one item at a time, that ``pyarrow`` readers cross
both ways, that raw-byte batching is observable through ``batch_byte_size``,
and that a row read back is the message that made it.
"""

from __future__ import annotations

import copy
import io
import pathlib
from typing import Any, Iterator

import pyarrow as pa
import pytest

from yggdryl import DataType, Field, Scalar, TextLine
from yggdryl.fix import (
    FixCodec,
    FixMessages,
    FixMsg,
    FixRegistry,
    fix_schema,
    fix_schema_carrying,
)

REPO = pathlib.Path(__file__).resolve().parent.parent.parent.parent
SEED = REPO / "config" / "fix"

# The one intake clock undated test bytes take, so a parse repeats.
CLOCK = DataType('datetime64(ns,"UTC")').scalar(1_704_190_530_000_000_000)


def _fixed(registry: FixRegistry, **pins: Any) -> FixCodec:
    """A codec whose undated messages all take ``CLOCK`` as their SendingTime.

    It reads every message type: the corpus below is a capture, and a capture
    holds the session traffic and the bridge rows stating no type that the
    :data:`DEFAULT_REFUSED_MSGTYPES` drop. A case about the refusals says so
    for itself.
    """
    pins.setdefault("exclude_msgtypes", [])
    return FixCodec(registry, default_sending_time=CLOCK, **pins)


@pytest.fixture(scope="module")
def _seed_catalog() -> FixRegistry:
    return FixRegistry.from_handle(SEED)


@pytest.fixture
def seed(_seed_catalog: FixRegistry) -> FixRegistry:
    return copy.copy(_seed_catalog)


# A JSON document a bridge logs, which the codec does not read: one row, one
# `unknown` message.
DOCUMENT = (
    b'{"request":{"mbean":"com.ullink.ulbridge:type=*","type":"read"},'
    b'"value":{"com.ullink.ulbridge:name=A,type=Bridge":{"Name":"A"},'
    b'"com.ullink.ulbridge:name=B,type=Bridge":{"Name":"B"}},"status":200}'
)

# Every shape a real capture holds, the corpus the readers are tested on.
CAPTURE = [
    b"sending >> 8=FIX.4.2|9=176|35=D|11=ORDER-1|55=AAPL|54=1|10=203| << queued seq=1092",
    b"raw 8=FIX.4.4|9=224|35=8|17=E1|37=O9|31=12.75|32=50|10=118|",
    b"8=FIX.4.4|35=8|58=quoting #A=1 and #B=2|10=1|",
    b"sending >> 8=FIX.4.2|35=UL|#SYMBOL=TTF|#SIDE=1|10=044|",
    b"8=FIX.4.4|35=D|11=ORDER-1|SYMBOL=AAPL|SIDE=1|10=000",
    b"toBridge #ISINCODE=XX|#SYMBOL=TTF|#SIDE=1",
    b"ACCOUNT=A1|MSGTYPE=D|CLORDID=ORDER-1|SYMBOL=AAPL|SIDE=1",
    b"<Order ClOrdID='XML-1'>body</Order>",
    b"Receiving XmlApi: <Execution ExecID='E1'></Execution>",
    b"no level printed by this plugin",
]

# The capture lines that carry a message: a line that opens no frame, states
# no bridge pair and carries no document carries nothing to read.
CARRYING = [CAPTURE[at] for at in (0, 1, 2, 3, 4, 5, 6, 7, 8)]

ORDER = b"8=FIX.4.4|35=D|11=A1|52=20240102-10:15:30|55=AAPL|VenueThing=7|9999=x|10=0|"
REPORT = b"8=FIX.4.4|35=8|52=20240102-10:15:30|39=1|150=F|38=100|14=40|32=40|31=10.5|10=0|"


def _capture(lines: list[bytes], rows: int) -> pa.RecordBatchReader:
    """The capture as the batches a text reader hands the codec."""
    schema = pa.schema([pa.field("body", pa.binary(), nullable=False)])
    batches = [
        pa.record_batch([pa.array(lines[at : at + rows], pa.binary())], schema=schema)
        for at in range(0, len(lines), max(rows, 1))
    ]
    return pa.RecordBatchReader.from_batches(schema, batches)


def _wide() -> list[bytes]:
    """Two hundred wide orders, about 450 bytes each."""
    return [
        b"8=FIX.4.4|35=D|11=ORDER-%06d|58=%s|10=0|" % (index, b"x" * 400) for index in range(200)
    ]


def _one(codec: FixCodec, line: bytes) -> FixMsg:
    messages = codec.parse_line(line)
    message = next(messages)
    assert next(messages, None) is None, "one message"
    return message


def _column(table: pa.Table, name: str) -> list[Any]:
    return table.column(name).to_pylist()


def test_parse_lines_pulls_one_line_at_a_time_and_continues_past_a_refused_one(
    seed: FixRegistry,
) -> None:
    pulled = 0

    def lines() -> Iterator[bytes]:
        nonlocal pulled
        for line in CARRYING:
            pulled += 1
            yield line

    stream = _fixed(seed).parse_lines(lines())
    assert isinstance(stream, FixMessages)
    assert iter(stream) is stream
    assert pulled == 0
    first = next(stream)
    assert pulled == 1
    assert first.by_tag(11).as_py() == "ORDER-1"
    assert len(list(stream)) == len(CARRYING) - 1
    assert pulled == len(CARRYING)
    assert next(stream, None) is None


def test_an_item_that_is_not_bytes_is_refused_where_it_is_met(seed: FixRegistry) -> None:
    stream = _fixed(seed).parse_lines([CARRYING[0], 17])
    assert next(stream) is not None
    with pytest.raises(TypeError):
        next(stream)
    assert next(stream, None) is None


def test_parse_text_lines_pulls_one_line_at_a_time(seed: FixRegistry) -> None:
    codec = _fixed(seed, capture_names=["msgpluginid"])
    lines = [TextLine(index, line, ["ULB"]) for index, line in enumerate(CARRYING)]
    read = list(codec.parse_text_lines(lines))
    assert len(read) == len(CARRYING)
    assert all(held.capture().msgpluginid == "ULB" for held in read)
    # A line's own captures are facts about the capture, never entries.
    assert all(all(tag != 65009 for tag, _, _, _ in held.entries()) for held in read)


def test_the_codec_answers_the_pins_it_was_given(seed: FixRegistry) -> None:
    plain = FixCodec(seed)
    assert plain.registry == seed
    assert plain.default_sending_time is None
    assert plain.separator is None
    assert plain.payload_column == "body"
    assert plain.null_values == ["", "null", "<null>"]
    assert plain.batch_byte_size == 128 * 1024 * 1024
    assert repr(plain).startswith("FixCodec(")
    with pytest.raises(TypeError):
        hash(plain)

    pinned = FixCodec(
        seed,
        default_sending_time=CLOCK,
        separator=124,
        payload_column="line",
        null_values=["<none>"],
        direction="R",
        batch_byte_size=1 << 20,
    )
    assert pinned.default_sending_time == CLOCK
    assert pinned.separator == 124
    assert pinned.payload_column == "line"
    assert pinned.null_values == ["<none>"]
    assert pinned.direction == "R"
    assert pinned.batch_byte_size == 1 << 20
    assert copy.copy(pinned).registry == seed

    # A stated absence produces no field at all.
    silent = _fixed(seed, null_values=["<none>"])
    assert next(silent.parse_line(b"8=FIX.4.4|35=D|55=<none>|10=0|")).get_by_tag(55) is None

    # A codec pins no version and no dialect: the dictionary is one namespace.
    with pytest.raises(TypeError):
        FixCodec(seed, version="4.2")  # type: ignore[call-arg]
    with pytest.raises(TypeError):
        FixCodec(seed, branch="cme")  # type: ignore[call-arg]


def test_threads_read_what_one_thread_reads_and_a_message_carries_its_rows_cells(
    seed: FixRegistry,
) -> None:
    one = _fixed(seed)
    four = _fixed(seed, threads=4)
    assert one.threads == 1
    assert four.threads == 4
    assert _fixed(seed, threads=0).threads == 1

    # The line doors answer on four threads what they answer on one: the
    # same messages, in the same order.
    def stated(messages: Iterator[Any]) -> list[tuple[Scalar, bytes]]:
        return [(held.curruuid, held.into_bytes(ord("|"))) for held in messages]

    assert stated(four.parse_lines(CAPTURE)) == stated(one.parse_lines(CAPTURE))
    parsed = four.parse_text_arrow_reader(_capture(CAPTURE, 3)).read_all()
    assert parsed.equals(one.parse_text_arrow_reader(_capture(CAPTURE, 3)).read_all())
    assert stated(four.messages(parsed)) == stated(one.messages(parsed))

    # A message read back out of a row carries the row's own cells - the
    # body its line was cut from - and one parsed from bytes carries none.
    held = next(iter(one.messages(parsed)))
    assert dict(held.carried)["body"].as_py() == CAPTURE[0]
    assert next(one.parse_line(CAPTURE[0])).carried == []


def test_the_schema_is_decided_before_the_first_row_is_read(seed: FixRegistry) -> None:
    reader = _fixed(seed).parse_text_arrow_reader(_capture([], 1))
    names = reader.schema.names
    # The capture's own column leads; the fixed columns follow, the crate's
    # own first - a table is read by time and joined by identity.
    assert names[0] == "body"
    assert names[1] == "currunix"
    # A column is read by name rather than by position: the bands the row is
    # laid out in are the core's to order.
    for named in ("beginstring", "msgtype", "sendingtime", "symbol", "fixentries"):
        assert names.count(named) == 1, named
    # The one record closes the row, under the counter that counts it.
    assert names[-2:] == ["nofixentries", "fixentries"]
    assert reader.schema.field("msgtype").metadata[b"FIX:tag"] == b"35"
    # The identities cross as what a lake reads: a UUID and a 64-bit integer.
    assert reader.schema.field("curruuid").type == pa.uuid()
    assert reader.schema.field("currhashcode").type == pa.uint64()
    # And an empty capture yields no batch at all.
    assert reader.read_all().num_rows == 0


def test_a_capture_answers_one_row_per_message_not_one_per_line(seed: FixRegistry) -> None:
    parsed = _fixed(seed).parse_text_arrow_reader(_capture(CAPTURE, len(CAPTURE))).read_all()
    assert parsed.num_rows == len(CARRYING), "one row a message"
    assert [bytes(body) for body in _column(parsed, "body")] == CARRYING
    msgtype = _column(parsed, "msgtype")
    assert msgtype[0] == "D", "a framed row states its type"
    # Every row settles its identity, so the non-null columns are filled.
    assert all(held is not None for held in _column(parsed, "curruuid"))
    assert all(held is not None for held in _column(parsed, "currhashcode"))
    assert len(_column(parsed, "fixentries")[0]) >= 1


def test_several_small_input_batches_accumulate_into_one_output_batch(seed: FixRegistry) -> None:
    lines = _wide()
    whole = _fixed(seed).parse_text_arrow_reader(_capture(lines, 10)).read_all()
    assert whole.num_rows == 200
    assert len(whole.to_batches()) == 1, "one batch under the byte target"

    bounded = _fixed(seed, batch_byte_size=5 * 10 * 470).parse_text_arrow_reader(_capture(lines, 10))
    batches = list(bounded)
    assert 2 <= len(batches) < 20, len(batches)
    assert sum(batch.num_rows for batch in batches) == 200


def test_one_large_input_batch_splits_by_rows_in_proportion(seed: FixRegistry) -> None:
    lines = _wide()
    target = 4096
    batches = list(
        _fixed(seed, batch_byte_size=target).parse_text_arrow_reader(_capture(lines, len(lines)))
    )
    assert len(batches) > 1
    assert sum(batch.num_rows for batch in batches) == 200, "the bound shapes batches"
    closed = batches[:-1]
    rows = closed[0].num_rows
    assert all(batch.num_rows == rows for batch in closed)

    # A target no row fits under closes a batch after every row.
    each = list(
        _fixed(seed, batch_byte_size=1).parse_text_arrow_reader(_capture(lines, len(lines)))
    )
    assert len(each) == 200
    assert all(batch.num_rows == 1 for batch in each)


def test_messages_to_batches_close_on_the_records_raw_bytes(seed: FixRegistry) -> None:
    codec = _fixed(seed)
    schema = fix_schema(seed)
    lines = _wide()
    one = list(codec.arrow_reader(schema, codec.parse_lines(lines)))
    assert len(one) == 1 and one[0].num_rows == 200

    bounded = _fixed(seed, batch_byte_size=10 * 450)
    many = list(bounded.arrow_reader(schema, codec.parse_lines(lines)))
    assert 1 < len(many) < 40, len(many)
    assert sum(batch.num_rows for batch in many) == 200

    each = list(_fixed(seed, batch_byte_size=1).arrow_reader(schema, codec.parse_lines(lines)))
    assert len(each) == 200


def test_messages_and_arrow_reader_invert_each_other(seed: FixRegistry) -> None:
    codec = _fixed(seed, null_values=[])
    schema = fix_schema(seed)
    parsed = list(codec.parse_lines(CARRYING))
    again = list(codec.messages(codec.arrow_reader(schema, parsed)))
    assert len(again) == len(parsed)
    for held, message in zip(again, parsed):
        # The same content, the same wire, the same digest, the same identity.
        assert held.entries() == message.entries()
        assert held.into_bytes(124) == message.into_bytes(124)
        assert held.digest() == message.digest()
        assert held.currhashcode == message.currhashcode
        assert held.curruuid == message.curruuid
        assert held.into_row(schema) == message.into_row(schema)
    # And the batches the second pass makes are the batches the first made.
    first = codec.arrow_reader(schema, parsed).read_all()
    second = codec.arrow_reader(schema, again).read_all()
    assert first.equals(second)


def test_messages_pull_from_the_reader_one_batch_at_a_time(seed: FixRegistry) -> None:
    codec = _fixed(seed)
    source = codec.parse_text_arrow_reader(_capture(CARRYING, 3))
    stream = codec.messages(source)
    assert isinstance(stream, FixMessages)
    assert len(list(stream)) == len(CARRYING)


def test_a_python_failure_behind_a_batch_stream_arrives_with_the_batch(seed: FixRegistry) -> None:
    codec = _fixed(seed)

    def messages() -> Iterator[FixMsg]:
        yield _one(codec, CARRYING[0])
        raise RuntimeError("the source gave up")

    reader = codec.arrow_reader(fix_schema(seed), messages())
    # The failure travels as the reader's own error and arrives where the
    # batch it would have landed in does, carrying the native sentence.
    with pytest.raises(Exception, match="the source gave up"):
        reader.read_all()


def test_byte_in_byte_out_over_the_whole_corpus(seed: FixRegistry) -> None:
    # The convention that drops a stated absence is deliberately not
    # byte-preserving, so it is turned off to measure the reader.
    codec = _fixed(seed, null_values=[], separator=124)
    sink = io.BytesIO()
    written = codec.write_arrow_reader(
        codec.parse_text_arrow_reader(_capture(CARRYING, len(CARRYING))), sink
    )
    assert written == len(CARRYING)
    back = sink.getvalue().split(b"\n")
    assert back.pop() == b""
    assert len(back) == len(CARRYING)


def test_a_batch_with_no_record_cannot_be_written(seed: FixRegistry) -> None:
    codec = _fixed(seed)
    batch = codec.parse_text_arrow_reader(_capture(CARRYING, len(CARRYING))).read_all()
    sink = io.BytesIO()
    with pytest.raises(ValueError, match="fixentries"):
        codec.write_arrow_reader(batch.select(["symbol"]), sink)
    assert sink.getvalue() == b"", "refused before a row was read"

    class Refusing(io.RawIOBase):
        def write(self, _: Any) -> int:
            raise OSError("disk full")

    with pytest.raises(OSError, match="disk full"):
        codec.write_arrow_reader(batch, Refusing())


def test_a_row_reads_back_into_the_message_that_made_it(seed: FixRegistry) -> None:
    codec = _fixed(seed)
    schema = fix_schema(seed)
    parsed = _one(codec, ORDER)
    row = parsed.into_row(schema)

    held = FixMsg.from_row(schema, row, seed)

    assert held.entries() == parsed.entries()
    assert held.into_bytes(124) == parsed.into_bytes(124)
    assert held.digest() == parsed.digest()
    assert held.currhashcode == parsed.currhashcode
    assert held.curruuid == parsed.curruuid
    assert held.crossuuid == parsed.crossuuid
    assert held.currunix == parsed.currunix
    assert held.header() == parsed.header()
    for tag in (11, 55):
        assert held.by_tag(tag) == parsed.by_tag(tag), tag
    # And it makes the row it came from, whole, without reading a clock.
    assert held.into_row(schema) == row
    # The process default is the registry when none is named.
    assert FixMsg.from_row(schema, row).registry is not None


def test_a_captures_own_columns_are_carried_and_never_become_facts(seed: FixRegistry) -> None:
    """A message carries what the reader said about its line, and states none of it."""
    codec = _fixed(seed)
    capture = Field(
        "line",
        DataType.from_fields(
            [Field("url", "utf8"), Field("rownum", "int64"), Field("body", "binary")]
        ),
        nullable=False,
    )
    schema = fix_schema_carrying(capture, fix_schema(seed))
    # A carried column is nullable whatever the capture declared: only a pass
    # holding the source row can state one.
    assert schema.field("url").nullable
    parsed = _one(codec, ORDER)

    # A message parsed out of a line carries nothing: the capture's own
    # columns are null in its row, the one the crate tags among them.
    assert parsed.carried == []
    row = parsed.into_row(schema)
    held_row = row.as_py()
    for carrier in ("url", "rownum", "body", "sourceurl"):
        assert held_row[schema.index_of(carrier)] is None, carrier

    # A row a reader stated them on reads back carrying them, each under its
    # column's name: no child, no entry, nothing to answer by name, and the
    # content identity untouched.
    stated = list(held_row)
    stated[schema.index_of("url")] = "file:///capture.log"
    stated[schema.index_of("rownum")] = 42
    again = FixMsg.from_row(schema, stated, seed)
    assert again.entries() == parsed.entries()
    assert again.currhashcode == parsed.currhashcode
    assert {name: value.as_py() for name, value in again.carried} == {
        "url": "file:///capture.log",
        "rownum": 42,
    }
    for carrier in ("url", "rownum", "body"):
        assert again.field.index_of(carrier) is None, carrier
    # And writing one onto the message is refused rather than silently kept.
    with pytest.raises(ValueError):
        again.set("sourceurl", "file:///capture.log")

    # So a message states them again at their columns, and nowhere else.
    written = again.into_row(schema).as_py()
    assert written[schema.index_of("url")] == "file:///capture.log"
    assert written[schema.index_of("rownum")] == 42
    for carrier in ("body", "sourceurl"):
        assert written[schema.index_of(carrier)] is None, carrier
    assert "65026=" not in again.into_text("|")


def test_a_row_without_the_entries_column_has_no_content(seed: FixRegistry) -> None:
    codec = _fixed(seed)
    wide = fix_schema(seed)
    narrow = Field(
        "fix",
        DataType.from_fields(
            [column for column in wide if column.name not in ("fixentries", "nofixentries")]
        ),
        nullable=False,
    )
    parsed = _one(codec, ORDER)
    row = parsed.into_row(narrow)
    held = FixMsg.from_row(narrow, row, seed)
    # The content is rebuilt from the record, so a row without it holds the
    # typed facts alone.
    assert held.entries() == []
    assert held.header() == parsed.header()
    assert held.currunix == parsed.currunix
    # The content columns go with it: a row that kept no record cannot say
    # what the message stated beyond its typed facts.
    again = held.into_row(narrow).as_py()
    assert again[narrow.index_of("symbol")] is None
    assert again[narrow.index_of("currunix")] == row.as_py()[narrow.index_of("currunix")]
    # A row that does not fit the schema is refused.
    with pytest.raises(ValueError):
        FixMsg.from_row(narrow, {"nosuchcolumn": 1}, seed)


def test_the_lifecycle_twin_walks_the_rows_a_batch_holds(seed: FixRegistry) -> None:
    """``lifecycle`` over batches: the same schema in and out, nothing reparsed."""
    codec = _fixed(seed)
    schema = fix_schema(seed)
    life = [
        b"8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|38=100|60=20260102-10:15:30.000|10=0|",
        b"8=FIX.4.4|35=8|11=A1|37=O1|17=E1|150=0|39=0|55=AAPL|60=20260102-10:15:30.250|10=0|",
        b"8=FIX.4.4|35=8|11=A1|37=O1|17=E2|150=F|39=2|55=AAPL|60=20260102-10:15:31.000|10=0|",
    ]
    parsed = list(codec.parse_lines(life))
    table = codec.arrow_reader(schema, parsed).read_all()
    # A parse chains nothing: the place in the chain and the predecessor are
    # what the walk states.
    assert _column(table, "seqnum") == [None, None, None]
    assert _column(table, "prevuuid") == [None, None, None]

    walked = codec.lifecycle_arrow_reader(table).read_all()
    assert walked.schema == table.schema, "the same schema in and out"
    assert _column(walked, "seqnum") == [None, 1, 2], "the first of a chain is where it starts"
    assert _column(walked, "prevuuid")[0] is None
    assert all(held is not None for held in _column(walked, "prevuuid")[1:])
    assert _column(walked, "fixentries") == _column(table, "fixentries"), "the record is untouched"

    # The rows the twin wrote are the messages the stream walk answers.
    by_rows = list(codec.messages(walked))
    by_stream = list(codec.lifecycle(parsed))
    for held, message in zip(by_rows, by_stream):
        assert held.seqnum == message.seqnum
        assert held.prevuuid == message.prevuuid
        assert held.curruuid == message.curruuid
        assert held.crossuuid == message.crossuuid
        assert held.currhashcode == message.currhashcode


def test_format_answers_the_rows_one_message_field_holds(seed: FixRegistry) -> None:
    """The verb a consumer reads by: messages in, rows under a field out."""
    codec = _fixed(seed)
    target = fix_schema(seed)

    messages = list(codec.parse_line(ORDER))
    rows = codec.format_messages(messages, target)
    assert len(rows) == 1
    held = rows[0].as_py()
    assert held[target.index_of("symbol")] == "AAPL"
    assert held[target.index_of("fixentries")]

    # The Arrow twin answers the same row, deciding its schema before a row
    # is read.
    source = codec.arrow_reader(fix_schema(seed), messages)
    formatted = codec.format_arrow_reader(source, target)
    assert [field.name for field in formatted.schema] == [column.name for column in target]
    batched = formatted.read_all()
    assert batched.num_rows == 1
    assert batched.column("symbol").to_pylist() == ["AAPL"]


def test_a_column_a_narrow_row_dropped_is_lifted_out_of_the_record(seed: FixRegistry) -> None:
    """Formatting a narrow row into a wider field reads the record."""
    codec = _fixed(seed)
    wide = fix_schema(seed)
    keep = (
        "beginstring",
        "msgtype",
        "sendingtime",
        "currunix",
        "creaunix",
        "currhashcode",
        "crosshashcode",
        "curruuid",
        "crossuuid",
        "fixentries",
        "nofixentries",
    )
    narrow = Field(
        "fix",
        DataType.from_fields([wide[wide.index_of(name)] for name in keep]),
        nullable=False,
    )
    assert narrow.index_of("symbol") is None

    parsed = _one(codec, ORDER)
    stored = parsed.into_row(narrow)
    held = FixMsg.from_row(narrow, stored, seed)
    row = codec.format_messages([held], wide).pop().as_py()
    assert row[wide.index_of("symbol")] == "AAPL"
