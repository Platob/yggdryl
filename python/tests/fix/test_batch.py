"""The FIX Arrow reader stays lazy and keeps capture provenance."""

from __future__ import annotations

import pathlib
from collections.abc import Iterator
from datetime import datetime, timezone

import pyarrow as pa
import pytest

from yggdryl import Field, fix
from yggdryl.fix import FixRegistry, parse_arrow_reader

REPO = pathlib.Path(__file__).resolve().parent.parent.parent.parent
SEED = REPO / "config" / "fix"


@pytest.fixture
def registry() -> FixRegistry:
    """The repository's small FIX dictionary."""
    return FixRegistry.from_handle(SEED)


def _schema(*extra: pa.Field, body_type: pa.DataType = pa.binary()) -> pa.Schema:
    return pa.schema(
        [
            pa.field(
                "url",
                pa.string(),
                nullable=False,
                metadata={b"source": b"capture"},
            ),
            pa.field("rownum", pa.int64(), nullable=False),
            pa.field("branch", pa.string(), nullable=False),
            pa.field("body", body_type, nullable=False),
            pa.field("fixbranch", pa.string(), nullable=True),
            *extra,
        ],
        metadata={b"python.class": b"Message"},
    )


def _batch(schema: pa.Schema, rownum: int) -> pa.RecordBatch:
    body: str | bytes = f"8=FIX.4.4|35=D|11=ORDER-{rownum}|55=AAPL|10=0|"
    if pa.types.is_binary(schema.field("body").type):
        body = body.encode()
    return pa.RecordBatch.from_arrays(
        [
            pa.array(["file:///capture.log"]),
            pa.array([rownum], type=pa.int64()),
            pa.array(["provenance"]),
            pa.array([body], type=schema.field("body").type),
            pa.array([None], type=pa.string()),
        ],
        schema=schema,
    )


def _body_reader(*bodies: bytes) -> pa.RecordBatchReader:
    schema = _schema()
    batch = pa.RecordBatch.from_arrays(
        [
            pa.array(["file:///capture.log"] * len(bodies)),
            pa.array(range(1, len(bodies) + 1), type=pa.int64()),
            pa.array(["provenance"] * len(bodies)),
            pa.array(bodies, type=pa.binary()),
            pa.array([None] * len(bodies), type=pa.string()),
        ],
        schema=schema,
    )
    return pa.RecordBatchReader.from_batches(schema, [batch])


def test_arrow_parse_is_lazy_bounded_and_source_first(registry: FixRegistry) -> None:
    schema = _schema()
    pulled: list[int] = []

    def batches() -> Iterator[pa.RecordBatch]:
        for rownum in (1, 2):
            pulled.append(rownum)
            yield _batch(schema, rownum)

    source = pa.RecordBatchReader.from_batches(schema, batches())
    parsed = parse_arrow_reader(
        source,
        registry,
        batch_row_size=1,
        max_row_size=1,
    )

    assert pulled == [], "construction reads the schema, not a source batch"
    assert parsed.schema.names[:5] == [
        "url",
        "rownum",
        "branch",
        "body",
        "fixbranch",
    ]
    assert "35" in parsed.schema.names
    assert parsed.schema.field("url").metadata == {b"source": b"capture"}
    assert b"python.class" not in (parsed.schema.metadata or {})

    batch = parsed.read_next_batch()
    assert pulled == [1]
    assert batch.num_rows == 1
    assert batch.column("rownum").to_pylist() == [1]
    assert batch.column("branch").to_pylist() == ["provenance"]
    assert batch.column("35").null_count == 0
    with pytest.raises(StopIteration):
        parsed.read_next_batch()
    assert pulled == [1], "the result row bound stops before the next source batch"


def test_source_collision_and_zero_batch_bound_fail_before_pull(
    registry: FixRegistry,
) -> None:
    pulled = False

    def batches() -> Iterator[pa.RecordBatch]:
        nonlocal pulled
        pulled = True
        yield _batch(_schema(), 1)

    collision = _schema(pa.field("EnTrIeS", pa.string()))
    source = pa.RecordBatchReader.from_batches(collision, batches())
    with pytest.raises(ValueError, match='duplicate "EnTrIeS"'):
        parse_arrow_reader(source, registry)
    assert not pulled

    source = pa.RecordBatchReader.from_batches(_schema(), batches())
    with pytest.raises(ValueError, match="positive row count"):
        parse_arrow_reader(source, registry, batch_row_size=0)
    assert not pulled


def test_registry_may_use_the_process_default() -> None:
    assert "parse_arrow_reader" in fix.__all__
    source = pa.RecordBatchReader.from_batches(_schema(), [])
    parsed = parse_arrow_reader(source, registry=None)
    assert parsed.schema.names[:5] == [
        "url",
        "rownum",
        "branch",
        "body",
        "fixbranch",
    ]


def test_utf8_payload_is_accepted_too(registry: FixRegistry) -> None:
    schema = _schema(body_type=pa.string())
    source = pa.RecordBatchReader.from_batches(schema, [_batch(schema, 1)])
    parsed = parse_arrow_reader(source, registry)
    assert parsed.read_next_batch().column("35").null_count == 0


