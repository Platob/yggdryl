"""``python/src/spill.rs``: the bound a column stays resident under, and the
folder it spills to.

Mirrors ``rust/tests/root/spill.rs`` for ``SpillOptions`` itself, and pins
the one fold every ``spill`` door reads its arguments through: ``options`` -
the process default for ``None`` - with ``byte_size`` and ``folder`` set on a
copy where given, ``...`` being a keyword not given. The process default is
read once per process, so what reads the environment runs in a fresh
interpreter.
"""

from __future__ import annotations

import copy
import inspect
import os
import pathlib
import pickle
import subprocess
import sys
import textwrap

import pytest

import yggdryl
from yggdryl import DEFAULT_SPILL_BYTE_SIZE, Field, Serie, SpillOptions, Url
from yggdryl.holder import LocalFile, LocalFolder, LocalPath


def prices(rows: int) -> Serie:
    return Serie.from_scalars(Field("price", "int64", nullable=False), list(range(rows)))


def test_new_options_are_the_default_bound_over_the_platform_temporary_folder() -> None:
    options = SpillOptions()
    assert DEFAULT_SPILL_BYTE_SIZE == 64 * 1024 * 1024
    assert yggdryl.DEFAULT_SPILL_BYTE_SIZE == DEFAULT_SPILL_BYTE_SIZE
    assert options.byte_size == DEFAULT_SPILL_BYTE_SIZE
    assert options.folder is None
    assert not options.is_never()
    # The signature states the core's default, which the options carry.
    signature = inspect.signature(SpillOptions)
    assert signature.parameters["byte_size"].default == options.byte_size
    assert signature.parameters["folder"].default is None


def test_never_is_the_largest_bound_and_zero_spills_everything_rather_than_nothing() -> None:
    assert SpillOptions.NEVER == 2**64 - 1
    assert SpillOptions(SpillOptions.NEVER).is_never()
    assert not SpillOptions(0).is_never()
    assert not SpillOptions(SpillOptions.NEVER - 1).is_never()
    with pytest.raises(OverflowError):
        SpillOptions(-1)


def test_every_folder_spelling_names_one_location(tmp_path: pathlib.Path) -> None:
    expected = SpillOptions(4096, LocalFolder(tmp_path))
    held = expected.folder
    assert isinstance(held, LocalFolder)
    for spelling in (
        tmp_path,
        str(tmp_path),
        Url.from_path(tmp_path),
        str(Url.from_path(tmp_path)),
        LocalPath(tmp_path),
        held,
    ):
        options = SpillOptions(4096, spelling)
        assert options == expected, spelling
        assert hash(options) == hash(expected)
        folder = options.folder
        assert isinstance(folder, LocalFolder)
        assert str(folder.url) == str(held.url)
    # Each fact moves alone.
    assert SpillOptions(4096).folder is None
    assert SpillOptions(folder=tmp_path).byte_size == DEFAULT_SPILL_BYTE_SIZE


def test_a_folder_that_is_no_local_directory_is_refused_by_name(tmp_path: pathlib.Path) -> None:
    with pytest.raises(ValueError, match="only a file URI"):
        SpillOptions(0, "s3://bucket/spill")
    with pytest.raises(TypeError, match="LocalFile"):
        SpillOptions(0, LocalFile(tmp_path / "file.bin"))
    with pytest.raises(TypeError):
        SpillOptions(0, 7)


def test_options_over_one_folder_path_are_equal_and_over_two_are_not(
    tmp_path: pathlib.Path,
) -> None:
    def over(name: str) -> SpillOptions:
        return SpillOptions(folder=tmp_path / name)

    assert over("a") == over("a")
    assert hash(over("a")) == hash(over("a"))
    assert over("a") != over("b")
    # A stated folder is not the platform default, even when it names the
    # same directory: `None` is no statement.
    assert over("a") != SpillOptions()
    assert over("a") != SpillOptions(DEFAULT_SPILL_BYTE_SIZE - 1, tmp_path / "a")
    assert SpillOptions() != SpillOptions(0)
    assert SpillOptions() == SpillOptions()
    assert SpillOptions() != DEFAULT_SPILL_BYTE_SIZE


def test_options_are_immutable_and_copy_pickle_and_print_as_themselves(
    tmp_path: pathlib.Path,
) -> None:
    for options in (SpillOptions(), SpillOptions(0, tmp_path)):
        assert pickle.loads(pickle.dumps(options)) == options
        assert copy.copy(options) == options
        assert copy.deepcopy(options) == options
        assert {options: 1}[copy.copy(options)] == 1
    assert repr(SpillOptions(7)) == "SpillOptions(byte_size=7)"
    assert repr(SpillOptions(0, tmp_path)) == (
        f'SpillOptions(byte_size=0, folder="{Url.from_path(tmp_path)}")'
    )
    with pytest.raises(AttributeError):
        SpillOptions().byte_size = 1  # type: ignore[misc]


