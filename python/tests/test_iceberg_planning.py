"""Filtered reads, scoped writes, and the maintenance a table needs to stay one.

Everything here is about *not* touching what a call has no business touching: a
filtered scan reads one partition, a plan reads no rows at all, an overwrite of
one partition carries every other file forward untouched, and an expiry drops
only the snapshots retention no longer names.
"""

from __future__ import annotations

import pathlib
import time

import pyarrow as pa
import pytest

from yggdryl import IOBase
from yggdryl.iceberg import ScanPlan, Table, assign_field_ids

SCHEMA = pa.schema(
    [
        pa.field("id", pa.int64(), nullable=False),
        pa.field("venue", pa.string()),
    ]
)


def _rows(start: int = 1) -> pa.RecordBatch:
    """Three rows across two venues and the absence of one."""
    return pa.record_batch(
        {"id": [start, start + 1, start + 2], "venue": ["XNAS", "XNYS", None]},
        schema=SCHEMA,
    )


def _row(key: int, venue: str | None) -> pa.RecordBatch:
    """One row, for the writes that are about a single key."""
    return pa.record_batch({"id": [key], "venue": [venue]}, schema=SCHEMA)


@pytest.fixture
def numbered() -> object:
    """The shared schema, with the field identifiers Iceberg resolves by."""
    return assign_field_ids(SCHEMA)


@pytest.fixture
def table(tmp_path: pathlib.Path, numbered: object) -> Table:
    """A partitioned table with nothing written to it yet."""
    return Table.create(IOBase(tmp_path / "trades"), numbered, ["venue"])


@pytest.fixture
def filled(table: Table) -> Table:
    """Two commits over the same three partitions: six files, two manifests.

    Two commits rather than one is what makes the manifest counts mean
    something - a table whose whole history is one manifest cannot show that a
    manifest was skipped.
    """
    table.append(_rows())
    table.append(_rows(4))
    return table


class TestFilteredScans:
    """A filter is answered by the plan for a partition column, by rows for the rest."""

    def test_a_filtered_scan_reads_only_the_matching_partition(
        self, filled: Table
    ) -> None:
        rows = filled.scan_where({"venue": "XNAS"}).read_all()

        assert rows.column("venue").to_pylist() == ["XNAS", "XNAS"]
        assert sorted(rows.column("id").to_pylist()) == [1, 4]
        # The plan and the read agree, which is what says the rows were skipped
        # rather than read and discarded.
        assert filled.plan({"venue": "XNAS"}).files_planned == 2

    def test_a_filter_naming_a_column_the_schema_does_not_declare_is_refused(
        self, filled: Table
    ) -> None:
        # A misspelled column must not read as "matches nothing": an empty
        # answer to a typo is the failure this refusal exists to catch.
        with pytest.raises(ValueError, match='got "market"'):
            filled.scan_where({"market": "XNAS"})

    def test_the_absence_of_a_partition_value_is_spelled_null(
        self, filled: Table
    ) -> None:
        rows = filled.scan_where({"venue": "null"}).read_all()

        assert rows.column("venue").to_pylist() == [None, None]
        assert sorted(rows.column("id").to_pylist()) == [3, 6]

    def test_a_filter_on_a_column_no_partition_carries_still_selects_rows(
        self, filled: Table
    ) -> None:
        # `id` is not a partition column, so statistics can only bound a file;
        # the rows that come back must still be exactly the matching rows.
        rows = filled.scan_where({"id": "5"}).read_all()

        assert rows.to_pydict() == {"id": [5], "venue": ["XNYS"]}

    def test_a_projection_rides_alongside_the_filter(self, filled: Table) -> None:
        wanted = pa.schema([pa.field("id", pa.int64(), nullable=False)])

        rows = filled.scan_where([("venue", "XNYS")], wanted).read_all()

        assert rows.column_names == ["id"]
        assert sorted(rows.column("id").to_pylist()) == [2, 5]

    def test_filtering_by_nothing_reads_the_whole_table(self, filled: Table) -> None:
        # The filters are optional, so `scan_where()` has to be `scan()` rather
        # than an accidental empty read.
        assert filled.scan_where().read_all().num_rows == 6
        assert filled.scan_where({}).read_all().num_rows == 6


