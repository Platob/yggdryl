"""The codec's streams and Arrow twins, and the message holder's setters.

Every rule here is the core's, pinned in ``rust/tests/fix/batch.rs`` and
``rust/tests/fix/message.rs``; what these check is the crossing - that a
Python iterable is pulled one item at a time, that ``pyarrow`` readers cross
both ways, that raw-byte batching is observable through ``batch_byte_size``,
and that a write to a message is typed by the dictionary and leaves the wire
alone.
"""

from __future__ import annotations

import copy
import io
import pathlib
from typing import Any, Iterator

import pyarrow as pa
import pytest

from yggdryl import DataType, Field, Scalar, TextLine
from yggdryl.fix import FixCodec, FixMessages, FixMsg, FixRegistry, fix_schema, fix_schema_carrying

REPO = pathlib.Path(__file__).resolve().parent.parent.parent.parent
SEED = REPO / "config" / "fix"


@pytest.fixture(scope="module")
def _seed_catalog() -> FixRegistry:
    return FixRegistry.from_handle(SEED)


@pytest.fixture
def seed(_seed_catalog: FixRegistry) -> FixRegistry:
    return copy.copy(_seed_catalog)


BULK_CONFIG = (
    b'{"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:name=*,plugin-type=FIX,type=Plugin","type":"read"},'
    b'"value":{"com.ullink.ulbridge.sessioninterfaces.plugins:name=A,plugin-type=FIX,type=Plugin":{"Name":"A"},'
    b'"com.ullink.ulbridge.sessioninterfaces.plugins:name=B,plugin-type=FIX,type=Plugin":{"Name":"B"}},"status":200}'
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
    b"After Enrichment -> ACCOUNT=ACCT-000117 CLIENTID=MCFP2 VENUE=XPAR",
    b"Referential(dbi|equity|dbi;GB00BN7SWP63_XLON_GBX|[quantity-type=])",
    b"<Order ClOrdID='XML-1'>body</Order>",
    b"Receiving XmlApi: <Execution ExecID='E1'></Execution>",
    b"Message rejected because : ignoring OMSSales expiry message",
    b"no level printed by this plugin",
    b"heartbeat emitted seq=7",
]

ORDER = b"8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|VenueThing=7|9999=x|10=0|"
REPORT = b"8=FIX.4.4|35=8|39=1|150=F|38=100|14=40|32=40|31=10.5|54=1|10=0|"


def _config_registry() -> FixRegistry:
    registry = FixRegistry()
    registry.with_ulbridge_fields()
    direction = Field("MsgDirection", DataType("msgdirection"))
    direction.fix.tag = 385
    registry.insert(direction)
    return registry


def _capture(lines: list[bytes], rows: int) -> pa.RecordBatchReader:
    """The capture as the batches a text reader hands the codec, ``rows`` lines to a batch."""
    schema = pa.schema([pa.field("body", pa.binary(), nullable=False)])
    batches = [
        pa.record_batch([pa.array(lines[at : at + rows], pa.binary())], schema=schema)
        for at in range(0, len(lines), max(rows, 1))
    ]
    return pa.RecordBatchReader.from_batches(schema, batches)


def _wide() -> list[bytes]:
    """Two hundred wide orders, about 450 bytes each."""
    return [b"8=FIX.4.4|35=D|11=ORDER-%06d|58=%s|10=0|" % (index, b"x" * 400) for index in range(200)]


def _one(codec: FixCodec, line: bytes) -> FixMsg:
    messages = codec.parse_line(line)
    message = next(messages)
    assert next(messages, None) is None, "one message"
    return message


def _column(table: pa.Table, name: str) -> list[Any]:
    return table.column(name).to_pylist()


def _stated(message: FixMsg) -> dict[int, Scalar]:
    """Every tag a message's children carry, beside the value each holds."""
    held: dict[int, Scalar] = {}
    for child in message.field:
        tag = child.fix.tag
        if tag is not None:
            held[tag] = message.by_tag(tag)
    return held


