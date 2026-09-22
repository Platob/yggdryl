//! `rust/src/s3/aws/sts.rs`: trading a role for a session, and what a
//! lapsed one does next.

mod protocol {
    use yggdryl::IOBase;
    use yggdryl::s3::AwsOptions;

    use crate::mod_::{BUCKET, file_with, options, payload, store};

    /// Whether a recorded request is the STS exchange rather than a bucket one.
    fn is_exchange(request: &crate::server::Recorded) -> bool {
        request
            .query
            .iter()
            .any(|(name, value)| name == "Action" && value == "AssumeRole")
    }

    #[test]
    fn a_named_role_is_traded_for_a_session_once_and_signs_everything_after() {
        let store = store();
        store.put(BUCKET, "lake/part.bin", &payload(64));
        let role = yggdryl::s3::AssumedRole::new("arn:aws:iam::123456789012:role/lake-reader")
            .with_session_name("power-desk")
            .with_external_id("desk-42")
            // STS is a host of its own on AWS; the fixture shares this one.
            .with_endpoint(store.endpoint());
        let handle = file_with(
            "lake/part.bin",
            options(&store).with_aws(AwsOptions::default().with_assumed_role(role)),
        );

        store.clear_requests();
        assert_eq!(handle.read_all_bytes().expect("a read"), payload(64));
        assert_eq!(handle.read_range_bytes(0, 8).expect("a read"), payload(8));

        let recorded = store.requests();
        let exchanges: Vec<_> = recorded
            .iter()
            .filter(|request| is_exchange(request))
            .collect();
        assert_eq!(
            exchanges.len(),
            1,
            "one exchange, however many requests follow it: {:?}",
            recorded.iter().map(|r| &r.path).collect::<Vec<_>>()
        );

        // The exchange says what was asked for, and is signed for STS rather than
        // for S3 - a different service in the scope is a different signing key.
        let exchange = exchanges[0];
        let asked = |name: &str| {
            exchange
                .query
                .iter()
                .find(|(held, _)| held == name)
                .map(|(_, value)| value.as_str())
        };
        assert_eq!(asked("Version"), Some("2011-06-15"));
        assert_eq!(
            asked("RoleArn"),
            Some("arn:aws:iam::123456789012:role/lake-reader")
        );
        assert_eq!(asked("RoleSessionName"), Some("power-desk"));
        assert_eq!(asked("ExternalId"), Some("desk-42"));
        assert_eq!(asked("DurationSeconds"), Some("3600"));
        let authorization = |request: &crate::server::Recorded| {
            request
                .headers
                .iter()
                .find(|(name, _)| name == "authorization")
                .map(|(_, value)| value.clone())
                .unwrap_or_default()
        };
        assert!(
            authorization(exchange).contains("/sts/aws4_request"),
            "{}",
            authorization(exchange)
        );

        // And every bucket request after it is signed as the role's session.
        let bucket_requests: Vec<_> = recorded
            .iter()
            .filter(|request| !is_exchange(request))
            .collect();
        assert_eq!(bucket_requests.len(), 2);
        for request in bucket_requests {
            let signed = authorization(request);
            assert!(signed.contains("Credential=ASIAlake-reader/"), "{signed}");
            assert!(signed.contains("/s3/aws4_request"), "{signed}");
        }
    }

    #[test]
    fn a_lapsed_session_is_traded_again_rather_than_signed_with() {
        let store = store();
        store.put(BUCKET, "lake/part.bin", &payload(64));
        // Every session comes back already expired, which is the refresh path
        // without waiting an hour for it.
        store.expire_roles(true);
        let role = yggdryl::s3::AssumedRole::new("arn:aws:iam::123456789012:role/lake-reader")
            .with_endpoint(store.endpoint());
        let handle = file_with(
            "lake/part.bin",
            options(&store).with_aws(AwsOptions::default().with_assumed_role(role)),
        );

        store.clear_requests();
        assert_eq!(handle.read_all_bytes().expect("a read"), payload(64));
        assert_eq!(handle.read_all_bytes().expect("a read"), payload(64));
        let exchanges = store
            .requests()
            .iter()
            .filter(|request| is_exchange(request))
            .count();
        assert_eq!(
            exchanges, 2,
            "a session past its expiry is not signed with, it is replaced"
        );
    }
}