class TestRefScans:
    """A branch or tag is read as the snapshot it names, not as the present."""

    def test_a_branch_reads_the_snapshot_it_names_rather_than_the_current_one(
        self, table: Table
    ) -> None:
        table.append(_rows())
        assert table.current_snapshot is not None
        first = table.current_snapshot.snapshot_id
        table.create_branch("nightly", first)
        table.append(_rows(4))

        assert table.scan().read_all().num_rows == 6
        assert table.scan_ref("nightly").read_all().num_rows == 3
        # The filters mean on a ref what they mean on the present.
        pinned = table.scan_ref("nightly", {"venue": "XNAS"}).read_all()
        assert pinned.to_pydict() == {"id": [1], "venue": ["XNAS"]}

    def test_a_ref_the_table_does_not_carry_names_the_refs_it_does(
        self, filled: Table
    ) -> None:
        with pytest.raises(ValueError, match=r'got "nightly"; it has \[main\]'):
            filled.scan_ref("nightly")


class TestPlanning:
    """A plan is what the metadata decided, before a single row was read."""

    def test_a_plan_accounts_for_every_live_file_it_did_not_read(
        self, filled: Table
    ) -> None:
        total = len(filled.data_files())
        assert total == 6

        plan = filled.plan({"venue": "XNAS"})

        assert isinstance(plan, ScanPlan)
        # Nothing may be quietly dropped: every live file is either planned or
        # explicitly skipped, which is the arithmetic that makes "it skipped
        # four files" a claim rather than a hope.
        assert plan.files_planned + plan.files_skipped == total
        assert (plan.files_planned, plan.files_skipped) == (2, 4)
        assert plan.manifests_read == 2

    def test_a_plan_reports_the_rows_the_equivalent_scan_yields(
        self, filled: Table
    ) -> None:
        # An identity partition filter is settled by the plan alone - every row
        # of a matching file holds the value - so the counted rows and the read
        # rows have to be the same number, without a row being read to find out.
        for filters in ({}, {"venue": "XNAS"}, {"venue": "XNYS"}, {"venue": "null"}):
            assert (
                filled.plan(filters).record_count
                == filled.scan_where(filters).read_all().num_rows
            )

    def test_a_plan_counts_the_rows_of_the_files_and_not_of_the_answer(
        self, tmp_path: pathlib.Path, numbered: object
    ) -> None:
        flat = Table.create(IOBase(tmp_path / "flat"), numbered)
        flat.append(_rows())

        plan = flat.plan({"id": "1"})

        # `id` carries no partition, so statistics bound a file rather than
        # select a row: the plan counts the whole file it could not exclude,
        # and the scan then filters the rows inside it. Reading `record_count`
        # as "the answer's size" would be wrong here, and that is the point.
        assert plan.files_planned == 1
        assert plan.record_count == 3
        assert flat.scan_where({"id": "1"}).read_all().num_rows == 1

    def test_a_filter_matching_no_partition_opens_no_manifest_at_all(
        self, filled: Table
    ) -> None:
        plan = filled.plan({"venue": "XLON"})

        # The manifest-list summaries settle an identity partition filter, so
        # the cheapest level answered it: nothing was opened to find out.
        assert plan.files_planned == 0
        assert plan.record_count == 0
        assert plan.manifests_read == 0
        assert plan.manifests_skipped == 2

    def test_a_filter_no_row_can_satisfy_still_reports_the_manifests_it_read(
        self, filled: Table
    ) -> None:
        plan = filled.plan({"id": "999"})

        # `id` is not partitioned, so no summary could exclude a manifest: both
        # had to be opened, and the column statistics inside them are what
        # excluded every file.
        assert plan.files_planned == 0
        assert plan.files_skipped == 6
        assert plan.manifests_read == 2
        assert plan.manifests_skipped == 0
        assert filled.scan_where({"id": "999"}).read_all().num_rows == 0

    def test_planning_an_earlier_snapshot_reports_that_snapshot(
        self, filled: Table
    ) -> None:
        first = filled.snapshots[0].snapshot_id

        earlier = filled.plan_at(first)
        now = filled.plan()

        assert (earlier.record_count, earlier.files_planned) == (3, 3)
        assert (now.record_count, now.files_planned) == (6, 6)
        # History is planned by the same rules the present is planned by.
        assert filled.plan_at(first, {"venue": "XNAS"}).files_planned == 1

    def test_planning_a_snapshot_the_table_does_not_retain_is_refused(
        self, filled: Table
    ) -> None:
        with pytest.raises(ValueError, match="expected a retained snapshot id"):
            filled.plan_at(999)

    def test_an_empty_table_plans_nothing_rather_than_failing(
        self, table: Table
    ) -> None:
        # A table with no snapshot has no manifests, which is an answer and not
        # an error - the same way an empty scan reads as no rows.
        plan = table.plan()

        assert (plan.record_count, plan.files_planned, plan.files_skipped) == (0, 0, 0)
        assert (plan.manifests_read, plan.manifests_skipped) == (0, 0)

    def test_a_plan_refuses_an_undeclared_filter_column_as_a_scan_does(
        self, filled: Table
    ) -> None:
        with pytest.raises(ValueError, match='got "market"'):
            filled.plan({"market": "XNAS"})
        with pytest.raises(ValueError, match='got "market"'):
            filled.plan_at(filled.snapshots[0].snapshot_id, {"market": "XNAS"})