def test_parse_lines_pulls_one_line_at_a_time_and_continues_past_a_refused_one() -> None:
    codec = FixCodec(_config_registry())
    pulled = 0

    def lines() -> Iterator[bytes]:
        nonlocal pulled
        for line in [BULK_CONFIG, b""]:
            pulled += 1
            yield line

    messages = codec.parse_lines(lines())
    assert isinstance(messages, FixMessages)
    assert iter(messages) is messages
    # Nothing is pulled until the stream is asked.
    assert pulled == 0
    assert next(messages).by_name("Name").as_py() == "A"
    assert pulled == 1
    # The second MBean comes out of the same line, without the next pull.
    assert next(messages).by_name("Name").as_py() == "B"
    assert pulled == 1
    # An empty line is not a row at all: raised where it is met, and the
    # stream goes on to say it is done.
    with pytest.raises(ValueError):
        next(messages)
    assert pulled == 2
    assert next(messages, None) is None
    assert next(messages, None) is None


def test_an_item_that_is_not_bytes_is_refused_where_it_is_met(seed: FixRegistry) -> None:
    codec = FixCodec(seed)
    mixed = codec.parse_lines([b"8=FIX.4.4|35=D|11=A|10=0|", "8=FIX.4.4|35=D|11=B|10=0|"])
    assert next(mixed).by_tag(11).as_py() == "A"
    with pytest.raises(TypeError, match="bytes"):
        next(mixed)
    assert next(mixed, None) is None
    # A generator that raises raises as itself.

    def failing() -> Iterator[bytes]:
        yield b"8=FIX.4.4|35=D|11=A|10=0|"
        raise RuntimeError("the source broke")

    broken = codec.parse_lines(failing())
    assert next(broken).by_tag(11).as_py() == "A"
    with pytest.raises(RuntimeError, match="the source broke"):
        next(broken)
    # Something that is not iterable is refused before anything is pulled.
    with pytest.raises(TypeError):
        codec.parse_lines(42)


def test_parse_text_lines_pulls_one_line_at_a_time(seed: FixRegistry) -> None:
    codec = FixCodec(seed, capture_names=["beginstring"])
    pulled = 0

    def lines() -> Iterator[TextLine]:
        nonlocal pulled
        for body in [b"8=FIX.4.4|35=D|11=A|10=0|", b"8=FIX.4.4|35=D|11=B|10=0|"]:
            pulled += 1
            yield TextLine(pulled - 1, body)

    messages = codec.parse_text_lines(lines())
    assert pulled == 0
    assert next(messages).by_tag(11).as_py() == "A"
    assert pulled == 1
    assert next(messages).by_tag(11).as_py() == "B"
    assert next(messages, None) is None
    # A capture speaks per row: `beginstring` reads the frame at 4.2, where
    # tag 32 is `lastshares`, and the column is still the dictionary's own.
    (old,) = list(
        codec.parse_text_lines([TextLine(0, b"8=FIX.4.4|35=8|32=100|10=0|", ["FIX.4.2"])])
    )
    assert old.field.index_of("lastqty") is not None
    assert old.get_by_name("lastshares") is not None
    # A bulk document is many messages, and the stream door yields each.
    assert len(list(FixCodec(_config_registry()).parse_text_lines([TextLine(0, BULK_CONFIG)]))) == 2


def test_the_codec_answers_the_pins_it_was_given(seed: FixRegistry) -> None:
    bare = FixCodec(seed)
    assert bare.version is None
    assert bare.separator is None
    assert bare.payload_column == "body"
    assert bare.null_values == ["", "null", "<null>"]
    assert bare.direction == "sent"
    # The default target, stated once in the core and read here.
    assert bare.batch_byte_size == 128 * 1024 * 1024

    pinned = FixCodec(
        seed,
        version="FIX.4.2",
        separator=124,
        payload_column="line",
        null_values=["<none>"],
        direction="RECV",
        batch_byte_size=4096,
    )
    assert pinned.version == "4.2"
    assert pinned.separator == 124
    assert pinned.payload_column == "line"
    assert pinned.null_values == ["<none>"]
    assert pinned.direction == "recv"
    assert pinned.batch_byte_size == 4096
    assert FixCodec(seed, direction="unknown").direction == "unknown"
    with pytest.raises(ValueError, match="sent, recv, unknown"):
        FixCodec(seed, direction="sideways")
    # The payload column names a batch column; a line's body is its own, so
    # the line door reads the same frame without naming anything.
    (read,) = list(pinned.parse_text_lines([TextLine(0, b"8=FIX.4.2|35=D|11=A|10=0|")]))
    assert read.by_tag(11).as_py() == "A"


