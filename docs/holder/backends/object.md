# Object stores

Objects on Amazon S3, Google Cloud Storage, and Azure Blob Storage as three
[`IOBase`](../iobase/bytes.md) handles: `Path` a location, `Folder` a prefix or
container, `File` one object.

## Contract

| | |
| --- | --- |
| Owns | `holder::object::{Path, Folder, File}`, the `IOPath`, `IOFolder`, `IOFile` roles of [Holder](../index.md) |
| Feature | `object`, not default: the signed client and its TLS stack are a cost a local consumer never pays |
| Stores | Amazon S3 and every store answering its API, Google Cloud Storage, Azure Blob Storage and Data Lake Storage Gen2 |
| Bindings | An `s3:`, `s3a:`, `s3n:`, `gs:`, `gcs:`, `az:`, `abfs:`, `abfss:`, `wasb:`, or `wasbs:` [`Url`](../../uri/index.md) selects this backend; Rust and Python also take the knobs |
| Protocol | Each store's REST API directly - SigV4, OAuth 2.0 bearer tokens, Azure Shared Key - over synchronous HTTP/1.1. No SDK, no async runtime, no object-store layer |
| Dispatch | `Provider` is the one value that says which store answers, and the only thing the transport, the retry, the staging model and the three roles ever branch on |
| Lazy | Constructing touches nothing; credentials, tokens, region, and endpoint resolve on the first request |
| Addressing | Virtual-hosted on AWS, path style elsewhere or when the bucket holds a dot; Google's JSON API and Azure always path style; `with_path_style` overrides |
| Absence | A missing object reads empty and sizes zero; a delete of nothing succeeds; a refusal is `Error::Remote` |
| Credentials | One pair serves all three: an access key on S3, an HMAC key on Google, an account name and shared key on Azure - a pair the caller hands over, never one swept out of the environment under another store's name |
| Identity | S3 walks the AWS chain or trades it for an STS role; Google walks Application Default Credentials; Azure signs, carries a SAS, or holds an Entra ID token |
| Secrets | A `user:password` written into a location signs the request and never appears in the URL a handle reports; user information without a password is a name, not a key - it is where Azure's Hadoop spellings write the container - and it stays |
| Pooling | One connection serves many requests; a ranged read drains its body so it stays reusable |
| Retry | Full jitter over a doubling window, a token budget so a failing store is not hammered, and `Retry-After` where the store sends one |
| Recovery | A stream cut part way through resumes from the byte it stopped at, not from the beginning |
| Encryption | One value, each store's own shapes: `SSE-S3`/`SSE-KMS`/`DSSE`/`SSE-C`, CMEK and customer-supplied keys, encryption scopes and customer-provided keys |
| Configuration | `ObjectOptions::with_properties` reads PyIceberg's `s3.*`, `gcs.*` and `adls.*` names, PyArrow's arguments, and each store's environment |
| Environment | The whole environment is swept under `AWS_`, `GOOGLE_`, `AZURE_` and `YGGDRYL_`, or whatever prefixes are set, through the same names |

## What each operation costs

This is the contract the backend exists for, so it is stated as a number and
asserted by tests rather than intended. `Folder::stats`, `File::stats`, and
`Path::stats` report what actually went out.

| operation | Amazon S3 | Google Cloud Storage | Azure Blob Storage |
| --- | --- | --- | --- |
| building any handle, resolving a child, reading a media type or a partition | none | none | none |
| resolving a `lake/` location | none - the slash already said it is a container | none | none |
| resolving any other location | one listing of a single key, or two | the same | the same |
| a ranged read | one ranged `GET` | one ranged `GET` with `alt=media` | one `GET` with `x-ms-range` |
| a whole read, a full stream drain, or a digest of any size | one `GET` | one `GET` | one `GET` |
| the size of a listed object | none - the listing already stated it | none | none |
| `size` on a closed handle | one `HEAD`; none while open | one `objects.get`; none while open | one `HEAD`; none while open |
| a whole write | one `PUT` | one `multipart/related` `POST` | one `PUT` |
| a large write | `parts + 2` | `chunks + 1` | `blocks + 1` |
| an append | one `GET` and one write; none of the `GET` while open | the same | the same |
| a removal | one `DELETE`, issued without a probe | one `objects.delete` | one `DELETE` |
| a listing, one level or a whole subtree | one request per 1000 entries | the same | the same |
| emptying or removing a prefix | one listing and one bulk delete per 1000 keys | per 100 | per 256 |

