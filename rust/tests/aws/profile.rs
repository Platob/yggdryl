//! `rust/src/aws/profile.rs`: the shared files `~/.aws/config` and
//! `~/.aws/credentials`, read the way the AWS tools read them.
//!
//! A machine's own files would exercise none of these edges reliably, and a
//! test may not read them, so every file here is text the test spells: handed
//! to a session through `with_config_text` and `with_credentials_text` for what
//! a caller reaches through `Profile`, and to `Files` and the legacy readers
//! through `yggdryl::internals::aws_profile` for what it cannot.

use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use yggdryl::aws::{CredentialSource, Credentials, Profile, Session, Sso};
use yggdryl::internals::aws_profile::{
    Files, boto_config, ec2_credential_file, expand_path, expand_vars, is_partial_set, service_key,
    split_command, split_words,
};

/// The profile `name` two spelled files hold, read through a session that
/// consults nothing else.
fn read_profile(config: &str, credentials: &str, name: &str) -> Option<Profile> {
    Session::new()
        .with_environment(false)
        .with_config_text(config)
        .with_credentials_text(credentials)
        .with_profile(name)
        .profile()
}

/// The profile `name` the configuration file alone holds.
fn configured(config: &str, name: &str) -> Profile {
    read_profile(config, "", name).unwrap_or_else(|| panic!("the configuration file spells {name}"))
}

/// The set `profile` holds, which must be readable.
fn pair(profile: &Profile) -> Option<Credentials> {
    profile.credentials().expect("a readable set")
}

/// The text of a refusal, which is invalid data naming the profile.
fn refusal<T: std::fmt::Debug>(answer: yggdryl::Result<T>) -> String {
    let error = answer.expect_err("a refusal");
    assert!(
        matches!(&error, yggdryl::Error::Io(io) if io.kind() == std::io::ErrorKind::InvalidData),
        "a profile's refusal is invalid data: {error:?}"
    );
    error.to_string()
}

mod sections {
    use super::*;

    #[test]
    fn the_configuration_file_prefixes_every_profile_but_default_and_a_bare_name_is_no_profile() {
        const CONFIG: &str = "\
[default]
region = us-east-1

[profile trading]
region = eu-west-3

[trading]
region = ap-south-1

[plugins]
cli_legacy_plugin_path = /opt/plugins
";
        let default = configured(CONFIG, "default");
        assert_eq!(default.name(), "default");
        assert_eq!(
            default.region(),
            Some("us-east-1"),
            "[default] is the default profile"
        );

        let trading = configured(CONFIG, "trading");
        assert_eq!(trading.name(), "trading");
        assert_eq!(
            trading.region(),
            Some("eu-west-3"),
            "a bare [trading] in the configuration file is not the profile, and its keys stay out of it"
        );
        assert_eq!(
            trading.get("cli_legacy_plugin_path"),
            None,
            "a section nothing reads keeps its own keys"
        );
        assert_eq!(
            Files::parse(Some(CONFIG), None).names(),
            ["default", "trading"],
            "neither the bare section nor [plugins] is a profile"
        );
    }

    #[test]
    fn the_later_of_default_and_profile_default_replaces_the_earlier_whole() {
        const CONFIG: &str = "\
[default]
region = us-east-1
output = json

[profile default]
region = eu-west-3
";
        let default = configured(CONFIG, "default");
        assert_eq!(
            default.output(),
            None,
            "a key only the earlier spelling states is not the profile's, as botocore's build_profile_map reads it"
        );
        assert_eq!(
            default.region(),
            Some("eu-west-3"),
            "the later spelling is the profile"
        );
        assert_eq!(Files::parse(Some(CONFIG), None).names(), ["default"]);

        const REVERSED: &str = "\
[profile default]
region = eu-west-3
output = json

[default]
region = us-east-1
";
        let default = configured(REVERSED, "default");
        assert_eq!(default.output(), None, "whichever spelling comes first");
        assert_eq!(default.region(), Some("us-east-1"));

        const REPEATED: &str = "\
[default]
region = us-east-1

[default]
output = json
";
        let default = configured(REPEATED, "default");
        assert_eq!(
            (default.region(), default.output()),
            (Some("us-east-1"), Some("json")),
            "one spelling repeated extends the section as any repeated section does"
        );

        let laid_over = read_profile(CONFIG, "[default]\noutput = text\n", "default")
            .expect("the default profile");
        assert_eq!(
            (laid_over.region(), laid_over.output()),
            (Some("eu-west-3"), Some("text")),
            "the credentials file still lays its values over the section that won"
        );
    }

    #[test]
    fn a_quoted_profile_name_keeps_its_space_and_a_header_of_three_words_is_no_profile() {
        const CONFIG: &str = "\
[profile \"my name\"]
region = eu-west-3

[profile 'single quoted']
output = text

[ profile   spaced ]
region = us-west-2

[profile two words]
region = us-east-1
";
        let quoted = configured(CONFIG, "my name");
        assert_eq!(quoted.name(), "my name");
        assert_eq!(quoted.region(), Some("eu-west-3"));
        assert_eq!(configured(CONFIG, "single quoted").output(), Some("text"));
        assert_eq!(
            configured(CONFIG, "spaced").region(),
            Some("us-west-2"),
            "whitespace around and inside a header separates words"
        );
        assert!(
            read_profile(CONFIG, "", "two words").is_none(),
            "an unquoted name of two words is no profile"
        );
        assert_eq!(
            Files::parse(Some(CONFIG), None).names(),
            ["my name", "single quoted", "spaced"]
        );
    }

    #[test]
    fn a_header_followed_by_a_comment_is_that_section_as_the_aws_cli_reads_it() {
        const CREDENTIALS: &str = "\
[old]
aws_access_key_id = AKIAOLD
aws_secret_access_key = old-secret
[default]   # dumped 12:30
aws_access_key_id = AKIANEW
aws_secret_access_key = new-secret
";
        assert_eq!(
            Files::parse(None, Some(CREDENTIALS)).names(),
            ["default", "old"],
            "the comment after the header is no part of its name"
        );
        assert_eq!(
            read_profile("", CREDENTIALS, "old").and_then(|old| pair(&old)),
            Some(Credentials::new("AKIAOLD", "old-secret")),
            "the section before keeps its own keys"
        );
        assert_eq!(
            read_profile("", CREDENTIALS, "default").and_then(|default| pair(&default)),
            Some(Credentials::new("AKIANEW", "new-secret"))
        );
    }

    #[test]
    fn the_credentials_file_names_every_section_bare() {
        const CREDENTIALS: &str = "\
[default]
aws_access_key_id = AKIADEFAULT
aws_secret_access_key = default-secret

[trading]
aws_access_key_id = AKIATRADING
aws_secret_access_key = trading-secret

[profile desk]
aws_access_key_id = AKIADESK
aws_secret_access_key = desk-secret
";
        let trading =
            read_profile("", CREDENTIALS, "trading").expect("a bare section is a profile");
        assert_eq!(
            pair(&trading),
            Some(Credentials::new("AKIATRADING", "trading-secret"))
        );
        assert_eq!(
            read_profile("", CREDENTIALS, "default").and_then(|default| pair(&default)),
            Some(Credentials::new("AKIADEFAULT", "default-secret"))
        );
        assert!(
            read_profile("", CREDENTIALS, "desk").is_none(),
            "the credentials file has no profile prefix"
        );
        assert_eq!(
            Files::parse(None, Some(CREDENTIALS)).names(),
            ["default", "profile desk", "trading"],
            "[profile desk] in the credentials file is a profile of that whole name"
        );
    }

