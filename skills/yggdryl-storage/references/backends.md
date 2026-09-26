# yggdryl-storage backends: configuration and cost

Tables only; runnable code for each backend is in the language references.
Every backend answers the same `IOBase` calls - this page is what differs:
how a location is spelled, what configures it, and what each call costs.

## Which backend a location reaches

| Spelling | Backend | Rust | Python | JavaScript |
| --- | --- | --- | --- | --- |
| no scheme, `/abs`, `C:\x`, `file:` | Local (memory-mapped) | `LocalPath`/`LocalFolder`/`LocalFile`, `Holder::local` | `IOBase(p)`, `LocalPath(p)`, `LocalFolder(p)`, `LocalFile(p)` | `new IOBase(p)` |
| none (a Buffer reports a `mem://...` identity, but that spelling is refused as input) | Buffer | `holder::Buffer::new()` / `Buffer::from_bytes(v)` | `IOBase.from_bytes(b)` | `IOBase.fromBytes(b)` |
| `s3`, `s3a`, `s3n` | Amazon S3 and S3-compatible (MinIO, ...) | `s3::file/folder/located` (`s3` feature) | `IOBase(url)`, `S3File/S3Folder/S3Path` | `new IOBase(url)` |
| `gs`, `gcs` | Google Cloud Storage | same | same | same |
| `az`, `abfs`, `abfss`, `wasb`, `wasbs` | Azure Blob Storage | same | same | same |
| an Arrow filesystem + opaque path | Filesystems | `FsPath/FsFolder/FsFile::from_path(Arc<dyn FileSystem>, path, uri)` | `IOBase.from_fs(pyarrow_fs, path, uri=)` | `IOBase.fromFs(handler, path, uri?)` |
| `file:///x.zip#member/path` | ZIP member | `zip::from_url(&url)`, `zip::mount(holder)` | Rust only | Rust only |
| `urn:ns:a:b` | the path the name spells, under the working directory | `Uri::locator` then a backend | `IOBase(Urn(...))` | `new IOBase(new Urn(...))` |
| `arn:aws:s3:::bucket/key` | the `s3:` URL it names | `Arn::locator` | `IOBase(Arn(...))` | `new IOBase(new Arn(...))` |
| `s3tables://...` | refused: no byte backend speaks S3 Tables | | | |

A handle reports the spelling it was handed (`s3a://` stays `s3a://`), and a
child keeps its parent's spelling.

## Local

| Fact | Rule |
| --- | --- |
| roles | `LocalPath` (undecided, resolves once), `LocalFolder`, `LocalFile` (memory-mapped leaf) |
| state | the canonical `file:` `Url` only; construction creates, opens and maps nothing |
| roots | `LocalFolder::temporary()` (platform temp dir), `home()` (`HOME`, then `USERPROFILE`; the absence names both), `config()` (= home + `.config`); none creates a directory |
| `LocalFile::create(path)` | truncates; `LocalFile::new(path)` does not |
| mapping | `size` is logical, `capacity` mapped; appends remap a logarithmic number of times; `flush`/`close` unmap and publish the logical length |
| hazard | the mapping aliases the file: another process truncating it raises SIGBUS - `copy_into` a `Buffer` for a snapshot |
| listing | sorted; a recursive walk never enters `.git`, `.venv`, `.DS_Store`; `include_private` lists dot-names (`Url::is_private`) |
| `open` caches | the descriptor and the mapping |
| cost | a `pread` over a mapped file is a `memcpy`; a page cache only pays where a fetch is real |

## Filesystems (Arrow `FileSystem`)

| Fact | Rule |
| --- | --- |
| seam | Rust `yggdryl::fs::FileSystem` (`MemoryFileSystem`, `LocalFileSystem` ship as references); Python any `pyarrow.fs.FileSystem`; JavaScript the synchronous handler protocol |
| path | opaque: never parsed, decoded or normalized - `bucket/v=a%2Fb.bin` reaches the store literally |
| `from_uri` (Python) | resolves a URI to a pyarrow filesystem once; `options=` override the query and go to `pyarrow.fs.S3FileSystem` |
| `from_uri` (JavaScript) | local URIs only; an `s3:` URI reports `Unsupported` - bind an implementation with `fromFs` |
| streams | `open_input_file` (random read), `open_input_stream` (sequential), `open_output_stream` (truncating), `open_append_stream`; each retains one backend stream |
| copy / move | one native call on one filesystem; across two, a bounded chunked copy published only after success |
| identity | `bound_uri` may carry secrets - log `masked_uri`; `same_location` needs filesystem equality plus byte-equal paths |
| JS handler | `typeName`, `equals`, `normalizePath`, `fileInfo`, `list`, `createDir`, `deleteDir`, `deleteDirContents`, `deleteRootDirContents`, `deleteFile`, `copyFile`, `move`, `openInputFile`, `openInputStream`, `openOutputStream`, `openAppendStream`; `bigint` sizes, offsets, ns mtimes |