def test_the_process_default_is_read_once_and_never_installed_over() -> None:
    # Every door that lays a column out has read the default by now, so it
    # is settled, and the value every caller saw cannot change under them.
    prices(1)
    resolved = SpillOptions.from_env()
    assert SpillOptions.from_env() == resolved
    with pytest.raises(ValueError, match="already resolved"):
        SpillOptions.install_env(SpillOptions(0))
    assert SpillOptions.from_env() == resolved


def fresh(script: str, **environment: str) -> subprocess.CompletedProcess[str]:
    """Run `script` in a fresh interpreter under `environment`, the spill
    variables otherwise unset."""
    variables = {
        name: value
        for name, value in os.environ.items()
        if name not in ("YGGDRYL_SPILL_BYTE_SIZE", "YGGDRYL_SPILL_FOLDER")
    }
    return subprocess.run(
        [sys.executable, "-c", textwrap.dedent(script)],
        capture_output=True,
        text=True,
        timeout=120,
        check=False,
        env={**variables, **environment},
    )


def test_the_environment_states_the_default_and_a_refused_value_names_its_variable(
    tmp_path: pathlib.Path,
) -> None:
    stated = fresh(
        f"""
        from yggdryl import SpillOptions, Url
        options = SpillOptions.from_env()
        assert options.is_never(), options
        assert str(options.folder.url) == str(Url.from_path({str(tmp_path)!r})), options
        """,
        YGGDRYL_SPILL_BYTE_SIZE="Never",
        YGGDRYL_SPILL_FOLDER=str(tmp_path),
    )
    assert stated.returncode == 0, stated.stderr
    refused = fresh(
        """
        from yggdryl import SpillOptions
        try:
            SpillOptions.from_env()
        except ValueError as error:
            print(error)
        else:
            raise SystemExit("a bound that is no byte count was read")
        """,
        YGGDRYL_SPILL_BYTE_SIZE="lots",
    )
    assert refused.returncode == 0, refused.stderr
    assert "YGGDRYL_SPILL_BYTE_SIZE" in refused.stdout and "lots" in refused.stdout


def test_install_env_states_the_default_before_anything_reads_it() -> None:
    installed = fresh(
        """
        from yggdryl import Field, Serie, SpillOptions
        SpillOptions.install_env(SpillOptions(0))
        assert SpillOptions.from_env() == SpillOptions(0)
        try:
            SpillOptions.install_env(SpillOptions())
        except ValueError as error:
            assert "already resolved" in str(error), error
        else:
            raise SystemExit("a second install was taken")
        # Every door that lays a column out settles under it.
        prices = Serie.from_scalars(Field("price", "int64", nullable=False), [1, 2, 3])
        assert prices.is_spilled() and prices.resident_size() == 0
        """,
        YGGDRYL_SPILL_BYTE_SIZE="1",
    )
    assert installed.returncode == 0, installed.stderr


class TestFold:
    """``options`` - the process default for ``None`` - with ``byte_size``
    and ``folder`` set on a copy where given, read through ``Serie.spill``."""

    def test_no_options_is_the_process_default_which_a_small_column_is_under(self) -> None:
        column = prices(1_024)
        column.spill()
        assert not column.is_spilled()
        assert column.resident_size() == column.memory_size()
        column.spill(None)
        assert not column.is_spilled()

    def test_a_keyword_is_set_on_a_copy_and_wins_over_the_options(self) -> None:
        options = SpillOptions(0)
        column = prices(1_024)
        column.spill(options, byte_size=SpillOptions.NEVER)
        assert not column.is_spilled()
        assert options.byte_size == 0
        column.spill(byte_size=0)
        assert column.is_spilled()
        assert column.resident_size() == 0
        assert column.memory_size() == prices(1_024).memory_size()

    def test_a_given_folder_is_where_the_file_is_made_and_none_clears_it(
        self, tmp_path: pathlib.Path
    ) -> None:
        missing = tmp_path / "missing"
        column = prices(256)
        with pytest.raises(ValueError, match="missing"):
            column.spill(byte_size=0, folder=missing)
        # A refused folder leaves the column as it was.
        assert not column.is_spilled()
        assert column.as_py() == list(range(256))
        stated = SpillOptions(0, missing)
        with pytest.raises(ValueError, match="missing"):
            column.spill(stated)
        column.spill(stated, folder=None)
        assert column.is_spilled()
        assert stated.folder is not None
        held = prices(256)
        held.spill(SpillOptions(0), folder=tmp_path)
        assert held.is_spilled()
        # The file is unlinked as it opens.
        assert list(tmp_path.iterdir()) == []

    def test_a_keyword_of_the_wrong_kind_is_refused_before_anything_spills(self) -> None:
        column = prices(256)
        with pytest.raises(TypeError):
            column.spill(byte_size="0")  # type: ignore[arg-type]
        with pytest.raises(TypeError):
            column.spill(folder=7)
        with pytest.raises(TypeError):
            column.spill(0)  # type: ignore[arg-type]
        assert not column.is_spilled()
