"""The decoded text row, its entry tree, and the path that addresses one."""

from __future__ import annotations

import pickle

import pytest

import yggdryl
from yggdryl import FieldPath, TextLine, TextOptions
from yggdryl.holder import Buffer

CAPTURE = b"8=FIX|55=AAPL|35=D\n35=D|55=MSFT\n"


def lifted() -> TextOptions:
    options = TextOptions()
    options.lift_names = ["55"]
    return options


def lifted_58() -> TextOptions:
    options = TextOptions()
    options.lift_names = ["58"]
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
        assert lines[0].body == "8=FIX|55=AAPL|35=D"
        assert lines[0].url is not None

    def test_a_read_wanting_no_entry_builds_no_tree(self) -> None:
        lines = list(source().read_text_lines(options=TextOptions()))
        assert lines[0].entries is None
        assert lines[0].bodytype is None

    def test_a_lifted_path_builds_the_tree_and_is_found(self) -> None:
        lines = list(source().read_text_lines(options=lifted()))
        assert lines[0].entries is not None
        assert lines[0].get_entry_by_path("55").value == "AAPL"
        assert lines[1].get_entry_by_path("55").value == "MSFT"

    def test_a_miss_is_none_and_the_raising_form_says_so(self) -> None:
        line = next(iter(source().read_text_lines(options=lifted())))
        assert line.get_entry_by_path("nosuch") is None
        with pytest.raises(ValueError, match="nosuch"):
            line.entry_by_path("nosuch")

    def test_a_resolved_path_and_its_text_reach_the_same_entry(self) -> None:
        line = next(iter(source().read_text_lines(options=lifted())))
        assert line.get_entry_by_path(FieldPath('"55"')).value == "AAPL"

    def test_the_setter_creates_and_the_remover_takes(self) -> None:
        line = next(iter(source().read_text_lines(options=lifted())))
        line.set_entry_by_path("order.price", b"12")
        assert line.get_entry_by_path("order.price").value == "12"
        line.set_entry_by_path("order.price", "13")
        assert line.get_entry_by_path("order.price").value == "13"
        line.set_entry_by_path("order.price", bytearray(b"14"))
        line.set_entry_by_path("order.price", memoryview(b"15"))
        assert line.get_entry_by_path("order.price").value_bytes == b"15"
        with pytest.raises(TypeError, match="str, bytes, bytearray, or memoryview"):
            line.set_entry_by_path("order.price", 16)  # type: ignore[arg-type]
        removed = line.remove_entry_by_path("order")
        assert removed is not None
        assert line.get_entry_by_path("order.price") is None

    def test_entries_index_and_measure(self) -> None:
        line = next(iter(source().read_text_lines(options=lifted())))
        entries = line.entries
        assert len(entries) >= 2
        assert entries[0].key == "8"
        assert entries[0].key_bytes == b"8"
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


class TestTextIsDecodedWhereTheLineIsMade:
    def test_one_latin_1_byte_among_utf_8_reaches_the_body_as_text(self) -> None:
        line = TextLine(0, b"58=caf\xe9|10=0|")
        assert line.body == "58=caf\u00e9|10=0|"
        assert line.decoded_byte_size == 1
        # A valid `\u00e9` beside the lone byte is kept as it is.
        mixed = TextLine(0, b"58=caf\xe9 caf\xc3\xa9|10=0|")
        assert mixed.body == "58=caf\u00e9 caf\u00e9|10=0|"
        assert mixed.decoded_byte_size == 1

    def test_a_text_body_costs_no_decode(self) -> None:
        for body in ("58=caf\u00e9|10=0|", b"58=caf\xc3\xa9|10=0|"):
            line = TextLine(0, body)
            assert line.body == "58=caf\u00e9|10=0|"
            assert line.decoded_byte_size == 0

    def test_every_byte_spelling_is_one_intake(self) -> None:
        for body in (bytearray(b"58=caf\xe9|"), memoryview(b"58=caf\xe9|")):
            line = TextLine(0, body)
            assert line.body == "58=caf\u00e9|"
            assert line.decoded_byte_size == 1
        with pytest.raises(TypeError, match="str, bytes, bytearray, or memoryview"):
            TextLine(0, 7)  # type: ignore[arg-type]

    def test_the_five_undefined_cp1252_bytes_read_as_the_controls_of_their_number(
        self,
    ) -> None:
        for byte in (0x81, 0x8D, 0x8F, 0x90, 0x9D):
            line = TextLine(0, b"a" + bytes([byte]) + b"b")
            assert line.body == "a" + chr(byte) + "b", hex(byte)
            assert line.decoded_byte_size == 1
        # The defined row reads as Windows-1252 tables it.
        assert TextLine(0, b"\x80 \x93q\x94").body == "\u20ac \u201cq\u201d"

    def test_captures_answer_text_and_count_with_the_body(self) -> None:
        line = TextLine(0, b"body", ["FIX.4.4", None])
        assert line.captures == ("FIX.4.4", None)
        assert line.decoded_byte_size == 0

    def test_an_entry_answers_text_and_its_bytes_beside_it(self) -> None:
        line = next(iter(source(b"58=caf\xe9|10=0|\n").read_text_lines(options=lifted_58())))
        assert line.decoded_byte_size == 1
        entry = line.entry_by_path("58")
        assert entry.value == "caf\u00e9"
        assert isinstance(entry.value, str)
        assert entry.value_bytes == "caf\u00e9".encode()
        assert isinstance(entry.value_bytes, bytes)
        assert entry.key == "58"
        assert entry.key_bytes == b"58"


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


class TestFieldPathAlias:
    def test_a_trailing_as_names_what_the_path_reached(self) -> None:
        path = FieldPath("order.line[0].price as price")
        assert path.alias == "price"
        assert path.column_name == "price"
        assert len(path) == 4
        assert str(path) == "order.line[0].price as price"

    def test_without_an_alias_the_last_segment_names_it(self) -> None:
        path = FieldPath("order.line.price")
        assert path.alias is None
        assert path.column_name == "price"

    def test_a_segment_may_still_be_named_as(self) -> None:
        assert FieldPath("order.as").alias is None
        assert FieldPath("order.as").column_name == "as"
        assert FieldPath("assets").alias is None

    def test_an_alias_is_part_of_the_value(self) -> None:
        assert FieldPath("price as unit") != FieldPath("price")
        assert FieldPath("price as unit") == FieldPath("price as unit")

    def test_a_malformed_alias_is_refused(self) -> None:
        for text in ("price as", "price as one two", "as name"):
            with pytest.raises(ValueError, match="field path"):
                FieldPath(text)

    def test_a_lifted_path_names_its_column_with_its_alias(self) -> None:
        options = TextOptions()
        options.lift_names = ['"55" as symbol']
        names = [child.name for child in options.source_field()]
        assert "symbol" in names
        assert "55" not in names
        line = next(iter(source(b"55=AAPL\n").read_text_lines(options=options)))
        assert line.get_entry_by_path("55").value == "AAPL"