class TestOverwritingAPartition:
    """Replacing one partition is the whole point: every other file is carried."""

    def test_replacing_one_partition_leaves_every_other_partition_untouched(
        self, filled: Table
    ) -> None:
        untouched = filled.scan_where({"venue": "XNYS"}).read_all().to_pydict()
        absent = filled.scan_where({"venue": "null"}).read_all().to_pydict()

        filled.overwrite_where({"venue": "XNAS"}, _row(100, "XNAS"))

        assert filled.scan_where({"venue": "XNAS"}).read_all().to_pydict() == {
            "id": [100],
            "venue": ["XNAS"],
        }
        # The rows nobody named must come back byte for byte the same, in the
        # same order: a carried file is carried, not rewritten.
        assert filled.scan_where({"venue": "XNYS"}).read_all().to_pydict() == untouched
        assert filled.scan_where({"venue": "null"}).read_all().to_pydict() == absent
        assert sorted(filled.scan().read_all().column("id").to_pylist()) == [
            2,
            3,
            5,
            6,
            100,
        ]

    def test_the_replaced_snapshot_is_retained_and_still_reads_the_old_rows(
        self, filled: Table
    ) -> None:
        assert filled.current_snapshot is not None
        before = filled.current_snapshot.snapshot_id

        filled.overwrite_where({"venue": "XNAS"}, _row(100, "XNAS"))

        assert filled.current_snapshot is not None
        assert filled.current_snapshot.operation == "overwrite"
        # Retention is what makes the replacement reversible.
        assert len(filled.snapshots) == 3
        old = filled.scan_at(before).read_all()
        assert sorted(old.column("id").to_pylist()) == [1, 2, 3, 4, 5, 6]

    def test_an_undeclared_filter_column_is_refused_before_anything_is_written(
        self, filled: Table
    ) -> None:
        before = filled.version

        with pytest.raises(ValueError, match='got "market"'):
            filled.overwrite_where({"market": "XNAS"}, _row(100, "XNAS"))

        # A refused write must be a write that never happened.
        assert filled.version == before
        assert filled.scan().read_all().num_rows == 6

    def test_overwriting_by_no_filter_replaces_every_row(self, filled: Table) -> None:
        filled.overwrite_where({}, _row(100, "XNAS"))

        assert filled.scan().read_all().to_pydict() == {
            "id": [100],
            "venue": ["XNAS"],
        }


