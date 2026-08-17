use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use yggdryl::{Authority, MediaType, MimeType, Scheme, Uri, UriPath, Url, Urn};

#[test]
fn validated_components_are_concrete_canonical_values() {
    let scheme = Scheme::from_str("HTTPS").unwrap();
    let authority = Authority::from_str("user@example.test:8443").unwrap();
    let path = UriPath::from_str("/trades/2026/book.parquet").unwrap();

    assert_eq!(scheme.as_str(), "https");
    assert_eq!(authority.as_str(), "user@example.test:8443");
    assert_eq!(path.as_str(), "/trades/2026/book.parquet");
    assert_eq!(
        path.segments().collect::<Vec<_>>(),
        ["trades", "2026", "book.parquet"]
    );
    assert_eq!(path.file_name(), Some("book.parquet"));
    assert_eq!(path.extension(), Some("parquet"));
}

#[test]
fn uri_parsing_exposes_non_nullable_core_components() {
    let uri = Uri::from_str("HTTPS://example.test/archive/report.tar.gz?q=1#summary").unwrap();

    assert_eq!(uri.scheme().as_str(), "https");
    assert_eq!(uri.authority().as_str(), "example.test");
    assert_eq!(uri.path().as_str(), "/archive/report.tar.gz");
    assert_eq!(uri.query(), Some("q=1"));
    assert_eq!(uri.fragment(), Some("summary"));
    assert_eq!(
        uri.path_segments().collect::<Vec<_>>(),
        ["archive", "report.tar.gz"]
    );
    assert_eq!(uri.file_name(), Some("report.tar.gz"));
    assert_eq!(uri.extension(), Some("gz"));
    assert_eq!(uri.extensions().collect::<Vec<_>>(), ["tar", "gz"]);
    assert_eq!(Uri::from_str(&uri.to_string()).unwrap(), uri);
}

#[test]
fn windows_drive_and_unc_paths_normalize_independently_of_host_os() {
    let windows_path = std::path::PathBuf::from(r"C:\Users\Ada Lovelace\report.parquet");
    let drive = Uri::from_path(&windows_path).unwrap();
    assert_eq!(
        drive.to_string(),
        "file:///C:/Users/Ada%20Lovelace/report.parquet"
    );
    assert_eq!(drive.scheme().as_str(), "file");
    assert_eq!(drive.authority().as_str(), "");
    assert_eq!(
        drive.path().as_str(),
        "/C:/Users/Ada%20Lovelace/report.parquet"
    );
    assert!(!drive.to_string().contains('\\'));

    let unc = Uri::from_path(r"\\server\share\prices\ticks.csv").unwrap();
    assert_eq!(unc.to_string(), "file://server/share/prices/ticks.csv");
    assert_eq!(unc.authority().as_str(), "server");
    assert_eq!(unc.path().as_str(), "/share/prices/ticks.csv");
    assert_eq!(unc.extension(), Some("csv"));

    assert_eq!(
        Uri::from_str(r"file:///C:\Users\Ada\report.parquet")
            .unwrap()
            .to_string(),
        "file:///C:/Users/Ada/report.parquet"
    );
    assert_eq!(
        Uri::from_str(r"file://server\share\report.parquet")
            .unwrap()
            .to_string(),
        "file://server/share/report.parquet"
    );

    let prefixed = Uri::from_str(r"file:///c:\Ada%20Lovelace\report.csv?raw=1#rows").unwrap();
    assert_eq!(
        prefixed.to_string(),
        "file:///C:/Ada%20Lovelace/report.csv?raw=1#rows"
    );
    assert_eq!(prefixed.query(), Some("raw=1"));
    assert_eq!(prefixed.fragment(), Some("rows"));
    assert_eq!(
        Uri::from_str(r"file:c:\data\ticks.arrow")
            .unwrap()
            .to_string(),
        "file:///C:/data/ticks.arrow"
    );
    assert_eq!(
        Uri::from_str(r"file:/c:\data\ticks.arrow")
            .unwrap()
            .to_string(),
        "file:///C:/data/ticks.arrow"
    );
    assert!(Uri::from_str(r"file:///C:\bad%GG\report.csv").is_err());
}

