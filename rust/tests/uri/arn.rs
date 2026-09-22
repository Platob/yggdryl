//! `rust/src/uri/arn.rs`: Amazon Resource Names, the fields AWS decides, and
//! the location an Amazon S3 name addresses.

mod vocabulary {

    use yggdryl::{Arn, Uri, Urn};

    /// Every shape AWS writes an ARN in reads back as its five fields, with the
    /// resource left exactly as the service spelled it.
    #[test]
    fn every_arn_shape_reads_back_as_its_five_fields() {
        let cases = [
            // partition, service, region, account, resource
            (
                "arn:aws:s3:::trades/2026/part.parquet",
                ("aws", "s3", None, None, "trades/2026/part.parquet"),
            ),
            ("arn:aws:s3:::trades", ("aws", "s3", None, None, "trades")),
            (
                "arn:aws:iam::123456789012:user/David",
                ("aws", "iam", None, Some("123456789012"), "user/David"),
            ),
            (
                "arn:aws:iam::aws:policy/AdministratorAccess",
                (
                    "aws",
                    "iam",
                    None,
                    Some("aws"),
                    "policy/AdministratorAccess",
                ),
            ),
            (
                "arn:aws:lambda:us-east-1:123456789012:function:my-fn:1",
                (
                    "aws",
                    "lambda",
                    Some("us-east-1"),
                    Some("123456789012"),
                    "function:my-fn:1",
                ),
            ),
            (
                "arn:aws-cn:ec2:cn-north-1:123456789012:instance/i-1234567890abcdef0",
                (
                    "aws-cn",
                    "ec2",
                    Some("cn-north-1"),
                    Some("123456789012"),
                    "instance/i-1234567890abcdef0",
                ),
            ),
            (
                "arn:aws-us-gov:glue:us-gov-west-1:123456789012:table/lake/trades",
                (
                    "aws-us-gov",
                    "glue",
                    Some("us-gov-west-1"),
                    Some("123456789012"),
                    "table/lake/trades",
                ),
            ),
            // A policy matches a set of resources with wildcards in any field.
            (
                "arn:aws:s3:*:*:trades/*",
                ("aws", "s3", Some("*"), Some("*"), "trades/*"),
            ),
        ];

        for (source, (partition, service, region, account, resource)) in cases {
            let arn = Arn::from_str(source).unwrap();

            assert_eq!(arn.to_string(), source, "{source}");
            assert_eq!(arn.partition(), partition, "{source}");
            assert_eq!(arn.service(), service, "{source}");
            assert_eq!(arn.region(), region, "{source}");
            assert_eq!(arn.account(), account, "{source}");
            assert_eq!(arn.resource(), resource, "{source}");
            assert_eq!(arn.scheme().as_str(), "arn", "{source}");
            assert_eq!(arn.authority().as_str(), "", "{source}");

            // The five fields are the whole of the value: writing them back
            // spells the same ARN.
            assert_eq!(
                Arn::from_parts(
                    partition,
                    service,
                    region.unwrap_or(""),
                    account.unwrap_or(""),
                    resource
                )
                .unwrap(),
                arn,
                "{source}"
            );
        }
    }

    /// The resource splits at its first `/` or `:`, and the separator the
    /// service chose is reported rather than normalized away.
    #[test]
    fn the_resource_splits_at_the_separator_its_service_wrote() {
        let slashed = Arn::from_str("arn:aws:iam::123456789012:user/David").unwrap();
        assert_eq!(slashed.resource_type(), Some("user"));
        assert_eq!(slashed.resource_id(), "David");
        assert_eq!(slashed.resource_separator(), Some('/'));

        let coloned = Arn::from_str("arn:aws:lambda:us-east-1:1:function:my-fn:1").unwrap();
        assert_eq!(coloned.resource_type(), Some("function"));
        assert_eq!(coloned.resource_id(), "my-fn:1");
        assert_eq!(coloned.resource_separator(), Some(':'));

        let bare = Arn::from_str("arn:aws:s3:::trades").unwrap();
        assert_eq!(bare.resource_type(), None);
        assert_eq!(bare.resource_id(), "trades");
        assert_eq!(bare.resource_separator(), None);
    }