class TestMerging:
    """A merge is the upsert: a stored key is updated, an unknown one appended."""

    def test_a_merge_updates_a_stored_key_and_appends_an_unknown_one(
        self, table: Table
    ) -> None:
        table.append(_rows())

        table.merge(
            pa.record_batch(
                {"id": [2, 9], "venue": ["XPAR", "XLON"]}, schema=SCHEMA
            ),
            ["id"],
        )

        rows = table.scan().read_all().sort_by("id").to_pydict()
        # 2 was stored, so it moved partitions rather than doubling; 9 was not,
        # so it arrived. Everything else is exactly as it was.
        assert rows == {
            "id": [1, 2, 3, 9],
            "venue": ["XNAS", "XPAR", None, "XLON"],
        }

    def test_a_merge_scoped_to_one_partition_leaves_the_others_as_they_were(
        self, filled: Table
    ) -> None:
        others = sorted(
            filled.scan_where({"venue": "XNAS"}).read_all().column("id").to_pylist()
        )

        filled.merge_where(
            {"venue": "XNYS"},
            pa.record_batch({"id": [2, 8], "venue": ["XNYS", "XNYS"]}, schema=SCHEMA),
            ["id"],
        )

        # 2 was already in the scoped partition and was updated in place; 8 was
        # not stored anywhere and was appended.
        assert sorted(
            filled.scan_where({"venue": "XNYS"}).read_all().column("id").to_pylist()
        ) == [2, 5, 8]
        # The partitions the filter excluded were never even read.
        assert (
            sorted(
                filled.scan_where({"venue": "XNAS"}).read_all().column("id").to_pylist()
            )
            == others
        )
        assert filled.scan().read_all().num_rows == 7

    def test_merging_on_no_column_at_all_is_an_overwrite(self, table: Table) -> None:
        table.append(_rows())

        # Every row would match every row, so the only honest reading of "no
        # match key" is a replacement.
        table.merge(_rows(10), [])

        assert table.scan().read_all().column("id").to_pylist() == [10, 11, 12]
        assert table.current_snapshot is not None
        assert table.current_snapshot.operation == "overwrite"

    def test_a_match_key_the_schema_does_not_declare_is_refused(
        self, filled: Table
    ) -> None:
        before = filled.version

        with pytest.raises(ValueError, match='got "market"'):
            filled.merge(_row(100, "XNAS"), ["market"])

        assert filled.version == before
        assert filled.scan().read_all().num_rows == 6

    def test_one_string_is_not_an_iterable_of_match_keys(self, filled: Table) -> None:
        # "id" is four characters, and reading it as four column names would
        # silently merge on nothing the caller meant.
        with pytest.raises(TypeError, match="not one string"):
            filled.merge(_row(100, "XNAS"), "id")

    def test_a_value_the_column_cannot_read_is_refused_under_a_strict_cast(
        self, filled: Table
    ) -> None:
        before = filled.version
        text = pa.schema(
            [
                pa.field("id", pa.string(), nullable=False),
                pa.field("venue", pa.string()),
            ]
        )
        unreadable = pa.record_batch({"id": ["nine"], "venue": ["XNAS"]}, schema=text)

        with pytest.raises(ValueError, match="Cast error"):
            filled.merge(unreadable, ["id"], safe=False)

        assert filled.version == before

    def test_a_merge_filter_the_schema_does_not_declare_is_refused(
        self, filled: Table
    ) -> None:
        with pytest.raises(ValueError, match='got "market"'):
            filled.merge_where({"market": "XNAS"}, _row(100, "XNAS"), ["id"])


class TestExpiringSnapshots:
    """Expiry drops what retention no longer names, and nothing else."""

    def test_defaults_retain_override_and_explicit_ids(self, filled: Table) -> None:
        first = filled.snapshots[0].snapshot_id
        assert filled.current_snapshot is not None
        current = filled.current_snapshot.snapshot_id
        before = filled.version

        # Fresh snapshots survive the default five-day cutoff. A retain
        # override also protects both snapshots from a future cutoff.
        assert filled.expire_snapshots() == []
        assert filled.expire_snapshots(int(time.time() * 1000) + 60_000, 2) == []
        assert filled.expire_snapshots(0, snapshot_ids=[999]) == []
        assert filled.version == before

        with pytest.raises(ValueError, match="retain_last.*at least 1"):
            filled.expire_snapshots(retain_last=0)
        with pytest.raises(ValueError, match="cannot expire current snapshot"):
            filled.expire_snapshots(snapshot_ids=[current])

        # Explicit ids join age selection, so an old cutoff does not stop this
        # known, unprotected ancestor from being removed.
        assert filled.expire_snapshots(0, snapshot_ids=[first]) == [first]
        assert filled.scan().read_all().num_rows == 6

    def test_a_cutoff_older_than_everything_expires_nothing_and_spends_no_version(
        self, filled: Table
    ) -> None:
        before = filled.version
        retained = [snapshot.snapshot_id for snapshot in filled.snapshots]

        assert filled.expire_snapshots(0) == []

        # An expiry with nothing to do must not write a metadata document: the
        # check runs on a copy first, so a no-op costs no version.
        assert filled.version == before
        assert [snapshot.snapshot_id for snapshot in filled.snapshots] == retained

    def test_a_cutoff_past_an_early_snapshot_expires_it_and_the_current_survives(
        self, table: Table
    ) -> None:
        for start in (1, 4, 7):
            table.append(_rows(start))
        early = [snapshot.snapshot_id for snapshot in table.snapshots[:2]]
        assert table.current_snapshot is not None
        current = table.current_snapshot.snapshot_id
        before = table.version

        expired = table.expire_snapshots(int(time.time() * 1000) + 60_000)

        assert sorted(expired) == sorted(early)
        assert table.version == before + 1
        # The current snapshot is always retained, whatever the cutoff says,
        # and it is still a complete table.
        assert [snapshot.snapshot_id for snapshot in table.snapshots] == [current]
        assert table.current_snapshot is not None
        assert table.current_snapshot.snapshot_id == current
        assert table.scan().read_all().num_rows == 9

    def test_an_expired_snapshot_is_no_longer_one_the_table_will_read(
        self, table: Table
    ) -> None:
        table.append(_rows())
        table.append(_rows(4))
        first = table.snapshots[0].snapshot_id

        assert table.expire_snapshots(int(time.time() * 1000) + 60_000) == [first]

        # Time travel to a dropped snapshot must be refused rather than read
        # from files that happen to still be on disk.
        with pytest.raises(ValueError, match="expected a retained snapshot id"):
            table.scan_at(first)
        with pytest.raises(ValueError, match="expected a retained snapshot id"):
            table.plan_at(first)

    def test_a_tagged_snapshot_survives_a_cutoff_that_would_reach_it(
        self, table: Table
    ) -> None:
        table.append(_rows())
        first = table.snapshots[0].snapshot_id
        table.create_tag("release", first)
        table.append(_rows(4))

        assert table.expire_snapshots(int(time.time() * 1000) + 60_000) == []

        # A ref anchors its target: retention is honored before the age cutoff.
        assert table.scan_at(first).read_all().num_rows == 3


