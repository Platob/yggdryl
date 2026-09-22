# ARN

This page owns `Arn`, the AWS spelling of a resource name, and the location an Amazon S3 name addresses.

An ARN is a [`Uri`](index.md) whose scheme is `arn`, so everything on this page is one canonical identifier read a second way — the same relationship [`Url` and `Urn`](url-urn.md) have with it.

## Contract

| Aspect | Rule |
| --- | --- |
| Shape | `arn:partition:service:region:account:resource` — five fields, and the resource carries its own `/` and `:` structure |
| `Arn` requires | the `arn` scheme, no authority, a partition, a service, and a non-empty resource |
| Empty fields | `region` and `account` may be empty, which is how a global service — IAM, Amazon S3, CloudFront — spells "every region" and "no owning account"; both read back as `None` |
| Case | `partition`, `service` and `region` fold to lower case, the way a URN's namespace does; `account` and `resource` are the owner's and the service's text and stay as written |
| Field bytes | `partition`, `service`, `region`, `account`: ASCII letters, digits, hyphens, underscores, and the `*` and `?` a policy writes to match a set of them |
| Resource | whatever the service writes, validated as URI path text; a `?` or `#` inside it must be percent-encoded, because an ARN carries no query and no fragment |
| Resource split | `resource_type` / `resource_id` split at the resource's first `/` or `:`, and `resource_separator` reports which one the service wrote |
| `bucket` | the container, whichever store names one: the bucket on Amazon S3, the table bucket on Amazon S3 Tables |
| `key` | Amazon S3 only, and only the bucket form — no region and no account; `""` for the bucket alone |
| `table` | Amazon S3 Tables only: what `bucket/<table-bucket>/table/<table>` names below its container. `key` is `None` there, because a table is not an object |
| `locator` | an Amazon S3 bucket ARN answers the `s3:` URL it addresses and an Amazon S3 Tables ARN the `s3tables:` one; every other service is refused by name |
| Filenames | [accessors](path.md) read the resource, not the whole path; setters leave the five fields alone |
| Bindings | Rust, Python and JavaScript each answer the fields, the resource and `locator`. Python's `Arn` is a subclass of `Uri`, so `Uri("arn:…")` answers one; JavaScript has no class inheritance here, so `Uri.from("arn:…")` answers a `Uri` and `intoArn()` narrows it |
| Errors | Rust `Err`, Python `ValueError`, JavaScript throw, each naming the field that refused |

## Use

=== "Rust"

    ```rust
    use yggdryl::{Arn, Uri};

    let arn = Arn::from_str("arn:aws:s3:::market-data/2026/part.parquet")?;

    assert_eq!(arn.partition(), "aws");
    assert_eq!(arn.service(), "s3");
    assert_eq!(arn.region(), None);
    assert_eq!(arn.account(), None);
    assert_eq!(arn.resource(), "market-data/2026/part.parquet");

    // The resource reads as a path, so the filename accessors work over it.
    assert_eq!(arn.file_name(), Some("part.parquet"));
    assert_eq!(arn.extension(), Some("parquet"));

    // One canonical value, read two ways.
    assert_eq!(Uri::from(&arn).to_string(), arn.to_string());
    assert_eq!(Uri::from_str(&arn.to_string())?.into_arn()?, arn);

    // The three fields AWS decides fold; the rest stay as written.
    let folded = Arn::from_str("ARN:AWS:S3:US-EAST-1:123456789012:Trades/Part.PARQUET")?;
    assert_eq!(folded.to_string(), "arn:aws:s3:us-east-1:123456789012:Trades/Part.PARQUET");
    assert_eq!(folded.region(), Some("us-east-1"));
    assert_eq!(folded.account(), Some("123456789012"));
    ```

