"""The CI planner reads the table, the workflow and the change the way they mean.

`.github/workflows/ci.yml` runs these in its `changes` job before it plans, so
a planner that would skip a proof fails the run instead.
"""

from __future__ import annotations

import importlib.util
import json
import pathlib
import re
import subprocess
import sys
import tempfile
import tomllib
import unittest

SCRIPT = pathlib.Path(__file__).resolve().parents[1] / "ci" / "plan.py"
SPEC = importlib.util.spec_from_file_location("ci_plan", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
plan = importlib.util.module_from_spec(SPEC)
# A dataclass reads its own module back while it is being defined.
sys.modules[SPEC.name] = plan
SPEC.loader.exec_module(plan)

ROOT = plan.ROOT
CONFIG = plan.load_config()
WORKFLOW = plan.load_workflow()
EVERY = set(WORKFLOW) - {plan.PLANNER, plan.GATE}
# The jobs a push to `main` runs: every one, the leaf matrix while a leaf
# crate is listed.
ALL = EVERY if CONFIG.leaves else EVERY - {plan.LEAVES_JOB}
DAY = 20_000
# Every line under `[leaves]`, listed or still commented out: the tests
# switch on the leaves they need, so they hold whichever the table lists.
LEAF_LINE = re.compile(r"^(?:# )?([a-z]+) = \{ package = .*\}$", re.MULTILINE)


def planned(paths, *, config=CONFIG, labels=(), ledger=(), pull_request=True):
    return plan.make_plan(
        config,
        WORKFLOW,
        pull_request=pull_request,
        changed=list(paths),
        labels=list(labels),
        salt="",
        ledger=list(ledger),
        day=DAY,
        fingerprints=False,
    )


def with_leaves(*names: str) -> plan.Config:
    """The repository's table with these leaves switched on, as it lists them.

    A leaf already listed stays as it is, so the same test holds before and
    after the change that creates its crate.
    """
    text = plan.ROWS.read_text(encoding="utf-8")
    lines = {match.group(1): match.group(0) for match in LEAF_LINE.finditer(text)}
    for name in names:
        assert name in lines, f"rows.toml has no `[leaves]` line for `{name}`"
        text = text.replace(lines[name], lines[name].removeprefix("# "))
    with tempfile.TemporaryDirectory() as folder:
        path = pathlib.Path(folder) / "rows.toml"
        path.write_text(text, encoding="utf-8")
        return plan.load_config(path)


def leaf_jobs(config: plan.Config, name: str) -> set[str]:
    """What a change to `name` alone runs, read off the table."""
    leaf = config.leaves[name]
    return plan.closure({plan.LEAVES_JOB, *config.leaf_jobs, *leaf.jobs}, WORKFLOW)


class TheTableAgrees(unittest.TestCase):
    def test_the_table_the_workflow_and_the_tree_agree(self) -> None:
        self.assertEqual(plan.check(CONFIG, WORKFLOW), [])

    def test_every_pattern_matches_what_git_matches(self) -> None:
        tracked = subprocess.run(
            ["git", "ls-files", "-z"], cwd=ROOT, check=True, capture_output=True
        ).stdout.decode().split("\0")
        patterns = {p for row in CONFIG.rows.values() for p in row.paths} | set(CONFIG.inert)
        for pattern in sorted(patterns):
            with self.subTest(pattern=pattern):
                by_git = set(
                    subprocess.run(
                        ["git", "ls-files", "-z", "--", f":(glob){pattern}"],
                        cwd=ROOT, check=True, capture_output=True,
                    ).stdout.decode().split("\0")
                ) - {""}
                ours = {path for path in tracked if path and plan.glob_regex(pattern).match(path)}
                self.assertEqual(ours, by_git)

    def test_the_files_left_unknown_run_everything(self) -> None:
        # Left out of every row on purpose: `.gitattributes` decides what a
        # checkout writes, and nothing anyone could name reads the `order` pair.
        # Whether they are tracked is not the planner's business: adding,
        # changing or deleting one runs everything and never fails the plan.
        unknown = {".gitattributes", "order.toml", "order.yaml"}
        self.assertEqual(set(plan.unclassified(sorted(unknown), CONFIG)), unknown)
        for path in sorted(unknown):
            with self.subTest(path=path):
                result = planned([path])
                self.assertEqual(result.jobs, ALL)
                self.assertEqual(result.full, "1 path(s) no row knows")

    def test_every_row_reads_the_lock_and_the_workflow(self) -> None:
        # A row's fingerprint is what the ledger trusts; a row that read no
        # `Cargo.lock` would be proven across a dependency bump.
        for name, row in CONFIG.rows.items():
            if not row.jobs:
                continue
            with self.subTest(row=name):
                patterns = CONFIG.resolved(name)
                self.assertIn("Cargo.lock", patterns)
                self.assertTrue(set(CONFIG.rows["workflow"].paths) <= set(patterns))

    def test_every_artifact_outlives_a_next_day_rerun(self) -> None:
        # "Re-run failed jobs" downloads what the passed jobs uploaded.
        for path in [plan.WORKFLOW, ROOT / ".github" / "actions" / "rust-lane" / "action.yml"]:
            text = path.read_text(encoding="utf-8")
            uploads = text.count("actions/upload-artifact@")
            with self.subTest(path=path.name):
                self.assertGreater(uploads, 0)
                self.assertEqual(text.count("retention-days: 3"), uploads)

    def test_every_job_runs_on_one_image(self) -> None:
        # A lane's units are read fresh only by a consumer on the producer's image.
        for job in WORKFLOW.values():
            with self.subTest(job=job.name):
                self.assertIn("    runs-on: ubuntu-24.04", job.body)

    def test_every_lane_is_restored_over_the_cache_it_was_built_on(self) -> None:
        # A consumer restoring another key would find the lane's units stale
        # against different dependencies and compile the core again, silently.
        def lanes(job):
            blocks, block = [], None
            for line in job.body.splitlines():
                if line.strip() == "- uses: ./.github/actions/rust-lane":
                    block = {}
                    blocks.append(block)
                elif block is not None and line.startswith("          ") and ":" in line:
                    key, _, value = line.strip().partition(":")
                    block[key] = value.strip()
                elif block is not None and not line.startswith("        with:"):
                    block = None
            return blocks

        built = {}
        for job in WORKFLOW.values():
            blocks = lanes(job)
            packs = [block["lane"] for block in blocks if block.get("mode") == "pack"]
            setups = [block for block in blocks if block.get("mode") == "setup"]
            for lane in packs:
                built[lane] = (job.name, setups[0].get("cache"))
        self.assertEqual(sorted(built), ["default", "full", "interop"])
        restored = 0
        for job in WORKFLOW.values():
            for block in lanes(job):
                if block.get("mode") != "restore":
                    continue
                restored += 1
                producer, cache = built[block["lane"]]
                with self.subTest(job=job.name):
                    self.assertEqual(block.get("cache"), cache)
                    self.assertIn(producer, job.needs)
        self.assertEqual(restored, 9)

    def test_a_job_needs_only_what_it_reuses(self) -> None:
        # What builds in place restores no lane and starts at once.
        for job in ["cli", "node-addon", "docs-rust", "python-wheel", "python-freethreaded", "leaves"]:
            with self.subTest(job=job):
                self.assertEqual(WORKFLOW[job].needs, ("changes",))
                self.assertNotIn("mode: restore", WORKFLOW[job].body)

    def test_the_named_shards_are_targets_the_core_has(self) -> None:
        manifest = tomllib.loads((ROOT / "rust" / "Cargo.toml").read_text(encoding="utf-8"))
        tests = [{"kind": ["test"], "name": path.stem} for path in (ROOT / "rust" / "tests").glob("*.rs")]
        benches = [{"kind": ["bench"], "name": bench["name"]} for bench in manifest["bench"]]
        targets = [{"kind": ["lib"], "name": "yggdryl"}, *tests, *benches]
        for lane in plan.LANES:
            arguments = {
                shard: plan.shard_arguments(CONFIG, "yggdryl", lane, shard, targets)
                for shard in [*CONFIG.shards["yggdryl"][lane], "rest"]
            }
            selected = [
                (flag, name)
                for flags in arguments.values()
                for flag, name in zip(flags, flags[1:])
                if flag in ("--test", "--bench")
            ]
            with self.subTest(lane=lane):
                self.assertEqual(len(selected), len(set(selected)))
                self.assertEqual(len(selected), len(tests) + len(benches))
                self.assertEqual(arguments["rest"][0], "--lib")


class ThePlan(unittest.TestCase):
    def test_a_push_runs_every_job(self) -> None:
        result = planned([], pull_request=False)
        self.assertEqual(result.jobs, ALL)
        self.assertEqual(result.full, "not a pull request")

    def test_a_core_change_runs_every_job(self) -> None:
        self.assertEqual(planned(["rust/src/serie.rs"]).jobs, ALL)

    def test_a_lock_change_runs_every_job(self) -> None:
        self.assertEqual(planned(["Cargo.lock"]).jobs, ALL)

    def test_a_core_test_change_runs_the_core_alone(self) -> None:
        self.assertEqual(planned(["rust/tests/root/serie.rs"]).jobs, {
            "core-default", "core-full", "core-tests-default", "core-tests-full",
            "lint-default", "lint-full", "iceberg-msrv", "fmt",
        })

    def test_the_capture_the_bindings_replay_runs_them_too(self) -> None:
        jobs = planned(["rust/tests/fix/ulbridge.log"]).jobs
        self.assertTrue({"python", "node", "cli", "core-tests-full"} <= jobs)

    def test_a_python_change_runs_the_python_jobs(self) -> None:
        result = planned(["python/yggdryl/serie.py"])
        self.assertEqual(result.jobs, {
            "python-wheel", "python", "python-freethreaded",
            "spark-interop", "docs-python", "inventory",
        })
        self.assertEqual(result.python_legs, list(CONFIG.python_legs))

    def test_a_python_test_change_runs_the_suites_alone(self) -> None:
        self.assertEqual(planned(["python/tests/test_serie.py"]).jobs, {
            "python-wheel", "python", "python-freethreaded", "spark-interop",
        })

    def test_a_node_change_runs_the_node_jobs(self) -> None:
        self.assertEqual(planned(["node/src/serie.rs"]).jobs, {
            "node-addon", "node", "docs-javascript", "inventory", "lint-full", "fmt",
        })

    def test_a_cli_change_runs_what_carries_the_command(self) -> None:
        jobs = planned(["cli/src/main.rs"]).jobs
        self.assertTrue({"cli", "python-wheel", "python", "lint-full", "fmt"} <= jobs)
        self.assertFalse({"core-default", "core-full", "core-tests-default", "core-tests-full"} & jobs)

    def test_a_page_runs_the_examples(self) -> None:
        self.assertEqual(planned(["docs/types/serie.md"]).jobs, {
            "docs-rust", "docs-python", "python-wheel", "docs-javascript", "node-addon",
        })

    def test_a_generated_manifest_runs_the_node_job(self) -> None:
        jobs = planned(["docs/assets/fix.json"]).jobs
        self.assertIn("node", jobs)

    def test_an_exchange_driver_runs_its_exchange(self) -> None:
        self.assertEqual(planned(["scripts/check_zip_interop.py"]).jobs, {"zip-interop", "core-default"})
        self.assertEqual(planned(["scripts/check_object_interop.py"]).jobs, {"object-interop", "core-interop"})

    def test_an_exchange_half_runs_every_exchange(self) -> None:
        jobs = planned(["rust/tests/interop/zip.rs"]).jobs
        self.assertTrue({"zip-interop", "avro-interop", "excel-interop", "object-interop",
                         "azure-interop", "gcs-interop", "pyiceberg-interop"} <= jobs)

    def test_an_inert_change_runs_nothing(self) -> None:
        result = planned([".handoff/next/MARKET_SPLIT_NEXT.md", "AGENTS.md", "mkdocs.yml"])
        self.assertEqual(result.jobs, set())
        self.assertIsNone(result.full)

    def test_a_path_no_row_knows_runs_everything(self) -> None:
        for path in ["order.toml", "rust/build.rs", "rust/fixtures/a.bin", "scripts/new_tool.py"]:
            with self.subTest(path=path):
                result = planned([path])
                self.assertEqual(result.jobs, ALL)
                self.assertEqual(result.unclassified, [path])

    def test_a_workflow_change_runs_everything(self) -> None:
        self.assertEqual(planned([".github/ci/rows.toml"]).jobs, ALL)

    def test_the_full_label_runs_everything(self) -> None:
        self.assertEqual(planned(["AGENTS.md"], labels=["ci:full"]).jobs, ALL)

    def test_the_ledger_skips_a_row_proven_at_its_fingerprint(self) -> None:
        jobs = planned(["docs/types/serie.md"], ledger=[("docs-rust", "", DAY - 1)]).jobs
        self.assertNotIn("docs-rust", jobs)
        self.assertIn("docs-python", jobs)

    def test_a_stale_ledger_line_proves_nothing(self) -> None:
        old = DAY - CONFIG.ledger_days
        jobs = planned(["docs/types/serie.md"], ledger=[("docs-rust", "", old)]).jobs
        self.assertIn("docs-rust", jobs)

    def test_a_producer_runs_for_the_row_that_needs_it(self) -> None:
        proven = [(row, "", DAY) for row in ("python", "spark", "wheel")]
        jobs = planned(["python/yggdryl/serie.py"], ledger=proven).jobs
        self.assertEqual(jobs, {"docs-python", "python-wheel", "inventory"})

    def test_a_page_plans_no_core_lane(self) -> None:
        jobs = planned(["docs/types/serie.md"]).jobs
        self.assertFalse({"core-default", "core-full", "core-interop"} & jobs)
        self.assertEqual(plan.producers("cli", WORKFLOW), ())

    def test_the_outputs_frame_every_job(self) -> None:
        outputs = planned(["scripts/check_zip_interop.py"]).outputs(CONFIG)
        self.assertEqual(outputs["jobs"], " core-default zip-interop ")
        self.assertEqual(outputs["leaves"], "[]")
        self.assertEqual(json.loads(outputs["python_legs"]), list(CONFIG.python_leaf_legs))


class TheLeaves(unittest.TestCase):
    # The leaves the rows name exercises: none, an exchange, two exchanges and
    # the MSRV job. Each holds whether its line is listed yet or not.
    EXCHANGES = {
        "market": set(),
        "fix": set(),
        "xmla": set(),
        "avro": {"avro-interop"},
        "excel": {"excel-interop"},
        "parquet": {"pyiceberg-interop", "spark-interop"},
        "iceberg": {"pyiceberg-interop", "spark-interop", "iceberg-msrv"},
    }
    # What no leaf-only change runs.
    NEVER = {
        "core-tests-default", "core-tests-full", "lint-default", "lint-full",
        "python-freethreaded", "docs-python", "docs-javascript",
        "object-interop", "azure-interop", "gcs-interop", "zip-interop",
    }

    def test_a_leaf_change_runs_what_proves_the_leaf_and_nothing_more(self) -> None:
        config = with_leaves(*self.EXCHANGES)
        for name, exchanges in self.EXCHANGES.items():
            with self.subTest(leaf=name):
                result = planned([f"rust/{name}/src/lib.rs"], config=config)
                common = {"leaves", "fmt", "inventory", "docs-rust", "python-wheel", "python",
                          "node", "node-addon", "cli"}
                self.assertTrue(common | exchanges <= result.jobs)
                self.assertEqual(result.jobs, leaf_jobs(config, name) | {
                    job for after in self.downstream(config, name) for job in leaf_jobs(config, after)
                } | {"fmt"})
                self.assertFalse(self.NEVER & result.jobs)
                self.assertEqual(result.python_legs, list(config.python_leaf_legs))
                self.assertIsNone(result.full)

    @staticmethod
    def downstream(config: plan.Config, name: str) -> set[str]:
        found, grew = {name}, True
        while grew:
            grew = False
            for other, leaf in config.leaves.items():
                if other not in found and found & set(leaf.after):
                    found.add(other)
                    grew = True
        return found

    def test_a_leaf_runs_its_lanes_and_one_pytest_leg(self) -> None:
        config = with_leaves("market", "fix")
        result = planned(["rust/fix/src/codec.rs"], config=config)
        self.assertEqual({entry["name"] for entry in result.leaves}, {"fix"})
        named = config.shards.get("yggdryl-fix", {})
        self.assertEqual(
            sorted((entry["lane"], entry["shard"]) for entry in result.leaves),
            sorted((lane, shard) for lane in plan.LANES for shard in [*named.get(lane, {}), "rest"]),
        )
        self.assertEqual(result.python_legs, ["pyarrow>=18"])
        self.assertEqual(json.loads(result.outputs(config)["python_legs"]), ["pyarrow>=18"])

    def test_one_cache_per_leaf_saved_by_its_full_rest_shard(self) -> None:
        config = with_leaves("market")
        entries = [
            entry for entry in planned(["rust/market/src/side.rs"], config=config).leaves
            if entry["name"] == "market"
        ]
        self.assertEqual({entry["cache"] for entry in entries}, {"leaf-market"})
        self.assertEqual(
            [(entry["lane"], entry["shard"]) for entry in entries if entry["save"] == "true"],
            [("full", "rest")],
        )

    def test_a_leaf_and_a_binding_change_run_both_legs(self) -> None:
        config = with_leaves("market")
        result = planned(["rust/market/src/side.rs", "python/tests/test_serie.py"], config=config)
        self.assertEqual(result.python_legs, list(config.python_legs))
        self.assertIn("python-freethreaded", result.jobs)

    def test_an_upstream_leaf_reruns_the_leaves_after_it(self) -> None:
        config = with_leaves("market", "fix")
        result = planned(["rust/market/src/side.rs"], config=config)
        self.assertEqual({entry["name"] for entry in result.leaves}, {"market", "fix"})

    def test_a_core_change_runs_every_leaf(self) -> None:
        config = with_leaves("market", "fix")
        result = planned(["rust/src/serie.rs"], config=config)
        self.assertEqual(result.jobs, EVERY)
        self.assertEqual({entry["name"] for entry in result.leaves}, {"market", "fix"} | set(CONFIG.leaves))
        self.assertEqual(result.python_legs, list(config.python_legs))

    def test_a_leaf_manifest_runs_everything(self) -> None:
        # A manifest can move the lock and every cargo command loads it.
        config = with_leaves("market")
        self.assertEqual(planned(["rust/market/Cargo.toml"], config=config).jobs, EVERY)

    def test_a_leaf_with_an_msrv_is_checked_at_it(self) -> None:
        config = with_leaves("avro", "parquet", "iceberg")
        result = planned(["rust/iceberg/src/scan.rs"], config=config)
        msrv = [entry for entry in result.leaves if entry["lane"] == "msrv"]
        self.assertEqual(msrv, [{
            "name": "iceberg", "package": "yggdryl-iceberg", "lane": "msrv", "shard": "rest",
            "flags": "--features s3tables", "cache": "", "toolchain": plan.msrv_toolchain(), "save": "false",
        }])

    def test_a_crate_under_rust_needs_a_line(self) -> None:
        with tempfile.TemporaryDirectory() as folder:
            root = pathlib.Path(folder)
            (root / "rust" / "src").mkdir(parents=True)
            (root / "rust" / "spare").mkdir()
            (root / "rust" / "spare" / "Cargo.toml").write_text("[package]\n")
            (root / "Cargo.toml").write_text('[workspace]\nmembers = ["rust", "rust/spare"]\n')
            problems = plan.check_leaves(CONFIG, root)
        self.assertIn("rust/spare/Cargo.toml has no line under [leaves] in rows.toml", problems)
        self.assertIn("Cargo.toml: member `rust/spare` is neither a binding nor a leaf rows.toml lists", problems)

    def test_a_line_needs_its_crate(self) -> None:
        with tempfile.TemporaryDirectory() as folder:
            root = pathlib.Path(folder)
            (root / "rust" / "src").mkdir(parents=True)
            (root / "Cargo.toml").write_text('[workspace]\nmembers = ["rust"]\n')
            problems = plan.check_leaves(with_leaves("market"), root)
        self.assertIn("rows.toml: leaf `market` has no rust/market/Cargo.toml", problems)


class Shards(unittest.TestCase):
    TARGETS = [
        {"kind": ["lib"], "name": "crate"},
        {"kind": ["test"], "name": "fix"},
        {"kind": ["test"], "name": "graph"},
        {"kind": ["test"], "name": "root"},
        {"kind": ["bench"], "name": "fix"},
        {"kind": ["bench"], "name": "types"},
        {"kind": ["example"], "name": "capture"},
    ]

    def config(self, **lanes) -> plan.Config:
        config = plan.load_config()
        config.shards = {"crate": {lane: {k: tuple(v) for k, v in named.items()} for lane, named in lanes.items()}}
        return config

    def test_rest_is_every_target_no_shard_names(self) -> None:
        config = self.config(full={"fix": ["test:fix", "bench:fix"]})
        self.assertEqual(plan.shard_arguments(config, "crate", "full", "fix", self.TARGETS),
                         ["--test", "fix", "--bench", "fix"])
        self.assertEqual(plan.shard_arguments(config, "crate", "full", "rest", self.TARGETS),
                         ["--lib", "--examples", "--test", "graph", "--test", "root", "--bench", "types"])

    def test_an_unsharded_lane_is_all_targets(self) -> None:
        config = self.config()
        self.assertEqual(plan.shard_arguments(config, "crate", "default", "rest", self.TARGETS), [
            "--lib", "--examples", "--test", "fix", "--bench", "fix",
            "--test", "graph", "--test", "root", "--bench", "types",
        ])

    def test_a_missing_or_doubled_target_is_refused(self) -> None:
        missing = self.config(full={"gone": ["test:gone"]})
        with self.assertRaisesRegex(plan.PlanError, "no target of"):
            plan.shard_arguments(missing, "crate", "full", "rest", self.TARGETS)
        doubled = self.config(full={"a": ["test:fix"], "b": ["test:fix"]})
        with self.assertRaisesRegex(plan.PlanError, "is in shards"):
            plan.shard_arguments(doubled, "crate", "full", "rest", self.TARGETS)


class TheGate(unittest.TestCase):
    @staticmethod
    def needs(planned_jobs: str, **results: str) -> dict:
        needs = {"changes": {"result": "success", "outputs": {"jobs": planned_jobs}}}
        needs.update({job: {"result": result, "outputs": {}} for job, result in results.items()})
        return needs

    def test_planned_passed_and_the_rest_skipped_is_green(self) -> None:
        self.assertEqual(plan.verdict(self.needs(" a ", a="success", b="skipped")), [])

    def test_a_planned_job_skipped_is_red(self) -> None:
        self.assertEqual(plan.verdict(self.needs(" a ", a="skipped")), ["a: skipped, expected success"])

    def test_an_unplanned_job_that_ran_is_red(self) -> None:
        self.assertEqual(plan.verdict(self.needs(" ", b="success")), ["b: success, expected skipped"])

    def test_a_failed_planner_is_red(self) -> None:
        self.assertEqual(len(plan.verdict({"changes": {"result": "failure", "outputs": {}}})), 1)

    def test_the_ledger_keeps_the_newest_line_per_proof(self) -> None:
        with tempfile.TemporaryDirectory() as folder:
            path = pathlib.Path(folder) / "ledger.txt"
            plan.write_ledger([("a", "x", 1), ("a", "x", 3), ("b", "y", 2), ("c", "z", 0)], 2, path)
            self.assertEqual(plan.read_ledger(path), [("a", "x", 3), ("b", "y", 2)])


class Globs(unittest.TestCase):
    def test_the_glob_magic(self) -> None:
        cases = {
            "rust/src/**": (["rust/src/a.rs", "rust/src/a/b.rs"], ["rust/srcx/a.rs", "rust/tests/a.rs"]),
            "python/*": (["python/Cargo.toml"], ["python/src/lib.rs"]),
            "**/*.rs": (["a.rs", "cli/src/main.rs"], ["a.rsx", "README.md"]),
            "scripts/generate_*.py": (["scripts/generate_mic_table.py"], ["scripts/ci/generate_x.py"]),
        }
        for pattern, (yes, no) in cases.items():
            for path in yes:
                self.assertTrue(plan.glob_regex(pattern).match(path), (pattern, path))
            for path in no:
                self.assertFalse(plan.glob_regex(pattern).match(path), (pattern, path))

    def test_a_pattern_the_two_engines_could_read_apart_is_refused(self) -> None:
        for pattern in ["rust/[ab]", "rust/**.rs", "/rust", "rust/"]:
            with self.assertRaises(plan.PlanError):
                plan.glob_regex(pattern)


if __name__ == "__main__":
    unittest.main()