Three of those are worth spelling out.

**Resolving a location** is one listing bounded to a single key: a key is the
smallest string starting with itself, so an object there is the first answer,
and a key under it coming back instead says the location is a prefix. The
second listing settles the case neither answer covers - `.` is `0x2E` and `/`
is `0x2F`, so `lake/part.parquet` sorts *between* `lake/part` and the keys
under `lake/part/`, and a sibling spelled that way hides the prefix from the
first answer.

A **recursive listing** is one flat listing, not one request per directory: all
three answer keys in byte order, which already is depth-first pre-order once
the containers a key implies are emitted before it, so a lake of ten thousand
parts across a thousand partitions costs ten requests rather than a thousand. A
key spelled with a trailing slash is a directory marker, and it lists once, as
the container it names.

A **ranged read learns the object's length** from the `Content-Range` it comes
back with, so an open scope that reads and then asks the size pays nothing for
the answer. A closed handle asks again, because a length is only true of the
moment the store stated it.

## Which store answers

`Provider` is the sole dispatcher. A location's scheme picks it, and every
place the three stores differ - a hostname, a header prefix, a request's path,
an upload protocol, a limit - reads that one value and nothing else.

| store | schemes | authorized by | large write |
| --- | --- | --- | --- |
| `Provider::Aws` | `s3`, `s3a`, `s3n` | Signature Version 4 | a multipart upload of numbered parts |
| `Provider::Google` | `gs`, `gcs` | an OAuth 2.0 bearer token | a resumable session of ranged chunks |
| `Provider::Azure` | `az`, `abfs`, `abfss`, `wasb`, `wasbs` | Shared Key, a SAS, or a bearer token | staged blocks committed as a list |

What each dialect owns is only what its store spells for itself. The transport,
the retry budget, the streaming reader that resumes a cut transfer, the staging
model behind a write, the listing pipeline, and the three handle roles are
written once and serve all three.

## Use

Every binding reaches the same handles through the location's own scheme.
Rust and Python also take the knobs - Python as an `options` mapping on the
three role classes, in whichever vocabulary the caller already has.

=== "Rust"

    ```{ .rust .no_run }
    use yggdryl::IOBase;
    use yggdryl::holder::object;

    // Credentials, tokens, region, and endpoint resolve the way each store's
    // own tools resolve them, and nothing is read until the first request.
    let mut part = object::file("s3://trades/lake/year=2026/part.parquet")?;

    // A footer read transfers the footer, not the file.
    let footer = part.read_range_bytes(part.size().saturating_sub(8), 8)?;
    assert_eq!(footer.len(), 8);

    // The same call against the other two stores.
    let google = object::file("gs://trades/lake/year=2026/part.parquet")?;
    let azure = object::file("abfss://lake@trades.dfs.core.windows.net/part.parquet")?;
    assert_eq!(google.key(), "lake/year=2026/part.parquet");
    assert_eq!(azure.key(), "part.parquet");
    ```

=== "Python"

    ```{ .python .ignore }
    from yggdryl import IOBase, ObjectFile

    # The same class, the same methods: the scheme picks the backend.
    part = IOBase("s3://trades/lake/year=2026/part.parquet")
    footer = part.read_range_bytes(part.size - 8, 8)
    blob = IOBase("gs://trades/lake/year=2026/part.parquet")

    # Or the role by name, with the catalog's properties handed over whole.
    part = ObjectFile(
        "trades",
        "lake/year=2026/part.parquet",
        provider="gs",
        options=catalog.properties,
    )
    ```