#[test]
fn slash_paths_and_backslash_relative_paths_receive_file_scheme() {
    assert_eq!(
        Uri::from_path("/var/lib/data.arrow").unwrap().to_string(),
        "file:///var/lib/data.arrow"
    );
    assert_eq!(
        Uri::from_path(r"relative\folder\data.arrow")
            .unwrap()
            .to_string(),
        "file:relative/folder/data.arrow"
    );
    assert_eq!(
        Uri::from_path("///var/lib/data.arrow").unwrap().to_string(),
        "file:///var/lib/data.arrow"
    );
    assert_eq!(
        Uri::from_path(r"c:\data\ticks.arrow").unwrap(),
        Uri::from_path(r"C:\data\ticks.arrow").unwrap()
    );
}

#[test]
fn file_identifiers_round_trip_through_utf8_platform_paths() {
    let source = PathBuf::from(r"C:\Users\Ada Lovelace\café.arrow");
    let uri = Uri::from_path(&source).unwrap();
    let path = uri.to_path().unwrap();
    assert_eq!(
        path.to_string_lossy().replace('\\', "/"),
        "C:/Users/Ada Lovelace/café.arrow"
    );
    assert_eq!(uri.clone().into_path().unwrap(), path);
    assert_eq!(PathBuf::try_from(&uri).unwrap(), path);
    assert_eq!(PathBuf::try_from(uri.clone()).unwrap(), path);

    let rebuilt = Uri::try_from(path.clone()).unwrap();
    assert_eq!(rebuilt, uri);
    assert_eq!(Uri::try_from(path.as_path()).unwrap(), uri);
    assert_eq!(Uri::try_from(&path).unwrap(), uri);

    let url = uri.to_url().unwrap();
    assert_eq!(url.to_path().unwrap(), path);
    assert_eq!(PathBuf::try_from(&url).unwrap(), path);
    assert_eq!(Url::try_from(path.clone()).unwrap(), url);
    assert_eq!(Url::try_from(path.as_path()).unwrap(), url);
    assert_eq!(Url::try_from(&path).unwrap(), url);
    assert_eq!(url.clone().into_path().unwrap(), path);

    let unc = Uri::from_path(r"\\server\share\prices 2026\ticks.csv").unwrap();
    assert_eq!(
        unc.to_path().unwrap().to_string_lossy().replace('\\', "/"),
        "//server/share/prices 2026/ticks.csv"
    );
    assert_eq!(
        Uri::from_str("file://server")
            .unwrap()
            .to_path()
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/"),
        "//server/"
    );
    let relative = Uri::from_path(Path::new("relative/folder/data.arrow")).unwrap();
    assert_eq!(
        relative.to_path().unwrap(),
        PathBuf::from("relative/folder/data.arrow")
    );
    let posix = Uri::from_path(Path::new("/var/lib/café.arrow")).unwrap();
    assert_eq!(
        posix
            .to_path()
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/"),
        "/var/lib/café.arrow"
    );
}

#[test]
fn encoded_unc_authorities_round_trip_through_all_path_conversions() {
    let cases = [
        (
            "file://caf%C3%A9/share/x.arrow",
            "//caf\u{e9}/share/x.arrow",
        ),
        (
            "file://server%20name/share/x.arrow",
            "//server name/share/x.arrow",
        ),
        (
            "file://server%25name/share/x.arrow",
            "//server%name/share/x.arrow",
        ),
        (
            "file://[fe80::1%25zone]/share/x.arrow",
            "//[fe80::1%zone]/share/x.arrow",
        ),
        (
            "file://server:445/share/x.arrow",
            "//server:445/share/x.arrow",
        ),
    ];

    for (text, expected_path) in cases {
        let uri = Uri::from_str(text).unwrap();
        let path = uri.to_path().unwrap();
        assert_eq!(path.to_string_lossy().replace('\\', "/"), expected_path);
        assert_eq!(uri.clone().into_path().unwrap(), path);
        assert_eq!(PathBuf::try_from(&uri).unwrap(), path);
        assert_eq!(PathBuf::try_from(uri.clone()).unwrap(), path);

        assert_eq!(Uri::from_path(&path).unwrap(), uri);
        assert_eq!(Uri::try_from(path.as_path()).unwrap(), uri);
        assert_eq!(Uri::try_from(&path).unwrap(), uri);
        assert_eq!(Uri::try_from(path.clone()).unwrap(), uri);

        let url = uri.to_url().unwrap();
        assert_eq!(url.to_path().unwrap(), path);
        assert_eq!(url.clone().into_path().unwrap(), path);
        assert_eq!(PathBuf::try_from(&url).unwrap(), path);
        assert_eq!(PathBuf::try_from(url.clone()).unwrap(), path);
        assert_eq!(Url::from_path(&path).unwrap(), url);
        assert_eq!(Url::try_from(path.as_path()).unwrap(), url);
        assert_eq!(Url::try_from(&path).unwrap(), url);
        assert_eq!(Url::try_from(path).unwrap(), url);
    }

    assert_eq!(
        Uri::from_path(r"\\server%name\share\x.arrow")
            .unwrap()
            .to_string(),
        "file://server%25name/share/x.arrow"
    );
}

