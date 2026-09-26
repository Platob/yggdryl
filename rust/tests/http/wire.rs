//! `rust/src/http/wire.rs`.

use std::io::{self, Cursor, Read, Write};

use yggdryl::Error;
use yggdryl::http::{
    Headers, HttpVersion, Method, RequestHead, ResponseHead, Status, decode_chunked,
    encode_chunked, parse_request, parse_response, render_request, render_response,
};

/// The position and reason of a refusal with target `http message`.
fn refusal(error: &Error) -> (usize, String) {
    match error {
        Error::Parse {
            target: "http message",
            position,
            reason,
        } => (*position, reason.to_string()),
        other => panic!("expected an `http message` parse error, got {other:?}"),
    }
}

fn request_refusal(wire: &[u8]) -> (usize, String) {
    let error = parse_request(wire).expect_err("a refusal");
    refusal(&error)
}

fn response_refusal(wire: &[u8]) -> (usize, String) {
    let error = parse_response(wire).expect_err("a refusal");
    refusal(&error)
}

fn headers(pairs: &[(&str, &str)]) -> Headers {
    Headers::from_entries(pairs.iter().copied()).unwrap()
}

mod refusals {
    use super::*;

    #[test]
    fn obsolete_line_folding_is_refused_where_the_folded_line_starts() {
        let (position, reason) = request_refusal(b"GET / HTTP/1.1\r\nX-A: 1\r\n b\r\n\r\n");
        assert_eq!(position, 24);
        assert!(reason.contains("folding"), "{reason}");
        let (position, _) = request_refusal(b"GET / HTTP/1.1\r\nX-A: 1\r\n\tb\r\n\r\n");
        assert_eq!(position, 24);
    }

    #[test]
    fn a_bare_cr_inside_a_line_is_refused_where_it_lies() {
        let (position, reason) = request_refusal(b"GET / HTTP/1.1\r\nX-A: a\rb\r\n\r\n");
        assert_eq!(position, 22);
        assert!(reason.contains("bare CR"), "{reason}");
        // In the request line too.
        let (position, _) = request_refusal(b"GET /\r HTTP/1.1\r\n\r\n");
        assert_eq!(position, 5);
    }

    #[test]
    fn a_bare_lf_ends_the_line_so_what_follows_must_be_a_field_line() {
        let (position, reason) = request_refusal(b"GET / HTTP/1.1\r\nX-A: a\nb\r\n\r\n");
        assert_eq!(position, 23);
        assert!(reason.contains("`name: value`"), "{reason}");
    }

    #[test]
    fn a_line_above_8_kib_is_refused_before_it_is_read() {
        let mut wire = b"GET /".to_vec();
        wire.extend(std::iter::repeat_n(b'a', 8200));
        wire.extend_from_slice(b" HTTP/1.1\r\n\r\n");
        let (position, reason) = request_refusal(&wire);
        assert_eq!(position, 0);
        assert!(reason.contains("8192"), "{reason}");

        // A field line of exactly 8192 bytes is read; one byte more is not.
        let head = b"GET / HTTP/1.1\r\n";
        let value_len = 8192 - "X-A: ".len();
        let mut wire = head.to_vec();
        wire.extend_from_slice(b"X-A: ");
        wire.extend(std::iter::repeat_n(b'v', value_len));
        wire.extend_from_slice(b"\r\n\r\n");
        let (parsed, _) = parse_request(&wire).unwrap();
        assert_eq!(parsed.headers.get("x-a").map(str::len), Some(value_len));

        let mut wire = head.to_vec();
        wire.extend_from_slice(b"X-A: ");
        wire.extend(std::iter::repeat_n(b'v', value_len + 1));
        wire.extend_from_slice(b"\r\n\r\n");
        let (position, _) = request_refusal(&wire);
        assert_eq!(position, head.len());

        // A long line with no terminator at all is the same refusal, not a truncation.
        let mut wire = b"GET /".to_vec();
        wire.extend(std::iter::repeat_n(b'a', 9000));
        let (position, reason) = request_refusal(&wire);
        assert_eq!(position, 0);
        assert!(reason.contains("8192"), "{reason}");
    }