=== "JavaScript"

    ```{ .javascript .ignore }
    const { IOBase } = require('yggdryl')

    const part = new IOBase('s3://trades/lake/year=2026/part.parquet')
    const footer = part.readRangeBytes(part.size - 8, 8)
    const blob = new IOBase('abfss://lake@trades.dfs.core.windows.net/part.parquet')
    ```

## Naming an object

A location and a name are not the same thing. `s3://trades/a%20b/c.txt` is a
URL; `a b/c.txt` is the key the store uses. `file` and `folder` take the first,
`file_at` and `folder_at` take the second, and encoding belongs to the latter.

A raw name also does not say which store holds it, where a location's scheme
does, so `file_at` takes the store as its first argument.

```rust
use yggdryl::holder::object::{self, Provider};

let handle = object::file_at(Provider::Aws, "trades", "lake/a b/part.parquet")?;
assert_eq!(handle.key(), "lake/a b/part.parquet");
assert_eq!(handle.url().to_string(), "s3://trades/lake/a%20b/part.parquet");

// A prefix always ends in the delimiter, whether or not the caller wrote one.
let lake = object::folder_at(Provider::Google, "trades", "lake")?;
assert_eq!(lake.prefix(), "lake/");
assert_eq!(lake.url().to_string(), "gs://trades/lake");
```

## Every spelling of the scheme

Ten spellings, three stores. `s3`, `s3a`, and `s3n` name Amazon S3; `gs` and
`gcs` name Google Cloud Storage; `az`, `abfs`, `abfss`, `wasb`, and `wasbs`
name Azure Blob Storage. The extra names differ only in the connector that once
read them, so every one selects this backend and addresses the same object. A
handle reports the spelling it was handed, so a location written by another
tool survives the round trip through a child, a listing, or a log.

Azure has two location shapes, and both are read: `az://container/blob` leaves
the account to configuration - name it with `AzureOptions::with_account`, an
`adls.account-name` property, or `AZURE_STORAGE_ACCOUNT_NAME`, and a location
with none is refused rather than sent somewhere - and the Hadoop form attaches
the container to the account's own host, which says both at once.

```rust
use yggdryl::holder::object;
use yggdryl::IOBase;

let hadoop = object::file("s3a://trades/lake/part.parquet")?;
assert_eq!(hadoop.bucket(), "trades");
assert_eq!(hadoop.key(), "lake/part.parquet");
assert_eq!(hadoop.url().to_string(), "s3a://trades/lake/part.parquet");

// A child keeps the spelling its parent was named with.
let child = object::folder("s3n://trades/lake/")?.child_by_path("part.parquet")?;
assert_eq!(
    child.url().map(ToString::to_string),
    Some("s3n://trades/lake/part.parquet".to_owned())
);

// Google's two names, and Azure's container attached to its account host.
let google = object::file("gcs://trades/lake/part.parquet")?;
assert_eq!(google.bucket(), "trades");

let azure = object::file("abfss://lake@trades.dfs.core.windows.net/part.parquet")?;
assert_eq!(azure.bucket(), "lake");
assert_eq!(azure.key(), "part.parquet");
```

## Reaching a store that is not the published one

MinIO, Ceph, other S3-compatible gateways, `fake-gcs-server`, and Azurite are
named by their endpoint, either in the location or through `ObjectOptions`. A
port, an IP literal, or `localhost` in the URL names an endpoint rather than a
container, so a local store reads the way the published one does.