    #[test]
    fn sso_session_and_services_sections_are_no_profiles() {
        const CONFIG: &str = "\
[profile dev]
region = eu-west-3

[sso-session corp]
sso_start_url = https://corp.awsapps.com/start
sso_region = eu-west-1

[services local]
s3 =
  endpoint_url = http://localhost:9000
";
        assert_eq!(Files::parse(Some(CONFIG), None).names(), ["dev"]);
        assert!(read_profile(CONFIG, "", "corp").is_none());
        assert!(read_profile(CONFIG, "", "local").is_none());
    }

    #[test]
    fn files_list_every_profile_either_file_holds_in_name_order() {
        const CONFIG: &str = "[default]\nregion = us-east-1\n[profile b]\nregion = eu-west-3\n";
        const CREDENTIALS: &str =
            "[a]\naws_access_key_id = AKIAA\naws_secret_access_key = s\n[default]\noutput = json\n";
        let files = Files::parse(Some(CONFIG), Some(CREDENTIALS));
        assert_eq!(files.names(), ["a", "b", "default"]);
        let session = Session::new()
            .with_environment(false)
            .with_config_text(CONFIG)
            .with_credentials_text(CREDENTIALS);
        assert_eq!(
            session.available_profiles(),
            files.names(),
            "a session lists what the files hold"
        );
        assert_eq!(
            files
                .profile("default")
                .and_then(|default| default.output().map(str::to_owned)),
            Some("json".to_owned()),
            "the default profile is laid over from the credentials file"
        );

        let empty = Files::parse(None, None);
        assert!(empty.names().is_empty(), "no file, no profile");
        assert!(
            empty.profile("default").is_none(),
            "no file, not even the default"
        );
        assert!(
            read_profile(CONFIG, CREDENTIALS, "nobody").is_none(),
            "a profile nobody wrote is absent, not an error"
        );
    }
}

mod lines {
    use super::*;

    #[test]
    fn keys_fold_to_lower_case_values_keep_theirs_and_either_delimiter_splits() {
        const CONFIG: &str = "\
[profile desk]
Region: eu-west-3
OUTPUT = json
endpoint_url: http://localhost:4566
role_session_name = PowerDesk
Credential_Process = /opt/bin/keys --profile desk
";
        let desk = configured(CONFIG, "desk");
        assert_eq!(desk.get("region"), Some("eu-west-3"), "a colon delimits");
        assert_eq!(desk.get("REGION"), Some("eu-west-3"), "a lookup folds too");
        assert_eq!(desk.region(), Some("eu-west-3"));
        assert_eq!(desk.output(), Some("json"));
        assert_eq!(
            desk.endpoint_url(),
            Some("http://localhost:4566"),
            "the first delimiter splits, so a URL keeps its own colon"
        );
        assert_eq!(
            desk.get("role_session_name"),
            Some("PowerDesk"),
            "a value keeps its case"
        );
        assert_eq!(
            desk.credential_process(),
            Some("/opt/bin/keys --profile desk")
        );
    }

    #[test]
    fn comments_and_lines_before_any_section_are_skipped() {
        const CONFIG: &str = "\
region = before-any-section
# a comment
; another comment
[profile desk]
  # an indented comment
region = eu-west-3
; region = us-east-1
# output = json
";
        let desk = configured(CONFIG, "desk");
        assert_eq!(
            desk.region(),
            Some("eu-west-3"),
            "a commented value is no value"
        );
        assert_eq!(desk.output(), None);
        assert_eq!(
            Files::parse(Some(CONFIG), None).names(),
            ["desk"],
            "a line before any header belongs to no profile"
        );
    }

    #[test]
    fn a_later_duplicate_key_wins_and_a_repeated_section_extends_the_first() {
        const CONFIG: &str = "\
[profile desk]
region = us-east-1
region = eu-west-3

[profile desk]
output = json
";
        let desk = configured(CONFIG, "desk");
        assert_eq!(desk.region(), Some("eu-west-3"), "the later value wins");
        assert_eq!(
            desk.output(),
            Some("json"),
            "the section spelled again adds to it"
        );
    }

    #[test]
    fn an_indented_block_is_a_table_under_its_key_and_an_unindented_key_closes_it() {
        const CONFIG: &str = "\
[profile desk]
region = eu-west-3
s3 =
  addressing_style = path
  Use_Accelerate_Endpoint: true
\tpayload_signing_enabled = false
output = json
  stray = belongs to nothing
";
        let desk = configured(CONFIG, "desk");
        assert_eq!(desk.s3("addressing_style"), Some("path"));
        assert_eq!(
            desk.s3("ADDRESSING_STYLE"),
            Some("path"),
            "a nested lookup folds"
        );
        assert_eq!(
            desk.nested("S3", "use_accelerate_endpoint"),
            Some("true"),
            "a nested key folds and takes a colon"
        );
        assert_eq!(
            desk.s3("payload_signing_enabled"),
            Some("false"),
            "a tab indents"
        );
        assert_eq!(desk.get("s3"), None, "a table is no top-level value");
        assert_eq!(
            desk.get("addressing_style"),
            None,
            "a nested key stays nested"
        );
        assert_eq!(
            desk.output(),
            Some("json"),
            "an unindented key closes the table"
        );
        assert_eq!(
            desk.get("stray"),
            None,
            "an indented line under a value is skipped"
        );
        assert_eq!(desk.s3("stray"), None, "and never reopens the closed table");
        assert_eq!(
            desk.nested("region", "anything"),
            None,
            "a value is no table"
        );
        assert_eq!(desk.nested("dynamodb", "anything"), None, "an absent table");
    }