    #[test]
    fn more_than_256_field_lines_are_refused_at_the_257th() {
        let mut wire = b"GET / HTTP/1.1\r\n".to_vec();
        for index in 0..256 {
            wire.extend_from_slice(format!("X-{index}: v\r\n").as_bytes());
        }
        let mut accepted = wire.clone();
        accepted.extend_from_slice(b"\r\n");
        let (head, _) = parse_request(&accepted).unwrap();
        assert_eq!(head.headers.len(), 256);

        let at = wire.len();
        wire.extend_from_slice(b"X-256: v\r\n\r\n");
        let (position, reason) = request_refusal(&wire);
        assert_eq!(position, at);
        assert!(reason.contains("256"), "{reason}");
    }

    #[test]
    fn a_content_length_that_is_not_a_decimal_is_refused_at_its_line() {
        for value in [
            "abc",
            "-1",
            "1e3",
            "",
            "18446744073709551616",
            "5 5",
            "0x10",
            "+5",
        ] {
            let wire = format!("GET / HTTP/1.1\r\nContent-Length: {value}\r\n\r\n");
            let (position, reason) = request_refusal(wire.as_bytes());
            assert_eq!(position, 16, "{value:?}");
            assert!(reason.contains("Content-Length"), "{value:?}: {reason}");
        }
    }

    #[test]
    fn a_content_length_disagreeing_with_a_repetition_is_refused_at_the_repetition() {
        let (position, reason) =
            request_refusal(b"GET / HTTP/1.1\r\nContent-Length: 5\r\nContent-Length: 6\r\n\r\n");
        assert_eq!(position, 35);
        assert!(reason.contains("expected 5, got 6"), "{reason}");
        let (position, _) = request_refusal(b"GET / HTTP/1.1\r\nContent-Length: 5, 6\r\n\r\n");
        assert_eq!(position, 16);

        // Agreeing repetitions are one fact, stated once.
        let (head, body) = parse_request(
            b"GET / HTTP/1.1\r\nContent-Length: 5\r\nContent-Length: 5, 5\r\n\r\nhello",
        )
        .unwrap();
        assert_eq!(head.headers.get("content-length"), Some("5"));
        assert_eq!(body, b"hello");
    }

    #[test]
    fn content_length_beside_transfer_encoding_is_refused_at_the_coding() {
        let (position, reason) = request_refusal(
            b"POST / HTTP/1.1\r\nContent-Length: 3\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n0\r\n\r\n",
        );
        assert_eq!(position, 36);
        assert!(reason.contains("ambiguous"), "{reason}");
    }

    #[test]
    fn a_transfer_coding_other_than_chunked_alone_is_refused() {
        for value in ["gzip", "gzip, chunked", "chunked, chunked", "", "identity"] {
            let wire = format!("HTTP/1.1 200 OK\r\nTransfer-Encoding: {value}\r\n\r\n");
            let (position, reason) = response_refusal(wire.as_bytes());
            assert_eq!(position, 17, "{value:?}");
            assert!(reason.contains("chunked"), "{value:?}: {reason}");
        }
    }

    #[test]
    fn a_chunk_size_that_is_not_hexadecimal_is_refused_at_the_chunk() {
        let head = "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n";
        let (position, reason) =
            response_refusal(format!("{head}zz\r\nabc\r\n0\r\n\r\n").as_bytes());
        assert_eq!(position, head.len());
        assert!(reason.contains("hexadecimal"), "{reason}");
        let (position, reason) = response_refusal(format!("{head}\r\nabc\r\n0\r\n\r\n").as_bytes());
        assert_eq!(position, head.len());
        assert!(reason.contains("hexadecimal"), "{reason}");
        let (position, reason) =
            response_refusal(format!("{head}3x\r\nabc\r\n0\r\n\r\n").as_bytes());
        assert_eq!(position, head.len() + 1);
        assert!(reason.contains("chunk extension"), "{reason}");
        let (position, reason) =
            response_refusal(format!("{head}10000000000000000\r\nabc\r\n0\r\n\r\n").as_bytes());
        assert_eq!(position, head.len());
        assert!(reason.contains("64 bits"), "{reason}");
        // The second chunk is named by its own position.
        let (position, _) =
            response_refusal(format!("{head}3\r\nabc\r\nQ\r\n0\r\n\r\n").as_bytes());
        assert_eq!(position, head.len() + 8);
    }

    #[test]
    fn chunk_data_must_end_in_crlf_and_the_body_must_be_complete() {
        let head = "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n";
        let (position, reason) =
            response_refusal(format!("{head}3\r\nabcd\r\n0\r\n\r\n").as_bytes());
        assert_eq!(position, head.len() + 6);
        assert!(reason.contains("CRLF after the chunk data"), "{reason}");

        let wire = format!("{head}5\r\nabc");
        let (position, reason) = response_refusal(wire.as_bytes());
        assert_eq!(position, wire.len());
        assert!(reason.contains("unexpected end"), "{reason}");

        let wire = format!("{head}3\r\nabc\r\n0\r\n");
        let (position, reason) = response_refusal(wire.as_bytes());
        assert_eq!(position, wire.len());
        assert!(reason.contains("unexpected end"), "{reason}");
    }