```rust
use yggdryl::holder::object::{Credentials, ObjectOptions};

let options = ObjectOptions::default()
    .with_endpoint("http://localhost:9000")
    .with_credentials(Credentials::new("minioadmin", "minioadmin"))
    .with_region("us-east-1");
assert_eq!(options.endpoint(), Some("http://localhost:9000"));

// The options record what the caller asked for; the store that answers is what
// clamps it, because the three floors differ - 5 MiB on S3, a multiple of
// 256 KiB on Google, a block on Azure.
assert_eq!(ObjectOptions::default().with_part_size(1).part_size(), 1);
```

Where each unset knob is found:

| knob | URL | environment | file | default |
| --- | --- | --- | --- | --- |
| endpoint | hostname, with its port | `AWS_ENDPOINT_URL_S3`, `STORAGE_EMULATOR_HOST`, `AZURE_STORAGE_BLOB_ENDPOINT` | `~/.aws/config`, a connection string | the store's published host |
| region | a recognized AWS or Google hostname | `AWS_REGION`, `AWS_DEFAULT_REGION` | `~/.aws/config` | `us-east-1` |
| credentials | `s3://key:secret@bucket/` | `AWS_ACCESS_KEY_ID`, `AZURE_STORAGE_ACCOUNT_KEY` | `~/.aws/credentials` | the instance's own identity, then anonymous |
| Google identity | | `GOOGLE_APPLICATION_CREDENTIALS` | the file `gcloud` wrote | the metadata server |
| Azure identity | | `AZURE_TENANT_ID`, `AZURE_CLIENT_ID`, `AZURE_CLIENT_SECRET` | a connection string | the instance's own identity |
| profile | | `AWS_PROFILE` | | `default` |
| path style | | `AWS_S3_FORCE_PATH_STYLE` | | virtual-hosted on AWS, path style elsewhere |

`with_environment(false)` shuts all of that off, so only explicit values and the
URL decide - which is what a test wants and what a sandboxed process may need.

The environment is *swept* rather than looked up by name, so every knob the
property map accepts is also an environment variable, under any prefix the
caller names:

```rust
use yggdryl::holder::object::ObjectOptions;

// The defaults: one prefix per store, plus this crate's own.
assert_eq!(
    ObjectOptions::default().environment_prefixes(),
    [
        "AWS_".to_owned(),
        "GOOGLE_".to_owned(),
        "AZURE_".to_owned(),
        "YGGDRYL_".to_owned()
    ]
);

// A deployment that spells its configuration its own way gets every knob -
// `TRADING_ENDPOINT`, `TRADING_SSE_TYPE`, `TRADING_ROLE_ARN` - rather than the
// handful someone remembered to wire up.
let options = ObjectOptions::default().with_environment_prefix("TRADING_");
assert_eq!(options.environment_prefixes().len(), 5);

// Explicit wins, which is what makes the order above a fact rather than a
// special case per knob: a knob left at its default takes the ambient answer,
// one the caller set keeps theirs.
let ambient = ObjectOptions::from_properties([("region", "us-east-1"), ("max_attempts", "9")])?;
let explicit = ObjectOptions::default().with_region("eu-west-1").under(&ambient);
assert_eq!(explicit.region(), Some("eu-west-1"));
assert_eq!(explicit.max_attempts(), 9);
```

## When the store is having a bad day

A throttle, a 5xx, and a connection that never established are retried; a
refusal is not, because it is the store's verdict rather than a hiccup. Three
things make that safe at scale.

**The wait is drawn, not computed.** Doubling alone puts every client that
failed at the same instant back on the wire at the same instant, which is the
herd the backoff exists to prevent. Each wait is a uniform draw from zero to
the doubled window, from a per-client sequence, so a hundred clients that
failed together return spread out.

**The retries are budgeted.** Each costs tokens from a bucket that starts at
500 and is refunded by success. A healthy client always has budget; one whose
requests are all failing runs out and fails fast, so a partial outage does not
become a self-inflicted one. What is left is reported as
`StatsSnapshot::retry_tokens`, and a falling number is the sign of a store in
trouble rather than of a slow one.

