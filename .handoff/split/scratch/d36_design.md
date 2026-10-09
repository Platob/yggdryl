### D36 - `yggdryl-s3`: the object-store backend as the ninth crate, through a storage-backend extension point

**Decision.** The object-store backend (`rust/src/s3/`: Amazon S3, Google
Cloud Storage, Azure Blob Storage) reaches `Holder` through a register, as a
medium reaches `Media`, and leaves for `rust/s3/` as `yggdryl-s3`. `aws/` and
`auth/` stay in the core under the `aws` feature.

1. **The register.** `rust/src/holder/backend.rs`: `StorageBackend` - `name()
   -> &'static str` (the crate), `schemes() -> &'static [Scheme]`,
   `is_property(&str) -> bool`, `holder(&Url, &[(String, String)]) ->
   Result<Holder>` - one `static` per backend, claimed all-or-none on
   `plugin::Register<Scheme, &'static dyn StorageBackend>` (`claim_backend`,
   `backend_for`, `backends`), asked by `Holder::from_url` after the
   identifier is lowered and after the local and ZIP arms, before `http`, its
   answer `described` as every other (media type, codec - the pin P3 writes
   first, missing today). A scheme a core arm holds (`file`, `zip`, `http`,
   `https`, `memory`, ...) is refused at claim. `Locator` stays what it is - an
   object a location names, asked before lowering - and is not widened. The
   ten object-store schemes (`s3 s3a s3n gs gcs az abfs abfss wasb wasbs`) are
   the backend's claim; `Holder::is_backend_property` reads
   `backend_for(scheme)?.is_property`; a scheme no claim answers is refused
   naming the crate to install.
2. **The handle.** `Holder::Registered(Box<dyn RegisteredHandle>)`,
   `RegisteredHandle: IOBase + Sync + Debug` with `implementation_name`,
   `exists(&self) -> bool` (its role's), `reopen(&self) -> Result<Holder>`
   (what `from_handle` answers: the same client, nothing sent), `as_any` and
   `as_any_mut`. `S3Folder`, `S3Path` and `S3File` leave the enum: three `cfg`
   arms out of each of the six exhaustive matches, one `Registered` arm in.
3. **The capabilities** the wildcards specialized on S3 become `IOBase`
   methods with defaults - no second storage trait: `upload_from(&mut self,
   source: &mut dyn Read, length: u64) -> Result<()>` (default: the source
   read whole then `write_all_bytes`, today's fall-through), `discard(&self)
   -> Result<bool>` (default `false`: nothing staged to drop, the caller
   removes), `as_leaf(&self) -> Result<Option<Holder>>` and
   `as_container(&self) -> Result<Option<Holder>>` (default `None`: the handle
   as it is), `set_known_size(&mut self, size: u64)` (default no-op);
   `into_byte_stream`'s `Registered` arm reads the existing
   `owned_stream_bytes` (S3's lazy resuming `GET`, one request) else the
   cursor reader. `delegate_iobase!` forwards them, so every wrapper keeps
   them. `iceberg/staging.rs` (`upload`, `unpublished`, `leaf`, `container`,
   `sized`), `iceberg/catalog/mod.rs` `folder_role` and Python's `Role::of`
   lose their S3 arms and call the verbs. Every request-count pin holds: the
   trio keeps its own `IOBase` impls, and construction, child resolution and
   a reopen stay request-free.
4. **`Site::Store`** becomes `Site::Opened { url: Url, open: Arc<dyn
   Fn(&Properties) -> Result<Holder> + Send + Sync> }` - `Eq`/`Hash` by url,
   `Debug` by hand - built by `s3tables/` over the backend under the bucket's
   session; `warehouse/handle.rs` names no AWS or S3 type.
5. **`aws/` and `auth/` stay core**, under `aws`: who this process is to AWS
   for every service, `with_sigv4` an inherent method of `http::Request`,
   `ureq` in no published signature. The 58 non-public items `s3/` reaches
   land in `implementer` (gated as their owners are, the ureq-typed
   forwarders included - the door is hidden); the 23 `cfg(feature = "s3")`
   sites in `aws/`, `auth/`, `http/` and `xml/` re-key to the feature of the
   module that holds them.
6. **Edges.** D15 amended: `yggdryl-iceberg`'s `s3tables` depends on
   `yggdryl-s3` (which implies `yggdryl[aws]`) and installs it; the core's `s3`
   feature is deleted at S6d (P3 keeps it in place); the bindings and the CLI
   link the crate and install it at import and start; a pure-Rust
   `Holder::from_url("s3://..")` before `install()` is refused naming
   `yggdryl-s3`, as a medium is.
7. **Pins.** The 11 core-resident pins over S3 handles and the five
   `accounting::iceberg` tests move at S6d to `rust/iceberg/tests/` (a
   dev-dependency on the leaf from the core's tests is a cycle; from
   iceberg's it is the `s3tables` edge); `FakeS3` stays in
   `rust/tests/support/`, `#[path]`-included by the crate's tests; the three
   exchange scripts keep their `--test interop s3::<dialect>::` and move with
   the tests.
8. **Bindings.** Python's `S3File`/`S3Folder`/`S3Path` classes pick by
   `downcast_ref` through `as_any` (the `warehouse.rs` precedent), `cloned`
   replaced by `from_handle`; Node's `folder_holder_for` links the crate.
9. **CI.** `[leaves]` gains `s3 = { package = "yggdryl-s3", jobs =
   ["object-interop", "azure-interop", "gcs-interop"] }` and `iceberg` takes
   `after = ["s3"]`; the planner's `LEAF_LINE` reads `[a-z][a-z0-9]*`,
   `EXCHANGES` gains the three, `NEVER` drops them for `s3`, its tests with
   it.
10. **Slices.** P3 in place: the register, `Holder::Registered`, the `IOBase`
    capabilities, `Site::Opened`, the trio claimed by the core itself under
    `CORE` at startup, the missing `from_url` pin, the pages (the Object
    stores section of `docs/holder/index.md` gains the extension point); S6d
    the move to `rust/s3/` with `install()`, the leaf line, the tests moved.
    P3's files are disjoint from P4's and P5's (`holder/`, `iobase.rs`,
    `iceberg/staging.rs`, `iceberg/catalog/mod.rs`, `warehouse/handle.rs`,
    `s3tables/`, `s3/`, the bindings' handle files), so it runs beside them
    and lands when its chain is clean.

Names: `StorageBackend`, `RegisteredHandle`, `Holder::Registered`,
`claim_backend`, `backend_for`, `backends`, `Site::Opened`.