    #[test]
    fn a_trailer_may_not_carry_a_framing_field() {
        let head = "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n";
        for name in ["Content-Length", "Transfer-Encoding", "Host", "Trailer"] {
            let wire = format!("{head}3\r\nabc\r\n0\r\n{name}: 3\r\n\r\n");
            let (position, reason) = response_refusal(wire.as_bytes());
            assert_eq!(position, head.len() + 11, "{name}");
            assert!(reason.contains("trailer"), "{name}: {reason}");
        }
    }

    #[test]
    fn a_body_shorter_than_its_content_length_is_refused_at_the_end() {
        let wire = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nabc";
        let (position, reason) = response_refusal(wire);
        assert_eq!(position, wire.len());
        assert!(reason.contains("states 5 bytes, 3 follow"), "{reason}");
    }

    #[test]
    fn bytes_after_the_message_are_refused_where_the_message_ended() {
        let (position, reason) =
            response_refusal(b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\nabcdef");
        assert_eq!(position, 41);
        assert!(reason.contains("3 bytes after"), "{reason}");
        // A request with neither framing has no body at all.
        let (position, _) = request_refusal(b"GET / HTTP/1.1\r\n\r\nabc");
        assert_eq!(position, 18);
        // A 204 has no body whatever the headers say.
        let (position, _) =
            response_refusal(b"HTTP/1.1 204 No Content\r\nContent-Length: 3\r\n\r\nabc");
        assert_eq!(position, 46);
        // After a chunked body too.
        let (position, _) = response_refusal(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n0\r\n\r\nX",
        );
        assert_eq!(position, 60);
    }

    #[test]
    fn a_truncated_head_is_refused_at_the_end_of_the_input() {
        for wire in [
            &b"GET / HTTP/1.1"[..],
            b"GET / HTTP/1.1\r\n",
            b"GET / HTTP/1.1\r\nHost: x\r\n",
            b"",
        ] {
            let (position, reason) = request_refusal(wire);
            assert_eq!(position, wire.len(), "{wire:?}");
            assert!(reason.contains("unexpected end"), "{wire:?}: {reason}");
        }
    }

    #[test]
    fn a_malformed_request_line_is_refused_at_the_token() {
        let (position, reason) = request_refusal(b"FETCH / HTTP/1.1\r\n\r\n");
        assert_eq!(position, 0);
        assert!(reason.contains("FETCH"), "{reason}");
        let (position, reason) = request_refusal(b"GET / HTTP/2\r\n\r\n");
        assert_eq!(position, 6);
        assert!(reason.contains("HTTP/2"), "{reason}");
        let (position, reason) = request_refusal(b"GET  HTTP/1.1\r\n\r\n");
        assert_eq!(position, 4);
        assert!(reason.contains("target"), "{reason}");
        let (position, _) = request_refusal(b"GET /a\x7fb HTTP/1.1\r\n\r\n");
        assert_eq!(position, 6);
        let (position, _) = request_refusal(b"GET /\r\n\r\n");
        assert_eq!(position, 4);
        let (position, _) = request_refusal(b"GET\r\n\r\n");
        assert_eq!(position, 0);
        let (position, _) = request_refusal(b"GET / HTTP/1.1 extra\r\n\r\n");
        assert_eq!(position, 6);
    }

    #[test]
    fn a_malformed_status_line_is_refused_at_the_token() {
        let (position, reason) = response_refusal(b"HTTP/1.1 20 OK\r\n\r\n");
        assert_eq!(position, 9);
        assert!(reason.contains("three-digit"), "{reason}");
        let (position, reason) = response_refusal(b"HTTP/1.1 600 Too Far\r\n\r\n");
        assert_eq!(position, 9);
        assert!(reason.contains("600"), "{reason}");
        let (position, reason) = response_refusal(b"HTTP/1.1 200OK\r\n\r\n");
        assert_eq!(position, 12);
        assert!(reason.contains("space"), "{reason}");
        let (position, _) = response_refusal(b"HTTP/2 200 OK\r\n\r\n");
        assert_eq!(position, 0);
        let (position, _) = response_refusal(b"HTTP/1.1\r\n\r\n");
        assert_eq!(position, 0);
        let (position, reason) = response_refusal(b"HTTP/1.1 200 O\x01K\r\n\r\n");
        assert_eq!(position, 14);
        assert!(reason.contains("reason phrase"), "{reason}");
    }

    #[test]
    fn a_malformed_field_line_is_refused_at_the_byte() {
        let (position, reason) = request_refusal(b"GET / HTTP/1.1\r\nX-A : 1\r\n\r\n");
        assert_eq!(position, 19);
        assert!(reason.contains("field name"), "{reason}");
        let (position, reason) = request_refusal(b"GET / HTTP/1.1\r\n: 1\r\n\r\n");
        assert_eq!(position, 16);
        assert!(reason.contains("field name"), "{reason}");
        let (position, reason) = request_refusal(b"GET / HTTP/1.1\r\nX-A: a\x01b\r\n\r\n");
        assert_eq!(position, 22);
        assert!(reason.contains("control byte 0x01"), "{reason}");
        let (position, _) = request_refusal(b"GET / HTTP/1.1\r\nX-A: a\x7f\r\n\r\n");
        assert_eq!(position, 22);
        let (position, _) = request_refusal(b"GET / HTTP/1.1\r\nno colon\r\n\r\n");
        assert_eq!(position, 16);
        let (position, _) = request_refusal(b"GET / HTTP/1.1\r\nX\xc3\xa9: 1\r\n\r\n");
        assert_eq!(position, 17);
    }
}

mod versions {
    use super::*;

