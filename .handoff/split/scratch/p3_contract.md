# P3 contract - D36 in place: the storage-backend extension point

Design: `scratchpad/d36_design.md` (normative; its numbered decisions are this contract's
sections). Inputs: `scratchpad/s3_backend_map/design_inputs.md` (Part A facts with anchors)
and the four maps beside it. Program rules (AGENTS.md is in your context): no test code
under any `src/`; no back-compat; one commit; the exact trailer; no model identifier in the
repo; never push to main, never publish, never touch live AWS, never run the three exchange
scripts against anything but their own fetched servers; a request-count pin never moves - a
count that moves is a design defect to fix, not a number to re-pin.

## Scope of P3 (in place)
Nothing moves to a new crate. The core gains the register and the `Registered` variant,
`IOBase` gains the capability verbs with defaults, the S3 trio implements `RegisteredHandle`
and the overrides, `S3_BACKEND` (a `StorageBackend` static in `s3/mod.rs`) is claimed by the
core itself under `CORE` at first use (as the media are seeded), `Site::Store` becomes
`Site::Opened`, the wildcards call the verbs, the bindings pick classes by `as_any`, the pins
and pages state the shape. The `s3` feature stays. `rows.toml`'s `[leaves]` stays as is (the
`s3` line arrives with the crate in S6d).

## Stage 1 - define workers, disjoint files, no cargo
Worktree: `git -C /home/user/yggdryl worktree add /home/user/yggdryl-p3 -b wip/p3 wip/s3`
(the most advanced tree; S3's uncompiled sweep touches no file below; P2's four lines in
`holder/mod.rs` and `iobase/iomedia.rs` are merged by the lane manager). Each worker edits only
its set; a request to another worker is written in the report, never applied. `rustfmt
--edition 2024 --check` on every changed `.rs` is the one tool run.

- **W1 register and holder**: `rust/src/holder/backend.rs` (new: `StorageBackend`,
  `RegisteredHandle`, the `static BACKENDS: Register<Scheme, &'static dyn StorageBackend>`,
  `claim_backend(backend, by) -> Result<Claim>` all-or-none over `schemes()` with the core
  schemes refused by name, `backend_for(scheme) -> Option<&'static dyn StorageBackend>`,
  `backends()`, `seed()` claiming `s3::S3_BACKEND` under `plugin::CORE` under the `s3`
  feature), `rust/src/holder/mod.rs` (`Holder::Registered(Box<dyn RegisteredHandle>)`; the
  three `cfg(feature = "s3")` S3 variants deleted; `from_url`: the backend asked after lowering
  and the local/ZIP arms, before `http`, the query properties `is_property` refused as today
  through `backend_for`, the answer `described`; `from_handle` -> `reopen`; `exists`,
  `as_io(_mut)`, `as_media(_mut)` one arm each; `into_byte_stream`'s `Registered` arm over
  `owned_stream_bytes` else the cursor; `is_backend_property` through `backend_for`; the
  refusal for a scheme no claim answers naming the crate to install - the install sentence the
  media refusal uses, re-spelled for a backend), `rust/src/holder/locator.rs` (doc: the
  locator names objects, the backend holds bytes), `rust/src/lib.rs` exports,
  `.api-inventory.txt` holder section. Report the exact `Holder` doc rewrite (holder/mod.rs:60-66
  refuses `Box<dyn IOBase>` today - replace with the `RegisteredHandle` sentence).