class TestFastForward:
    """A branch moves only forward, which is why it cannot lose history."""

    def test_a_branch_moves_to_a_descendant_snapshot(self, table: Table) -> None:
        table.append(_rows())
        assert table.current_snapshot is not None
        first = table.current_snapshot.snapshot_id
        table.create_branch("nightly", first)
        table.append(_rows(4))
        assert table.current_snapshot is not None
        second = table.current_snapshot.snapshot_id

        table.fast_forward("nightly", second)

        assert table.snapshot_by_ref("nightly").snapshot_id == second
        assert table.scan_ref("nightly").read_all().num_rows == 6

    def test_a_target_that_is_not_a_descendant_is_refused_naming_both_ends(
        self, table: Table
    ) -> None:
        table.append(_rows())
        assert table.current_snapshot is not None
        first = table.current_snapshot.snapshot_id
        table.append(_rows(4))
        assert table.current_snapshot is not None
        second = table.current_snapshot.snapshot_id
        table.create_branch("nightly", second)

        # Moving the branch back to its head's parent would silently drop the
        # commits between them, which is the one thing a fast-forward promises
        # it cannot do.
        with pytest.raises(ValueError, match=f"expected {first} to descend from"):
            table.fast_forward("nightly", first)

        assert table.snapshot_by_ref("nightly").snapshot_id == second

    def test_moving_a_branch_the_table_does_not_have_is_refused(
        self, filled: Table
    ) -> None:
        assert filled.current_snapshot is not None

        with pytest.raises(ValueError, match='expected a branch named "nightly"'):
            filled.fast_forward("nightly", filled.current_snapshot.snapshot_id)

    def test_a_target_the_table_does_not_retain_is_refused(self, table: Table) -> None:
        table.append(_rows())

        with pytest.raises(ValueError, match="unknown snapshot id 999"):
            table.fast_forward("main", 999)


class TestManifestsOfASnapshot:
    """A snapshot is named by identifier, because the table owns what it retains."""

    def test_the_manifests_of_a_retained_snapshot_are_that_snapshots(
        self, filled: Table
    ) -> None:
        first = filled.snapshots[0].snapshot_id

        earlier = filled.manifests_at(first)
        current = filled.manifests()

        # The second commit carries the first commit's manifest forward, so the
        # earlier snapshot is a strict subset - and every manifest it names was
        # added by it.
        assert len(earlier) == 1
        assert len(current) == 2
        assert [manifest.added_snapshot_id for manifest in earlier] == [first]
        assert {manifest.path for manifest in earlier} <= {
            manifest.path for manifest in current
        }
        assert earlier[0].added_files_count == 3
        assert earlier[0].added_rows_count == 3

    def test_an_id_the_table_does_not_retain_is_refused_naming_the_ids_it_does(
        self, filled: Table
    ) -> None:
        retained = ", ".join(
            str(snapshot.snapshot_id) for snapshot in filled.snapshots
        )

        with pytest.raises(
            ValueError,
            match=f"expected a retained snapshot id, got 999; the table retains "
            rf"\[{retained}\]",
        ):
            filled.manifests_at(999)