    #[test]
    fn a_version_is_one_of_two_spellings() {
        assert_eq!(HttpVersion::ALL, [HttpVersion::Http10, HttpVersion::Http11]);
        assert_eq!(HttpVersion::default(), HttpVersion::Http11);
        for (version, text) in [
            (HttpVersion::Http10, "HTTP/1.0"),
            (HttpVersion::Http11, "HTTP/1.1"),
        ] {
            assert_eq!(version.as_str(), text);
            assert_eq!(version.as_ref(), text);
            assert_eq!(version.to_string(), text);
            assert_eq!(HttpVersion::from_str(text).unwrap(), version);
            assert_eq!(
                HttpVersion::from_str(&text.to_lowercase()).unwrap(),
                version
            );
            assert_eq!(text.parse::<HttpVersion>().unwrap(), version);
            assert_eq!(
                serde_json::to_string(&version).unwrap(),
                format!("\"{text}\"")
            );
            assert_eq!(
                serde_json::from_str::<HttpVersion>(&format!("\"{text}\"")).unwrap(),
                version
            );
        }
        for text in ["HTTP/2", "HTTP/1.2", "HTTP/1", "1.1", ""] {
            let error = HttpVersion::from_str(text).unwrap_err();
            assert!(
                matches!(
                    error,
                    Error::Parse {
                        target: "http version",
                        ..
                    }
                ),
                "{text:?}: {error}"
            );
        }
        assert!(HttpVersion::Http10 < HttpVersion::Http11);
    }
}

mod round_trips {
    use super::*;

    #[test]
    fn a_request_with_a_content_length_renders_and_parses_back_unchanged() {
        let head = RequestHead {
            method: Method::Post,
            target: "/rows?limit=10".to_owned(),
            version: HttpVersion::Http11,
            headers: headers(&[
                ("Host", "example.com"),
                ("Content-Type", "application/json"),
                ("Content-Length", "13"),
            ]),
        };
        let body = br#"{"rows":[1,2]}"#;
        assert_eq!(body.len(), 14);
        // The head owns what it states: the body is written as given.
        let head = RequestHead {
            headers: headers(&[
                ("Host", "example.com"),
                ("Content-Type", "application/json"),
                ("Content-Length", "14"),
            ]),
            ..head
        };
        let wire = render_request(&head, body);
        assert_eq!(
            wire,
            b"POST /rows?limit=10 HTTP/1.1\r\ncontent-length: 14\r\ncontent-type: application/json\r\nhost: example.com\r\n\r\n{\"rows\":[1,2]}"
        );
        let (parsed, parsed_body) = parse_request(&wire).unwrap();
        assert_eq!(parsed, head);
        assert_eq!(parsed_body, body);
    }