    #[test]
    fn a_crlf_file_reads_its_keys_tables_and_pairs_as_an_lf_one() {
        // Spelled with escapes: the compiler normalizes a line break written
        // in the source, so only an escaped `\r` reaches the reader.
        const CONFIG: &str = "# written by Notepad\r\n\
[profile desk]\r\n\
region = eu-west-3\r\n\
; region = us-east-1\r\n\
s3 =\r\n\
\x20\x20addressing_style = path\r\n\
\tpayload_signing_enabled = false\r\n\
output = json\r\n\
\r\n\
[profile \"quoted desk\"]\r\n\
region = ap-south-1";
        const CREDENTIALS: &str = "[desk]\r\n\
aws_access_key_id = AKIACRLF\r\n\
aws_secret_access_key = crlf-secret\r\n\
aws_session_token = crlf-token\r\n\
aws_credential_expiration = 2026-10-03T03:20:00Z\r\n";
        let crlf = read_profile(CONFIG, CREDENTIALS, "desk").expect("the profile desk");
        let lf = read_profile(
            &CONFIG.replace("\r\n", "\n"),
            &CREDENTIALS.replace("\r\n", "\n"),
            "desk",
        )
        .expect("the profile desk");
        assert_eq!(crlf.region(), Some("eu-west-3"));
        assert_eq!(
            crlf.output(),
            Some("json"),
            "an unindented key closes the table"
        );
        assert_eq!(crlf.s3("addressing_style"), Some("path"));
        assert_eq!(crlf.s3("payload_signing_enabled"), Some("false"));
        assert_eq!(
            pair(&crlf),
            Some(
                Credentials::new("AKIACRLF", "crlf-secret")
                    .with_session_token("crlf-token")
                    .with_expiry(UNIX_EPOCH + Duration::from_secs(1_790_997_600))
            ),
            "the pair and its expiry carry no carriage return"
        );
        assert_eq!(pair(&crlf), pair(&lf));
        assert_eq!(
            crlf.iter().collect::<Vec<_>>(),
            lf.iter().collect::<Vec<_>>(),
            "every value reads as the LF file's"
        );
        assert!(
            crlf.iter()
                .all(|(key, value)| !key.ends_with('\r') && !value.ends_with('\r')),
            "no key or value keeps a carriage return"
        );
        assert_eq!(
            read_profile(CONFIG, CREDENTIALS, "quoted desk")
                .and_then(|quoted| quoted.region().map(str::to_owned)),
            Some("ap-south-1".to_owned()),
            "a quoted header and a last line with no break read too"
        );
        assert_eq!(
            Files::parse(Some(CONFIG), Some(CREDENTIALS)).names(),
            ["desk", "quoted desk"]
        );
    }

    #[test]
    fn an_indented_key_right_after_a_header_is_a_key_as_the_aws_parser_reads_it() {
        let profile = Session::new()
            .with_environment(false)
            .with_config_text("[profile desk]\n  region = eu-west-3\n  output = json\n")
            .with_profile("desk")
            .profile()
            .expect("the profile");
        assert_eq!(profile.region(), Some("eu-west-3"));
        assert_eq!(profile.output(), Some("json"));
    }

    #[test]
    fn a_profile_iterates_its_values_in_key_order_and_reads_its_flags() {
        const CONFIG: &str = "\
[profile desk]
use_fips_endpoint = True
use_dualstack_endpoint = false
region = eu-west-3
s3 =
  addressing_style = path
output = json
experimental = maybe
nothing =
";
        let desk = configured(CONFIG, "desk");
        assert_eq!(
            desk.iter().collect::<Vec<_>>(),
            [
                ("experimental", "maybe"),
                ("output", "json"),
                ("region", "eu-west-3"),
                ("use_dualstack_endpoint", "false"),
                ("use_fips_endpoint", "True"),
            ],
            "tables and empty values are no values"
        );
        assert_eq!(desk.get("nothing"), None, "an empty value is absent");
        assert_eq!(
            desk.flag("use_fips_endpoint"),
            Some(true),
            "true in any case"
        );
        assert_eq!(desk.flag("USE_DUALSTACK_ENDPOINT"), Some(false));
        assert_eq!(
            desk.flag("experimental"),
            Some(false),
            "an unknown spelling is false"
        );
        assert_eq!(desk.flag("missing"), None, "an absent flag is no answer");
    }

    #[test]
    fn a_profile_flag_reads_the_one_boolean_table_every_flag_in_the_crate_reads() {
        for (spelling, expected) in [
            ("true", true),
            ("T", true),
            ("tru", true),
            ("yes", true),
            ("Y", true),
            ("ye", true),
            ("on", true),
            ("1", true),
            ("false", false),
            ("f", false),
            ("fals", false),
            ("no", false),
            ("N", false),
            ("off", false),
            ("of", false),
            ("0", false),
            ("maybe", false),
            ("2", false),
        ] {
            let config = format!("[profile desk]\nuse_fips_endpoint = {spelling}\n");
            assert_eq!(
                configured(&config, "desk").flag("use_fips_endpoint"),
                Some(expected),
                "{spelling:?}"
            );
        }
    }
}

mod services {
    use super::*;

    #[test]
    fn the_services_section_a_profile_names_answers_each_service_endpoint() {
        const CONFIG: &str = "\
[profile local]
services = local-stack
endpoint_url = http://localhost:4566

[services local-stack]
s3 =
  endpoint_url = http://localhost:9000
secrets_manager =
  endpoint_url = http://localhost:9001
sso_oidc =
  endpoint_url = http://localhost:9002
dynamodb =
  region = us-west-2

[profile elsewhere]
services = nowhere

[profile emptied]
services = empty

[services empty]

[profile unnamed]
services =

[profile plain]
region = eu-west-3
";
        let local = configured(CONFIG, "local");
        assert_eq!(
            local.endpoint_url(),
            Some("http://localhost:4566"),
            "every service's"
        );
        assert_eq!(
            local
                .service_endpoint_url("s3")
                .expect("a section the profile defines"),
            Some("http://localhost:9000")
        );
        assert_eq!(
            local
                .service_endpoint_url("S3")
                .expect("a section the profile defines"),
            Some("http://localhost:9000"),
            "a service id folds"
        );
        assert_eq!(
            local
                .service_endpoint_url("Secrets Manager")
                .expect("a section the profile defines"),
            Some("http://localhost:9001"),
            "a space is spelled as an underscore"
        );
        assert_eq!(
            local
                .service_endpoint_url("sso-oidc")
                .expect("a section the profile defines"),
            Some("http://localhost:9002"),
            "a hyphen is spelled as an underscore"
        );
        assert_eq!(
            local
                .service_endpoint_url("dynamodb")
                .expect("a section the profile defines"),
            None,
            "a service entry without endpoint_url states none"
        );
        assert_eq!(
            local
                .service_endpoint_url("sts")
                .expect("a section the profile defines"),
            None,
            "a service with no entry"
        );
        assert_eq!(
            configured(CONFIG, "plain")
                .service_endpoint_url("s3")
                .expect("no section named"),
            None
        );

        // A section the profile names and nobody wrote - or wrote empty, or
        // did not name at all - is refused as botocore refuses it, rather than
        // leaving the published host a typo would.
        for (name, expected) in [
            (
                "elsewhere",
                "names services nowhere, which no [services nowhere] section defines",
            ),
            (
                "emptied",
                "names services empty, which no [services empty] section defines",
            ),
            (
                "unnamed",
                "states services without naming a [services] section",
            ),
        ] {
            let message = refusal(configured(CONFIG, name).service_endpoint_url("s3"));
            assert!(
                message.contains(&format!("the profile {name}")),
                "{message}"
            );
            assert!(message.contains(expected), "{message}");
        }
    }

    #[test]
    fn a_service_key_is_the_service_id_folded_with_underscores() {
        assert_eq!(service_key("S3"), "s3");
        assert_eq!(service_key("Secrets Manager"), "secrets_manager");
        assert_eq!(service_key("sso-oidc"), "sso_oidc");
        assert_eq!(service_key(" DynamoDB "), "dynamodb", "trimmed first");
    }
}