## Object stores (S3, Google Cloud Storage, Azure Blob Storage)

Rust: the non-default `s3` feature (implies `aws`). Python and Node.js: built
in. Each REST API is spoken directly over synchronous HTTP/1.1 - no SDK, no
async runtime.

### Naming

| Input | Reads as |
| --- | --- |
| `s3://trades/lake/part.parquet` | bucket `trades`, key `lake/part.parquet` |
| `s3://trades.s3.eu-west-3.amazonaws.com/part.parquet` | virtual-hosted: bucket `trades`, region `eu-west-3` |
| `s3://localhost:9000/trades/lake/` | first part with a port, IP literal, `localhost`, or ending `.com`/`.io` is the **host**; bucket is the next part |
| `gs://trades.storage.googleapis.com/lake/x` | bucket `trades`, endpoint `storage.googleapis.com` |
| `abfss://lake@trades.dfs.core.windows.net/x` | container `lake` (user position), account `trades` |
| `az://lake/x` (bare container) | the account comes from the options (`account_name`, `adls.account-name`, `AZURE_STORAGE_ACCOUNT_NAME`) |
| raw key with spaces or `%` | Rust `s3::file_at(Provider::Aws, bucket, key)`, Python `S3File(bucket, key, provider="s3")` - the handle escapes it |
| a prefix | always ends in the delimiter (`folder_at(.., "lake")` has prefix `lake/`) |

### Request count per operation (asserted by tests)

| Operation | Amazon S3 | Google Cloud Storage | Azure Blob Storage |
| --- | --- | --- | --- |
| build a handle, a child, a media type, a partition | 0 | 0 | 0 |
| resolve a `lake/` (trailing slash) location | 0 | 0 | 0 |
| resolve any other `S3Path` role | 1 single-key listing, or 2 | same | same |
| ranged read | 1 ranged `GET` | 1 `GET` `alt=media` | 1 `GET` `x-ms-range` |
| whole read, stream drain, digest | 1 `GET` | 1 `GET` | 1 `GET` |
| size of a listed object | 0 | 0 | 0 |
| `size` on a closed handle | 1 `HEAD` (0 while open) | 1 `objects.get` (0 while open) | 1 `HEAD` (0 while open) |
| whole write | 1 `PUT` | 1 `multipart/related` `POST` | 1 `PUT` |
| large write | parts + 2 | chunks + 1 | blocks + 1 |
| append | 1 `GET` + 1 write (no `GET` while open) | same | same |
| remove | 1 `DELETE`, no probe | 1 `objects.delete` | 1 `DELETE` |
| listing, one level or a subtree | 1 per 1000 entries | same | same |
| empty or remove a prefix | 1 listing + 1 bulk delete per 1000 keys | per 100 | per 256 |

A recursive listing is one flat listing (keys in byte order are depth-first
pre-order). A ranged read learns the length from `Content-Range`;
`S3File::with_known_size(n)` takes one a manifest already stated (an Iceberg
scan reads each data file with one `GET`). `open` caches metadata, never bytes -
do it before wrapping a remote handle in `buffered`.

### Configuration precedence

| Order | Source |
| --- | --- |
| 1 | an explicit value (`S3Options::with_*`, Python/JS `options` property) - always wins |
| 2 | the URL (host, port, region in the host, user info) |
| 3 | the environment, swept under `AWS_`, `GOOGLE_`, `AZURE_`, `YGGDRYL_` (plus `with_environment_prefix("TRADING_")`) |
| 4 | the store's own files (`~/.aws/config`, `~/.aws/credentials`, ADC JSON) |
| 5 | the default |

`with_environment(false)` leaves only explicit values and the URL. Property
names match loosely (case, `-`, `_`, `.` are one; a store or tool prefix is
dropped), so `s3.access-key-id`, `AWS_ACCESS_KEY_ID` and `access_key` are one
knob. A value that will not parse, or a knob the client cannot honor
(`s3.signer.uri`), is refused rather than dropped. Sizes may carry a unit
(`8MiB`); durations are seconds.