    #[test]
    fn a_request_without_a_body_states_no_length_unless_its_method_carries_one() {
        let get = RequestHead {
            method: Method::Get,
            target: "/".to_owned(),
            version: HttpVersion::Http10,
            headers: headers(&[("Host", "example.com")]),
        };
        let wire = render_request(&get, b"");
        assert_eq!(wire, b"GET / HTTP/1.0\r\nhost: example.com\r\n\r\n");
        let (parsed, body) = parse_request(&wire).unwrap();
        assert_eq!(parsed, get);
        assert!(body.is_empty());

        // A GET with a body is framed, so the body survives.
        let wire = render_request(&get, b"query");
        assert_eq!(
            wire,
            b"GET / HTTP/1.0\r\nhost: example.com\r\ncontent-length: 5\r\n\r\nquery"
        );
        let (parsed, body) = parse_request(&wire).unwrap();
        assert_eq!(parsed.headers.get("content-length"), Some("5"));
        assert_eq!(body, b"query");

        // A POST of nothing says so.
        let post = RequestHead {
            method: Method::Post,
            ..get
        };
        let wire = render_request(&post, b"");
        assert_eq!(
            wire,
            b"POST / HTTP/1.0\r\nhost: example.com\r\ncontent-length: 0\r\n\r\n"
        );
        let (parsed, body) = parse_request(&wire).unwrap();
        assert_eq!(parsed.headers.get("content-length"), Some("0"));
        assert!(body.is_empty());
    }

    #[test]
    fn a_response_without_a_stated_framing_gains_a_content_length() {
        let head = ResponseHead {
            version: HttpVersion::Http11,
            status: Status::OK,
            reason: "OK".to_owned(),
            headers: headers(&[("Content-Type", "text/plain")]),
        };
        let wire = render_response(&head, b"hello");
        assert_eq!(
            wire,
            b"HTTP/1.1 200 OK\r\ncontent-type: text/plain\r\ncontent-length: 5\r\n\r\nhello"
        );
        let (parsed, body) = parse_response(&wire).unwrap();
        let mut expected = head.clone();
        expected.headers.insert("content-length", "5").unwrap();
        assert_eq!(parsed, expected);
        assert_eq!(body, b"hello");
        // Rendering what was parsed reads back as itself: the fixed point,
        // field lines in lexical order.
        let rendered = render_response(&parsed, &body);
        assert_eq!(
            rendered,
            b"HTTP/1.1 200 OK\r\ncontent-length: 5\r\ncontent-type: text/plain\r\n\r\nhello"
        );
        assert_eq!(parse_response(&rendered).unwrap(), (parsed, body));
    }

    #[test]
    fn a_chunked_response_renders_as_chunks_and_parses_back_decoded() {
        let head = ResponseHead {
            version: HttpVersion::Http11,
            status: Status::OK,
            reason: "OK".to_owned(),
            headers: headers(&[("Transfer-Encoding", "chunked"), ("X-A", "1")]),
        };
        let wire = render_response(&head, b"hello world");
        assert_eq!(
            wire,
            b"HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\nx-a: 1\r\n\r\nb\r\nhello world\r\n0\r\n\r\n"
        );
        let (parsed, body) = parse_response(&wire).unwrap();
        assert_eq!(body, b"hello world");
        assert_eq!(
            parsed.headers,
            headers(&[("content-length", "11"), ("x-a", "1")])
        );
        assert_eq!(parsed.status, Status::OK);

        // An empty chunked body is the last chunk alone.
        let wire = render_response(&head, b"");
        assert!(wire.ends_with(b"\r\n\r\n0\r\n\r\n"));
        let (parsed, body) = parse_response(&wire).unwrap();
        assert!(body.is_empty());
        assert_eq!(parsed.headers.get("content-length"), Some("0"));
    }

    #[test]
    fn a_chunked_request_round_trips_the_same_way() {
        let head = RequestHead {
            method: Method::Put,
            target: "/upload".to_owned(),
            version: HttpVersion::Http11,
            headers: headers(&[("Transfer-Encoding", "chunked"), ("Host", "h")]),
        };
        let wire = render_request(&head, b"payload");
        assert_eq!(
            wire,
            b"PUT /upload HTTP/1.1\r\nhost: h\r\ntransfer-encoding: chunked\r\n\r\n7\r\npayload\r\n0\r\n\r\n"
        );
        let (parsed, body) = parse_request(&wire).unwrap();
        assert_eq!(body, b"payload");
        assert_eq!(parsed.method, Method::Put);
        assert_eq!(
            parsed.headers,
            headers(&[("content-length", "7"), ("host", "h")])
        );
    }