=== "Python"

    ```python
    from yggdryl import Arn, Uri

    arn = Arn("arn:aws:s3:::market-data/2026/part.parquet")

    assert arn.partition == "aws"
    assert arn.service == "s3"
    assert arn.region is None
    assert arn.account is None
    assert arn.resource == "market-data/2026/part.parquet"

    assert arn.file_name == "part.parquet"
    assert arn.extension == "parquet"

    assert str(Uri(arn)) == str(arn)
    assert Uri(str(arn)) == arn

    folded = Arn("ARN:AWS:S3:US-EAST-1:123456789012:Trades/Part.PARQUET")
    assert str(folded) == "arn:aws:s3:us-east-1:123456789012:Trades/Part.PARQUET"
    assert folded.region == "us-east-1"
    assert folded.account == "123456789012"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Arn, Uri } = require('yggdryl')

    const arn = Arn.from('arn:aws:s3:::market-data/2026/part.parquet')

    assert.equal(arn.partition, 'aws')
    assert.equal(arn.service, 's3')
    assert.equal(arn.region, null)
    assert.equal(arn.account, null)
    assert.equal(arn.resource, 'market-data/2026/part.parquet')

    assert.equal(arn.fileName, 'part.parquet')
    assert.equal(arn.extension, 'parquet')

    assert.equal(Uri.from(arn).toString(), arn.toString())

    const folded = Arn.from('ARN:AWS:S3:US-EAST-1:123456789012:Trades/Part.PARQUET')
    assert.equal(folded.toString(), 'arn:aws:s3:us-east-1:123456789012:Trades/Part.PARQUET')
    assert.equal(folded.region, 'us-east-1')
    assert.equal(folded.account, '123456789012')
    ```

## What the service decides

The five fields are AWS's; the sixth is the service's, and how it spells it is the service's choice rather than a canonical form. `resource_type` and `resource_id` therefore report the split at the resource's first `/` or `:` without normalizing either away, and `bucket` and `key` are the door for what that split *means* on Amazon S3.

=== "Rust"

    ```rust
    use yggdryl::Arn;

    // A service that writes `type/id`, and one that writes `type:id`.
    let user = Arn::from_str("arn:aws:iam::123456789012:user/David")?;
    assert_eq!(user.resource_type(), Some("user"));
    assert_eq!(user.resource_id(), "David");
    assert_eq!(user.resource_separator(), Some('/'));

    let function = Arn::from_str("arn:aws:lambda:us-east-1:123456789012:function:trades:1")?;
    assert_eq!(function.resource_type(), Some("function"));
    assert_eq!(function.resource_id(), "trades:1");
    assert_eq!(function.resource_separator(), Some(':'));

    // An AWS-owned resource names `aws` where an account would be.
    let policy = Arn::from_str("arn:aws:iam::aws:policy/AdministratorAccess")?;
    assert_eq!(policy.account(), Some("aws"));

    // A policy matches a set of resources with wildcards in any field.
    let every = Arn::from_str("arn:aws:s3:*:*:market-data/*")?;
    assert_eq!(every.region(), Some("*"));
    assert_eq!(every.resource(), "market-data/*");
    ```

=== "Python"

    ```python
    from yggdryl import Arn

    user = Arn("arn:aws:iam::123456789012:user/David")
    assert user.resource_type == "user"
    assert user.resource_id == "David"
    assert user.resource_separator == "/"

    function = Arn("arn:aws:lambda:us-east-1:123456789012:function:trades:1")
    assert function.resource_type == "function"
    assert function.resource_id == "trades:1"
    assert function.resource_separator == ":"

    policy = Arn("arn:aws:iam::aws:policy/AdministratorAccess")
    assert policy.account == "aws"

    every = Arn("arn:aws:s3:*:*:market-data/*")
    assert every.region == "*"
    assert every.resource == "market-data/*"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Arn } = require('yggdryl')

    const user = Arn.from('arn:aws:iam::123456789012:user/David')
    assert.equal(user.resourceType, 'user')
    assert.equal(user.resourceId, 'David')
    assert.equal(user.resourceSeparator, '/')

    const fn = Arn.from('arn:aws:lambda:us-east-1:123456789012:function:trades:1')
    assert.equal(fn.resourceType, 'function')
    assert.equal(fn.resourceId, 'trades:1')
    assert.equal(fn.resourceSeparator, ':')

    const policy = Arn.from('arn:aws:iam::aws:policy/AdministratorAccess')
    assert.equal(policy.account, 'aws')
    ```

