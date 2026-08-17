"""``IOBase`` and ``Url`` answer the questions ``pathlib.Path`` answers."""

from __future__ import annotations

import pathlib

import pytest

from yggdryl import IOBase, Url


@pytest.fixture
def lake(tmp_path: pathlib.Path) -> pathlib.Path:
    """Build a small Hive-partitioned lake with one private staging area."""
    root = tmp_path / "lake"
    for year in ("2024", "2025"):
        for month in ("01", "02"):
            leaf = root / f"year={year}" / f"month={month}"
            leaf.mkdir(parents=True)
            (leaf / "part-0.parquet").write_bytes(b"parquet")
            (leaf / "notes.txt").write_text("notes", encoding="utf-8")
    staging = root / ".staging"
    staging.mkdir()
    (staging / "part-0.parquet").write_bytes(b"draft")
    return root


class TestPathlibParity:
    """The same calls a ``Path`` answers, answered by the core."""

    def test_a_handle_reports_what_is_there(self, lake: pathlib.Path) -> None:
        handle = IOBase(lake)

        assert handle.exists()
        assert handle.is_dir()
        assert not handle.is_file()
        assert handle.name == "lake"

    def test_a_missing_location_is_empty_rather_than_an_error(
        self, tmp_path: pathlib.Path
    ) -> None:
        absent = IOBase(tmp_path / "absent.arrows")

        assert not absent.exists()
        # Reads skip, so probing a location needs no existence check first.
        assert absent.read_bytes() == b""
        assert absent.size == 0

    def test_children_are_resolved_the_way_paths_are(self, lake: pathlib.Path) -> None:
        by_operator = IOBase(lake) / "year=2024" / "month=01" / "part-0.parquet"
        by_method = IOBase(lake).joinpath("year=2024", "month=01", "part-0.parquet")

        assert by_operator.name == by_method.name == "part-0.parquet"
        assert by_operator.read_bytes() == b"parquet"
        assert by_operator.parent.name == "month=01"

    def test_iterdir_skips_private_entries_by_default(self, lake: pathlib.Path) -> None:
        handle = IOBase(lake)

        assert sorted(entry.name for entry in handle.iterdir()) == [
            "year=2024",
            "year=2025",
        ]
        assert ".staging" in {entry.name for entry in handle.iterdir(include_private=True)}
        # Iterating the handle itself is iterdir, as it is for a Path.
        assert len(list(handle)) == 2

    def test_glob_and_rglob_select_the_same_leaves(self, lake: pathlib.Path) -> None:
        handle = IOBase(lake)

        assert len(handle.glob("**/*.parquet")) == 4
        assert len(handle.rglob("*.parquet")) == 4
        assert len(handle.glob("year=2024/**/*.parquet")) == 2
        # One plain segment stays at one level, where there are no leaves.
        assert handle.glob("*.parquet") == []

    def test_a_write_creates_and_a_read_returns_it(self, tmp_path: pathlib.Path) -> None:
        handle = IOBase(tmp_path / "trades.txt")

        assert handle.write_text("AAPL") == 4
        assert handle.read_text() == "AAPL"
        assert handle.exists()
        assert handle.size == 4

        handle.unlink()
        assert handle.read_bytes() == b""

    def test_positional_access_needs_no_mode(self, tmp_path: pathlib.Path) -> None:
        handle = IOBase(tmp_path / "positional.bin")
        handle.write_bytes(b"symbol,price")

        # Random access is the contract, so there is nothing to open or seek.
        assert handle.pread(0, 6) == b"symbol"
        handle.pwrite(0, b"SYMBOL")
        assert handle.read_bytes() == b"SYMBOL,price"
        assert handle.append(b"!") == 12

    def test_mkdir_and_touch_bring_a_location_into_being(
        self, tmp_path: pathlib.Path
    ) -> None:
        folder = IOBase(tmp_path / "nested" / "deep")
        folder.mkdir()
        assert folder.is_dir()

        leaf = folder / "empty.arrows"
        leaf.touch()
        assert leaf.exists()
        assert leaf.size == 0

        leaf.write_text("kept")
        leaf.touch()
        # `touch` never truncates an existing leaf, as it does not for a Path.
        assert leaf.read_text() == "kept"

    def test_a_memory_handle_needs_no_location(self) -> None:
        handle = IOBase.from_bytes(b"AAPL")

        assert handle.read_bytes() == b"AAPL"
        # A buffer still has an identity, but not one the file system knows.
        assert handle.url.scheme == "mem"
        with pytest.raises(ValueError):
            handle.__fspath__()