def test_the_schema_is_decided_before_the_first_row_is_read(seed: FixRegistry) -> None:
    reader = FixCodec(seed).parse_text_arrow_reader(_capture([], 1))
    names = reader.schema.names
    # The capture's own column leads; the fixed columns follow, named by their
    # folded names, each carrying its tag on the field.
    assert names[:6] == ["body", "beginstring", "bodylength", "msgtype", "sendercompid", "targetcompid"]
    assert names[-2:] == ["nofixentries", "nounmappedfixentries"]
    assert reader.schema.field("msgtype").metadata[b"fix:tag"] == b"35"
    # And an empty capture yields no batch at all.
    assert reader.read_all().num_rows == 0


def test_a_row_in_is_a_row_out(seed: FixRegistry) -> None:
    parsed = FixCodec(seed).parse_text_arrow_reader(_capture(CAPTURE, len(CAPTURE))).read_all()
    assert parsed.num_rows == len(CAPTURE), "every line, including the ones that are not messages"
    msgtype = _column(parsed, "msgtype")
    assert msgtype[0] == "D", "a framed row states its type"
    assert msgtype[-1] is None, "a sentence states no type"
    # The arrival record closes every row that carried one.
    assert len(_column(parsed, "nofixentries")[0]) == 7


def test_several_small_input_batches_accumulate_into_one_output_batch(seed: FixRegistry) -> None:
    lines = _wide()
    # Twenty input batches of ten rows, far under the default target.
    whole = FixCodec(seed).parse_text_arrow_reader(_capture(lines, 10)).read_all()
    assert whole.num_rows == 200
    assert len(whole.to_batches()) == 1, "one batch under the byte target"

    # Under a target holding about five input batches, the output batches are
    # fewer than the input ones and no row is lost.
    bounded = FixCodec(seed, batch_byte_size=5 * 10 * 470).parse_text_arrow_reader(_capture(lines, 10))
    batches = list(bounded)
    assert 2 <= len(batches) < 20, len(batches)
    assert sum(batch.num_rows for batch in batches) == 200
    for batch in batches[:-1]:
        assert batch.num_rows > 10, "an input batch did not close an output batch alone"


def test_one_large_input_batch_splits_by_rows_in_proportion(seed: FixRegistry) -> None:
    lines = _wide()
    target = 4096
    batches = list(FixCodec(seed, batch_byte_size=target).parse_text_arrow_reader(_capture(lines, len(lines))))
    assert len(batches) > 1
    assert sum(batch.num_rows for batch in batches) == 200, "the bound shapes batches, it does not drop rows"
    # One input batch charges every row the same share of its bytes, so the
    # cut is even, and that many rows of raw capture is about the target.
    closed = batches[:-1]
    rows = closed[0].num_rows
    assert all(batch.num_rows == rows for batch in closed)
    per_row = sum(map(len, lines)) // len(lines)
    assert target / 2 <= rows * per_row <= target * 2

    # A target no row fits under closes a batch after every row, so one
    # enormous line can never produce an empty batch.
    each = list(FixCodec(seed, batch_byte_size=1).parse_text_arrow_reader(_capture(lines, len(lines))))
    assert len(each) == 200
    assert all(batch.num_rows == 1 for batch in each)


def test_messages_to_batches_close_on_the_arrival_records_raw_bytes(seed: FixRegistry) -> None:
    codec = FixCodec(seed)
    schema = fix_schema(seed)
    lines = _wide()
    one = list(codec.arrow_reader(schema, codec.parse_lines(lines)))
    assert len(one) == 1 and one[0].num_rows == 200

    # A bound of about ten lines of pairs cuts the stream into batches of
    # about ten, and every row survives the cut.
    bounded = FixCodec(seed, batch_byte_size=10 * 450)
    many = list(bounded.arrow_reader(schema, codec.parse_lines(lines)))
    assert 10 <= len(many) < 40, len(many)
    assert sum(batch.num_rows for batch in many) == 200
    assert all(5 <= batch.num_rows <= 20 for batch in many[:-1])

    # A target of one byte is a batch a message.
    each = list(FixCodec(seed, batch_byte_size=1).arrow_reader(schema, codec.parse_lines(lines)))
    assert len(each) == 200