## Where it is

An Amazon S3 ARN names a bucket and, below it, a key, which is exactly what an `s3:` URL locates — so it is the one ARN that answers `locator`, and the one that can be opened. Every other service addresses something no URL locates, and says so by name.

=== "Rust"

    ```rust
    use yggdryl::{Arn, Uri, Url};

    let object = Arn::from_str("arn:aws:s3:::market-data/2026/part.parquet")?;
    assert_eq!(object.bucket(), Some("market-data"));
    // The location door reads the same name written as text.
    assert_eq!(Url::from_location("arn:aws:s3:::market-data/2026/part.parquet")?, object.locator()?);
    assert_eq!(object.key(), Some("2026/part.parquet"));
    assert_eq!(object.locator()?, Url::from_str("s3://market-data/2026/part.parquet")?);

    // The whole identifier answers the same location its narrowing does.
    assert_eq!(
        Uri::from_str("arn:aws:s3:::market-data/2026/part.parquet")?.locator()?,
        object.locator()?
    );

    // The bucket alone locates the container, with the empty key below it.
    let bucket = Arn::from_str("arn:aws:s3:::market-data")?;
    assert_eq!(bucket.key(), Some(""));
    assert_eq!(bucket.locator()?.to_string(), "s3://market-data");

    // An access point carries a region and an account, so it is not that form.
    let access_point = Arn::from_str("arn:aws:s3:us-west-2:123456789012:accesspoint/reports")?;
    assert_eq!(access_point.bucket(), None);
    assert!(access_point.locator().is_err());
    assert!(Arn::from_str("arn:aws:iam::123456789012:user/David")?.locator().is_err());
    ```

=== "Python"

    ```python
    from yggdryl import Arn, Uri, Url

    obj = Arn("arn:aws:s3:::market-data/2026/part.parquet")
    assert Url("arn:aws:s3:::market-data/2026/part.parquet") == obj.locator()
    assert obj.bucket == "market-data"
    assert obj.key == "2026/part.parquet"
    assert obj.locator() == Url("s3://market-data/2026/part.parquet")
    assert Uri("arn:aws:s3:::market-data/2026/part.parquet").locator() == obj.locator()

    bucket = Arn("arn:aws:s3:::market-data")
    assert bucket.key == ""
    assert str(bucket.locator()) == "s3://market-data"

    for named in (
        "arn:aws:s3:us-west-2:123456789012:accesspoint/reports",
        "arn:aws:iam::123456789012:user/David",
    ):
        try:
            Arn(named).locator()
            raise AssertionError("expected a rejection")
        except ValueError:
            pass
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Arn, Uri } = require('yggdryl')

    const object = Arn.from('arn:aws:s3:::market-data/2026/part.parquet')
    assert.equal(object.bucket, 'market-data')
    assert.equal(object.key, '2026/part.parquet')
    assert.equal(object.locator().toString(), 's3://market-data/2026/part.parquet')
    assert.equal(
      Uri.from('arn:aws:s3:::market-data/2026/part.parquet').locator().toString(),
      object.locator().toString(),
    )

    const bucket = Arn.from('arn:aws:s3:::market-data')
    assert.equal(bucket.key, '')
    assert.equal(bucket.locator().toString(), 's3://market-data')

    assert.throws(() => Arn.from('arn:aws:iam::123456789012:user/David').locator())
    ```

## Amazon S3 Tables

A table bucket holds tables rather than objects, and AWS addresses one only by ARN. It is still a container and a name below it — the same two positions every store writes — so `bucket` reads the table bucket, `table` reads the table, and `locator` answers the `s3tables:` URL those two spell. The [store accessors](index.md) read that URL back the way they read an `s3:` one.