| Knob | This crate | PyIceberg | PyArrow |
| --- | --- | --- | --- |
| endpoint | `endpoint` | `s3.endpoint`, `gcs.service.host`, `adls.endpoint` | `endpoint_override`, `scheme` |
| region | `region` | `s3.region` | `region` |
| credentials | `access_key_id`, `secret_access_key`, `session_token` | `s3.access-key-id`, `s3.secret-access-key` | `access_key`, `secret_key` |
| anonymous | `anonymous` | | `anonymous` |
| addressing | `path_style` | `s3.force-virtual-addressing` | `force_virtual_addressing` |
| timeouts | `timeout`, `connect_timeout` | `s3.request-timeout`, `s3.connect-timeout` | `request_timeout`, `connect_timeout` |
| proxy | `proxy` | `s3.proxy-uri` | `proxy_options` |
| encryption | `sse_type`, `sse_key`, `sse_md5`, `kms_key_id`, `encryption_scope` | `s3.sse.type`, `s3.sse.key`, `s3.sse.md5` | |
| containers | `allow_container_creation`, `allow_bucket_creation` | | `allow_bucket_creation` |
| uploads | `part_size`, `multipart_threshold` | `s3.multipart.part-size-bytes` | |
| retries | `max_attempts`, `num_retries` | `s3.retry.num-retries` | |
| listing | `list_page_size`, `page_size`, `max_keys`, `max_results` | | |

| Store | Knobs of its own |
| --- | --- |
| Amazon S3 | `profile`, `role_arn`, `role_session_name`, `external_id`, `mfa_serial`, `source_profile`, `credential_source`, `web_identity_token_file`, `sts_endpoint`, `sso_*`, `credential_process`, `config_file`, `shared_credentials_file`, `ca_bundle`, `use_fips_endpoint`, `use_dualstack_endpoint`, `ec2_metadata_*`, `payload_signing`, `storage_class`, `requester_pays`, `checksum_algorithm` (`CRC32`/`SHA256`) |
| Google | `project_id` (`gcs.project-id`, `GOOGLE_CLOUD_PROJECT`), `user_project`, `quota_project_id`, `credentials_file` (`GOOGLE_APPLICATION_CREDENTIALS`), `credentials_json`, `access_token`, `impersonate_service_account`, `storage_class`, `predefined_acl` |
| Azure | `account_name`, `account_key`, `connection_string`, `sas_token`, `tenant_id`, `client_id`, `client_secret`, `federated_token_file`, `managed_identity`, `blob_type` (`block`/`append`/`page`), `access_tier`, `encryption_scope`, `api_version`, `data_lake`, `authority_host` |

Rust builders: `S3Options::default().with_endpoint(..).with_region(..)
.with_credentials(Credentials::new(k, s)).with_path_style(true)
.with_session(session).with_google(GoogleOptions..).with_azure(AzureOptions..)`,
`S3Options::from_properties(pairs)?`, `explicit.under(&ambient)`,
`with_container_creation(false)`, `with_default_metadata([...])`.

### AWS identity (`aws::Session`)

The credential chain is botocore's, in botocore's order; a configured but
broken source is recorded and passed over, and the walk refuses only when
every source has been asked, naming each.

