# P3 define - reconciled (D36 in place), wip/p3 at /home/user/yggdryl-p3

State: every cross-worker request is applied or verified already applied. `rustfmt --edition 2024 --check` is clean on all 50 changed `.rs` files. `generate_internals.py --check` is current, `check_api_inventory.py` is current, and `mkdocs build --strict` passes (built into the scratchpad, then removed). No cargo has been run. Nothing is committed. `.handoff/` is untouched.

Three most important lines:
1. The blocking request from all four workers is applied: `impl IOBase for Holder` now forwards `upload_from`, `set_known_size`, `discard`, `as_leaf` and `as_container` (rust/src/holder/mod.rs). Without it, every S3 override was unreachable through a `Holder` and the S3, Iceberg-staging and warehouse request-count pins would have moved.
2. No residue of the deleted variants is left in rust/, node/ or cli/. All 33 grep matches are the Python classes `S3File`/`S3Folder`/`S3Path`, which D36.8 keeps.
3. A merge into the program branch (49bcab3e3, which has P2) gives four textual conflicts: AGENTS.md, avro/batch.rs, parquet/mod.rs and text/handle.rs. text/handle.rs also needs a semantic fix: Text's `upload_from` and `discard` must invalidate P2's MediaCache before forwarding.

## Files edited, all workers (54 modified, 2 new; +1748/-690)

- **W1, register:** rust/src/holder/backend.rs (new), holder/mod.rs, holder/locator.rs, .api-inventory.txt (holder sections).
- **W2, IOBase and the trio:**
  - rust/src/iobase.rs, iobase/bytes.rs (`delegate_iobase!`, `__delegate_resolved_iobase`, `Box<dyn IOBase>`).
  - holder/counted.rs, holder/buffered/mod.rs, media/mod.rs, text/handle.rs, xxhash/handle.rs.
  - The media wrappers: parquet/mod.rs, csv/media.rs, avro/batch.rs, ipc/mod.rs, excel/media.rs, xmla/media.rs.
  - coding/coded.rs, coding/mod.rs, charset/transcoded.rs.
  - warehouse/table.rs, warehouse/media.rs.
  - s3/mod.rs, s3/file.rs, s3/path.rs, s3/folder.rs.
- **W3:** iceberg/staging.rs, iceberg/catalog/mod.rs, warehouse/handle.rs, s3tables/catalog.rs.
- **W4:** python/src/iobase.rs, python/src/holder/handles.rs, python/src/holder/fs.rs, node/src/iobase.rs.
- **W5:**
  - Tests: rust/tests/holder/backend.rs (new), holder.rs, holder/mod_.rs, root/iobase.rs, iobase_calls.rs, s3.rs, s3/{file,path,folder,mod_}.rs, iceberg/catalog/mod_.rs, warehouse/{handle,media}.rs.
  - Docs and inventory: docs/holder/index.md, docs/architecture.md, AGENTS.md, .api-inventory.txt (IOBase and s3 entries), skills/yggdryl-storage/references/rust.md.