mod credentials {
    use super::*;

    #[test]
    fn the_credentials_file_wins_over_the_configuration_file_for_one_profile() {
        const CONFIG: &str = "\
[profile desk]
region = us-east-1
output = json
aws_access_key_id = AKIACONFIG
aws_secret_access_key = config-secret
";
        const CREDENTIALS: &str = "\
[desk]
region = eu-west-3
aws_access_key_id = AKIACREDENTIALS
aws_secret_access_key = credentials-secret
";
        let desk = read_profile(CONFIG, CREDENTIALS, "desk").expect("both files spell desk");
        assert_eq!(
            desk.region(),
            Some("eu-west-3"),
            "the credentials file's value wins"
        );
        assert_eq!(
            desk.output(),
            Some("json"),
            "a key only the configuration file spells stays"
        );
        assert_eq!(
            pair(&desk),
            Some(Credentials::new("AKIACREDENTIALS", "credentials-secret")),
            "the credentials file's pair wins"
        );
        assert_eq!(
            pair(&configured(CONFIG, "desk")),
            Some(Credentials::new("AKIACONFIG", "config-secret")),
            "the configuration file's pair stands alone"
        );
    }

    #[test]
    fn a_pair_carries_its_session_token_under_either_name_and_its_account() {
        const CREDENTIALS: &str = "\
[modern]
aws_access_key_id = AKIAMODERN
aws_secret_access_key = modern-secret
aws_session_token = modern-token
aws_account_id = 123456789012

[legacy]
aws_access_key_id = AKIALEGACY
aws_secret_access_key = legacy-secret
aws_security_token = legacy-token

[both]
aws_access_key_id = AKIABOTH
aws_secret_access_key = both-secret
aws_security_token = security-token
aws_session_token = session-token

[half]
aws_access_key_id = AKIAHALF
region = eu-west-3
";
        let modern = read_profile("", CREDENTIALS, "modern").and_then(|modern| pair(&modern));
        assert_eq!(
            modern,
            Some(
                Credentials::new("AKIAMODERN", "modern-secret")
                    .with_session_token("modern-token")
                    .with_account_id("123456789012")
            )
        );
        assert!(
            modern.is_some_and(|keys| keys.is_temporary()),
            "a token is a temporary set"
        );

        let legacy = read_profile("", CREDENTIALS, "legacy").and_then(|legacy| pair(&legacy));
        assert_eq!(
            legacy.as_ref().and_then(Credentials::session_token),
            Some("legacy-token"),
            "aws_security_token is the session token's older name"
        );

        let both = read_profile("", CREDENTIALS, "both").and_then(|both| pair(&both));
        assert_eq!(
            both.as_ref().and_then(Credentials::session_token),
            Some("security-token"),
            "aws_security_token is read before aws_session_token, botocore's TOKENS order"
        );
        let reordered = read_profile(
            "",
            "[both]\naws_access_key_id = AKIABOTH\naws_secret_access_key = both-secret\n\
             aws_session_token = session-token\naws_security_token = security-token\n",
            "both",
        )
        .and_then(|both| pair(&both));
        assert_eq!(
            reordered.as_ref().and_then(Credentials::session_token),
            Some("security-token"),
            "the order is the names', not the lines'"
        );

        let half = read_profile("", CREDENTIALS, "half").expect("a profile with half a pair");
        let message = refusal(half.credentials());
        assert!(
            message.contains("half")
                && message.contains("credentials file")
                && message.contains("aws_access_key_id without aws_secret_access_key"),
            "a key id without its secret is a refusal naming the profile, the file and the missing key: {message}"
        );
        assert!(
            !message.contains("AKIAHALF"),
            "no value is quoted: {message}"
        );
        assert_eq!(half.region(), Some("eu-west-3"));
    }

    #[test]
    fn half_a_set_is_a_refusal_naming_what_is_missing_and_nothing_is_no_set() {
        const CREDENTIALS: &str = "\
[secret_only]
aws_secret_access_key = lonely-secret

[token_only]
aws_session_token = lonely-token

[region_only]
region = eu-west-3
";
        let read = |name| read_profile("", CREDENTIALS, name).expect("the profile");
        let message = refusal(read("secret_only").credentials());
        assert!(
            message.contains("aws_secret_access_key without aws_access_key_id"),
            "{message}"
        );
        assert!(
            !message.contains("lonely-secret"),
            "the secret never renders: {message}"
        );
        let message = refusal(read("token_only").credentials());
        assert!(
            message.contains("aws_session_token without aws_access_key_id"),
            "{message}"
        );
        assert!(
            !message.contains("lonely-token"),
            "the token never renders: {message}"
        );
        assert_eq!(
            pair(&read("region_only")),
            None,
            "a profile holding no key at all holds no set, and that is no refusal"
        );
        let message = refusal(
            configured("[profile desk]\naws_access_key_id = AKIADESK\n", "desk").credentials(),
        );
        assert!(
            message.contains("configuration file"),
            "the refusal names the file the half set is in: {message}"
        );
    }