| Order | Source |
| --- | --- |
| 1 | an explicit set (`with_credentials`, a pair in options or the URL) |
| 2 | an explicit role (`with_assumed_role(AssumedRole::new(arn))`), signed by its `source_profile` / `credential_source` or the rest of the chain |
| 3 | environment: `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, `AWS_SESSION_TOKEN`, with `AWS_CREDENTIAL_EXPIRATION` and `AWS_ACCOUNT_ID` |
| 4 | a profile assuming a role (`source_profile`, `credential_source`, web identity token) |
| 5 | IAM Identity Center (the token `aws sso login` cached) |
| 6 | the credentials file, then `credential_process`, then the config file, then legacy boto files |
| 7 | the container endpoint, then the instance metadata service |

| Behavior | Rule |
| --- | --- |
| laziness | `Session::new()` states nothing; the chain is walked once, on the first request, and cached |
| refresh | a temporary set is replaced 15 minutes before it lapses; a failed refresh keeps the set until it actually lapses; `ExpiredToken` walks the chain once more |
| caches | assumed-role and SSO tokens are read from and written to `~/.aws/cli/cache` and `~/.aws/sso/cache` in the CLI's shape |
| sealing | `Session::new().with_environment(false)` reads no variable and no file unless given (`with_config_text`, `with_variables`, `with_directory`) |
| profile/region/endpoint | `with_profile`, else `AWS_DEFAULT_PROFILE`, `AWS_PROFILE`; `with_region`, else `AWS_REGION`, the profile's own; `endpoint_url("s3")` from `AWS_ENDPOINT_URL_S3` or the profile's `[services]`; these `AWS_*` variables are the session's, never swept into `S3Options` |
| Google / Azure | `GoogleOptions::with_impersonation(sa)` signs one `iamcredentials` call; Azure takes an Entra ID application on `AzureOptions` |

### Encryption, retries, failures

| `Encryption` | Amazon S3 | Google | Azure |
| --- | --- | --- | --- |
| `Default` | bucket rule | bucket rule | container rule |
| `Managed` | `SSE-S3` | default | default |
| `Kms(KmsKey)` | `SSE-KMS` / `aws:kms:dsse` | CMEK `kmsKeyName` | refused (use a scope) |
| `Customer(CustomerKey)` | `SSE-C` + MD5 | `x-goog-encryption-key` + SHA-256 | `x-ms-encryption-key` + SHA-256 |
| `Scope(name)` | refused | refused | `x-ms-encryption-scope` |

Throttles, 5xx and failed connections retry with full-jitter waits from a
token budget of 500, honoring `Retry-After` up to 30 s; a refusal (403) is not
retried; a stream cut part way resumes at the byte it stopped at. Absence and
conflict stay typed (`is_absent()`, `is_conflict()`); everything else is
`Error::Remote` with the store's own code. `stats()` answers the requests
that actually went out (`StatsSnapshot`).

## Buffered (page cache)

| Option | Default | Rule |
| --- | --- | --- |
| page size | 64 KiB | rounded up to a power of two, clamped to 64 B ..= 1 GiB |
| max bytes | 8 MiB | pinned pages included; clamped up to two pages, never rejected |
| TTL | 30 s from last access | pinned pages never expire |

| Behavior | Rule |
| --- | --- |
| miss | fetches one aligned page; a read crossing pages copies each straight into the caller's buffer |
| pins | the first and the current last page (magic bytes, a Parquet footer, an IPC schema); a moved end releases the old last page |
| writes | `pwrite` writes through and patches or drops overlapped pages; `truncate` drops what a resize could change; `close`, `clear`, `remove`, `clear_cache` drop the cache |
| composition | `Buffered<Coded<_>>` caches decoded pages; `Coded<Buffered<_>>` the encoded transport; wrapping twice reconfigures one cache |
| spellings | Rust `buffered(BufferedOptions::default().with_page_size(n).with_max_bytes(n).with_ttl(d))`; Python `buffered(page_size=, max_bytes=, ttl=seconds)`; JS `buffered({ pageSize, maxBytes, ttlMs })` |
| when it pays | over a store or a syscall per read (4x to 15x); over memory or a mapped file it costs ~2.7x |

## ZIP (Rust only)

| Fact | Rule |
| --- | --- |
| roles | `ZipNode` (root or member prefix), `ZipLeaf` (one member), `ZipPath` (whichever is there) |
| mount | `zip::mount(holder)` / `Holder::zip(holder)`; a `.zip` leaf stays a leaf until mounted; `ZipArchive::new(h).with_restart_stride(n).try_with_codec(c)?.with_level(l).mount()` |
| address | the archive URL plus the member path as fragment: `file:///lake/day.zip#trades/eu%20ndx.csv`; `zip::from_url` reopens it; no fragment is the archive itself |
| nesting | `day.zip#inner.zip//trades/eu.csv`; a `.zip` member is stored, so its members stay one positional read away |
| codings | `Codec::Identity`, `Codec::Deflate`, `Codec::Zstd` only; gzip and zlib refused by name; a `.gz` member is stored, not recoded; precedence: the member's own coding, then `Identity` for an already coded name, then the archive default |
| random access | members this crate compresses carry a restart map (extra field `0x5967`, default stride 64 KiB, at most 2048 points): a read decodes one stride, not the prefix; `with_restart_stride(0)` writes solid members |
| foreign members | no map: decoded from the first byte - `open()` a member for many reads |
| writing | one streaming writer (a member costs one window); a positional write republishes the member on `flush`; `archive.flush()` writes the central directory once; removal compacts on the next flush |
| refused | encrypted members, paths climbing above the root |
| integrity | a whole read verifies the CRC-32 |
| counting | `ZipArchive::handle_reads()` / `handle_writes()` |

| Operation | Handle reads | Handle writes |
| --- | --- | --- |
| mount and parse the directory | 2 (3 past the 64 KiB tail) | 0 |
| listing, glob, `size`, `partitions`, member metadata, restart map | 0 | 0 |
| positional or whole read of a stored member | 1 | 0 |
| positional read of a compressed member | 1 per encoded window decoded | 0 |
| first seek into a mapped member / first read of a foreign member | +1 | 0 |
| write one member within one window | 0 | 1 |
| write a longer member | 0 | 1 per window + 1 settle |
| publish | 0 | 2, or 3 when the archive shrank |