#[test]
fn encoded_drive_colons_cannot_change_file_path_structure() {
    for (text, expected_position) in [
        ("file:/C%3A/secret.arrow", 2),
        ("file:C%3A/secret.arrow", 1),
        ("file:///C%3a/secret.arrow", 2),
        ("file:///%43%3A/secret.arrow", 1),
        ("file:///%43:/secret.arrow", 1),
        ("file:%43%3A/secret.arrow", 0),
        ("file:%43:/secret.arrow", 0),
    ] {
        let uri = Uri::from_str(text).unwrap();
        let error = uri.to_path().unwrap_err();
        assert!(
            matches!(
                error,
                yggdryl::Error::Parse {
                    target: "file URI path",
                    position,
                    ..
                } if position == expected_position
            ),
            "{error:?}"
        );
        assert!(uri.clone().into_path().is_err());
        assert!(PathBuf::try_from(&uri).is_err());
        assert!(PathBuf::try_from(uri.clone()).is_err());

        if let Ok(url) = uri.to_url() {
            assert!(url.to_path().is_err());
            assert!(url.clone().into_path().is_err());
            assert!(PathBuf::try_from(&url).is_err());
            assert!(PathBuf::try_from(url).is_err());
        }
    }

    let canonical = Uri::from_str("file:///C:/secret.arrow").unwrap();
    assert_eq!(
        Uri::from_path(canonical.to_path().unwrap()).unwrap(),
        canonical
    );
}

#[test]
fn escaped_ascii_authority_syntax_cannot_change_unc_structure() {
    for text in [
        "file://server%3A445/share/x.arrow",
        "file://user%40host/share/x.arrow",
        "file://%5B%3A%3A1%5D/share/x.arrow",
        "file://%41/share/x.arrow",
        "file://server%2Dname/share/x.arrow",
        "file://server%21name/share/x.arrow",
    ] {
        let uri = Uri::from_str(text).unwrap();
        assert!(
            matches!(
                uri.to_path(),
                Err(yggdryl::Error::Parse {
                    target: "file URI authority",
                    ..
                })
            ),
            "{text}"
        );
    }

    for text in [
        "file://caf%C3%A9/share/x.arrow",
        "file://server%20name/share/x.arrow",
        "file://server%25name/share/x.arrow",
        "file://[fe80::1%25eth0]/share/x.arrow",
    ] {
        let uri = Uri::from_str(text).unwrap();
        assert_eq!(
            Uri::from_path(uri.to_path().unwrap()).unwrap(),
            uri,
            "{text}"
        );
    }
}

#[test]
fn file_path_projection_rejects_ambiguous_or_non_utf8_escapes() {
    assert!(
        Uri::from_str("https://example.test/file.arrow")
            .unwrap()
            .to_path()
            .is_err()
    );
    assert!(
        Uri::from_str("file:///tmp/a%2Fb.arrow")
            .unwrap()
            .to_path()
            .is_err()
    );
    assert!(
        Uri::from_str("file:///tmp/a%5Cb.arrow")
            .unwrap()
            .to_path()
            .is_err()
    );
    assert!(
        Uri::from_str("file:///tmp/%FF.arrow")
            .unwrap()
            .to_path()
            .is_err()
    );
    let invalid_utf8 = Uri::from_str("file:///tmp/%41%FF.arrow")
        .unwrap()
        .to_path()
        .unwrap_err();
    assert!(matches!(
        invalid_utf8,
        yggdryl::Error::Parse {
            target: "file URI path",
            position: 8,
            ..
        }
    ));
    assert!(
        Uri::from_str("file:///tmp/%00.arrow")
            .unwrap()
            .to_path()
            .is_err()
    );
    for text in [
        "file:///tmp/%01/x.arrow",
        "file:///tmp/%7F/x.arrow",
        "file:///tmp/%C2%85/x.arrow",
        "file://server%01/share/x.arrow",
    ] {
        assert!(Uri::from_str(text).unwrap().to_path().is_err(), "{text}");
    }
    let tab = Uri::from_str("file:///tmp/a%09b.arrow").unwrap();
    assert_eq!(Uri::from_path(tab.to_path().unwrap()).unwrap(), tab);
    assert!(
        Uri::from_str("file:///tmp/data.arrow?download=true")
            .unwrap()
            .to_path()
            .is_err()
    );
    assert!(
        Uri::from_str("file:///tmp/data.arrow#rows")
            .unwrap()
            .to_path()
            .is_err()
    );
    assert!(Uri::from_str("file:").unwrap().to_path().is_err());
}