def test_the_filling_reader_fills_what_the_filling_pass_fills_and_leaves_the_record_alone(
    seed: FixRegistry,
) -> None:
    codec = FixCodec(seed)
    bare = codec.parse_text_arrow_reader(_capture([REPORT], 1)).read_all()
    assert _column(bare, "leavesqty") == [None]

    filled = codec.enrich_messages_arrow_reader(codec.parse_text_arrow_reader(_capture([REPORT], 1))).read_all()
    (message,) = list(codec.enrich_messages(codec.parse_lines([REPORT])))
    # The columns the message pass fills, with the values it fills.
    assert _column(filled, "leavesqty") == [message.by_tag(151).as_py()] == [60.0]
    assert _column(filled, "avgpx") == [message.by_tag(6).as_py()] == [10.5]
    # A derived tag the fixed row does not carry is filled on the message
    # and has no column to appear in: the row is a projection of the message.
    assert message.by_tag(381).as_py() == 420.0
    assert "grosstradeamt" not in filled.schema.names
    # The schema is the same schema: the carried column still leads.
    assert filled.schema == bare.schema
    # The arrival record is untouched either way.
    assert _column(filled, "nofixentries") == _column(bare, "nofixentries")


def test_messages_and_arrow_reader_invert_each_other(seed: FixRegistry) -> None:
    codec = FixCodec(seed, null_values=[])
    schema = fix_schema(seed)
    parsed = list(codec.parse_lines(CAPTURE))
    again = list(codec.messages(codec.arrow_reader(schema, parsed)))
    assert len(again) == len(parsed)
    for held, message in zip(again, parsed):
        # The same arrival record, the same wire, the same digest, the same
        # stated values by tag, and the same row again.
        assert held.entries() == message.entries()
        assert held.into_bytes(124) == message.into_bytes(124)
        assert held.digest() == message.digest()
        for tag in (35, 11, 55, 54, 17, 37):
            stated = message.get_by_tag(tag)
            if stated is not None and not stated.is_null():
                assert held.by_tag(tag) == stated, tag
        assert held.into_row(schema) == message.into_row(schema)
    # And the batches the second pass makes are the batches the first made.
    first = codec.arrow_reader(schema, parsed).read_all()
    second = codec.arrow_reader(schema, again).read_all()
    assert first.equals(second)


def test_messages_pull_from_the_reader_one_batch_at_a_time(seed: FixRegistry) -> None:
    codec = FixCodec(seed)
    source = codec.parse_text_arrow_reader(_capture(CAPTURE, 3))
    messages = codec.messages(source)
    assert isinstance(messages, FixMessages)
    # The first message needs the first batch and nothing after it.
    first = next(messages)
    assert first.by_tag(11).as_py() == "ORDER-1"
    assert first.by_name("body").as_py() == CAPTURE[0], "a carried column is a child of its own name"
    assert len(list(messages)) == len(CAPTURE) - 1


def test_a_python_failure_behind_a_batch_stream_arrives_with_the_batch(seed: FixRegistry) -> None:
    codec = FixCodec(seed)
    schema = fix_schema(seed)
    message = _one(codec, ORDER)
    reader = codec.arrow_reader(schema, [message, b"not a message"])
    # `pyarrow` pulls the batch, so the failure is that pull's: the message
    # is in the batch that was completed before it.
    with pytest.raises(pa.ArrowException, match="FixMsg"):
        reader.read_all()


def test_byte_in_byte_out_over_the_whole_corpus(seed: FixRegistry) -> None:
    # The convention that drops a stated absence is deliberately not
    # byte-preserving, so it is turned off to measure the reader rather than
    # the convention.
    codec = FixCodec(seed, null_values=[], separator=124)
    sink = io.BytesIO()
    written = codec.write_arrow_reader(codec.parse_text_arrow_reader(_capture(CAPTURE, len(CAPTURE))), sink)
    assert written == len(CAPTURE)
    back = sink.getvalue().split(b"\n")
    assert back.pop() == b""
    assert len(back) == len(CAPTURE)
    plain = FixCodec(seed, null_values=[])
    for line, source in zip(back, CAPTURE):
        assert line == next(plain.parse_line(source)).into_bytes(124), source


def test_a_batch_with_no_arrival_record_cannot_be_written(seed: FixRegistry) -> None:
    codec = FixCodec(seed)
    batch = codec.parse_text_arrow_reader(_capture(CAPTURE, len(CAPTURE))).read_all()
    facets = batch.select(["symbol", "side"])
    sink = io.BytesIO()
    with pytest.raises(ValueError, match="arrival record"):
        codec.write_arrow_reader(facets, sink)
    assert sink.getvalue() == b"", "refused before a row was read"
    # A sink that refuses raises as itself.

    class Refusing(io.RawIOBase):
        def write(self, _: Any) -> int:
            raise OSError("disk full")

    with pytest.raises(OSError, match="disk full"):
        codec.write_arrow_reader(batch, Refusing())