**A `Retry-After` is honoured** where the store sends one, up to thirty
seconds; past that it reads as "not now" rather than as an instruction, because
waiting minutes inside a call nobody can cancel is worse than failing.

Separately from all of that, **a stream that dies part way through resumes**.
The retry above covers the exchange up to the status; once the body is the
caller's, a connection cut half way through a gigabyte would fail the whole
transfer and everything already delivered would be read again. Instead the
client asks for the rest, from the byte it stopped at, and the caller sees one
uninterrupted stream. Only a transport failure resumes, and only while
consecutive failures stay under the attempt limit - a byte arriving resets
that, so a transfer that keeps moving survives any number of interruptions
while one that cannot deliver a byte stops rather than looping.

## Reading somebody else's configuration

A caller rarely starts from this crate's spellings. They start from a PyIceberg
catalog's properties, from the arguments PyArrow's filesystems take, or from
each store's own environment variable names - a vocabulary per store per tool,
for one set of knobs. `with_properties` reads all of them, matching names
loosely: case, `-`, `_`, and `.` are the same, and a leading `s3.`, `gcs.`,
`gs.`, `google.`, `gcp.`, `azure.`, `adls.`, `abfs.`, `client.`, or `aws_` is
dropped, so `s3.access-key-id`, `AWS_ACCESS_KEY_ID`, and `access_key` are one
knob.

A name two stores both have - `storage_class`, `client_id` - is applied to
*both*, because only the store that answers ever reads its own options, so
nothing is ambiguous by the time it matters.

```rust
use yggdryl::holder::object::ObjectOptions;

// Hand it the catalog's properties whole; what is not about a store is
// ignored, because most of a catalog's properties are not.
let options = ObjectOptions::from_properties([
    ("warehouse", "s3://trades/lake"),
    ("token", "a catalog bearer token, which is not a session token"),
    ("s3.endpoint", "http://localhost:9000"),
    ("s3.access-key-id", "minioadmin"),
    ("s3.secret-access-key", "minioadmin"),
    ("s3.force-virtual-addressing", "false"),
    ("s3.request-timeout", "30"),
])?;
assert_eq!(options.endpoint(), Some("http://localhost:9000"));
assert_eq!(options.path_style(), Some(true));

// A value that will not parse is heard here rather than at the store.
assert!(ObjectOptions::from_properties([("s3.request-timeout", "soon")]).is_err());
// And a knob this client cannot honor is refused rather than dropped, so
// nothing a caller asked for silently does not happen.
assert!(ObjectOptions::from_properties([("s3.signer.uri", "https://signer")]).is_err());

// One catalog can hold all three stores' properties at once, because a
// warehouse on one cloud and a backup on another is an ordinary thing to
// configure. Each name reaches its own store's options and no other's.
let options = ObjectOptions::from_properties([
    ("gcs.project-id", "trading-analytics"),
    ("gcs.oauth2.token", "ya29.a0AfH6"),
    ("adls.account-name", "trades"),
    ("adls.account-key", "a2V5"),
])?;
assert_eq!(options.google().project(), Some("trading-analytics"));
assert_eq!(options.azure().account(), Some("trades"));
```

What all three stores have:

| knob | this crate | PyIceberg | PyArrow |
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

What one store has:

| store | knob | names |
| --- | --- | --- |
| Amazon S3 | profile | `profile`, `AWS_PROFILE` |
| | role | `role_arn`, `role_session_name`, `external_id`, `sts_endpoint` |
| | payload signing | `payload_signing`, `sign_payload` |
| | storage class | `storage_class` |
| | requester pays | `requester_pays`, `request_payer` |
| | checksum | `checksum`, `checksum_algorithm` (`CRC32` or `SHA256`) |
| Google | project | `project`, `project_id`, `gcs.project-id`, `GOOGLE_CLOUD_PROJECT` |
| | billing | `user_project`, `quota_project_id` |
| | identity | `credentials_file`, `service_account_file`, `GOOGLE_APPLICATION_CREDENTIALS`, `credentials_json`, `access_token`, `impersonate_service_account` |
| | storage class, ACL | `storage_class`, `predefined_acl` |
| Azure | account | `account_name`, `account_key`, `AZURE_STORAGE_ACCOUNT_NAME`, `connection_string` |
| | signature | `sas_token`, `shared_access_signature` |
| | identity | `tenant_id`, `client_id`, `client_secret`, `federated_token_file`, `managed_identity` |
| | blob | `blob_type` (`block`, `append`, `page`), `access_tier`, `encryption_scope` |
| | endpoint | `api_version`, `data_lake`, `authority_host` |