    #[test]
    fn half_a_set_in_either_file_is_the_partial_set_refusal_and_no_other_refusal_is() {
        const CREDENTIALS: &str = "\
[key_only]
aws_access_key_id = AKIAHALF

[secret_only]
aws_secret_access_key = lonely-secret

[security_token_only]
aws_security_token = lonely-token

[shadowing]
aws_access_key_id = AKIASHADOW

[expiring]
aws_access_key_id = ASIADUMPED
aws_secret_access_key = s
x_security_token_expires = soon
";
        const CONFIG: &str = "\
[profile shadowing]
aws_access_key_id = AKIAWHOLE
aws_secret_access_key = whole-secret

[profile desk]
aws_secret_access_key = config-secret

[profile roleless]
role_arn = arn:aws:iam::123456789012:role/lake-reader
";
        let read = |name| read_profile(CONFIG, CREDENTIALS, name).expect("the profile");
        for (name, stated) in [
            (
                "key_only",
                "aws_access_key_id without aws_secret_access_key",
            ),
            (
                "secret_only",
                "aws_secret_access_key without aws_access_key_id",
            ),
            (
                "security_token_only",
                "aws_security_token without aws_access_key_id",
            ),
        ] {
            let profile = read(name);
            let error = profile.credentials().expect_err("half a set is refused");
            assert!(is_partial_set(&error), "{name}: {error}");
            let message = error.to_string();
            assert!(
                message.contains(&format!("the profile {name}"))
                    && message.contains("credentials file")
                    && message.contains(stated),
                "{name}: the refusal names the profile, the file and the missing key: {message}"
            );
            assert!(
                !message.contains("AKIAHALF") && !message.contains("lonely"),
                "{name}: no value is quoted: {message}"
            );
        }

        let shadowing = read("shadowing")
            .credentials()
            .expect_err("half a set in the credentials file is refused");
        assert!(
            is_partial_set(&shadowing),
            "a whole pair in the configuration file does not stand in for the half one written over it: {shadowing}"
        );

        let desk = read("desk").credentials().expect_err("half a set");
        assert!(is_partial_set(&desk), "{desk}");
        assert!(desk.to_string().contains("configuration file"), "{desk}");

        let expiring = read("expiring")
            .credentials()
            .expect_err("an expiry nothing reads");
        assert!(
            !is_partial_set(&expiring),
            "a whole set with an unreadable expiry is no partial set: {expiring}"
        );
        let roleless = read("roleless")
            .assumed_role()
            .expect_err("a role with no source");
        assert!(!is_partial_set(&roleless), "{roleless}");
        assert!(
            !is_partial_set(&yggdryl::Error::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "the profile key_only in the credentials file sets aws_access_key_id without aws_secret_access_key",
            ))),
            "only the refusal itself is recognised, never its wording"
        );
    }

    #[test]
    fn an_expiry_a_tool_wrote_beside_the_set_is_read_under_every_name_the_tools_write() {
        let lapses = UNIX_EPOCH + Duration::from_secs(1_790_997_600);
        // 2026-10-03T03:20:00Z, spelled as each tool writes it.
        for (key, value) in [
            ("aws_credential_expiration", "2026-10-03T03:20:00Z"),
            ("AWS_CREDENTIAL_EXPIRATION", "2026-10-03T03:20:00Z"),
            ("x_security_token_expires", "2026-10-03T05:20:00+02:00"),
            ("aws_session_expiration", "2026-10-03T03:20:00+0000"),
            ("aws_session_expiration", "2026-10-03T03:20:00+00:00"),
            ("aws_expiration", "2026-10-03T03:20:00.000Z"),
            ("expiration", "2026-10-03 03:20:00"),
        ] {
            let text = format!(
                "[dumped]\naws_access_key_id = ASIADUMPED\naws_secret_access_key = s\naws_session_token = t\n{key} = {value}\n"
            );
            let found = read_profile("", &text, "dumped")
                .and_then(|dumped| pair(&dumped))
                .expect("the dumped set");
            assert_eq!(found.expires_at(), Some(lapses), "{key} = {value}");
            assert!(found.is_temporary());
        }
        let found = read_profile(
            "",
            "[long]\naws_access_key_id = AKIALONG\naws_secret_access_key = s\n",
            "long",
        )
        .and_then(|long| pair(&long))
        .expect("a long-lived pair");
        assert_eq!(found.expires_at(), None, "no expiry written, none read");
    }

    #[test]
    fn an_expiry_nothing_reads_or_two_that_disagree_is_a_refusal_naming_the_keys() {
        let unreadable = read_profile(
            "",
            "[dumped]\naws_access_key_id = ASIADUMPED\naws_secret_access_key = s\nx_security_token_expires = soon\n",
            "dumped",
        )
        .expect("the profile");
        let message = refusal(unreadable.credentials());
        assert!(
            message.contains("x_security_token_expires") && message.contains("\"soon\""),
            "the key and the value nothing reads are named: {message}"
        );
        let disagreeing = read_profile(
            "",
            "[dumped]\naws_access_key_id = ASIADUMPED\naws_secret_access_key = s\n\
             aws_credential_expiration = 2026-10-03T03:20:00Z\naws_expiration = 2026-10-03T04:20:00Z\n",
            "dumped",
        )
        .expect("the profile");
        let message = refusal(disagreeing.credentials());
        assert!(
            message.contains("aws_credential_expiration") && message.contains("aws_expiration"),
            "both keys that disagree are named: {message}"
        );
        let agreeing = read_profile(
            "",
            "[dumped]\naws_access_key_id = ASIADUMPED\naws_secret_access_key = s\n\
             aws_credential_expiration = 2026-10-03T03:20:00Z\naws_expiration = 2026-10-03T05:20:00+02:00\n",
            "dumped",
        )
        .and_then(|dumped| pair(&dumped));
        assert!(agreeing.is_some(), "two spellings of one instant agree");
    }

    #[test]
    fn a_set_pasted_from_a_shell_block_reads_as_the_set_it_spells() {
        // The three blocks the IAM Identity Center portal and
        // `aws configure export-credentials` print, pasted under a header.
        let expected = Credentials::new("ASIAPASTED", "pasted/secret+key")
            .with_session_token("IQoJb3JpZ2luX2VjEJr//////////wEaCXVzLWVhc3QtMSJHMEUCIQ==");
        for block in [
            "export AWS_ACCESS_KEY_ID=\"ASIAPASTED\"\nexport AWS_SECRET_ACCESS_KEY=\"pasted/secret+key\"\n\
             export AWS_SESSION_TOKEN=\"IQoJb3JpZ2luX2VjEJr//////////wEaCXVzLWVhc3QtMSJHMEUCIQ==\"\n",
            "set AWS_ACCESS_KEY_ID=ASIAPASTED\nset AWS_SECRET_ACCESS_KEY=pasted/secret+key\n\
             set AWS_SESSION_TOKEN=IQoJb3JpZ2luX2VjEJr//////////wEaCXVzLWVhc3QtMSJHMEUCIQ==\n",
            "$Env:AWS_ACCESS_KEY_ID=\"ASIAPASTED\"\n$Env:AWS_SECRET_ACCESS_KEY=\"pasted/secret+key\"\n\
             $Env:AWS_SESSION_TOKEN=\"IQoJb3JpZ2luX2VjEJr//////////wEaCXVzLWVhc3QtMSJHMEUCIQ==\"\n",
            "aws_access_key_id = 'ASIAPASTED'   # dumped at 12:30\naws_secret_access_key = pasted/secret+key ; from the portal\n\
             aws_session_token = \"IQoJb3JpZ2luX2VjEJr//////////wEaCXVzLWVhc3QtMSJHMEUCIQ==\"\n",
        ] {
            let text = format!("[default]\n{block}");
            assert_eq!(
                read_profile("", &text, "default").and_then(|default| pair(&default)),
                Some(expected.clone()),
                "{block}"
            );
        }
    }
}

mod roles {
    use super::*;

    #[test]
    fn a_role_arn_that_is_not_an_arn_is_the_profiles_refusal() {
        let profile = configured(
            "[profile desk]\nrole_arn = lake-reader\nsource_profile = desk\n",
            "desk",
        );
        let message = refusal(profile.assumed_role());
        assert!(
            message.contains("desk")
                && message.contains("role_arn")
                && message.contains("not an ARN"),
            "{message}"
        );
    }

    const ARN: &str = "arn:aws:iam::123456789012:role/lake-reader";