def test_a_set_value_is_typed_by_the_registry_field_and_appended_when_absent(seed: FixRegistry) -> None:
    codec = FixCodec(seed)
    message = _one(codec, ORDER)
    before = len(message)
    declared = seed.field_by_tag(34)

    message.set(34, 7)

    assert len(message) == before + 1, "appended, not inserted"
    child = message.field.field_at(before)
    assert child.name == declared.name, "the dictionary's spelling"
    assert child.dtype == declared.dtype, "the dictionary's type"
    assert child.fix.tag == 34
    assert not child.nullable, "a stated value is non-null"
    assert message.by_tag(34).as_py() == 7
    assert message.by_name("MsgSeqNum").as_py() == 7, "reached by name through the registry"


def test_a_set_value_replaces_an_existing_child_in_place_and_keeps_the_tag_index(seed: FixRegistry) -> None:
    message = _one(FixCodec(seed), ORDER)
    before = _stated(message)
    at = message.field.index_of("symbol")

    message.set(55, "MSFT")
    message.set("Side", "2")

    assert message.field.index_of("symbol") == at, "same position"
    assert len(message) == len(before) + 2, "two unknown children beside the tagged ones"
    assert message.by_tag(55).as_py() == "MSFT"
    assert message.by_tag(54).as_py() == "2"
    for tag, value in before.items():
        if tag not in (55, 54):
            assert message.by_tag(tag) == value, tag


def test_a_set_leaves_the_entries_and_the_wire_untouched(seed: FixRegistry) -> None:
    parsed = _one(FixCodec(seed), ORDER)
    message = copy.copy(parsed)
    message.set(55, "MSFT")
    message.set(38, 100.0)
    assert message.remove(54) is not None
    assert message.entries() == parsed.entries()
    assert message.into_bytes(124) == ORDER
    assert message.digest() == parsed.digest(), "the digest is the arrival record's"


def test_a_null_is_stored_as_a_stated_null(seed: FixRegistry) -> None:
    message = _one(FixCodec(seed), ORDER)
    message.set(55, None)
    assert message.field.field_at(message.field.index_of("symbol")).nullable
    held = message.get_by_tag(55)
    assert held is not None and held.is_null()


def test_an_unknown_name_is_refused_and_the_message_stands(seed: FixRegistry) -> None:
    message = _one(FixCodec(seed), ORDER)
    before = copy.copy(message)
    with pytest.raises(KeyError, match="nosuchfield"):
        message.set("nosuchfield", "y")
    assert message == before
    # A value the field refuses is refused the same way.
    with pytest.raises(ValueError):
        message.set(34, "not a number")
    assert message == before
    # A key is a tag or a name, and a tag is never a bool.
    with pytest.raises(TypeError):
        message.set(True, "y")


def test_an_unknown_name_still_reaches_the_child_spelled_that_way(seed: FixRegistry) -> None:
    message = _one(FixCodec(seed), ORDER)
    at = message.field.index_of("venuething")
    message.set("Venue_Thing", "8")
    child = message.field.field_at(at)
    assert child.name == "venuething", "the child keeps its own field"
    assert child.dtype == DataType("utf8")
    assert message.by_name("venuething").as_py() == "8"


def test_a_bare_unknown_tag_is_appended_under_its_decimal_spelling(seed: FixRegistry) -> None:
    message = _one(FixCodec(seed), ORDER)
    message.set(7777, "custom")
    child = message.field.field_at(len(message) - 1)
    assert child.name == "7777"
    assert child.dtype == DataType("utf8")
    assert child.nullable
    assert message.by_tag(7777).as_py() == "custom"
    # A second write reaches the same child rather than a second one.
    message.set(7777, "again")
    assert message.by_tag(7777).as_py() == "again"
    assert message.field.index_of("7777") == len(message) - 1
    # The one the line already carried is replaced where it stands.
    message.set(9999, "y")
    assert message.by_tag(9999).as_py() == "y"