Sizes may carry the unit a configuration file writes them with - `8MiB`,
`32 MB` - and durations are seconds, whole or fractional.

## Reaching a store as somebody else

All three stores have the same shape for it, and each spells it its own way, so
each is a knob on that store's own options.

On AWS, a process that reaches a bucket in another account does not hold keys
for it: it holds keys allowed to *assume a role* that does. One STS request
trades the first for the second, and the session it hands back is what signs the
bucket - refreshed shortly before it lapses rather than asked for per request.

```rust
use std::time::Duration;

use yggdryl::holder::object::{AssumedRole, AwsOptions, GoogleOptions, ObjectOptions};

let role = AssumedRole::new("arn:aws:iam::123456789012:role/lake-reader")
    .with_session_name("power-desk")
    .with_external_id("desk-42")
    .with_duration(Duration::from_secs(3600));
let options =
    ObjectOptions::default().with_aws(AwsOptions::default().with_assumed_role(role));
assert_eq!(
    options.aws().assumed_role().map(AssumedRole::session_name),
    Some("power-desk")
);

// Google's shape: whatever the credential chain answers signs one call to
// `iamcredentials`, and the token that call returns is what reaches the store.
let options = ObjectOptions::default().with_google(
    GoogleOptions::default().with_impersonation("lake-reader@trading.iam.gserviceaccount.com"),
);
assert_eq!(
    options.google().impersonation(),
    Some("lake-reader@trading.iam.gserviceaccount.com")
);
```

The credential chain still answers, and what it answers is what signs the
exchange. A session is asked for once and reused until it nears its expiry, so
a thousand reads under a role cost one exchange, not a thousand. Azure's
equivalent is an Entra ID application or the instance's own identity, named on
`AzureOptions`.

## What a client is not allowed to do

Two lifecycle operations are worth being able to forbid, because a process that
reads a lake has no business creating or destroying a container. Both refuse
without a request: it is this client's own rule, and finding out from the store
would be a round trip and an audit-log entry.

```rust
use yggdryl::IOBase;
use yggdryl::holder::object::ObjectOptions;

let options = ObjectOptions::default()
    .with_container_creation(false)
    .with_container_deletion(false);
assert!(!options.container_creation());

// And every write can carry metadata: a name the store defines goes over as
// that header, anything else as user metadata under the store's own prefix -
// `x-amz-meta-`, `x-goog-meta-`, `x-ms-meta-` - and a name the write sets for
// itself is never displaced.
let options = options.with_default_metadata([("desk", "power"), ("cache-control", "no-store")]);
assert_eq!(
    options.default_metadata()[0],
    ("desk".to_owned(), "power".to_owned())
);
```

## Encryption at rest

All three stores encrypt at rest, and the interesting question in each is the
same one: *who holds the key* - which is the same thing as asking what a request
has to say. So the choice is one value across the three, and each store spells
it its own way or says it does not have that shape. It is a client knob rather
than a per-write argument, because a key the store never keeps has to be
presented again on every read.

| | what a write says | what a read carries |
| --- | --- | --- |
| `Encryption::Default` | nothing; the container's own rule applies | nothing |
| `Encryption::Managed` | the store's own keys | nothing |
| `Encryption::Kms` | which managed key | nothing |
| `Encryption::Customer` | the key itself | the key itself |
| `Encryption::Scope` | which account scope | nothing |