    /// The three fields AWS decides fold to lower case, so two spellings of one
    /// name are one value; the account and the resource stay as written.
    #[test]
    fn the_fields_aws_decides_fold_while_the_resource_keeps_its_case() {
        let arn = Arn::from_str("ARN:AWS:S3:US-EAST-1:123456789012:Trades/Part.PARQUET").unwrap();

        assert_eq!(
            arn.to_string(),
            "arn:aws:s3:us-east-1:123456789012:Trades/Part.PARQUET"
        );
        assert_eq!(
            arn,
            Arn::from_str("arn:aws:s3:us-east-1:123456789012:Trades/Part.PARQUET").unwrap()
        );
        assert_ne!(
            arn,
            Arn::from_str("arn:aws:s3:us-east-1:123456789012:trades/part.parquet").unwrap()
        );

        // An account alias is the owner's text, not AWS's, so its case stays.
        let owned = Arn::from_str("arn:aws:iam::AWS:policy/ReadOnly").unwrap();
        assert_eq!(owned.account(), Some("AWS"));
    }

    /// An ARN is one canonical URI, so conversion either way costs nothing and
    /// every value protocol reads the canonical text.
    #[test]
    fn an_arn_is_one_canonical_uri_in_both_directions() {
        let arn = Arn::from_str("arn:aws:s3:::trades/2026/part.parquet").unwrap();
        let uri = Uri::from(&arn);

        assert_eq!(uri.to_string(), arn.to_string());
        assert_eq!(Arn::try_from(uri.clone()).unwrap(), arn);
        assert_eq!(uri.clone().into_arn().unwrap(), arn);
        assert_eq!(arn.clone().into_uri(), uri);
        assert_eq!(
            Arn::from_json(&arn.clone().into_json().unwrap()).unwrap(),
            arn
        );
        assert_eq!(
            arn.stable_hash(),
            Arn::from_str(&arn.to_string()).unwrap().stable_hash()
        );

        // Ordering is the canonical text's, as it is for every identifier.
        let other = Arn::from_str("arn:aws:s3:::trades/2026/part.avro").unwrap();
        assert!(other < arn);

        // Each narrowed form refuses what it is not.
        assert!(
            Urn::from_str("urn:isbn:9780131103627")
                .unwrap()
                .into_uri()
                .into_arn()
                .is_err()
        );
        assert!(
            Uri::from_str("https://example.test/a")
                .unwrap()
                .into_arn()
                .is_err()
        );
        assert!(uri.into_url().is_err());
    }

    /// The resource reads as a path, so the filename accessors and their
    /// setters work over it exactly as they do over a URN's name.
    #[test]
    fn the_resource_reads_and_edits_as_a_path() {
        let mut arn = Arn::from_str("arn:aws:s3:::trades/2026/part.tar.gz").unwrap();

        assert_eq!(arn.file_name(), Some("part.tar.gz"));
        assert_eq!(arn.stem(), Some("part.tar"));
        assert_eq!(arn.extension(), Some("gz"));
        assert_eq!(arn.extensions().collect::<Vec<_>>(), ["tar", "gz"]);
        assert_eq!(arn.mime_type().to_string(), "application/gzip");
        assert_eq!(
            arn.path_segments().collect::<Vec<_>>(),
            ["aws:s3:::trades", "2026", "part.tar.gz"]
        );

        arn.set_stem("renamed").unwrap();
        assert_eq!(arn.to_string(), "arn:aws:s3:::trades/2026/renamed.gz");
        arn.set_extension("parquet").unwrap();
        assert_eq!(arn.to_string(), "arn:aws:s3:::trades/2026/renamed.parquet");
        assert!(arn.remove_extension());
        assert_eq!(arn.resource(), "trades/2026/renamed");
        arn.set_file_name("part.csv.zst").unwrap();
        assert_eq!(arn.extensions().collect::<Vec<_>>(), ["csv", "zst"]);
        assert!(arn.clear_extensions());
        assert_eq!(arn.to_string(), "arn:aws:s3:::trades/2026/part");

        // A mutation that cannot spell an ARN leaves the value alone.
        let before = arn.clone();
        assert!(arn.set_file_name("").is_err());
        assert_eq!(arn, before);
    }
}

mod refusals {

    use yggdryl::{Arn, Uri};

    /// Each refusal names the field it refused and where it is.
    #[test]
    fn every_malformed_arn_is_refused_by_the_field_that_refused_it() {
        let cases = [
            (
                "arn:aws:s3",
                "partition, service, region, account, and resource",
            ),
            (
                "arn:aws:s3::",
                "partition, service, region, account, and resource",
            ),
            ("arn::s3:::trades", "partition must not be empty"),
            ("arn:aws::::trades", "service must not be empty"),
            ("arn:::::", "partition must not be empty"),
            ("arn:aws:s3:::", "resource must not be empty"),
            ("arn:aws:s3.x:::trades", "wildcards"),
            ("arn:aws:s3:us~east~1::trades", "wildcards"),
            (
                "arn:aws:s 3:::trades",
                "not permitted in this URI component",
            ),
            ("arn://aws/s3", "authority"),
            ("arn:aws:s3:::trades?versionId=1", "query"),
            ("arn:aws:s3:::trades#row-1", "fragment"),
        ];

        for (source, reason) in cases {
            let error = Arn::from_str(source)
                .map(|arn| arn.to_string())
                .expect_err(source);
            assert!(
                error.to_string().contains(reason),
                "{source}: {error} does not name {reason:?}"
            );
        }

        // A URI of another scheme is refused as an ARN, not read as one.
        let error = Uri::from_str("s3://trades/part.parquet")
            .unwrap()
            .into_arn()
            .expect_err("s3 is not an arn");
        assert!(error.to_string().contains("scheme must be `arn`"));
    }
}