    #[test]
    fn a_status_without_a_body_renders_no_content_length_and_reads_none() {
        for (status, reason) in [
            (Status::NO_CONTENT, "No Content"),
            (Status::NOT_MODIFIED, "Not Modified"),
            (Status::CONTINUE, "Continue"),
        ] {
            let head = ResponseHead {
                version: HttpVersion::Http11,
                status,
                reason: reason.to_owned(),
                headers: headers(&[("ETag", "\"abc\"")]),
            };
            let wire = render_response(&head, b"");
            assert_eq!(
                wire,
                format!(
                    "HTTP/1.1 {} {reason}\r\netag: \"abc\"\r\n\r\n",
                    status.code()
                )
                .as_bytes()
            );
            let (parsed, body) = parse_response(&wire).unwrap();
            assert_eq!(parsed, head);
            assert!(body.is_empty());
        }
        // A 304 stating the length of the representation it stands for
        // carries no body, and keeps the header.
        let (parsed, body) =
            parse_response(b"HTTP/1.1 304 Not Modified\r\nContent-Length: 1234\r\n\r\n").unwrap();
        assert!(body.is_empty());
        assert_eq!(parsed.headers.get("content-length"), Some("1234"));
    }

    #[test]
    fn a_response_stating_neither_framing_reads_to_the_end_of_the_input() {
        let (head, body) =
            parse_response(b"HTTP/1.0 200 OK\r\nX-A: 1\r\n\r\nall of it\r\n").unwrap();
        assert_eq!(head.version, HttpVersion::Http10);
        assert_eq!(body, b"all of it\r\n");
        assert_eq!(head.headers.get("content-length"), None);
        let (_, body) = parse_response(b"HTTP/1.1 200 OK\r\n\r\n").unwrap();
        assert!(body.is_empty());
    }

    #[test]
    fn a_repeated_set_cookie_is_one_line_per_cookie_in_both_directions() {
        let wire = b"HTTP/1.1 200 OK\r\nSet-Cookie: a=1; Path=/\r\nSet-Cookie: b=2\r\nContent-Length: 0\r\n\r\n";
        let (head, _) = parse_response(wire).unwrap();
        assert_eq!(head.headers.get("set-cookie"), Some("a=1; Path=/\nb=2"));
        let rendered = render_response(&head, b"");
        assert_eq!(
            rendered,
            b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\nset-cookie: a=1; Path=/\r\nset-cookie: b=2\r\n\r\n"
        );
        let (again, _) = parse_response(&rendered).unwrap();
        assert_eq!(again, head);
        // Every other repeated field is a comma list, RFC 9110 5.3.
        let (head, _) =
            parse_response(b"HTTP/1.1 200 OK\r\nVary: A\r\nVary: B\r\nContent-Length: 0\r\n\r\n")
                .unwrap();
        assert_eq!(head.headers.get("vary"), Some("A, B"));
    }

    #[test]
    fn obs_text_is_transcribed_and_whitespace_around_a_value_is_dropped() {
        let (head, _) = parse_response(b"HTTP/1.1 200 caf\xe9 \xc3\xa9\r\nX-Name: \t caf\xe9 \t\r\nX-Empty:\r\nContent-Length: 0\r\n\r\n").unwrap();
        assert_eq!(head.reason, "café é");
        assert_eq!(head.headers.get("x-name"), Some("café"));
        assert_eq!(head.headers.get("x-empty"), Some(""));
        // A reason phrase may be empty, with or without its space.
        for wire in [&b"HTTP/1.1 200\r\n\r\n"[..], b"HTTP/1.1 200 \r\n\r\n"] {
            let (head, _) = parse_response(wire).unwrap();
            assert_eq!(head.reason, "");
            assert_eq!(head.status, Status::OK);
            assert_eq!(
                render_response(&head, b""),
                b"HTTP/1.1 200 \r\ncontent-length: 0\r\n\r\n"
            );
        }
    }

    #[test]
    fn a_bare_lf_is_read_as_the_line_terminator_and_names_are_case_insensitive() {
        let (head, body) =
            parse_request(b"get /x http/1.1\nHOST: example.com\ncontent-LENGTH: 2\n\nok").unwrap();
        assert_eq!(head.method, Method::Get);
        assert_eq!(head.version, HttpVersion::Http11);
        assert_eq!(head.headers.get("Host"), Some("example.com"));
        assert_eq!(body, b"ok");
        // Every other request-target form crosses as spelled.
        for target in ["*", "http://example.com/a?b=c", "example.com:443", "/a%20b"] {
            let wire = format!("OPTIONS {target} HTTP/1.1\r\n\r\n");
            let (head, _) = parse_request(wire.as_bytes()).unwrap();
            assert_eq!(head.target, target);
        }
    }
}

mod chunked {
    use super::*;