- **Reconciler (this pass):**
  - rust/src/holder/mod.rs: Holder forwards the five verbs. The placement is W2's and W3's (`upload_from` after `append_bytes`, `set_known_size` after `size`, `discard` after `remove`, `as_leaf`/`as_container` after `child_by_path`). W4 and W5 proposed the same bodies in other places. `pub(crate) use backend::unregistered;` is added and `from_url`'s last refusal goes through it.
  - rust/src/lib.rs: `scripts/generate_internals.py` was run. It deleted only `#[cfg(feature = "s3")] pub use crate::s3::file::internals as s3_file;`.
  - rust/src/s3/client.rs: the duplicate `short_upload` is deleted and `put_streamed` calls `crate::iobase::short_upload`. The message is byte-identical (W2's optional request).
  - rust/src/warehouse/catalog.rs: the `s3tables` no-factory refusal is now `crate::holder::unregistered(url.scheme())`. The text is identical and now has one owner (W1's optional request). The rustdoc of `holder/backend.rs` `unregistered` is widened to cover a catalog factory.
  - rust/src/scheme.rs: the stale rustdoc of `is_object_store`, `is_http` and `has_container` no longer says the predicate selects the backend; it names the register (`holder::backend_for`). This is W2's vocabulary note.
  - docs/uri/index.md: the "Store container" row says the same and links `../holder/index.md#storage-backends`.
  - rust/src/s3/path.rs: the `as_leaf` rustdoc now says an undecided location keeps the default `upload_from`, `discard` and `set_known_size` (W5's note to W2).

## Public names

Added:
- `holder::{StorageBackend, RegisteredHandle, claim_backend (= backend::claim), backend_for, backends}`.
- `Holder::Registered(Box<dyn RegisteredHandle>)`, `Holder::downcast_ref`, `Holder::downcast_mut`.
- The five `IOBase` verbs, each with a default: `upload_from`, `discard`, `as_leaf`, `as_container`, `set_known_size`.
- `s3::S3Backend` and `s3::S3_BACKEND`.
- `impl RegisteredHandle` and `impl From<_> for Holder` for `S3Path`, `S3Folder` and `S3File`.
- `holder::counted::Call::{UploadFrom, Discard, SetKnownSize, AsLeaf, AsContainer}`. `Call::COUNT` goes from 34 to 39, and `ALL` is in declaration order (checked).

Crate-private additions:
- `iobase::read_upload`; `iobase::short_upload` made `pub(crate)`.
- `holder::sibling` made `pub(crate)` and generic.
- `holder::unregistered`.
- `warehouse::handle::{Opener, Site::Opened}`.
- `Bucket::store_site`, `iceberg::catalog::no_folder`.

Removed:
- `Holder::{S3Folder, S3Path, S3File}`.
- `yggdryl::internals::s3_file::upload_from`.
- Crate-private: `Site::Store`, `S3File::upload_from` (inherent) and `S3File::discard -> Result<()>`, s3/client.rs `short_upload`.
- Python-private: `cloned`, `rebuilt_arrow_holder`, `store_sibling`.

`.api-bindings.txt` is unchanged: no binding spelling moved.

## Requests: applied, verified or declined

**Applied in this pass:** the `Holder` forwarding (W2, W3, W4, W5), the internals regeneration (W2, W5), the `short_upload` dedupe (W2), the single owner of the install sentence (W1), and the `S3Path` rustdoc (W5).

**Verified already applied:**
- W2→W5: tests/s3/file.rs calls `upload_from` as a method, and tests/s3.rs gates `s3/file.rs` on `s3` alone.
- W2→W5: the inventory lines; the defaults pinned in root/iobase.rs; the AGENTS.md iobase.rs row.
- W3→W2: `S3Path::as_leaf`/`as_container` exist.
- W3→W5: the AGENTS.md warehouse/ and s3tables/ rows. There are 0 `Site::Store` mentions anywhere.
- W1→W2/W3/W4/W5:
  - `crate::s3::S3_BACKEND` exists, with `"yggdryl-s3"`, the 10 schemes and `S3Options::is_property`; `holder()` is W1's body verbatim.
  - The `reopen` bodies go through `sibling`.
  - The `as_any` calls are fully qualified where they would be ambiguous.
  - The intra-doc link names are kept, and `byte_stream` stays `pub(crate)`.
  - W5's exact refusal texts match W1's.

**Not applied, with reasons:**
- ZIP's public inherent `ZipPath::as_leaf(&self) -> &ZipLeaf` and `ZipNode::as_leaf(&self, path)`, and the private `http::Request::discard -> Result<()>`, now spell the new IOBase verbs with another meaning. Renaming the ZIP pair is a public API change that D36 does not decide. It compiles, because the inherent method wins. **Lane manager decision.**
- W3's staging `container()` note: an `S3File` would now be re-described as its prefix, where the old arm left it a file. Every caller (iceberg/table.rs:761, 901, 1013, 1686) passes `root.child_by_path(METADATA_DIR)`, which answers an `S3Path`, so the behaviour is unchanged in practice. Left as is.
- Contract against design:
  - W4's `Role::of` picks the class by `downcast_ref` and falls back to the base `IOBase` class (`Role::Held`). The contract text said to ask `kind()`; D36.8 says `downcast_ref`, so the design is followed.
  - W1's `claim_backend` returns `Result<()>` where the contract text says `Result<Claim>`. The design says nothing on this, and every other claim door returns `Result<()>`, so it is kept.
- Python's `rebuilt` (now `Holder::from_handle`) behaves differently in seven observable ways (W4 report, items a-g):
  - (a) An HttpSession is held again as a Session.
  - (b) An HttpRequest is cloned with its defaults and authorization.
  - (c) One HTTP answer is refused as `io.UnsupportedOperation`.
  - (d) A ZIP member is held through its archive, and an archive root is refused.
  - (e) A Catalog, Namespace or Uri is cloned.
  - (f) and (g): the buffer and orphan-object refusals are now `Unsupported` through `storage_error`.

  No test or page matches the old texts (grepped). Pins for (c) and (d) are optional.
- Node's `JsIOBase::rebuilt` still rebuilds from the URL. The change does not name Node (AGENTS §4); the fix would be the same one-line redirect.

## Residue (grep `Holder::S3\|S3Folder(\|S3Path(\|S3File(` over rust python node cli)

- None in rust/, node/ or cli/.
- All 33 matches are the Python classes, which D36.8 keeps:
  - python/tests/holder/test_init.py
  - python/yggdryl/_native.pyi:4258/4271/4284
  - python/benchmarks/media/s3tables.py:177
  - python/src/holder/handles.rs:432-433, the rustdoc of the Python constructor
- `is_object_store()` remains in python/src/iobase.rs:111 and node/src/iobase.rs:129 (`folder_holder_for`). The bindings name the s3 module directly (D36.6/8), so this is correct.

## Assumptions to verify at the compiler (collected)

**Types and traits**
1. `&**inner` on `Box<dyn RegisteredHandle>` upcasts to `&dyn IOBase` and to `&dyn IOMedia` under MSRV 1.94, as `Media::Registered` already does.
2. `Holder` stays `Send + Sync`, since `RegisteredHandle: IOBase(Send) + Sync`.
3. `static SCHEMES: [Scheme; 10]` and the doctest's `static SHADOWED: [Scheme; 1]` compile: `Scheme` is `Sync` and its consts are usable in a static initializer. `Scheme` is `Ord + Display`, which was checked.
4. `.map(Holder::from)` infers the `From<S3*>` impls.
5. In Python, `handle.as_container()`/`is_container()` on `&Box<dyn RegisteredHandle>` resolve to the supertrait methods.
6. In W3's opener:
   - `Arc::new(closure)` coerces to `Opener` at the field site.
   - `open(properties)` can be called through `&Arc<dyn Fn>`.
   - `aws::Session` and `Properties` are `Send + Sync`.
7. `S3File::reopen` uses `IOBase::url(self)` to avoid the inherent `S3File::url() -> &Url`.

**Name resolution and lints**
8. The inherent methods named in "Not applied" win over the trait verbs.
9. `S3Path::remove` does `file.discard()?;` and drops the `bool` without a warning.
10. `read_upload`'s `source.take(length)` moves the `&mut dyn Read`, which is not used after.
11. Clippy pedantic has not been run on any new code (doc_markdown, match_same_arms, must_use, W5's unused `_named` guards).
12. Under `cargo doc -D warnings` (default and all features), the links in backend.rs to `IOBase::{upload_from, discard, as_leaf, as_container, set_known_size, owned_stream_bytes}` and the scheme.rs links to `crate::holder::backend_for` resolve.

**Tests and runtime behaviour**
13. Request counts are unchanged:
    - `upload_from` sends the same `PUT` or parts+2.
    - `discard` answers `true` and sends nothing.
    - `as_leaf`/`as_container`/`reopen` send no request.
    - `set_known_size` is the old `with_known_size`.

    The first proof is `--test s3 --features s3`, `--test iobase_calls --features s3`, `--test iceberg --features "iceberg s3"` (staging, catalog) and `--test s3tables`.
14. `into_byte_stream` now reads every registered handle through `owned_stream_bytes`. An `S3Path` reached through a `Holder::Uri` leaf now costs one lazy `GET` (W1). No pin is known for this.
15. W5's test backend:
    - The explicit `delegate_iobase!` list plus two `IOMedia` methods is the whole contract. P2's `read_origin_field` has a default; checked on 49bcab3e3.
    - FakeS3 logs a listing as a `GET` with key None.
    - `S3Folder` over an object's key gives the prefix `lake/part.parquet/`.
    - `pwrite` may `GET` before staging.
    - `Coded::wrap` keeps the csv base.
16. Error texts moved:
    - The query refusal now has target "storage location" and says "the `s3` backend".
    - Without `s3`, `s3://` gets the install sentence.
    - Python's `rebuilt` refusal types change, as listed above.
17. Without `s3`, `Site::of` gives an `s3:` `Holder::Uri` no site (it was `Native`). With `s3` on, nothing changes.
18. Staging `upload()` reads exactly `size` bytes through the default for a non-S3 target.

## Conflicts when merging the program branch into wip/p3

Simulated with `git merge-tree c015226b6 <P3 tree fa2fb3c1> 49bcab3e3` (old-style tree merge; no commit made). 22 files are changed in both; 4 have textual conflicts.

**Textual conflicts**
- **AGENTS.md:** the `iobase.rs` row (P3's capability verbs against P2's `read_origin_field`/`as_any` wording) sits next to the `field.rs` row (S3 stage 1 against P2). Keep P2's row and add P3's capability clause.
- **rust/src/avro/batch.rs** and **rust/src/parquet/mod.rs:** the `delegate_iobase!` list. P2 dropped `set_media_type` (the wrapper now overrides it to drop its cache), and in parquet it rewrote `size` over the cache. P3 added `set_known_size`. Resolve as P2's list plus `set_known_size`.
- **rust/src/text/handle.rs:**
  - P2's side: Text now owns `pwrite`, `truncate`, `create_bytes`, `write_all_bytes`, `append_bytes`, `set_media_type`, `open`, `opened`, `close`, `clear` and `remove`, each touching `self.cache`, and its delegate list is narrower.
  - P3's side: it added `upload_from`, `set_known_size` and `discard` to the old list.
  - Resolve as P2's list plus `set_known_size`, and make `upload_from` and `discard` overrides that call `self.cache.invalidate()` before forwarding. Otherwise an upload through Text skips the cache drop and leaves a stale line count.

**Clean merges, with semantic notes**
- **holder/mod.rs** and **iobase/bytes.rs:** P2's `read_origin_field` forwards sit on the `IOMedia` impls of `Holder` and `Box<dyn IOBase>`. They do not overlap P3's `IOBase` edits, and the `Registered` arm of `as_media` serves them.
- **media/mod.rs, warehouse/media.rs, warehouse/table.rs, coding/coded.rs, coding/mod.rs, holder/buffered/mod.rs, csv, excel, xmla, ipc:** P2's media-cache edits merge cleanly. After the merge, check that no delegate list both lists and overrides a verb (a duplicate method fails `cargo check`).
- **Media and MediaTable:** they forward `upload_from`/`discard` to the medium. The medium keeps the default, which calls its own cache-aware `write_all_bytes`, so this is correct.
- **iomedia.rs:** changed by S3 stage 1 and P2, not by P3.
- **tests/iobase_calls.rs:** P2's +73 lines add no S3 variant; line 498 merges to P3's `Holder::from`.
- **lib.rs:**
  - Overlaps: P2 (+2), the S3 settle (b8ddef717: `pub(crate) use parser::{folds_equal, normalized}`, no conflict), and the P4 and P5 worktrees.
  - Action: re-run `python scripts/generate_internals.py` after every merge.
- **.api-inventory.txt** and **docs/holder/index.md:** clean. Re-run `check_api_inventory.py` and `mkdocs build --strict`.

**Other lanes**
- The S3 settle (0a24f1806..b8ddef717) overlaps P3 only in lib.rs.
- The p4 and p5 worktrees also touch .api-inventory.txt, AGENTS.md and rust/src/lib.rs. The s4 and s5 worktrees touch none of P3's files.

## Next (lane manager, stage 2)

1. `cargo check -p yggdryl --all-targets --features "s3 s3tables iceberg http" --keep-going --message-format=short`, then the same with default features.
2. The phase suites: `--test holder` (default and `--features s3`), `--test root iobase`, `--test s3 --features s3`, `--test iobase_calls --features s3`, `--test iceberg --features "iceberg s3"` (staging, catalog), `--test s3tables --features s3tables`, `--test warehouse --features s3`.
3. `python scripts/check_docs_examples.py --lang rust` for the new Storage backends block.