#[test]
fn path_suffix_rules_ignore_directories_trailing_slashes_and_dotfiles() {
    let compound = UriPath::from_str("/archive.v1/report.tar.zst").unwrap();
    assert_eq!(compound.file_name(), Some("report.tar.zst"));
    assert_eq!(compound.extension(), Some("zst"));
    assert_eq!(compound.extensions().collect::<Vec<_>>(), ["tar", "zst"]);

    let hidden_compound = UriPath::from_str("/.env.local").unwrap();
    assert_eq!(hidden_compound.extension(), Some("local"));
    assert_eq!(hidden_compound.extensions().collect::<Vec<_>>(), ["local"]);

    for path in ["", "/", "/directory/", "/.env", "/name."] {
        let path = UriPath::from_str(path).unwrap();
        assert_eq!(path.extension(), None, "{path}");
        assert_eq!(path.extensions().count(), 0, "{path}");
    }
}

#[test]
fn path_stem_and_extension_mutations_are_canonical_and_atomic() {
    let mut path = UriPath::from_str("/archive/report.tar.gz").unwrap();
    assert_eq!(path.stem(), Some("report.tar"));

    path.set_stem("renamed%2efile").unwrap();
    assert_eq!(path.as_str(), "/archive/renamed%2Efile.gz");
    assert_eq!(path.stem(), Some("renamed%2Efile"));

    path.set_extension("zst").unwrap();
    assert_eq!(path.as_str(), "/archive/renamed%2Efile.zst");
    path.set_extensions(["csv", "gz"]).unwrap();
    assert_eq!(path.as_str(), "/archive/renamed%2Efile.csv.gz");
    assert!(path.remove_extension());
    assert_eq!(path.as_str(), "/archive/renamed%2Efile.csv");
    assert!(path.clear_extensions());
    assert_eq!(path.as_str(), "/archive/renamed%2Efile");
    assert!(!path.remove_extension());
    assert!(!path.clear_extensions());

    let mut hidden = UriPath::from_str("/.env.local").unwrap();
    assert_eq!(hidden.stem(), Some(".env"));
    hidden.set_extension("json").unwrap();
    assert_eq!(hidden.as_str(), "/.env.json");
    assert!(hidden.clear_extensions());
    assert_eq!(hidden.as_str(), "/.env");
    assert_eq!(hidden.stem(), Some(".env"));

    assert_eq!(UriPath::from_str("/file.").unwrap().stem(), Some("file."));
    assert_eq!(
        UriPath::from_str("/file..gz").unwrap().stem(),
        Some("file.")
    );

    let original = path.clone();
    for invalid in [
        "",
        "nested/name",
        "bad?name",
        "bad#name",
        "bad%",
        "market data.csv",
        "café.csv",
    ] {
        assert!(path.set_file_name(invalid).is_err(), "{invalid:?}");
        assert_eq!(path, original);
    }
    for invalid in ["", ".gz", "tar.gz", "bad/name", "bad%"] {
        assert!(path.set_extension(invalid).is_err(), "{invalid:?}");
        assert_eq!(path, original);
    }
    assert!(path.set_extensions(["json", "bad/name"]).is_err());
    assert_eq!(path, original);
}

#[test]
fn identifier_filename_mutations_preserve_non_path_components() {
    let mut authority_only = Url::from_str("https://example.test?q=1#part").unwrap();
    authority_only.set_file_name("data.json").unwrap();
    assert_eq!(
        authority_only.to_string(),
        "https://example.test/data.json?q=1#part"
    );

    let mut trailing = Uri::from_str("https://example.test/archive/?q=1#part").unwrap();
    trailing.set_file_name("report%20final.csv").unwrap();
    assert_eq!(
        trailing.to_string(),
        "https://example.test/archive/report%20final.csv?q=1#part"
    );
    assert_eq!(trailing.stem(), Some("report%20final"));
    trailing.set_extensions(["json", "gz"]).unwrap();
    assert_eq!(
        trailing.to_string(),
        "https://example.test/archive/report%20final.json.gz?q=1#part"
    );

    let unchanged = trailing.clone();
    assert!(trailing.set_stem("bad/name").is_err());
    assert_eq!(trailing, unchanged);
}