mod location {

    use yggdryl::{Arn, Uri, Url};

    /// An Amazon S3 ARN names what an `s3:` URL locates, so it opens; every
    /// other service names something no URL addresses, and says so.
    #[test]
    fn an_amazon_s3_arn_locates_and_every_other_service_refuses() {
        let object = Arn::from_str("arn:aws:s3:::trades/2026/part.parquet").unwrap();
        assert_eq!(object.bucket(), Some("trades"));
        assert_eq!(object.key(), Some("2026/part.parquet"));
        assert_eq!(
            object.locator().unwrap(),
            Url::from_str("s3://trades/2026/part.parquet").unwrap()
        );

        // A bucket alone locates the container, with the empty key below it.
        let bucket = Arn::from_str("arn:aws:s3:::trades").unwrap();
        assert_eq!(bucket.key(), Some(""));
        assert_eq!(bucket.locator().unwrap().to_string(), "s3://trades");

        // A prefix keeps its trailing slash, the way every key does.
        let prefix = Arn::from_str("arn:aws:s3:::trades/2026/").unwrap();
        assert_eq!(prefix.key(), Some("2026/"));
        assert_eq!(prefix.locator().unwrap().to_string(), "s3://trades/2026/");

        // An access point carries a region and an account, so it is not the
        // bucket form and names no bucket to read.
        let access_point =
            Arn::from_str("arn:aws:s3:us-west-2:123456789012:accesspoint/reports").unwrap();
        assert_eq!(access_point.bucket(), None);
        assert_eq!(access_point.key(), None);
        assert!(access_point.locator().is_err());

        // Amazon S3 Tables names a table bucket and a table below it, which is
        // the same pair of positions an `s3tables:` URL locates.
        let table = Arn::from_str("arn:aws:s3tables:us-east-1:123456789012:bucket/lake/table/t-a1")
            .unwrap();
        assert_eq!(table.bucket(), Some("lake"));
        assert_eq!(table.table(), Some("t-a1"));
        // A table is not an object, so it is not a key.
        assert_eq!(table.key(), None);
        assert_eq!(
            table.locator().unwrap(),
            Url::from_str("s3tables://lake/t-a1").unwrap()
        );

        // The container alone locates the table bucket, and names no table.
        let table_bucket =
            Arn::from_str("arn:aws:s3tables:us-east-1:123456789012:bucket/lake").unwrap();
        assert_eq!(table_bucket.bucket(), Some("lake"));
        assert_eq!(table_bucket.table(), None);
        assert_eq!(
            table_bucket.locator().unwrap().to_string(),
            "s3tables://lake"
        );

        // An S3 ARN names no table, and an S3 Tables resource that is not the
        // `bucket/...` form names no container at all.
        assert_eq!(object.table(), None);
        let policy = Arn::from_str("arn:aws:s3tables:us-east-1:123456789012:policy/deny").unwrap();
        assert_eq!(policy.bucket(), None);
        assert!(policy.locator().is_err());

        // The URL an ARN locates reads the same container back, whichever
        // store it names.
        for named in [
            "arn:aws:s3:::market-data/2026/part.parquet",
            "arn:aws:s3tables:us-east-1:123456789012:bucket/lake/table/t-a1",
        ] {
            let arn = Arn::from_str(named).unwrap();
            assert_eq!(arn.locator().unwrap().bucket(), arn.bucket(), "{named}");
        }

        let user = Arn::from_str("arn:aws:iam::123456789012:user/David").unwrap();
        assert!(user.bucket().is_none());
        let error = user.locator().expect_err("an IAM user locates nothing");
        assert!(error.to_string().contains("Amazon S3"));

        // The whole identifier answers the same location its narrowing does.
        assert_eq!(
            Uri::from_str("arn:aws:s3:::trades/2026/part.parquet")
                .unwrap()
                .locator()
                .unwrap(),
            object.locator().unwrap()
        );
    }
}
