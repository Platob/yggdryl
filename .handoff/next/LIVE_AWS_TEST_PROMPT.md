# Live AWS tests of the 0.1.22 candidate - the prompt for a local session

Paste the block below into Claude Code running on your own machine, in a clone of
`Platob/yggdryl`, signed in to AWS (`aws sso login` or `aws login`) with a profile
allowed to create and delete S3 Tables table buckets in one test region. The cloud
session that built this branch never touches AWS; this run is the one that asks the
service itself, and the merge of PR #209 - which publishes 0.1.22 - waits on its
report.

```text
You run the live AWS checks that gate release 0.1.22 of yggdryl (PR #209, branch
ccr-0fe6f9d0-ruymat of Platob/yggdryl). Read AGENTS.md first; it binds you.

Hard rules: never publish (no cargo publish without --dry-run, no maturin/twine/npm
publish), never push to main, never merge or mark the PR ready, no tag, no release
run. AWS: use only the profile and region I name below; create only table buckets
named `yggdryl-live-<yyyymmdd>-<n>` and delete every resource you created before
you report, however a step ended; touch no other bucket, table or namespace; never
print a secret key, a session token or a full access key id (the crate masks key
ids in its logs - keep it that way). Report exact commands and exact results; a
step you skipped is reported as skipped with its reason.

Profile: <PROFILE>   Region: <REGION>   (ask me if I did not fill these in)

1. Setup. `git fetch origin && git switch ccr-0fe6f9d0-ruymat && git pull --ff-only`.
   Confirm `python3 scripts/release_packages.py version` prints 0.1.22 and that the
   newest CI run on the branch is green (`gh run list --branch ccr-0fe6f9d0-ruymat
   --workflow CI --limit 1`). Create `python/.venv` (Python 3.12) and install
   `"maturin>=1.15,<2" "pyarrow>=18" "pytest>=8" tzdata xxhash`, then
   `VIRTUAL_ENV=python/.venv python/.venv/bin/python -m maturin develop --release -m python/Cargo.toml`.

2. The crate against the live service: one table's life in a table bucket of its
   own, commit included, cleaned up by the test itself.
   `YGGDRYL_S3TABLES_PROFILE=<PROFILE> YGGDRYL_S3TABLES_REGION=<REGION> \
     cargo test -p yggdryl --features s3tables --test s3tables live -- --ignored --nocapture`
   Every step prints `ok` or `FAILED: <reason>`; the test fails naming each failure.

3. The medallion pipeline, local first, then live, over the same capture.
   The capture: `rust/tests/support/ulbridge.log`, split into two objects the way
   the suite splits it (lines 1-72 and 73-end) under a folder `<CAPTURE>/`, read
   through the glob `<CAPTURE>/*.log`.
   a. Local baseline: `python/.venv/bin/python python/tests/medallion.py
      --bronze <TMP>/bronze --silver <TMP>/silver --logs '<CAPTURE>/*.log'
      --start 2026-08-14T00:00:00Z --end 2026-08-15T00:00:00Z --runs 2`.
   b. Two table buckets: `aws s3tables create-table-bucket --name
      yggdryl-live-<yyyymmdd>-bronze --region <REGION> --profile <PROFILE>` and the
      same for `-silver`; keep both ARNs.
   c. Live: the same command with `--bronze <BRONZE ARN> --silver <SILVER ARN>`,
      `AWS_PROFILE=<PROFILE>` set and `YGGDRYL_LOG_LEVEL=INFO` once if a stage fails.
   d. Compare: every table's `read`/`wrote`/`skipped` line of run 1 and of run 2
      must equal the local baseline's, run 2 must append nothing to
      `log_messages` (the keyed append skips every line), and the instruments
      table and every market table's `instrumentcode` column must be present and
      filled where the local run fills them. Record each stage's seconds and its
      request counts from both runs.

4. Optional, when PyIceberg installs (`pip install "pyiceberg[pyarrow]==0.11.1"
   "boto3>=1.34"`): `YGGDRYL_S3TABLES_ARN=<SILVER ARN> AWS_PROFILE=<PROFILE>
   python/.venv/bin/python python/benchmarks/media/s3tables.py --min-time 0.2
   --repeat 3`. It reports SKIPPED without its inputs; report what it printed.

5. Clean up: for each bucket you created, list its namespaces and tables
   (`aws s3tables list-namespaces`, `list-tables`), delete every table
   (`delete-table`), every namespace (`delete-namespace`), then the bucket
   (`delete-table-bucket`). Confirm with `aws s3tables list-table-buckets
   --prefix yggdryl-live-` that none of yours is left.

6. Report. Write `.handoff/next/LIVE_AWS_RESULTS.md`: the commit tested (`git
   rev-parse HEAD`), the region (never the account's secrets), each step's exact
   command and result (the live test's ok/FAILED lines, the two medallion runs'
   tables side by side with the local baseline, the timings, the requests), what
   was cleaned up, and every failure with its narrowest reproduction. Commit only
   that file, ending the message with the two lines
   `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>` and
   `Claude-Session: https://claude.ai/code/session_01Gfky7FUx35U5i4UJcrQGKp`,
   and push to ccr-0fe6f9d0-ruymat. Do not fix code in this session: a failure is
   reported, and the fix lands from the branch's own lane with CI.
```

What the run proves that CI cannot: the S3 Tables control plane and its object
store answering the crate's signed requests (CI runs against MinIO, Azurite,
fake-gcs-server and the crate's own fake control plane), the commit protocol's
`UpdateTableMetadataLocation` under the service's own conflict rule, and the
medallion pipeline's request counts over a real network.