def test_text_clock_derives_utc_microseconds_without_failing_the_stream() -> None:
    sending_time = Field("sendingtime", "utf8")
    sending_time.fix.tag = 52
    registry = FixRegistry.from_fields([sending_time])
    source = _body_reader(
        b"52=20260814-00:05:01.148123456|",
        b"52=20260814-00:05:01.148123|",
        b"52=19691231-23:59:59.999999|",
        b"52=not-a-clock|",
    )

    table = parse_arrow_reader(source, registry).read_all()
    assert table.schema.field("30004").type == pa.timestamp("us", tz="UTC")
    assert table.column("30004").to_pylist() == [
        datetime(2026, 8, 14, 0, 5, 1, 148123, tzinfo=timezone.utc),
        datetime(2026, 8, 14, 0, 5, 1, 148123, tzinfo=timezone.utc),
        datetime(1969, 12, 31, 23, 59, 59, 999999, tzinfo=timezone.utc),
        None,
    ]
    timestamp = table.column("30004").to_pylist()[0]
    assert table.column("30005").to_pylist() == [
        int(timestamp.timestamp()) // 3600 * 3600,
        int(timestamp.timestamp()) // 3600 * 3600,
        -3600,
        None,
    ]


def test_bridge_party_group_aligns_to_the_complete_registry_item(
    registry: FixRegistry,
) -> None:
    body = (
        b"toBridge #ISINCODE=XX0000084733|#CFICODE=FXXXSX|#SYMBOL=TTF|"
        b"#SIDE=1|#ORDERQTY=1200|#PRICE=41.2500|#NOPARTYIDS=2|"
        b"#NOPARTYIDS[0]=PARTYID=BUYSIDE\x01PARTYIDSOURCE=D\x01PARTYROLE=1|"
        b"#NOPARTYIDS[1]=PARTYID=XPAR\x01PARTYIDSOURCE=G\x01PARTYROLE=17|"
        b"#TRANSACTTIME=20260814-00:05:01.148|#UNKNOWNVENUEFIELD=Z9"
    )

    group = parse_arrow_reader(_body_reader(body), registry).read_all().column("453").to_pylist()[0]
    assert len(group) == 2
    assert group[0]["partyid"] == "BUYSIDE"
    assert group[1]["partyid"] == "XPAR"
    assert group[0]["partyrolequalifier"] is None
    assert group[1]["partyrolequalifier"] is None


def test_regulatory_group_timestamp_drives_capture_clock(
    registry: FixRegistry,
) -> None:
    body = (
        b"MSGTYPE=8|#NOTRDREGTIMESTAMPS=1|"
        b"#NOTRDREGTIMESTAMPS[0]=TRDREGTIMESTAMP=20240102-10:15:30.148123456\x01"
        b"TRDREGTIMESTAMPTYPE=1"
    )
    table = parse_arrow_reader(_body_reader(body), registry).read_all()
    assert table.column("30004").to_pylist() == [
        datetime(2024, 1, 2, 10, 15, 30, 148123, tzinfo=timezone.utc)
    ]
    assert table.column("30005").to_pylist()[0] is not None


def test_tz_timestamp_preserves_its_offset() -> None:
    tz_timestamp = Field("tztimestamp", 'datetime64(us,"UTC")')
    tz_timestamp.fix.tag = 52
    registry = FixRegistry.from_fields([tz_timestamp])
    table = parse_arrow_reader(
        _body_reader(
            b"52=20260814-00:05:01.148123456+02:00|",
            b"52=20260813-22:05:01.148123Z|",
            b"52=20260814-00:05:01.148123456+02|",
        ),
        registry,
    ).read_all()
    assert table.column("52").to_pylist() == [
        datetime(2026, 8, 13, 22, 5, 1, 148123, tzinfo=timezone.utc),
        datetime(2026, 8, 13, 22, 5, 1, 148123, tzinfo=timezone.utc),
        datetime(2026, 8, 13, 22, 5, 1, 148123, tzinfo=timezone.utc),
    ]


def test_historical_text_clock_conforms_to_fixed_timestamp(
    registry: FixRegistry,
) -> None:
    table = parse_arrow_reader(
        _body_reader(b"8=FIX.4.1|52=20260814-00:05:01.148123456|35=D|10=0|"),
        registry,
    ).read_all()
    assert table.column("52").to_pylist() == [
        datetime(2026, 8, 14, 0, 5, 1, 148123, tzinfo=timezone.utc)
    ]


def test_inferred_directions_use_fix_codes(registry: FixRegistry) -> None:
    table = parse_arrow_reader(
        _body_reader(
            b"sending >> 8=FIX.4.4|35=D|10=0|",
            b"receiving >> 8=FIX.4.4|35=D|10=0|",
        ),
        registry,
    ).read_all()
    assert table.column("385").to_pylist() == ["S", "R"]


def test_standard_numeric_party_group_uses_registry_layout(
    registry: FixRegistry,
) -> None:
    body = (
        b"8=FIX.4.4|35=D|453=2|448=BUYSIDE|447=D|452=1|"
        b"448=XPAR|447=G|452=17|10=0|"
    )
    group = parse_arrow_reader(_body_reader(body), registry).read_all().column("453").to_pylist()[0]
    assert [party["partyid"] for party in group] == ["BUYSIDE", "XPAR"]


def test_multibyte_malformed_date_is_null_without_losing_row(
    registry: FixRegistry,
) -> None:
    table = parse_arrow_reader(_body_reader("75=123é567|".encode()), registry).read_all()
    assert table.num_rows == 1
    assert table.column("75").to_pylist() == [None]
    assert table.column("body").to_pylist() == ["75=123é567|".encode()]


def test_fixt_row_carries_resolved_application_version(registry: FixRegistry) -> None:
    table = parse_arrow_reader(
        _body_reader(b"8=FIXT.1.1|1128=6|35=D|11=ORDER-1|10=0|"), registry
    ).read_all()
    assert table.column("30002").to_pylist() == ["4.4"]
