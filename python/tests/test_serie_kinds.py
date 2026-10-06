"""Native row/chunk/key kinds share generic reads, writes, and conversions."""

import pyarrow as pa
import pytest
import yggdryl as yd


def sample():
    return yd.Serie.from_(pa.table({"venue": ["a", "a", "b"], "qty": [1, 2, 3]}))


def test_streams_and_key_layouts_are_native():
    rows = sample()
    groups = rows.window_by("venue as desk")
    assert isinstance(groups, yd.KeySeries)
    assert len(groups) == 2
    first = groups[0]
    assert isinstance(first, yd.KeySerie)
    assert first.rownum == 0
    assert [field.name for field in first.key_field] == ["desk"]
    assert [field.name for field in first.serie_field] == ["qty"]
    assert [field.name for field in first.field] == ["desk", "qty"]
    assert str(first.key_paths[0]) == "venue"
    assert first.rows.into_arrow_batch().to_pydict() == {"qty": [1, 2]}
    with pytest.raises(TypeError):
        len(first)
    with pytest.raises(TypeError):
        iter(first)
    stream = groups.into_stream()
    assert isinstance(stream, yd.StreamSerie)
    assert isinstance(next(stream), yd.Scalar)
    assert len(stream.collect()) == 2


def test_chunk_stream_protocol_and_optional_bounds():
    stream = sample().into_stream().into_chunked_stream(row_size=2, byte_size=None)
    assert isinstance(stream, yd.StreamChunkedSerie)
    assert stream.schema.names == ["venue", "qty"]
    assert stream.read_next_batch().num_rows == 2
    assert stream.read_all().num_rows == 1
    with pytest.raises(StopIteration):
        stream.read_next_batch()
    assert [len(chunk) for chunk in sample().into_chunked_stream(None, None)] == [3]
    assert [len(chunk) for chunk in sample().into_chunked_stream(2)] == [3]
    assert [len(chunk) for chunk in sample().window_by("venue").into_chunked_stream()] == [2, 1]


def test_generic_reads_and_writes_accept_key_kinds(tmp_path):
    path = yd.holder.LocalPath(str(tmp_path / "rows.csv"))
    result = path.overwrite_serie(sample().window_by("venue"))
    assert result.written_rows == 3
    read = path.read_serie()
    assert isinstance(read, yd.Serie)
    assert len(list(read.into_stream())) == 3
    path.overwrite_serie(sample().into_stream())
    assert len(list(path.read_serie().into_stream())) == 3


def test_key_intake_refuses_untyped_value_lists():
    with pytest.raises(TypeError, match="Serie.from_"):
        sample().partition_by(["a", "a", "b"])
    assert len(sample().partition_by("venue")) == 2


def test_streamed_keys_lend_payloads_in_order():
    stream = sample().into_stream().window_by("venue")
    assert isinstance(stream, yd.StreamKeySerie)
    assert [field.name for field in stream.key_field] == ["venue"]
    first = next(stream)
    second = next(stream)
    with pytest.raises(ValueError):
        list(first.rows.into_stream())
    assert len(list(second.rows.into_stream())) == 1
    assert list(stream) == []


def test_retired_names_are_absent():
    for name in ("SerieReader", "SerieReaderWindows", "SerieReaderPartitions", "Records"):
        assert not hasattr(yd, name)
    assert not hasattr(sample(), "partition_by_paths")


def test_live_key_payload_reads_mutations_and_drop_finish_under_a_deadline():
    import subprocess
    import sys

    # A Python producer can be reached through the generic payload, including
    # its materializing mutators. A subprocess bounds any GIL/drop deadlock.
    subprocess.run(
        [sys.executable, "-c", """
import gc
import yggdryl as yd
root = yd.Field('row', 'struct<venue: utf8 not null, qty: int64 not null>', nullable=False)
def payload():
    walk = yd.StreamSerie.from_rows(root, iter([['a', 1], ['a', 2], ['b', 3]])).window_by('venue')
    return walk, next(walk).rows
walk, rows = payload()
assert rows.child('qty').as_py() == [1, 2]
walk, rows = payload()
rows.set_cell('qty', 0, 7)
assert rows.child('qty').as_py() == [7, 2]
walk, rows = payload()
assert rows.into_arrow_batch().num_rows == 2
walk, rows = payload()
del walk, rows
gc.collect()
"""],
        check=True,
        timeout=30,
    )