| | Amazon S3 | Google Cloud Storage | Azure Blob Storage |
| --- | --- | --- | --- |
| `Managed` | `SSE-S3` | already the default | already the default |
| `Kms` | `SSE-KMS`, `aws:kms:dsse` | a CMEK `kmsKeyName` | not this shape; use a scope |
| `Customer` | `SSE-C`, with the key's MD5 | `x-goog-encryption-key`, with its SHA-256 | `x-ms-encryption-key`, with its SHA-256 |
| `Scope` | not this shape | not this shape | `x-ms-encryption-scope` |

A combination a store does not have is refused once, when the client is built,
rather than silently dropped or discovered from the store on the first write.

```rust
use yggdryl::holder::object::{CustomerKey, Encryption, KmsKey, ObjectOptions, Provider};

// The bucket's own default, which is what an unset value means.
assert!(ObjectOptions::default().encryption().is_default());

// A named KMS key, an encryption context KMS records and requires again, and
// an S3 Bucket Key - one KMS call per bucket and window rather than per
// object, which is what makes writing a lake of small parts affordable.
let key = KmsKey::new("arn:aws:kms:eu-west-1:123456789012:key/abcd")
    .with_context(r#"{"desk":"power"}"#)
    .with_bucket_key(true);
let options = ObjectOptions::default().with_encryption(Encryption::Kms(key));
assert_eq!(
    options.encryption().write_headers(Provider::Aws)[0],
    ("x-amz-server-side-encryption", "aws:kms".to_owned())
);

// A key the caller holds. It is checked here rather than by the store: 32
// bytes, and the MD5 the AWS tools carry beside it has to be the key's.
let held = CustomerKey::from_base64("AwoRGB8mLTQ7QklQV15lbHN6gYiPlp2kq7K5wMfO1dw=")?;
assert_eq!(held.checksum(), "N+iD0sgzzGlyEXJzCNck5w==");
// The same key, three spellings, and two different digests of it: S3 asks for
// the MD5 and the other two for the SHA-256.
let encryption = Encryption::Customer(held);
assert_eq!(encryption.read_headers(Provider::Aws).len(), 3);
assert_eq!(
    encryption.read_headers(Provider::Google)[0],
    ("x-goog-encryption-algorithm", "AES256".to_owned())
);
assert_eq!(
    encryption.read_headers(Provider::Azure)[0],
    ("x-ms-encryption-algorithm", "AES256".to_owned())
);

// Nothing renders it.
let key = CustomerKey::new(&[7; 32])?;
assert!(format!("{key:?}").contains("<redacted>"));
```

The request that creates an object says how it is stored; every request that
touches its bytes carries a customer key and nothing else, because a part
inherits how its upload was created and the store remembers which of its *own*
keys it used. On Google the managed key is a query parameter rather than a
header, which is the one place this is not a header at all.

## Everything else is inherited

The three roles are the whole backend. Globs, Hive partitions, page caching,
content codings, IPC, Parquet, Avro, and Iceberg tables are not reimplemented
here: they are the [`IOBase`](../iobase/bytes.md) surfaces, and a record reader
over an object is the same reader that runs over a mapped file.

```{ .rust .no_run }
use yggdryl::IOBase;
use yggdryl::holder::buffered::BufferedOptions;
use yggdryl::holder::object;

let mut part = object::file("s3://trades/lake/part.parquet")?;
// Opening caches the object's metadata - never its bytes - so the questions a
// reader asks along the way stop being round trips. It is what to do before
// wrapping a remote handle in a page cache.
part.open()?;
let cached = part.buffered(BufferedOptions::default());
let _ = cached.read_range_bytes(0, 16)?;

// One partition of a lake, selected by path alone, with no call per file.
for leaf in object::folder("s3://trades/lake/")?.children_where(&[("year", "2026")], false)? {
    let _ = leaf?;
}
```