#[test]
fn urn_filename_mutations_are_scoped_to_the_namespace_specific_string() {
    let mut urn = Urn::from_str("urn:example:reports/data.csv?=raw#rows").unwrap();
    assert_eq!(urn.namespace(), "example");
    assert_eq!(urn.file_name(), Some("data.csv"));
    assert_eq!(urn.stem(), Some("data"));

    urn.set_file_name("renamed.json").unwrap();
    assert_eq!(
        urn.to_string(),
        "urn:example:reports/renamed.json?=raw#rows"
    );
    urn.set_extensions(["csv", "gz"]).unwrap();
    assert_eq!(
        urn.to_string(),
        "urn:example:reports/renamed.csv.gz?=raw#rows"
    );
    assert!(urn.remove_extension());
    assert_eq!(urn.to_string(), "urn:example:reports/renamed.csv?=raw#rows");
    assert!(urn.clear_extensions());
    assert_eq!(urn.to_string(), "urn:example:reports/renamed?=raw#rows");
    assert_eq!(urn.namespace(), "example");

    let mut flat = Urn::from_str("urn:example:data.csv").unwrap();
    flat.set_stem("other").unwrap();
    assert_eq!(flat.to_string(), "urn:example:other.csv");
    let unchanged = flat.clone();
    assert!(flat.set_file_name("bad/name").is_err());
    assert_eq!(flat, unchanged);
    assert_eq!(flat.namespace(), "example");
}

#[test]
fn identifiers_infer_and_apply_mime_and_encoded_media_suffixes() {
    let path = UriPath::from_str("/archive/report.csv.gz.zst").unwrap();
    assert_eq!(path.mime_type(), MimeType::ZSTD);
    let media = path.media_type();
    assert_eq!(media.base(), &MimeType::CSV);
    assert_eq!(media.encodings(), &[MimeType::GZIP, MimeType::ZSTD]);
    assert_eq!(media.extensions().collect::<Vec<_>>(), ["csv", "gz", "zst"]);

    assert_eq!(
        UriPath::from_str("/archive/unknown.custom")
            .unwrap()
            .media_type()
            .base(),
        &MimeType::OCTET_STREAM
    );
    assert_eq!(
        UriPath::from_str("/archive/no-extension")
            .unwrap()
            .mime_type(),
        MimeType::OCTET_STREAM
    );
    let archive = UriPath::from_str("/archive/report.tar.gz")
        .unwrap()
        .media_type();
    assert_eq!(archive.base(), &MimeType::TAR);
    assert_eq!(archive.encodings(), &[MimeType::GZIP]);
    let zip = UriPath::from_str("/archive/report.zip")
        .unwrap()
        .media_type();
    assert_eq!(zip.base(), &MimeType::ZIP);
    assert!(!zip.is_encoded());

    let mut uri = Uri::from_str("https://example.test/report.csv.gz?q=1#part").unwrap();
    assert_eq!(uri.mime_type(), MimeType::GZIP);
    uri.set_mime_type(MimeType::JSON).unwrap();
    assert_eq!(
        uri.to_string(),
        "https://example.test/report.csv.json?q=1#part"
    );

    let encoded = MediaType::from_parts(MimeType::CSV, [MimeType::GZIP, MimeType::ZSTD]).unwrap();
    uri.set_media_type(encoded).unwrap();
    assert_eq!(
        uri.to_string(),
        "https://example.test/report.csv.gz.zst?q=1#part"
    );

    let custom = MimeType::from_str("application/vnd.example").unwrap();
    let unchanged = uri.clone();
    assert!(uri.set_mime_type(custom.clone()).is_err());
    assert_eq!(uri, unchanged);
    assert!(
        uri.set_media_type(MediaType::from_parts(custom, [MimeType::GZIP]).unwrap())
            .is_err()
    );
    assert_eq!(uri, unchanged);

    uri.set_mime_type(MimeType::from_str("application/vnd.example+json").unwrap())
        .unwrap();
    assert_eq!(
        uri.to_string(),
        "https://example.test/report.csv.gz.json?q=1#part"
    );

    let mut urn = Urn::from_str("urn:example:reports/data.json").unwrap();
    urn.set_media_type(MediaType::from_parts(MimeType::CSV, [MimeType::GZIP]).unwrap())
        .unwrap();
    assert_eq!(urn.to_string(), "urn:example:reports/data.csv.gz");
    assert_eq!(urn.namespace(), "example");
}