    #[test]
    fn a_role_profile_names_its_source_and_every_parameter_of_the_exchange() {
        let config = format!(
            "\
[profile chained]
role_arn = {ARN}
source_profile = base
role_session_name = power-desk
external_id = desk-7
mfa_serial = arn:aws:iam::123456789012:mfa/trader
duration_seconds = 7200

[profile instance]
role_arn = {ARN}
credential_source = Ec2InstanceMetadata

[profile container]
role_arn = {ARN}
credential_source = ecscontainer

[profile environment]
role_arn = {ARN}
credential_source = Environment

[profile federated]
role_arn = {ARN}
web_identity_token_file = /var/run/secrets/token

[profile plain]
region = eu-west-3
"
        );
        let chained = configured(&config, "chained")
            .assumed_role()
            .expect("a well-formed role")
            .expect("a role");
        assert_eq!(chained.role_arn(), ARN);
        assert_eq!(chained.source_profile(), Some("base"));
        assert_eq!(chained.credential_source(), None);
        assert_eq!(chained.web_identity_token_file(), None);
        assert_eq!(chained.session_name(), Some("power-desk"));
        assert_eq!(chained.external_id(), Some("desk-7"));
        assert_eq!(
            chained.mfa_serial(),
            Some("arn:aws:iam::123456789012:mfa/trader")
        );
        assert_eq!(chained.duration(), Duration::from_secs(7200));

        let source = |name: &str| {
            configured(&config, name)
                .assumed_role()
                .expect("a well-formed role")
                .and_then(|role| role.credential_source())
        };
        assert_eq!(
            source("instance"),
            Some(CredentialSource::Ec2InstanceMetadata)
        );
        assert_eq!(
            source("container"),
            Some(CredentialSource::EcsContainer),
            "a source is read in any case"
        );
        assert_eq!(source("environment"), Some(CredentialSource::Environment));

        assert!(
            configured(&config, "federated")
                .assumed_role()
                .expect("a web identity profile is no refusal")
                .is_none(),
            "a profile stating web_identity_token_file is a web identity profile, not a role profile"
        );

        assert!(
            configured(&config, "plain")
                .assumed_role()
                .expect("no role is no refusal")
                .is_none(),
            "a profile without role_arn assumes nothing"
        );
    }

    #[test]
    fn a_profile_stating_web_identity_token_file_assumes_no_role_whatever_else_it_states() {
        let config = format!(
            "\
[profile federated]
role_arn = {ARN}
web_identity_token_file = /var/run/secrets/token
role_session_name = pod

[profile sourced]
role_arn = {ARN}
web_identity_token_file = /var/run/secrets/token
source_profile = base
credential_source = Environment

[profile typo]
role_arn = lake-reader
web_identity_token_file = /var/run/secrets/token
duration_seconds = an hour
"
        );
        for name in ["federated", "sourced", "typo"] {
            let profile = configured(&config, name);
            assert!(
                profile
                    .assumed_role()
                    .unwrap_or_else(|error| panic!("{name}: {error}"))
                    .is_none(),
                "{name}: botocore's AssumeRoleProvider leaves a web identity profile to the web identity step"
            );
            assert_eq!(
                profile.get("web_identity_token_file"),
                Some("/var/run/secrets/token"),
                "{name}: the profile still states its token file for that step"
            );
        }
    }

    #[test]
    fn duration_seconds_reads_every_spelling_a_setting_spells_a_length_with() {
        for (spelling, seconds) in [
            ("7200", 7200),
            ("+7200", 7200),
            ("7200s", 7200),
            ("7200 seconds", 7200),
            ("1e4", 10_000),
            ("3600000ms", 3600),
            ("7200.9", 7200),
            // STS's own bounds still clamp what is asked.
            ("60", 900),
            ("1d", 43_200),
        ] {
            let config = format!(
                "[profile hourly]\nrole_arn = {ARN}\nsource_profile = base\nduration_seconds = {spelling}\n"
            );
            let role = configured(&config, "hourly")
                .assumed_role()
                .unwrap_or_else(|error| panic!("{spelling:?}: {error}"))
                .expect("a role");
            assert_eq!(role.duration().as_secs(), seconds, "{spelling:?}");
        }
    }

    #[test]
    fn duration_seconds_that_no_length_spells_is_refused_naming_the_key_and_the_value() {
        for spelling in ["an hour", "-1", "1e30", "1m", "nan", "1h"] {
            let config = format!(
                "[profile hourly]\nrole_arn = {ARN}\nsource_profile = base\nduration_seconds = {spelling}\n"
            );
            let refused = refusal(configured(&config, "hourly").assumed_role());
            assert!(refused.contains("duration_seconds"), "{refused}");
            assert!(refused.contains(&format!("{spelling:?}")), "{refused}");
            assert!(
                refused.contains("seconds, with an optional fraction"),
                "{refused}"
            );
        }
    }

    #[test]
    fn a_role_profile_is_refused_with_two_sources_with_none_or_with_a_bad_duration() {
        let config = format!(
            "\
[profile both]
role_arn = {ARN}
source_profile = base
credential_source = Environment

[profile sourceless]
role_arn = {ARN}

[profile hourly]
role_arn = {ARN}
source_profile = base
duration_seconds = an hour

[profile laptop]
role_arn = {ARN}
credential_source = Laptop
"
        );
        let both = refusal(configured(&config, "both").assumed_role());
        assert!(both.contains("the profile both"), "{both}");
        assert!(
            both.contains("names both source_profile and credential_source"),
            "{both}"
        );

        let sourceless = refusal(configured(&config, "sourceless").assumed_role());
        assert!(
            sourceless.contains("the profile sourceless"),
            "{sourceless}"
        );
        assert!(
            sourceless
                .contains("without source_profile, credential_source or web_identity_token_file"),
            "{sourceless}"
        );

        let hourly = refusal(configured(&config, "hourly").assumed_role());
        assert!(hourly.contains("the profile hourly"), "{hourly}");
        assert!(hourly.contains("duration_seconds"), "{hourly}");
        assert!(
            hourly.contains("\"an hour\""),
            "the value is quoted: {hourly}"
        );

        let laptop = refusal(configured(&config, "laptop").assumed_role());
        assert!(laptop.contains("the profile laptop"), "{laptop}");
        assert!(laptop.contains("credential_source"), "{laptop}");
        assert!(
            laptop.contains("Laptop"),
            "the unknown source is named: {laptop}"
        );
    }
}

mod sso {
    use super::*;

    #[test]
    fn a_profile_signs_in_through_its_sso_session_or_the_legacy_keys() {
        const CONFIG: &str = "\
[profile dev]
sso_session = corp
sso_account_id = 123456789012
sso_role_name = LakeReader
region = eu-west-3

[sso-session corp]
sso_start_url = https://corp.awsapps.com/start
sso_region = eu-west-1
sso_registration_scopes = sso:account:access, codewhisperer:completions

[profile legacy]
sso_start_url = https://legacy.awsapps.com/start
sso_region = us-east-1
sso_account_id = 210987654321
sso_role_name = Auditor

[profile plain]
region = eu-west-3
";
        let dev = configured(CONFIG, "dev");
        assert_eq!(dev.sso_session_name(), Some("corp"));
        assert_eq!(
            dev.sso().expect("a complete sign-in"),
            Some(
                Sso::new(
                    "https://corp.awsapps.com/start",
                    "eu-west-1",
                    "123456789012",
                    "LakeReader"
                )
                .with_session_name("corp")
                .with_scopes(["sso:account:access", "codewhisperer:completions"])
            ),
            "the start URL, region and scopes come from the session, the account and role from the profile"
        );
        assert_eq!(
            dev.region(),
            Some("eu-west-3"),
            "the profile's region is its own"
        );

        let legacy = configured(CONFIG, "legacy");
        assert_eq!(legacy.sso_session_name(), None);
        let sign_in = legacy
            .sso()
            .expect("a complete sign-in")
            .expect("a sign-in");
        assert_eq!(
            sign_in,
            Sso::new(
                "https://legacy.awsapps.com/start",
                "us-east-1",
                "210987654321",
                "Auditor"
            )
        );
        assert_eq!(
            sign_in.session_name(),
            None,
            "the legacy shape names no session"
        );
        assert!(
            sign_in.scopes().is_empty(),
            "the legacy shape registers no scopes"
        );

        assert_eq!(
            configured(CONFIG, "plain")
                .sso()
                .expect("no sign-in is no refusal"),
            None
        );
    }

