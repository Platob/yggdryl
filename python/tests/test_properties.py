"""`python/src/properties.rs`: option properties given by name, and the
warning a name no property owns raises.

Every options-taking door takes the options' properties as keywords beside
`options`, each set on a copy of the options by its own setter. A keyword no
setter owns is skipped with an `UnknownPropertyWarning` naming it and the
closest property there is, so a typo is heard without failing the call.
"""

from __future__ import annotations

import pathlib
import warnings

import pyarrow as pa
import pytest

import yggdryl
from yggdryl import IOBase, RecordOptions, TextOptions, UnknownPropertyWarning


def _csv(tmp_path: pathlib.Path) -> IOBase:
    handle = IOBase(tmp_path / "trades.csv")
    handle.write_bytes(b"symbol;price\nAAPL;187\n")
    return handle


def test_the_warning_is_a_user_warning_at_the_package_root() -> None:
    assert issubclass(UnknownPropertyWarning, UserWarning)
    assert yggdryl.UnknownPropertyWarning is UnknownPropertyWarning


def test_an_unknown_keyword_warns_naming_the_closest_property(
    tmp_path: pathlib.Path,
) -> None:
    handle = _csv(tmp_path)

    # The typo is heard - with the property it most likely meant - and the
    # read still answers, under the options the other keywords set.
    with pytest.warns(
        UnknownPropertyWarning,
        match=r"RecordOptions has no settable property 'seperator'; it is ignored; "
        r"did you mean 'separator'\?",
    ):
        table = handle.read_arrow_reader(seperator=";", header=True).read_all()
    assert table.num_columns == 1, "the default separator read the one column"

    with warnings.catch_warnings():
        warnings.simplefilter("error")
        table = handle.read_arrow_reader(separator=";").read_all()
    assert table.column_names == ["symbol", "price"]


def test_a_name_far_from_every_property_suggests_none(tmp_path: pathlib.Path) -> None:
    with pytest.warns(UnknownPropertyWarning) as caught:
        _csv(tmp_path).read_arrow_reader(warehouse="s3://lake").read_all()
    assert "did you mean" not in str(caught[0].message)


def test_an_escalated_warning_is_the_error_it_names(tmp_path: pathlib.Path) -> None:
    handle = _csv(tmp_path)
    with warnings.catch_warnings():
        warnings.simplefilter("error", UnknownPropertyWarning)
        with pytest.raises(UnknownPropertyWarning, match="seperator"):
            handle.read_arrow_reader(seperator=";")


def test_the_warning_points_at_the_callers_line(tmp_path: pathlib.Path) -> None:
    handle = _csv(tmp_path)
    with pytest.warns(UnknownPropertyWarning) as caught:
        handle.read_arrow_field(seperator=";")
    assert caught[0].filename == __file__


def test_a_property_of_the_other_class_is_no_property_of_this_one(
    tmp_path: pathlib.Path,
) -> None:
    # `rowheader` is a text setting: on CSV options it names nothing to set.
    with pytest.warns(UnknownPropertyWarning, match="'rowheader'"):
        _csv(tmp_path).read_arrow_field(rowheader="^x")
    # A known name the encoding cannot honour is the setter's own refusal.
    with pytest.raises(ValueError, match=r"\$\.sheet"):
        _csv(tmp_path).read_arrow_field(sheet="Sheet1")


def test_a_read_only_name_is_no_settable_property(tmp_path: pathlib.Path) -> None:
    with pytest.warns(UnknownPropertyWarning, match="'mime_type'"):
        _csv(tmp_path).read_arrow_field(mime_type="text/plain")


def test_an_absent_value_is_skipped_and_its_name_still_checked(
    tmp_path: pathlib.Path,
) -> None:
    handle = _csv(tmp_path)
    with warnings.catch_warnings():
        warnings.simplefilter("error")
        assert handle.read_arrow_field(separator=...) == handle.read_arrow_field()
    with pytest.warns(UnknownPropertyWarning, match="'seperator'"):
        handle.read_arrow_field(seperator=...)


def test_the_given_options_are_copied_and_never_changed(
    tmp_path: pathlib.Path,
) -> None:
    options = RecordOptions("text/csv")
    table = _csv(tmp_path).read_arrow_reader(options=options, separator=";").read_all()
    assert table.column_names == ["symbol", "price"]
    assert options.separator == ","


@pytest.mark.parametrize("options_class", [RecordOptions, TextOptions])
def test_every_settable_property_is_a_keyword(
    options_class: type[RecordOptions] | type[TextOptions],
) -> None:
    # What an assignment accepts, the fold accepts: each writable attribute
    # set back to its own value by keyword warns about nothing.
    template = (
        RecordOptions("text/csv") if options_class is RecordOptions else TextOptions()
    )
    for name in dir(options_class):
        if name.startswith("_") or type(getattr(options_class, name)).__name__ != (
            "getset_descriptor"
        ):
            continue
        value = getattr(template, name)
        probe = template.__copy__()
        try:
            setattr(probe, name, value)
        except (AttributeError, TypeError, ValueError):
            continue
        with warnings.catch_warnings():
            warnings.simplefilter("error", UnknownPropertyWarning)
            if options_class is RecordOptions:
                RecordOptions("text/csv", **{name: value})
            else:
                TextOptions(**{name: value})


def test_text_properties_land_on_the_handles_own_text_options(
    tmp_path: pathlib.Path,
) -> None:
    handle = IOBase(tmp_path / "app.log")
    handle.write_bytes(b"INFO started\nWARN slow\n")
    table = handle.read_arrow_reader(rowheader=r"^(?<level>[A-Z]+) ").read_all()
    assert table.column("level").to_pylist() == ["INFO", "WARN"]
    assert pa.types.is_string(table.schema.field("body").type)