#[test]
fn url_conversion_validates_hierarchical_network_urls() {
    let uri = Uri::from_str("https://example.test/a/data.json?raw=true").unwrap();
    let url = Url::from_uri(uri.clone()).unwrap();

    assert_eq!(url.scheme().as_str(), "https");
    assert_eq!(url.authority().as_str(), "example.test");
    assert_eq!(url.path_segments().collect::<Vec<_>>(), ["a", "data.json"]);
    assert_eq!(url.extension(), Some("json"));
    assert_eq!(url.to_uri(), uri);
    assert_eq!(url.clone().into_uri(), uri);
    assert_eq!(Url::try_from(&uri).unwrap(), url);
    assert_eq!(Uri::from(&url), uri);

    assert!(Url::from_str("https:///missing-authority").is_err());
    assert!(Url::from_str("mailto:user@example.test").is_err());
    assert!(Url::from_str("urn:isbn:9780131103627").is_err());
}

#[test]
fn urn_conversion_preserves_namespace_and_namespace_specific_string() {
    let uri = Uri::from_str("urn:isbn:9780131103627#edition-2").unwrap();
    let urn = Urn::from_uri(uri.clone()).unwrap();

    assert_eq!(urn.scheme().as_str(), "urn");
    assert_eq!(urn.authority().as_str(), "");
    assert_eq!(urn.namespace(), "isbn");
    assert_eq!(urn.namespace_specific(), "9780131103627");
    assert_eq!(urn.fragment(), Some("edition-2"));
    assert_eq!(urn.to_uri(), uri);
    assert_eq!(urn.clone().into_uri(), uri);
    assert_eq!(uri.to_urn().unwrap(), urn);
    assert_eq!(uri.clone().into_urn().unwrap(), urn);
    assert_eq!(Urn::try_from(&uri).unwrap(), urn);
    assert_eq!(Uri::from(&urn), uri);
    assert_eq!(Urn::from_str(&urn.to_string()).unwrap(), urn);
    assert!(Urn::from_str("urn:example:value?+resolve?=query#part").is_ok());
    assert!(Urn::from_str("urn:example:value?=query").is_ok());

    for invalid in [
        "urn:x:value",
        "urn:-bad:value",
        "urn:bad-:value",
        "urn:isbn:",
        "urn:example:value?plain-query",
        "urn:example:value?+",
        "urn:example:value?=",
    ] {
        assert!(Urn::from_str(invalid).is_err(), "{invalid}");
    }
    assert!(Urn::from_uri(Uri::from_str("https://example.test").unwrap()).is_err());
}

#[test]
fn malformed_identifiers_report_parse_errors_instead_of_falling_back() {
    for invalid in [
        "https://example.test/%GG",
        "https://example.test/space here",
        "https://example.test/path\\part",
        "https://example.test/#bad fragment",
    ] {
        assert!(Uri::from_str(invalid).is_err(), "{invalid}");
    }

    // A token before the colon that is not a valid scheme is still an error;
    // the path fallback applies only when there is no scheme token at all.
    for bad_scheme in ["1http://example.test", "💥file:/tmp", "東京:/data"] {
        assert!(Uri::from_str(bad_scheme).is_err(), "{bad_scheme}");
    }

    // Scheme-less input is a filesystem path, not a malformed URI.
    for path_like in ["noscheme.example/path", "ééé", "/data/x.parquet"] {
        let parsed = Uri::from_str(path_like).unwrap_or_else(|error| {
            panic!("expected {path_like:?} to parse as a file path, got {error}")
        });
        assert_eq!(parsed.scheme(), &Scheme::FILE, "{path_like}");
    }

    assert!(Authority::from_str("[::1").is_err());
    assert!(Authority::from_str("::1").is_err());
    assert!(Authority::from_str("host:not-a-port").is_err());
    for malformed in [
        "[not-an-ip]",
        "[::gg]",
        "[vG.nope]",
        "host]",
        "foo[bar]",
        "[foo@bar]",
    ] {
        assert!(Authority::from_str(malformed).is_err(), "{malformed}");
    }
    assert_eq!(
        Uri::from_str("https://[2001:db8::1]:8443/a")
            .unwrap()
            .authority()
            .as_str(),
        "[2001:db8::1]:8443"
    );
    assert!(Uri::from_str("https://[fe80::1%25eth0]/").is_ok());
    assert!(Uri::from_str("https://[v1.fe80]/").is_ok());
}