No byte backend opens an `s3tables:` location: `is_object_store` stays false for it, so a reader speaks the S3 Tables catalog rather than fetching a key. `python/benchmarks/media/s3tables.py` is that reader end to end: it takes a table bucket ARN, has PyIceberg fill a table through the service's catalog, and reads the same table at the warehouse `s3:` location the catalog answers, timed beside PyIceberg's own scan.

=== "Rust"

    ```rust
    use yggdryl::{Arn, Uri, Url};

    let table = Arn::from_str("arn:aws:s3tables:us-east-1:123456789012:bucket/lake/table/t-a1")?;

    assert_eq!(table.service(), "s3tables");
    assert_eq!(table.bucket(), Some("lake"));
    assert_eq!(table.table(), Some("t-a1"));
    // A table is not an object, so it is not a key.
    assert_eq!(table.key(), None);
    assert_eq!(table.locator()?, Url::from_str("s3tables://lake/t-a1")?);

    // The container alone locates the table bucket, and names no table.
    let bucket = Arn::from_str("arn:aws:s3tables:us-east-1:123456789012:bucket/lake")?;
    assert_eq!(bucket.table(), None);
    assert_eq!(bucket.locator()?.to_string(), "s3tables://lake");

    // The URL reads the same two positions back.
    let located = Uri::from_str("s3tables://lake/t-a1")?;
    assert_eq!(located.bucket(), Some("lake"));
    assert_eq!(located.key(), Some("t-a1"));
    assert!(!located.scheme().is_object_store());
    assert!(located.scheme().has_container());
    ```

=== "Python"

    ```python
    from yggdryl import Arn, Uri, Url

    table = Arn("arn:aws:s3tables:us-east-1:123456789012:bucket/lake/table/t-a1")

    assert table.service == "s3tables"
    assert table.bucket == "lake"
    assert table.table == "t-a1"
    assert table.key is None
    assert table.locator() == Url("s3tables://lake/t-a1")

    bucket = Arn("arn:aws:s3tables:us-east-1:123456789012:bucket/lake")
    assert bucket.table is None
    assert str(bucket.locator()) == "s3tables://lake"

    located = Uri("s3tables://lake/t-a1")
    assert located.bucket == "lake"
    assert located.key == "t-a1"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Arn, Uri } = require('yggdryl')

    const table = Arn.from('arn:aws:s3tables:us-east-1:123456789012:bucket/lake/table/t-a1')

    assert.equal(table.service, 's3tables')
    assert.equal(table.bucket, 'lake')
    assert.equal(table.table, 't-a1')
    assert.equal(table.key, null)
    assert.equal(table.locator().toString(), 's3tables://lake/t-a1')

    const located = Uri.from('s3tables://lake/t-a1')
    assert.equal(located.bucket, 'lake')
    assert.equal(located.key, 't-a1')
    ```

## Edges

- `arn:aws:s3` or `arn:aws:s3::` → refused: an ARN carries five fields, and a missing one is not an empty one.
- `arn::s3:::trades` → the partition refused; `arn:aws::::trades` → the service refused; `arn:aws:s3:::` → the resource refused.
- `arn:aws:s3.x:::trades` → the service refused at the offending byte, because `.` is not a field byte.
- `arn:aws:s3:::trades?versionId=1` → refused: an ARN carries no query, so a `?` the resource holds is percent-encoded.
- `arn:aws:s3:::market-data` → `resource_type` is `None`: a resource with no separator is all identifier.
- `arn:aws:s3:us-west-2:123456789012:accesspoint/reports` → `bucket` and `key` are `None`, because only the region-less, account-less form is the bucket form.
- `arn:aws:s3tables:…:policy/deny` → `bucket` is `None` and `locator` refuses: only a `bucket/…` resource names a table bucket.
- `arn:aws:s3tables:…:bucket/lake` → `table` is `None`: the container alone names no table.
- Opening an `s3tables:` location → refused by its scheme (`filesystem "s3tables" does not support holding a location of this scheme`), because no byte backend speaks S3 Tables.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --test uri -- arn
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_uri.py -k arn
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="ARN" node/tests/uri.test.js
    ```