class TestPartitions:
    """A Hive layout stores columns in the path, and the path is readable."""

    def test_a_leaf_knows_the_partitions_above_it(self, lake: pathlib.Path) -> None:
        leaf = IOBase(lake) / "year=2024" / "month=01" / "part-0.parquet"

        assert leaf.partitions == (("year", "2024"), ("month", "01"))

    def test_children_where_selects_the_parts_to_rewrite(
        self, lake: pathlib.Path
    ) -> None:
        handle = IOBase(lake)

        year = handle.children_where({"year": "2024"})
        assert len(year) == 4
        assert all(entry.is_file() for entry in year)

        both = handle.children_where([("year", "2024"), ("month", "02")])
        assert len(both) == 2
        assert handle.children_where({"year": "1999"}) == []
        # No filter is every leaf.
        assert len(handle.children_where({})) == 8


class TestUrlPathlibParity:
    """A URL is a path with a scheme, so it answers the same questions."""

    def test_the_naming_properties_match_purepath(self) -> None:
        url = Url("file:///lake/trades/part-0.tar.gz")

        assert url.name == "part-0.tar.gz"
        assert url.stem == "part-0.tar"
        assert url.suffix == ".gz"
        assert url.suffixes == (".tar", ".gz")
        assert url.parts == ("lake", "trades", "part-0.tar.gz")
        assert url.is_absolute()

    def test_joining_matches_purepath(self) -> None:
        root = Url("file:///lake")

        assert str(root / "trades" / "part-0.arrows") == "file:///lake/trades/part-0.arrows"
        assert str(root.joinpath("trades", "part-0.arrows")) == "file:///lake/trades/part-0.arrows"
        assert str(root.joinpath(pathlib.PurePosixPath("trades"))) == "file:///lake/trades"

    def test_parents_run_from_the_closest_upwards(self) -> None:
        url = Url("file:///lake/trades/part-0.arrows")

        assert url.parent.name == "trades"
        assert [parent.name for parent in url.parents] == ["trades", "lake", ""]
        # A root is its own parent, which is what pathlib does.
        assert str(Url("file:///").parent) == "file:///"

    def test_renaming_matches_purepath(self) -> None:
        url = Url("file:///lake/part-0.arrows")

        assert Url(url).with_name("part-1.arrows").name == "part-1.arrows"
        assert Url(url).with_stem("part-1").name == "part-1.arrows"
        assert Url(url).with_suffix(".parquet").name == "part-0.parquet"
        assert Url(url).with_suffix("parquet").name == "part-0.parquet"
        assert Url(url).with_suffix("").name == "part-0"

    def test_matching_follows_the_gitignore_rule(self) -> None:
        url = Url("file:///lake/year=2024/part-0.parquet")

        assert url.match("*.parquet")
        assert url.match("lake/**/part-?.parquet")
        assert not url.match("lake/*.parquet")
        assert Url("file:///lake/**/*.parquet").is_glob()
        assert not url.is_glob()

    def test_relative_to_matches_purepath(self) -> None:
        root = Url("file:///lake")
        url = Url("file:///lake/year=2024/part-0.parquet")

        assert url.relative_to(root) == "year=2024/part-0.parquet"
        assert url.is_relative_to(root)
        assert not url.is_relative_to(Url("file:///other"))
        with pytest.raises(ValueError):
            url.relative_to(Url("file:///other"))

    def test_the_file_system_predicates_answer_for_a_local_url(
        self, lake: pathlib.Path
    ) -> None:
        url = Url(lake)

        assert url.exists()
        assert url.is_dir()
        assert not url.is_file()
        assert (url / "year=2024" / "month=01" / "part-0.parquet").is_file()
        assert (url / ".staging").is_private()

    def test_partitions_are_read_off_the_path(self) -> None:
        url = Url("file:///lake/year=2024/month=01/part-0.parquet")

        assert url.partitions == (("year", "2024"), ("month", "01"))
        assert url.partition("month") == "01"
        assert url.partition("day") is None