#[test]
fn structural_json_and_native_value_traits_round_trip() {
    let uri = Uri::from_str("https://example.test/a/b.csv?q=1#rows").unwrap();
    let value = serde_json::to_value(&uri).unwrap();
    assert_eq!(value["scheme"], "https");
    assert_eq!(value["authority"], "example.test");
    assert_eq!(value["path"], "/a/b.csv");
    assert!(!value["scheme"].is_null());
    assert!(!value["authority"].is_null());
    assert!(!value["path"].is_null());
    assert_eq!(Uri::from_json(&uri.to_json().unwrap()).unwrap(), uri);
    assert_eq!(
        Uri::from_json(&uri.clone().into_json().unwrap()).unwrap(),
        uri
    );

    let url = Url::from_uri(uri.clone()).unwrap();
    assert_eq!(Url::from_json(&url.to_json().unwrap()).unwrap(), url);
    let urn = Urn::from_str("urn:uuid:123e4567-e89b-12d3-a456-426614174000").unwrap();
    assert_eq!(Urn::from_json(&urn.to_json().unwrap()).unwrap(), urn);

    let ordered = BTreeSet::from([uri.clone()]);
    let hashed = HashSet::from([uri.clone()]);
    assert!(ordered.contains(&uri));
    assert!(hashed.contains(&uri));
    assert_eq!(
        uri.stable_hash(),
        Uri::from_str(&uri.to_string()).unwrap().stable_hash()
    );

    assert_eq!((&uri).into_iter().collect::<Vec<_>>(), ["a", "b.csv"]);
    assert_eq!(uri.path().segment_len(), 2);
    assert_eq!(uri.path().get_segment(1), Some("b.csv"));
    assert!(uri.path().contains_segment("a"));
    let (cursor, first) = uri.next_path_segment(0).unwrap();
    let (cursor, second) = uri.next_path_segment(cursor).unwrap();
    assert_eq!((first, second), ("a", "b.csv"));
    assert!(uri.next_path_segment(cursor).is_none());

    for malformed in [
        r#"{"scheme":"https","authority":"example.test","path":"/","has_authority":false,"query":null,"fragment":null}"#,
        r#"{"scheme":"https","authority":"example.test","path":"/","has_authority":true,"query":null,"fragment":null,"unknown":true}"#,
        r#"{"scheme":"1bad","authority":"","path":"","has_authority":false,"query":null,"fragment":null}"#,
    ] {
        assert!(Uri::from_json(malformed).is_err(), "{malformed}");
    }

    let parts = Uri::from_parts(
        Scheme::from_str("file").unwrap(),
        Authority::from_str("").unwrap(),
        UriPath::from_str("/c:/data.arrow").unwrap(),
        None,
        None,
    )
    .unwrap();
    assert_eq!(parts.to_string(), "file:///C:/data.arrow");
    assert_eq!(Uri::from_str(&parts.to_string()).unwrap(), parts);

    let drive_authority = Uri::from_json(
        r#"{"scheme":"file","authority":"c:","path":"/data.arrow","has_authority":true,"query":null,"fragment":null}"#,
    )
    .unwrap();
    assert_eq!(drive_authority.to_string(), "file:///C:/data.arrow");
    assert_eq!(
        Uri::from_json(&drive_authority.to_json().unwrap()).unwrap(),
        drive_authority
    );
}

#[test]
fn urn_namespace_errors_report_the_offending_original_byte() {
    let error = Urn::from_str("urn:a$:value").unwrap_err();
    assert!(
        matches!(
            error,
            yggdryl::Error::Parse {
                target: "urn",
                position: 5,
                ..
            }
        ),
        "{error:?}"
    );
}

#[test]
fn unc_server_errors_report_original_path_offsets() {
    let error = Uri::from_path("\\\\caf\u{e9}:port\\share").unwrap_err();
    assert!(
        matches!(
            error,
            yggdryl::Error::Parse {
                target: "path",
                position: 8,
                ..
            }
        ),
        "{error:?}"
    );
}

