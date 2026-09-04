"""Boundary cost of the FIX dictionary, against the native numbers.

Every case here is one crossing over a registry the core resolves: what is
measured is the coercion of the key - a tag, a branch, an identifier - the
wrapper the answer is put in, and - for the two loads - the shard read the
boundary only names. Run after installing the release wheel with::

    python benchmarks/fix.py --iterations 2000

The generated registry is written to a temporary folder and removed on the way
out, so the only tracked input is the seed dictionary at ``config/fix``.
"""

from __future__ import annotations

import argparse
import gc
import pathlib
import shutil
import statistics
import tempfile
import timeit
from collections.abc import Callable

from yggdryl import DataType, Field
from yggdryl.fix import STANDARD_BRANCH, FixMsg, FixRegistry

REPO = pathlib.Path(__file__).resolve().parent.parent.parent
SEED = REPO / "config" / "fix"

SEED_REGISTRY = FixRegistry.from_handle(SEED)
ORDER = Field(
    "NewOrderSingle",
    DataType.from_fields(
        [
            SEED_REGISTRY.field_by_tag(55),
            SEED_REGISTRY.field_by_tag(38),
            SEED_REGISTRY.field_by_name(STANDARD_BRANCH, "NoPartyIDs"),
        ]
    ),
    nullable=False,
)
MESSAGE = FixMsg(
    ORDER,
    {
        "Symbol": "AAPL",
        "OrderQty": 100,
        "NoPartyIDs": [{"PartyID": "BROKER", "PartyIDSource": "D", "PartyRole": 1}],
    },
    SEED_REGISTRY,
)
WIDE_FIELDS = 1_000
VENDOR_BRANCH = "cme"
VENDOR_FIELDS = 1_000


def _vendor_registry() -> FixRegistry:
    """The seed beside a vendor dictionary, for the cross-branch rows."""
    fields = [field for field in SEED_REGISTRY]
    for offset in range(VENDOR_FIELDS):
        tag = 5000 + offset
        field = Field(f"Venue{offset}", "utf8")
        field.fix.id = f"{VENDOR_BRANCH}:{tag}"
        field.fix.aliases = [f"VenueAlias{offset}"]
        fields.append(field)
    return FixRegistry.from_fields(fields)


TWO_BRANCHES = _vendor_registry()
TAGGED = Field("TradeID", "utf8")
TAGGED.fix.id = f"{VENDOR_BRANCH}:5001"


def _generated(root: pathlib.Path) -> pathlib.Path:
    """Write a registry of ``WIDE_FIELDS`` fields and answer its folder."""
    fields = []
    for tag in range(1, WIDE_FIELDS + 1):
        field = Field(f"Field{tag}", "utf8")
        field.fix.tag = tag
        field.fix.aliases = [f"Alias{tag}"]
        fields.append(field)
    FixRegistry.from_fields(fields).write_into(root)
    return root


def _tag_hit() -> object:
    return SEED_REGISTRY.get_field_by_tag(55)


def _alternate_tag_hit() -> object:
    return SEED_REGISTRY.get_field_by_tag(20)


def _id_hit() -> object:
    return SEED_REGISTRY.get_field_by_id("standard:55")


def _name_hit() -> object:
    return SEED_REGISTRY.get_field_by_name(STANDARD_BRANCH, "Symbol")


def _folded_name_hit() -> object:
    return SEED_REGISTRY.get_field_by_name(STANDARD_BRANCH, "symbol")


def _alias_hit() -> object:
    return SEED_REGISTRY.get_field_by_name(STANDARD_BRANCH, "ticker")


def _tag_miss() -> object:
    return SEED_REGISTRY.get_field_by_tag(9999)


def _name_miss() -> object:
    return SEED_REGISTRY.get_field_by_name(STANDARD_BRANCH, "Nope")


def _id_miss() -> object:
    return SEED_REGISTRY.get_field_by_id("cme:5001")


def _generic_tag_hit() -> object:
    return SEED_REGISTRY.get_field(55)


def _path_one_segment() -> object:
    return SEED_REGISTRY.field_by_path(STANDARD_BRANCH, "NoPartyIDs")