## Failures

Absence and conflict stay the two typed variants every caller already branches
on, so nothing matches on a message. Everything else the store says arrives as
`Error::Remote` with its own code and text intact.

```rust
use yggdryl::Error;

let refused = Error::remote(
    "s3", "GetObject", 403, "AccessDenied", "Access Denied", "s3://trades/lake/part.parquet",
);
assert!(!refused.is_absent());
assert!(!refused.is_conflict());
```

A throttle or a server-side error is retried with backoff; a refusal is not,
because it is the store's verdict rather than a hiccup. A bucket in another
region answers with the region it is in, and the correction is learned once.

## Measured against `object_store`

Both clients run against one in-process store over a real socket, on the same
payloads and keys, so the difference is the two implementations rather than two
networks. Criterion medians on a containerized x86_64 Linux host (Intel Xeon
@ 2.10 GHz, 4 cores, 16 GiB; rustc 1.94.1, thin LTO), `object_store` 0.13.2:

| operation | yggdryl | `object_store` |
| --- | ---: | ---: |
| read 4 MiB whole | 2.16 ms | 2.37 ms |
| read an 8 KiB footer | 62 us | 74 us |
| drain 4 MiB streamed | 1.93 ms | 2.35 ms |
| write 4 MiB, unsigned payload | 9.13 ms | 9.96 ms |
| write 4 MiB, signed payload | 12.31 ms | - |
| first three entries of 2000 | 2.36 ms | 2.80 ms |
| one level of 2000 | 161 us | 180 us |
| whole subtree of 2000 | 7.15 ms | 5.79 ms |

The subtree row is not like for like: this listing yields the container each key
implies as well, which `object_store` does not report at all, so it hands back
half again as many entries for the same pages. The write rows are the same
request under two payload policies, which is why both are given.

A loopback fixture exaggerates fixed cost - a real round trip is 1-20 ms,
against which 60 us is noise - so what these establish is that no per-request
cost is hiding, not that reads are faster in production. The request *count* is
what decides that, and it is held by the accounting tests above.

Regenerate with:

```console
cargo bench --bench holder --features object -- object_ --noplot
```

## Checked against a real store

A hand-written fake store proves the client agrees with itself, and it was
written from the same reading of the API, so the two can agree and both be
wrong. The fake answers all three dialects over one set of objects, which is
what lets a test write through Google's JSON API and read back through Azure's
REST API - but that only proves the three agree with *each other*.

`scripts/check_object_interop.py` is the other half: it provisions MinIO, has
`boto3` write objects, runs the Rust half against the same store, and then reads
back from the outside every object it wrote - bytes, length, a ranged read, and
the multipart upload, whose assembled content and `-{parts}` ETag are checked by
the reference client rather than by the writer. The exchange keys are the point:
a space, a plus, an equals, a percent, and a non-ASCII name each mean something
different as a URL component, as a signed canonical path, and as a raw key.

```console
python scripts/check_object_interop.py
```

The other two stores have their own drivers, and each is checked by the client
its own vendor ships:

```console
python scripts/check_azure_interop.py
python scripts/check_gcs_interop.py
```

Azurite is the one that matters most, because it is the one that checks the
*signature*: it recomputes the Shared Key `StringToSign` exactly as the service
does and answers `403` for anything that does not match, so the order of the
thirteen signed lines, the empty `Content-Length` for a zero-length body, the
account appearing twice in a path-style canonicalized resource, and the rule
that every sub-request of a batch is authorized on its own are all checked by a
server that never read this crate's source.

`fake-gcs-server` checks the *shape* of every request - the `/storage/v1` and
`/upload/storage/v1` paths, the object name escaped into one path segment,
`alt=media`, the `multipart/related` framing, and the resumable protocol's
`Content-Range` and 308 answers - and accepts any bearer token. So that driver
proves the dialect and not the identity, and it says so rather than implying
more.