- **W2 IOBase and the trio**: `rust/src/iobase.rs` (`upload_from`, `discard`, `as_leaf`,
  `as_container`, `set_known_size` with the design's defaults and rustdoc stating the default
  and who overrides), `rust/src/iobase/delegate.rs` or wherever `delegate_iobase!` lives (forward
  the five), every wrapper the macro does not cover (grep `impl IOBase for`: `Buffered`,
  `Coded`, `Transcoded`, `Hashed`, `Counted`, `Text`, `Media`, `Handle`, `Cursor`...) - forward
  or state why not; `rust/src/s3/file.rs`, `path.rs`, `folder.rs` (`impl RegisteredHandle`
  each: `exists` the role's, `reopen` what `from_handle`'s arm did - folder cloned, path/file
  through `sibling` and `as_directory`/`as_file` - `as_any`; `IOBase` overrides: `upload_from`
  = today's multipart, `discard` = today's `discard` answering `true`, `as_leaf`/`as_container`
  on `S3Path` = `as_file`/`as_directory` boxed into `Holder::Registered`, `set_known_size` =
  `with_known_size`; the `pub(crate)` `byte_stream` kept for `owned_stream_bytes`),
  `rust/src/s3/mod.rs` (`S3_BACKEND`: `name` `yggdryl-s3`, `schemes` the ten, `is_property`
  = `S3Options::is_property`, `holder` = today's `from_url` arm body over `located_with`),
  `rust/src/s3/properties.rs` if `is_property` needs a free form. The request counts are
  the pins: every override sends exactly what the variant arm sent.
- **W3 iceberg, warehouse, s3tables**: `rust/src/iceberg/staging.rs` (`upload`, `unpublished`,
  `leaf`, `container`, `sized`: the S3 arms deleted, the verbs called on any `Holder`),
  `rust/src/iceberg/catalog/mod.rs` (`folder_role` over `as_container`/`kind`),
  `rust/src/iceberg/table.rs` (`is_backend_property` call unchanged in spelling),
  `rust/src/warehouse/handle.rs` (`Site::Opened { url, open }`, `Eq`/`Hash` by url, `Debug` by
  hand, `resolve` calling the opener with the properties; `Site::Store` deleted; the `aws`
  gate and `S3Options` leave the file), `rust/src/s3tables/catalog.rs` and `mod.rs` (the one
  `Site::Store` constructor builds the opener: an `Arc<dyn Fn>` capturing the bucket's
  `Session`, region and store properties, calling `S3_BACKEND`'s door - through the backend
  statics directly, since both live in the core in P3 - and `S3Options::with_session`),
  `rust/src/holder/counted.rs`/`buffered.rs` only if the macro forwarding needs them.
- **W4 bindings**: `python/src/holder/handles.rs` (classes by `as_any().downcast_ref::<S3File>()`
  etc. - the `warehouse.rs:59-80` precedent), `python/src/iobase.rs` (`Role::of` a `Registered`
  arm asking the handle's `kind()`; `cloned` replaced by `Holder::from_handle`;
  `container_holder`), `python/src/holder/fs.rs` if it names the variants,
  `node/src/iobase.rs` (`folder_holder_for`), `.api-bindings.txt` (no entry changes unless a
  spelling moves; report).
- **W5 tests, docs, AGENTS.md**: new pins in `rust/tests/holder/backend.rs` (mirror of
  `holder/backend.rs`: the claim of a core scheme refused naming the arm, a second claim of
  `s3` refused naming `CORE`, `backend_for("s3")` answering under the feature, `backends()`),
  `rust/tests/holder/mod_.rs` (`from_url("s3://b/k.parquet")` answers a `Registered` handle
  whose `media_type` and `codec` are the URL's - the missing pin; `from_handle` on it reopens
  with zero requests; `exists`; `into_byte_stream` one `GET`), `rust/tests/root/iobase.rs`
  (the five defaults on a local file: `upload_from` writes the bytes whole, `discard` answers
  false, `as_leaf`/`as_container` None, `set_known_size` no-op), `rust/tests/iobase_calls.rs`
  rows re-spelled where they matched `Holder::S3*` (14 lines across tests; `git grep -n
  'Holder::S3'`), `rust/tests/s3/**` where variants are matched, `rust/tests/iceberg/staging.rs`
  if it names them; docs: `docs/holder/index.md` (Handles: a "Storage backends" subsection -
  the register, the claim, the refusal; Object stores: "claimed by the core until
  `yggdryl-s3` does"), `docs/architecture.md` if it lists the extension points; AGENTS.md:
  the `holder/` row (the register, `Holder::Registered`, `from_url` order), the
  `iobase.rs` row (the five verbs), the `local/, fs/, zip/, s3/` row, the "storage backend"
  row of Common changes (claimed through `holder::claim_backend`), the Object stores section's
  first paragraph, the `plugin.rs` row (the backends claim on it), the `warehouse/` row
  (`Site::Opened`); `.api-inventory.txt` for every new public name; skills if one names the
  variants.

Each worker returns: files edited, the public names added/removed, requests to other
workers, assumptions it could not verify without compiling.

## Stage 2 - the lane manager (after S3 is landed and chain_s3 done; P4/P5 may be in flight -
P3's files are disjoint, but the cargo lock is one: wait for the other lane's chain marker
before taking it)
1. Under the git lock: merge the program branch into `wip/p3`, resolve (`holder/mod.rs`,
   `iobase/iomedia.rs` from P2 expected), merge `wip/p3` into the program branch working tree.
2. `cargo check -p yggdryl --all-targets --features "s3 s3tables iceberg http" --keep-going
   --message-format=short` until clean, then default features; `cargo fmt --all`.
3. Phase suites: `--test holder`, `--test root iobase`, `--test s3 --features s3`, `--test
   iceberg --features "iceberg s3"` filtered to staging/catalog, `--test s3tables --features
   s3tables`, `--test iobase_calls --features s3` filtered to the S3 and warehouse rows; every
   request-count pin green without edit.
4. The chain `$S/logs/chain_p3.sh` (as chain_p2.sh; the whole run `--all-features
   --no-fail-fast` first), `python scripts/check_api_inventory.py`, `mkdocs build --strict`.
5. DESIGN.md gains "### P3 results"; the handoff's D36 row marked built in place; one commit
   with `$S/p3_commit_message.txt`, push, read CI to `CI result`, fix at cause. Report.