#[test]
fn a_scheme_less_string_falls_back_to_the_file_scheme() {
    // An absolute path becomes a rooted `file:` URL.
    let absolute = Uri::from_str("/data/trades.parquet").unwrap();
    assert_eq!(absolute.scheme(), &Scheme::FILE);
    assert_eq!(absolute.to_string(), "file:///data/trades.parquet");

    // A relative path stays relative under the same scheme.
    let relative = Uri::from_str("data/trades.parquet").unwrap();
    assert_eq!(relative.scheme(), &Scheme::FILE);
    assert_eq!(relative.path().as_str(), "data/trades.parquet");

    // A colon after the first separator is data, not a scheme.
    let colon_in_path = Uri::from_str("/data/2024-01-01T00:00:00/part.parquet").unwrap();
    assert_eq!(colon_in_path.scheme(), &Scheme::FILE);
    assert!(colon_in_path.path().as_str().contains("00:00:00"));

    // A real scheme still wins.
    let explicit = Uri::from_str("s3://bucket/key").unwrap();
    assert_eq!(explicit.scheme(), &Scheme::S3);
}

#[test]
fn parts_resolves_dot_and_dot_dot() {
    let path = UriPath::from_str("/a/b/../c/./d").unwrap();
    assert_eq!(path.parts(), vec!["a", "c", "d"]);

    // `..` past an absolute root is clamped, matching filesystem semantics.
    let escaping = UriPath::from_str("/../../a").unwrap();
    assert_eq!(escaping.parts(), vec!["a"]);

    // A relative path has no root to clamp against, so `..` is retained.
    let relative = UriPath::from_str("../../a").unwrap();
    assert_eq!(relative.parts(), vec!["..", "..", "a"]);
}

#[test]
fn joinpath_composes_like_a_shell_cd() {
    let base = UriPath::from_str("/warehouse/db").unwrap();
    assert_eq!(
        base.joinpath("table/data").unwrap().as_str(),
        "/warehouse/db/table/data"
    );
    assert_eq!(
        base.joinpath("../other").unwrap().as_str(),
        "/warehouse/other"
    );
    assert_eq!(
        base.joinpath("./here").unwrap().as_str(),
        "/warehouse/db/here"
    );

    // An absolute join replaces the path outright.
    assert_eq!(base.joinpath("/root").unwrap().as_str(), "/root");
}

#[test]
fn parents_walks_up_to_the_root_without_yielding_self() {
    let path = UriPath::from_str("/a/b/c").unwrap();
    let parents: Vec<String> = path
        .parents()
        .map(|value| value.as_str().to_owned())
        .collect();
    assert_eq!(
        parents,
        vec!["/a/b".to_owned(), "/a".to_owned(), "/".to_owned()]
    );

    assert_eq!(path.parent().unwrap().as_str(), "/a/b");
    assert_eq!(UriPath::from_str("/").unwrap().parent(), None);
}

#[test]
fn uri_and_url_navigation_preserve_every_other_component() {
    let url = Url::from_str("https://example.com/a/b/c?q=1#frag").unwrap();
    let joined = url.joinpath("../d").unwrap();
    assert_eq!(joined.path().as_str(), "/a/b/d");
    assert_eq!(joined.query(), Some("q=1"));
    assert_eq!(joined.fragment(), Some("frag"));
    assert_eq!(joined.authority().as_str(), "example.com");

    let parents: Vec<String> = url
        .parents()
        .map(|value| value.path().as_str().to_owned())
        .collect();
    assert_eq!(
        parents,
        vec!["/a/b".to_owned(), "/a".to_owned(), "/".to_owned()]
    );
    assert_eq!(url.parts(), vec!["a", "b", "c"]);
}

#[test]
fn default_port_is_reported_per_scheme() {
    assert_eq!(
        Url::from_str("http://example.com").unwrap().default_port(),
        Some(80)
    );
    assert_eq!(
        Url::from_str("https://example.com").unwrap().default_port(),
        Some(443)
    );
    assert_eq!(
        Uri::from_str("postgres://host/db").unwrap().default_port(),
        Some(5432)
    );
    assert_eq!(
        Uri::from_str("mysql://host/db").unwrap().default_port(),
        Some(3306)
    );
    // Object stores and metadata namespaces have no fixed listening port.
    assert_eq!(
        Uri::from_str("s3://bucket/key").unwrap().default_port(),
        None
    );
    assert_eq!(Uri::from_str("file:///tmp/x").unwrap().default_port(), None);
}