    #[test]
    fn a_sign_in_is_refused_naming_each_missing_key_and_an_undefined_session() {
        const CONFIG: &str = "\
[profile ghost]
sso_session = ghost
sso_account_id = 123456789012
sso_role_name = LakeReader

[profile roleless]
sso_session = corp
sso_account_id = 123456789012

[profile unregioned]
sso_session = regionless
sso_account_id = 123456789012
sso_role_name = LakeReader

[profile started]
sso_start_url = https://legacy.awsapps.com/start

[sso-session corp]
sso_start_url = https://corp.awsapps.com/start
sso_region = eu-west-1

[sso-session regionless]
sso_start_url = https://corp.awsapps.com/start
";
        let ghost = configured(CONFIG, "ghost");
        assert_eq!(ghost.sso_session_name(), None, "no section, no session");
        let message = refusal(ghost.sso());
        assert!(
            message.contains(
                "the profile ghost names sso_session ghost, which no [sso-session ghost] section defines"
            ),
            "{message}"
        );

        let message = refusal(configured(CONFIG, "roleless").sso());
        assert!(message.contains("the profile roleless"), "{message}");
        assert!(message.contains("without sso_role_name"), "{message}");
        assert!(
            !message.contains("sso_account_id"),
            "only what is missing: {message}"
        );

        let message = refusal(configured(CONFIG, "unregioned").sso());
        assert!(message.contains("without sso_region"), "{message}");

        let message = refusal(configured(CONFIG, "started").sso());
        assert!(
            message.contains("without sso_region, sso_account_id, sso_role_name"),
            "every missing key, in order: {message}"
        );
    }
}

mod words {
    use super::*;

