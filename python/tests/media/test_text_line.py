"""The decoded text row, its entry tree, and the path that addresses one."""

from __future__ import annotations

import pickle

import pytest

import yggdryl
from yggdryl import FieldPath, TextOptions
from yggdryl.holder import Buffer

CAPTURE = b"8=FIX|55=AAPL|35=D\n35=D|55=MSFT\n"


def lifted() -> TextOptions:
    options = TextOptions()
    options.lift_names = ["55"]
    return options


def source(payload: bytes = CAPTURE) -> Buffer:
    return Buffer.from_bytes(payload)


class TestFieldPath:
    def test_parses_every_segment_kind(self) -> None:
        path = FieldPath("order.line[0]")
        assert len(path) == 3
        assert path.segments == ("order", "line", 0)
        assert str(path) == "order.line[0]"

    def test_a_name_carrying_a_dot_is_one_segment(self) -> None:
        assert len(FieldPath('"a.b"')) == 1
        assert FieldPath('"a.b"').name == "a.b"
        assert len(FieldPath("a.b")) == 2

    def test_the_root_selects_what_it_is_applied_to(self) -> None:
        assert FieldPath.root().is_root
        assert len(FieldPath()) == 0

    def test_parents_and_joins_build_without_reparsing(self) -> None:
        path = FieldPath("order").join("line").join(2)
        assert str(path) == "order.line[2]"
        assert str(path.parent()) == "order.line"

    def test_a_malformed_path_is_refused_where_it_stopped(self) -> None:
        with pytest.raises(ValueError, match="field path"):
            FieldPath("order.")

    def test_equal_paths_hash_and_pickle_alike(self) -> None:
        path = FieldPath("order.line[0]")
        assert path == FieldPath("order.line[0]")
        assert hash(path) == hash(FieldPath("order.line[0]"))
        assert pickle.loads(pickle.dumps(path)) == path


class TestTextLine:
    def test_every_line_becomes_one_typed_row(self) -> None:
        lines = list(source().read_text_lines(options=TextOptions()))
        assert [line.index for line in lines] == [0, 1]
        assert lines[0].body == b"8=FIX|55=AAPL|35=D"
        assert lines[0].url is not None

    def test_a_read_wanting_no_entry_builds_no_tree(self) -> None:
        lines = list(source().read_text_lines(options=TextOptions()))
        assert lines[0].entries is None
        assert lines[0].bodytype is None

    def test_a_lifted_path_builds_the_tree_and_is_found(self) -> None:
        lines = list(source().read_text_lines(options=lifted()))
        assert lines[0].entries is not None
        assert lines[0].get_entry_by_path("55").value == b"AAPL"
        assert lines[1].get_entry_by_path("55").value == b"MSFT"

    def test_a_miss_is_none_and_the_raising_form_says_so(self) -> None:
        line = next(iter(source().read_text_lines(options=lifted())))
        assert line.get_entry_by_path("nosuch") is None
        with pytest.raises(ValueError, match="nosuch"):
            line.entry_by_path("nosuch")

    def test_a_resolved_path_and_its_text_reach_the_same_entry(self) -> None:
        line = next(iter(source().read_text_lines(options=lifted())))
        assert line.get_entry_by_path(FieldPath('"55"')).value == b"AAPL"

    def test_the_setter_creates_and_the_remover_takes(self) -> None:
        line = next(iter(source().read_text_lines(options=lifted())))
        line.set_entry_by_path("order.price", b"12")
        assert line.get_entry_by_path("order.price").value == b"12"
        removed = line.remove_entry_by_path("order")
        assert removed is not None
        assert line.get_entry_by_path("order.price") is None

    def test_entries_index_and_measure(self) -> None:
        line = next(iter(source().read_text_lines(options=lifted())))
        entries = line.entries
        assert len(entries) >= 2
        assert entries[0].key == b"8"
        assert entries[-1].value is not None

    def test_the_iterator_is_lazy_and_fuses(self) -> None:
        lines = source().read_text_lines(options=TextOptions())
        assert iter(lines) is lines
        assert next(lines).index == 0
        assert next(lines).index == 1
        with pytest.raises(StopIteration):
            next(lines)
        with pytest.raises(StopIteration):
            next(lines)


class TestTextOptions:
    def test_lift_names_round_trips(self) -> None:
        options = lifted()
        assert options.lift_names is not None
        options.lift_names = None
        assert options.lift_names is None

    def test_rename_columns_round_trips_and_renames(self) -> None:
        options = TextOptions()
        options.rename_columns = {"body": "payload"}
        assert options.rename_columns == {"body": "payload"}
        names = [child.name for child in options.source_field()]
        assert "payload" in names
        assert "body" not in names

    def test_a_rename_naming_no_column_is_refused(self) -> None:
        options = TextOptions()
        options.rename_columns = {"nosuch": "x"}
        with pytest.raises(ValueError, match="nosuch"):
            options.source_field()

    def test_a_lifted_column_appears_in_the_schema_before_any_read(self) -> None:
        names = [child.name for child in lifted().source_field()]
        assert "55" in names

    def test_the_options_stay_hashable_with_both_new_fields(self) -> None:
        options = lifted()
        options.rename_columns = {"body": "payload"}
        assert isinstance(hash(options), int)


def test_the_native_module_exports_every_new_name() -> None:
    for name in ("FieldPath", "TextLine", "TextEntry", "TextEntries", "TextLines"):
        assert hasattr(yggdryl, name) or hasattr(yggdryl.media, name), name