def _path_two_segments() -> object:
    return SEED_REGISTRY.field_by_path(STANDARD_BRANCH, "NoPartyIDs.PartyID")


def _vendor_id_hit() -> object:
    return TWO_BRANCHES.get_field_by_id("cme:5001")


def _vendor_name_hit() -> object:
    return TWO_BRANCHES.get_field_by_name(VENDOR_BRANCH, "Venue1")


def _vendor_alias_hit() -> object:
    return TWO_BRANCHES.get_field_by_name(VENDOR_BRANCH, "venuealias1")


def _cross_branch_miss() -> object:
    return TWO_BRANCHES.get_field_by_tag(5001)


def _standard_hit_over_two_branches() -> object:
    return TWO_BRANCHES.get_field_by_tag(55)


def _field_branch() -> object:
    return TAGGED.fix.branch


def _field_id() -> object:
    return TAGGED.fix.id


def _message_get_by_tag() -> object:
    return MESSAGE.get_by_tag(55)


def _message_get_by_id() -> object:
    return MESSAGE.get_by_id("standard:55")


def _message_get_by_name() -> object:
    return MESSAGE.get_by_name("ticker")


def _message_get_by_path() -> object:
    return MESSAGE.get_by_path("NoPartyIDs.0.PartyID")


def _message_branch() -> object:
    return MESSAGE.branch


def _measure(name: str, operation: Callable[[], object], iterations: int) -> None:
    samples = timeit.repeat(operation, number=iterations, repeat=7)
    median = statistics.median(samples)
    nanoseconds = median * 1_000_000_000 / iterations
    print(f"{name:36} {nanoseconds:14.1f} ns/op")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--iterations", type=int, default=10_000)
    args = parser.parse_args()
    if args.iterations < 1:
        parser.error("--iterations must be positive")

    workspace = pathlib.Path(tempfile.mkdtemp(prefix="yggdryl-bench-fix-"))
    gc.disable()
    try:
        generated = _generated(workspace / "generated")
        _measure("tag hit", _tag_hit, args.iterations)
        _measure("alternate tag hit", _alternate_tag_hit, args.iterations)
        _measure("id hit", _id_hit, args.iterations)
        _measure("name hit", _name_hit, args.iterations)
        _measure("name hit, folded query", _folded_name_hit, args.iterations)
        _measure("alias hit", _alias_hit, args.iterations)
        _measure("tag miss", _tag_miss, args.iterations)
        _measure("name miss", _name_miss, args.iterations)
        _measure("id miss", _id_miss, args.iterations)
        _measure("generic tag hit", _generic_tag_hit, args.iterations)
        _measure("field_by_path, one segment", _path_one_segment, args.iterations)
        _measure("field_by_path, two segments", _path_two_segments, args.iterations)
        _measure("vendor id hit, two branches", _vendor_id_hit, args.iterations)
        _measure("vendor name hit, two branches", _vendor_name_hit, args.iterations)
        _measure("vendor alias hit, two branches", _vendor_alias_hit, args.iterations)
        _measure("cross-branch tag miss", _cross_branch_miss, args.iterations)
        _measure(
            "standard tag hit, two branches",
            _standard_hit_over_two_branches,
            args.iterations,
        )
        _measure("field.fix.branch", _field_branch, args.iterations)
        _measure("field.fix.id", _field_id, args.iterations)
        _measure("message get_by_tag", _message_get_by_tag, args.iterations)
        _measure("message get_by_id", _message_get_by_id, args.iterations)
        _measure("message get_by_name", _message_get_by_name, args.iterations)
        _measure("message get_by_path", _message_get_by_path, args.iterations)
        _measure("message branch", _message_branch, args.iterations)
        loads = max(1, args.iterations // 100)
        _measure(
            "from_handle, the seed",
            lambda: FixRegistry.from_handle(SEED),
            loads,
        )
        _measure(
            f"from_handle, {WIDE_FIELDS} fields",
            lambda: FixRegistry.from_handle(generated),
            loads,
        )
    finally:
        gc.enable()
        shutil.rmtree(workspace, ignore_errors=True)


if __name__ == "__main__":
    main()