    #[test]
    fn a_windows_command_line_keeps_its_backslashes_and_groups_by_double_quotes_alone() {
        let windows = |text: &str| split_command(text, true);
        assert_eq!(
            windows(r"C:\Tools\vault.exe export dev"),
            [r"C:\Tools\vault.exe", "export", "dev"],
            "a path's backslashes are literal"
        );
        assert_eq!(
            windows(r#""C:\Program Files\aws-vault.exe" exec  trading	--json"#),
            [
                r"C:\Program Files\aws-vault.exe",
                "exec",
                "trading",
                "--json"
            ],
            "quotes group, blanks and tabs separate"
        );
        assert_eq!(
            windows(r#"helper a\"b c\\"d e" f\\\"g"#),
            ["helper", r#"a"b"#, r"c\d e", r#"f\"g"#],
            "backslashes before a quote halve, and an odd one escapes it"
        );
        assert_eq!(
            windows(r"C:\dir\ next"),
            [r"C:\dir\", "next"],
            "a trailing backslash stays"
        );
        assert_eq!(
            windows(r#"a "" b"#),
            ["a", "", "b"],
            "an empty pair of quotes is a word"
        );
        assert_eq!(windows("it's"), ["it's"], "a single quote is a character");
        assert!(windows(" \t ").is_empty());
        assert_eq!(
            split_command(r"C:\Tools\vault.exe export", false),
            split_words(r"C:\Tools\vault.exe export"),
            "elsewhere the line is POSIX words"
        );
    }

    #[test]
    fn whitespace_separates_words_and_nothing_is_no_word() {
        assert_eq!(
            split_words("aws  sso\tlogin\n --profile desk"),
            ["aws", "sso", "login", "--profile", "desk"]
        );
        assert!(split_words("").is_empty());
        assert!(
            split_words(" \t ").is_empty(),
            "whitespace alone is no word"
        );
    }

    #[test]
    fn quotes_keep_whitespace_and_join_what_touches_them() {
        assert_eq!(
            split_words("'single quoted' \"double quoted\""),
            ["single quoted", "double quoted"]
        );
        assert_eq!(
            split_words(r#"pre'fix'ed "con"cat"#),
            ["prefixed", "concat"],
            "a quoted part joins the word it touches"
        );
        assert_eq!(
            split_words("'' x \"\""),
            ["", "x", ""],
            "an empty quote is a word"
        );
        assert_eq!(
            split_words("\"unterminated quote"),
            ["unterminated quote"],
            "an unclosed quote runs to the end"
        );
        assert_eq!(
            split_words(r#""$HOME" '$HOME'"#),
            ["$HOME", "$HOME"],
            "nothing is expanded"
        );
    }

    #[test]
    fn a_backslash_escapes_outside_quotes_a_few_inside_double_quotes_and_none_inside_single() {
        assert_eq!(
            split_words(r"a\ b c\\d"),
            ["a b", r"c\d"],
            "outside quotes a backslash keeps the next character"
        );
        assert_eq!(
            split_words(r#"'keeps \ and "'"#),
            [r#"keeps \ and ""#],
            "single quotes keep everything"
        );
        assert_eq!(
            split_words(r#""escapes \" \\ but not \$ \` \n""#),
            [r#"escapes " \ but not \$ \` \n"#],
            "double quotes escape only a quote and a backslash, as Python's shlex does"
        );
        assert_eq!(
            split_words(r"trailing\"),
            ["trailing"],
            "a final backslash escapes nothing"
        );
    }
}

mod legacy {
    use super::*;

    /// The variables a test states, and no other.
    fn variables(
        pairs: &'static [(&'static str, &'static str)],
    ) -> impl Fn(&str) -> Option<String> {
        move |name: &str| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| (*value).to_owned())
        }
    }

    const NONE: &[(&str, &str)] = &[];

    #[test]
    fn a_leading_tilde_is_the_home_directory_and_another_user_s_is_not_guessed() {
        let home = Path::new("/home/trader");
        let none = variables(NONE);
        assert_eq!(
            expand_path("~/.aws/config", &none, Some(home)),
            PathBuf::from("/home/trader/.aws/config")
        );
        assert_eq!(
            expand_path("~", &none, Some(home)),
            PathBuf::from("/home/trader")
        );
        assert_eq!(
            expand_path("~trader/.aws/config", &none, Some(home)),
            PathBuf::from("~trader/.aws/config"),
            "another user's home is not guessed"
        );
        assert_eq!(
            expand_path("/etc/aws/config", &none, Some(home)),
            PathBuf::from("/etc/aws/config")
        );
        assert_eq!(
            expand_path("config/~/x", &none, Some(home)),
            PathBuf::from("config/~/x"),
            "only a leading tilde"
        );
        assert_eq!(
            expand_path("~/.aws/config", &none, None),
            PathBuf::from("~/.aws/config"),
            "no home, no expansion"
        );
    }

    #[test]
    fn variables_expand_before_the_home_as_botocore_opens_aws_config_file() {
        let home = Path::new("/home/trader");
        let stated = variables(&[
            ("AWS_TEST_DIR", "/srv/ci"),
            ("TILDE", "~"),
            ("HOME", "/home/elsewhere"),
        ]);
        assert_eq!(
            expand_path("$AWS_TEST_DIR/alt/config", &stated, Some(home)),
            PathBuf::from("/srv/ci/alt/config")
        );
        assert_eq!(
            expand_path("${AWS_TEST_DIR}/alt/config", &stated, Some(home)),
            PathBuf::from("/srv/ci/alt/config")
        );
        assert_eq!(
            expand_path("$TILDE/.aws/config", &stated, Some(home)),
            PathBuf::from("/home/trader/.aws/config"),
            "a variable whose value opens with a tilde is a home, since the home is expanded after"
        );
        assert_eq!(
            expand_path("$HOME/.aws/config", &stated, Some(home)),
            PathBuf::from("/home/elsewhere/.aws/config"),
            "$HOME is a variable like any other, read from what the session reads"
        );
        assert_eq!(
            expand_path("$MISSING/config", &stated, Some(home)),
            PathBuf::from("$MISSING/config"),
            "an unknown variable is kept as written, as Python keeps it"
        );
    }

    #[test]
    fn posix_variables_expand_as_posixpath_does() {
        let stated = variables(&[("DIR", "/srv"), ("A_1", "one"), ("NESTED", "$DIR")]);
        let posix = |text: &str| expand_vars(text, &stated, false);
        assert_eq!(posix("$DIR/x"), "/srv/x");
        assert_eq!(posix("${DIR}/x"), "/srv/x");
        assert_eq!(
            posix("$A_1-$A_1"),
            "one-one",
            "a name is letters, digits and _"
        );
        assert_eq!(posix("${DIR"), "${DIR", "an unclosed brace is kept");
        assert_eq!(posix("${}x"), "${}x", "an empty name is no variable");
        assert_eq!(posix("$ $/x$"), "$ $/x$", "a dollar naming nothing is kept");
        assert_eq!(posix("$NESTED"), "$DIR", "a value is never scanned again");
        assert_eq!(posix("$$DIR"), "$/srv", "POSIX has no $$ escape");
        assert_eq!(
            posix("%DIR%/'$DIR'"),
            "%DIR%/'/srv'",
            "neither percent signs nor quotes mean anything on POSIX"
        );
        assert_eq!(posix("caf\u{e9}/$DIR"), "caf\u{e9}//srv");
    }

    #[test]
    fn windows_variables_expand_as_ntpath_does() {
        let stated = variables(&[
            ("USERPROFILE", r"C:\Users\trader"),
            ("DIR", r"D:\ci"),
            ("WITH-DASH", "dash"),
        ]);
        let windows = |text: &str| expand_vars(text, &stated, true);
        assert_eq!(
            windows(r"%USERPROFILE%\.aws\work-config"),
            r"C:\Users\trader\.aws\work-config"
        );
        assert_eq!(windows(r"$DIR\config"), r"D:\ci\config");
        assert_eq!(windows(r"${DIR}\config"), r"D:\ci\config");
        assert_eq!(windows("$WITH-DASH"), "dash", "a bare name takes a hyphen");
        assert_eq!(windows("%%DIR%%"), "%DIR%", "%% is one percent sign");
        assert_eq!(windows("$$DIR"), "$DIR", "$$ is one dollar");
        assert_eq!(windows("%MISSING%/x"), "%MISSING%/x");
        assert_eq!(windows("${MISSING}/x"), "${MISSING}/x");
        assert_eq!(windows("$MISSING/x"), "$MISSING/x");
        assert_eq!(
            windows("'$DIR'/$DIR"),
            r"'$DIR'/D:\ci",
            "a quoted run is kept, quotes included"
        );
        assert_eq!(
            windows("'$DIR"),
            "'$DIR",
            "an unclosed quote keeps the rest"
        );
        assert_eq!(
            windows("%DIR"),
            "%DIR",
            "an unclosed percent keeps the rest"
        );
        assert_eq!(
            windows("${DIR $DIR"),
            "${DIR $DIR",
            "an unclosed brace keeps the rest"
        );
        assert_eq!(windows("$"), "$");
    }

    #[test]
    #[cfg(windows)]
    fn expand_path_reads_percent_variables_on_windows() {
        let stated = variables(&[("USERPROFILE", r"C:\Users\trader")]);
        assert_eq!(
            expand_path(r"%USERPROFILE%\.aws\work-config", &stated, None),
            PathBuf::from(r"C:\Users\trader\.aws\work-config")
        );
    }

    #[test]
    #[cfg(not(windows))]
    fn expand_path_leaves_percent_signs_alone_off_windows() {
        let stated = variables(&[("USERPROFILE", "/home/trader")]);
        assert_eq!(
            expand_path("%USERPROFILE%/.aws/config", &stated, None),
            PathBuf::from("%USERPROFILE%/.aws/config")
        );
    }

    #[test]
    fn the_ec2_credential_file_spells_one_pair_one_key_per_line() {
        let text =
            "# the EC2 tools' file\nAWSAccessKeyId=AKIAEC2\nAWSSecretKey = ec2/secret+key=\n";
        assert_eq!(
            ec2_credential_file(text),
            Some(Credentials::new("AKIAEC2", "ec2/secret+key=")),
            "values are trimmed and split at the first equals sign"
        );
        assert_eq!(
            ec2_credential_file("AWSAccessKeyId=AKIAEC2\n"),
            None,
            "a key id without its secret"
        );
        assert_eq!(
            ec2_credential_file("AWSAccessKeyId=AKIAEC2\nAWSSecretKey=\n"),
            None,
            "an empty secret is no secret"
        );
        assert_eq!(
            ec2_credential_file("awsaccesskeyid=AKIAEC2\nawssecretkey=s\n"),
            None,
            "the two names are spelled exactly"
        );
        assert_eq!(ec2_credential_file(""), None);
    }

    #[test]
    fn a_boto_configuration_reads_its_credentials_section_alone() {
        let text = "\
[Boto]
debug = 0

[Credentials]
aws_access_key_id = AKIABOTO
aws_secret_access_key = boto-secret
";
        assert_eq!(
            boto_config(text),
            Some(Credentials::new("AKIABOTO", "boto-secret"))
        );
        assert_eq!(
            boto_config("[credentials]\nAWS_ACCESS_KEY_ID = AKIALOWER\naws_secret_access_key: s\n"),
            Some(Credentials::new("AKIALOWER", "s")),
            "the section name in any case, keys folded, either delimiter"
        );
        assert_eq!(
            boto_config("[Boto]\naws_access_key_id = AKIABOTO\naws_secret_access_key = s\n"),
            None,
            "a pair in another section is not the boto pair"
        );
        assert_eq!(boto_config(""), None);
    }
}