    #[test]
    fn extensions_are_ignored_and_trailers_fold_into_the_headers() {
        let wire = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nTrailer: X-Sum\r\n\r\n\
                     4;name=value;flag\r\nWiki\r\n5 ; x=\"quoted;semi\"\r\npedia\r\nE\r\n in\r\n\r\nchunks.\r\n0;last\r\n\
                     X-Sum: 3\r\nX-Sum: 4\r\nX-Other: yes\r\n\r\n";
        let (head, body) = parse_response(wire).unwrap();
        assert_eq!(body, b"Wikipedia in\r\n\r\nchunks.");
        assert_eq!(head.headers.get("x-sum"), Some("3, 4"));
        assert_eq!(head.headers.get("x-other"), Some("yes"));
        assert_eq!(head.headers.get("content-length"), Some("23"));
        assert_eq!(head.headers.get("transfer-encoding"), None);
        assert_eq!(head.headers.get("trailer"), None);
        // Upper-case hex and a leading zero are hex.
        let (_, body) = parse_response(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n0B\r\nhello world\r\n000\r\n\r\n").unwrap();
        assert_eq!(body, b"hello world");
    }

    #[test]
    fn a_trailer_joins_a_header_of_the_same_name() {
        let (head, _) = parse_response(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nX-Sum: 1\r\n\r\n0\r\nX-Sum: 2\r\n\r\n",
        )
        .unwrap();
        assert_eq!(head.headers.get("x-sum"), Some("1, 2"));
    }
}

mod streaming {
    use super::*;

    /// A reader handing out one byte per call, so every fill is exercised.
    struct Trickle<'a>(&'a [u8]);