def test_remove_answers_the_value_and_the_other_tags_still_reach_their_children(seed: FixRegistry) -> None:
    message = _one(FixCodec(seed), ORDER)
    before = _stated(message)
    count = len(message)

    assert message.remove(55) == Scalar("AAPL")
    assert message.get_by_tag(55) is None
    assert len(message) == count - 1
    for tag, value in before.items():
        if tag != 55:
            assert message.by_tag(tag) == value, tag
    # By name, by decimal, and a miss.
    assert message.remove("VenueThing") == Scalar("7")
    assert message.remove(9999) == Scalar("x")
    assert message.remove(55) is None
    assert message.remove("nosuchfield") is None
    assert len(message) == count - 3
    assert message.into_bytes(124) == ORDER, "the entries are untouched"


def test_a_hashed_message_is_frozen_and_a_copy_takes_the_write(seed: FixRegistry) -> None:
    message = _one(FixCodec(seed), ORDER)
    held = {message}
    with pytest.raises(TypeError, match="hashed FixMsg is frozen"):
        message.set(55, "MSFT")
    with pytest.raises(TypeError, match="hashed FixMsg is frozen"):
        message.remove(55)
    assert message in held, "the hash still stands"
    written = copy.copy(message)
    written.set(55, "MSFT")
    assert written.by_tag(55).as_py() == "MSFT"
    assert written != message
    assert hash(copy.copy(written)) == hash(written)


def test_a_row_reads_back_into_the_message_that_made_it(seed: FixRegistry) -> None:
    codec = FixCodec(seed)
    schema = fix_schema(seed)
    parsed = _one(codec, ORDER)
    row = parsed.into_row(schema)

    held = FixMsg.from_row(schema, row, seed)

    assert held.field == schema, "the root is the schema"
    assert held.entries() == parsed.entries()
    assert held.into_bytes(124) == ORDER
    assert held.digest() == parsed.digest()
    for tag in (8, 35, 11, 55, 54):
        assert held.by_tag(tag) == parsed.by_tag(tag), tag
    # And it makes the row it came from, whole.
    assert held.into_row(schema) == row
    # A row read out of a batch as a mapping is the same row.
    table = codec.parse_text_arrow_reader(_capture([ORDER], 1)).read_all()
    (mapping,) = table.to_pylist()
    from_mapping = FixMsg.from_row(Field.from_arrow_schema(table.schema, "fix"), mapping, seed)
    assert from_mapping.entries() == parsed.entries()
    assert from_mapping.by_tag(55) == parsed.by_tag(55)
    # The process default is the registry when none is named.
    assert FixMsg.from_row(schema, row).registry is not None


def test_a_row_carrying_its_captures_own_columns_returns_to_its_schema_whole(seed: FixRegistry) -> None:
    codec = FixCodec(seed)
    capture = Field(
        "line",
        DataType.from_fields([Field("url", "utf8"), Field("rownum", "int64"), Field("body", "binary")]),
        nullable=False,
    )
    schema = fix_schema_carrying(capture, fix_schema(seed))
    parsed = _one(codec, ORDER)

    # A parsed message has no capture columns: they are null in its row.
    row = parsed.into_row(schema)
    assert row.as_py()[schema.index_of("url")] is None
    assert row.as_py()[schema.index_of("rownum")] is None

    # Read back, the message holds them as children, and a written one lands
    # in its column: a column no tag names takes the child of its name.
    held = FixMsg.from_row(schema, row, seed)
    assert held.into_row(schema) == row
    held.set("url", "file:///capture.log")
    held.set("rownum", 42)
    filled = held.into_row(schema).as_py()
    assert filled[schema.index_of("url")] == "file:///capture.log"
    assert filled[schema.index_of("rownum")] == 42
    assert filled[schema.index_of("symbol")] == "AAPL"
    again = FixMsg.from_row(schema, held.into_row(schema), seed)
    assert again.entries() == parsed.entries()
    assert again.by_name("url").as_py() == "file:///capture.log"


def test_a_row_without_the_entries_column_has_no_entries(seed: FixRegistry) -> None:
    codec = FixCodec(seed)
    wide = fix_schema(seed)
    narrow = Field(
        "fix",
        DataType.from_fields([column for column in wide if column.name not in ("nofixentries", "nounmappedfixentries")]),
        nullable=False,
    )
    parsed = _one(codec, ORDER)
    row = parsed.into_row(narrow)
    held = FixMsg.from_row(narrow, row, seed)
    assert held.entries() == []
    assert held.into_bytes(124) == b""
    assert held.by_tag(55) == parsed.by_tag(55)
    assert held.into_row(narrow) == row
    # A row that does not fit the schema is refused.
    with pytest.raises(ValueError):
        FixMsg.from_row(narrow, {"nosuchcolumn": 1}, seed)