    impl Read for Trickle<'_> {
        fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
            match (self.0.split_first(), out.first_mut()) {
                (Some((byte, rest)), Some(slot)) => {
                    *slot = *byte;
                    self.0 = rest;
                    Ok(1)
                }
                _ => Ok(0),
            }
        }
    }

    #[test]
    fn the_encoder_writes_one_chunk_per_write_and_finish_ends_the_body() {
        let mut writer = encode_chunked(Vec::new());
        assert_eq!(writer.write(b"").unwrap(), 0);
        writer.write_all(b"Wiki").unwrap();
        writer.write_all(b"pedia").unwrap();
        writer.write_all(&[b'x'; 300]).unwrap();
        writer.flush().unwrap();
        let wire = writer.finish().unwrap();
        let mut expected = b"4\r\nWiki\r\n5\r\npedia\r\n12c\r\n".to_vec();
        expected.extend_from_slice(&[b'x'; 300]);
        expected.extend_from_slice(b"\r\n0\r\n\r\n");
        assert_eq!(wire, expected);
        // Nothing but the last chunk for a body never written.
        assert_eq!(encode_chunked(Vec::new()).finish().unwrap(), b"0\r\n\r\n");
        // `into_inner` hands back what was written so far, unterminated.
        let mut writer = encode_chunked(Vec::new());
        writer.write_all(b"ab").unwrap();
        assert_eq!(writer.into_inner(), b"2\r\nab\r\n");
    }

    #[test]
    fn the_decoder_reads_what_the_encoder_wrote_under_every_buffer_size() {
        let payload: Vec<u8> = (0..200_000_u32).map(|index| (index % 251) as u8).collect();
        let mut writer = encode_chunked(Vec::new());
        for chunk in payload.chunks(70_000) {
            writer.write_all(chunk).unwrap();
        }
        let wire = writer.finish().unwrap();

        for buffer_size in [1_usize, 7, 4096, 8194, 65_536, 1 << 20] {
            let mut reader = decode_chunked(Cursor::new(&wire));
            assert!(reader.trailers().is_none());
            let mut decoded = Vec::new();
            let mut buffer = vec![0_u8; buffer_size];
            loop {
                let read = reader.read(&mut buffer).unwrap();
                if read == 0 {
                    break;
                }
                decoded.extend_from_slice(&buffer[..read]);
            }
            assert_eq!(decoded, payload, "buffer of {buffer_size}");
            assert!(reader.is_finished());
            assert_eq!(reader.consumed(), wire.len());
            assert!(reader.trailers().unwrap().is_empty());
            // Reading past the end stays at the end.
            assert_eq!(reader.read(&mut buffer).unwrap(), 0);
        }

        // One byte at a time from the source, too.
        let mut decoded = Vec::new();
        decode_chunked(Trickle(&wire))
            .read_to_end(&mut decoded)
            .unwrap();
        assert_eq!(decoded, payload);
    }

    #[test]
    fn trailers_are_answered_only_once_the_body_is_finished() {
        let wire = b"5;ext\r\nhello\r\n1\r\n!\r\n0\r\nExpires: 0\r\nX-Sum: 6\r\n\r\nEXTRA";
        let mut reader = decode_chunked(Cursor::new(&wire[..]));
        let mut first = [0_u8; 3];
        reader.read_exact(&mut first).unwrap();
        assert_eq!(&first, b"hel");
        assert!(reader.trailers().is_none());
        assert!(!reader.is_finished());
        let mut rest = String::new();
        reader.read_to_string(&mut rest).unwrap();
        assert_eq!(rest, "lo!");
        let trailers = reader.trailers().unwrap();
        assert_eq!(trailers.get("expires"), Some("0"));
        assert_eq!(trailers.get("x-sum"), Some("6"));
        // The message ends before the extra bytes, which stay the caller's.
        assert_eq!(reader.consumed(), wire.len() - "EXTRA".len());
        assert_eq!(reader.read(&mut [0; 8]).unwrap(), 0);
        assert!(reader.into_inner().position() > 0);
    }

    #[test]
    fn a_malformed_stream_is_an_invalid_data_error_naming_the_position() {
        let cases: [(&[u8], usize, &str); 5] = [
            (b"zz\r\nabc\r\n0\r\n\r\n", 0, "hexadecimal"),
            (b"3\r\nabcd\r\n0\r\n\r\n", 6, "CRLF after the chunk data"),
            (b"3\r\nabc\r\n0\r\nX: a\rb\r\n\r\n", 15, "bare CR"),
            (b"0\r\n bad\r\n\r\n", 3, "folding"),
            (b"0\r\nContent-Length: 1\r\n\r\n", 3, "trailer"),
        ];
        for (wire, expected_position, expected_reason) in cases {
            let error = decode_chunked(Cursor::new(wire))
                .read_to_end(&mut Vec::new())
                .expect_err("a refusal");
            assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{wire:?}");
            let inner = error.downcast::<Error>().unwrap();
            let (position, reason) = refusal(&inner);
            assert_eq!(position, expected_position, "{wire:?}: {reason}");
            assert!(reason.contains(expected_reason), "{wire:?}: {reason}");
        }
    }

    #[test]
    fn a_stream_that_ends_early_is_an_unexpected_eof_naming_the_position() {
        for wire in [
            &b""[..],
            b"3",
            b"3\r\nab",
            b"3\r\nabc",
            b"3\r\nabc\r\n",
            b"3\r\nabc\r\n0\r\n",
            b"3\r\nabc\r\n0\r\nX-A: 1\r\n",
        ] {
            let error = decode_chunked(Cursor::new(wire))
                .read_to_end(&mut Vec::new())
                .expect_err("a refusal");
            assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof, "{wire:?}");
            let inner = error.downcast::<Error>().unwrap();
            let (position, reason) = refusal(&inner);
            assert_eq!(position, wire.len(), "{wire:?}: {reason}");
            assert!(reason.contains("unexpected end"), "{wire:?}: {reason}");
        }
    }

    #[test]
    fn a_chunk_size_line_above_8_kib_and_too_many_trailers_are_refused() {
        let mut wire = b"1;".to_vec();
        wire.extend(std::iter::repeat_n(b'e', 9000));
        wire.extend_from_slice(b"\r\nx\r\n0\r\n\r\n");
        let error = decode_chunked(Cursor::new(&wire))
            .read_to_end(&mut Vec::new())
            .expect_err("a refusal");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        let (position, reason) = refusal(&error.downcast::<Error>().unwrap());
        assert_eq!(position, 0);
        assert!(reason.contains("8192"), "{reason}");

        let mut wire = b"0\r\n".to_vec();
        for index in 0..257 {
            wire.extend_from_slice(format!("X-{index}: v\r\n").as_bytes());
        }
        wire.extend_from_slice(b"\r\n");
        let error = decode_chunked(Cursor::new(&wire))
            .read_to_end(&mut Vec::new())
            .expect_err("a refusal");
        let (_, reason) = refusal(&error.downcast::<Error>().unwrap());
        assert!(reason.contains("256"), "{reason}");
    }
}
