//! The subset of S3's XML this backend reads and writes.
//!
//! S3 answers listings, bulk deletes, multipart uploads, and every failure
//! with small documents of a fixed shape, and takes three equally small ones
//! as request bodies. That is narrow enough for a deterministic scanner -
//! elements by local name in document order, the declaration, comments,
//! CDATA, the five named entities and numeric character references - to
//! cover without an XML dependency. Attributes (the `xmlns` on every root)
//! are skipped, namespace prefixes are dropped so names match locally,
//! unknown elements are ignored, and anything malformed is an [`XmlError`].
//!
//! Keys and prefixes are percent-decoded only when the page carries
//! `<EncodingType>url</EncodingType>`. S3 then applies form-encoding rules -
//! a space arrives as `+`, everything outside the unreserved set as `%XX`
//! over its UTF-8 bytes - so both are undone, as botocore (`unquote_plus`)
//! and the Java SDK (`URLDecoder`) do.

use std::fmt;

/// Nesting past which a scan stops. S3's deepest document is four levels,
/// so this bounds recursion on a hostile body without touching a real one.
const MAX_DEPTH: usize = 32;

/// A body that is not the document it was expected to be: malformed XML, a
/// different root, or a required element missing or unreadable.
#[derive(Debug)]
pub(crate) struct XmlError(pub(crate) String);

impl fmt::Display for XmlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for XmlError {}

/// One `<Contents>` entry of a listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ObjectSummary {
    /// The object key, percent-decoded when the page says
    /// `<EncodingType>url</EncodingType>`.
    pub(crate) key: String,
    /// The object size in bytes.
    pub(crate) size: u64,
    /// The `ETag` as given, quotes included.
    pub(crate) etag: Option<String>,
    /// The `LastModified` timestamp as given.
    pub(crate) last_modified: Option<String>,
}

/// One page of `ListObjectsV2`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ListPage {
    /// The objects in document order, which is key order.
    pub(crate) objects: Vec<ObjectSummary>,
    /// The `CommonPrefixes`, decoded like keys.
    pub(crate) prefixes: Vec<String>,
    /// Whether another page follows.
    pub(crate) is_truncated: bool,
    /// The token that fetches the next page; absent on the last one.
    pub(crate) next_continuation_token: Option<String>,
}

/// The fields of an `<Error>` document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ErrorBody {
    /// The error code (`NoSuchKey`, `AccessDenied`, ...); empty when absent.
    pub(crate) code: String,
    /// The human-readable message; empty when absent.
    pub(crate) message: String,
    /// The `RequestId` S3 stamps for support.
    pub(crate) request_id: Option<String>,
    /// The `Resource` the failure names, when it names one.
    pub(crate) resource: Option<String>,
}

/// One `<Error>` entry of a `<DeleteResult>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DeleteFailure {
    /// The key that was not deleted.
    pub(crate) key: String,
    /// The error code.
    pub(crate) code: String,
    /// The human-readable message.
    pub(crate) message: String,
}

/// Read one `ListObjectsV2` page.
///
/// # Errors
///
/// Malformed XML, a root other than `<ListBucketResult>`, or a `<Contents>`
/// without a `<Key>` or a numeric `<Size>`.
pub(crate) fn parse_list_objects(xml: &[u8]) -> Result<ListPage, XmlError> {
    let root = parse_root(xml, "ListBucketResult")?;
    let encoded = root
        .child_text("EncodingType")
        .is_some_and(|value| value.trim() == "url");
    let decode = |text: &str| {
        if encoded {
            decode_url(text)
        } else {
            text.to_owned()
        }
    };
    let mut page = ListPage::default();
    for contents in root.children("Contents") {
        let size = contents.required("Size")?;
        let size = size
            .trim()
            .parse()
            .map_err(|_| XmlError(format!("<Size> is not a number: `{size}`")))?;
        page.objects.push(ObjectSummary {
            key: decode(contents.required("Key")?),
            size,
            etag: contents.child_text("ETag").map(str::to_owned),
            last_modified: contents.child_text("LastModified").map(str::to_owned),
        });
    }
    for common in root.children("CommonPrefixes") {
        page.prefixes.push(decode(common.required("Prefix")?));
    }
    page.is_truncated = root
        .child_text("IsTruncated")
        .is_some_and(|value| value.trim() == "true");
    page.next_continuation_token = root
        .child_text("NextContinuationToken")
        .filter(|token| !token.is_empty())
        .map(str::to_owned);
    Ok(page)
}

/// `None` when the bytes are not an `<Error>` document.
pub(crate) fn parse_error(xml: &[u8]) -> Option<ErrorBody> {
    let root = parse_document(xml)
        .ok()
        .filter(|root| root.name == "Error")?;
    Some(error_body(&root))
}

/// `<InitiateMultipartUploadResult><UploadId>`.
///
/// # Errors
///
/// Malformed XML, another root, or a missing or empty `<UploadId>`.
pub(crate) fn parse_upload_id(xml: &[u8]) -> Result<String, XmlError> {
    let root = parse_root(xml, "InitiateMultipartUploadResult")?;
    let id = root.required("UploadId")?;
    if id.is_empty() {
        return Err(XmlError("empty <UploadId>".to_owned()));
    }
    Ok(id.to_owned())
}

/// `<CompleteMultipartUploadResult><ETag>`.
///
/// # Errors
///
/// Malformed XML or another root. S3 can answer 200 and still fail late,
/// with an `<Error>` body: that is an `Err` carrying `code: message`.
pub(crate) fn parse_complete_multipart(xml: &[u8]) -> Result<Option<String>, XmlError> {
    let root = parse_document(xml)?;
    match root.name.as_str() {
        "CompleteMultipartUploadResult" => Ok(root.child_text("ETag").map(str::to_owned)),
        "Error" => {
            let error = error_body(&root);
            Err(XmlError(format!("{}: {}", error.code, error.message)))
        }
        other => Err(XmlError(format!(
            "expected <CompleteMultipartUploadResult>, found <{other}>"
        ))),
    }
}

/// `<DeleteResult>`: the `<Error>` entries (quiet mode omits `<Deleted>`).
///
/// # Errors
///
/// Malformed XML, another root, or an `<Error>` entry without a `<Key>`.
pub(crate) fn parse_delete_result(xml: &[u8]) -> Result<Vec<DeleteFailure>, XmlError> {
    let root = parse_root(xml, "DeleteResult")?;
    root.children("Error")
        .map(|error| {
            Ok(DeleteFailure {
                key: error.required("Key")?.to_owned(),
                code: error.child_text("Code").unwrap_or_default().to_owned(),
                message: error.child_text("Message").unwrap_or_default().to_owned(),
            })
        })
        .collect()
}

/// `<CreateBucketConfiguration><LocationConstraint>{region}</LocationConstraint></CreateBucketConfiguration>`.
pub(crate) fn render_create_bucket(region: &str) -> String {
    format!(
        "<CreateBucketConfiguration><LocationConstraint>{}</LocationConstraint></CreateBucketConfiguration>",
        escape_text(region)
    )
}

/// `<Delete><Quiet>true</Quiet><Object><Key>..</Key></Object>...</Delete>`
/// with escaping; `<Quiet>` is omitted when not quiet, its default.
pub(crate) fn render_delete_objects(keys: &[String], quiet: bool) -> String {
    let mut xml = String::from("<Delete>");
    if quiet {
        xml.push_str("<Quiet>true</Quiet>");
    }
    for key in keys {
        xml.push_str("<Object><Key>");
        xml.push_str(&escape_text(key));
        xml.push_str("</Key></Object>");
    }
    xml.push_str("</Delete>");
    xml
}

/// `<CompleteMultipartUpload><Part><PartNumber>n</PartNumber><ETag>..</ETag></Part>...</CompleteMultipartUpload>`,
/// parts in the order given (S3 requires ascending part numbers).
pub(crate) fn render_complete_multipart(parts: &[(u32, String)]) -> String {
    let mut xml = String::from("<CompleteMultipartUpload>");
    for (number, etag) in parts {
        xml.push_str("<Part><PartNumber>");
        xml.push_str(&number.to_string());
        xml.push_str("</PartNumber><ETag>");
        xml.push_str(&escape_text(etag));
        xml.push_str("</ETag></Part>");
    }
    xml.push_str("</CompleteMultipartUpload>");
    xml
}

/// Escape `& < > " '` for element text.
pub(crate) fn escape_text(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            other => escaped.push(other),
        }
    }
    escaped
}

/// The fields of one `<Error>` element; an absent code or message reads as
/// empty rather than failing, since the document still says something went
/// wrong.
fn error_body(error: &Element) -> ErrorBody {
    ErrorBody {
        code: error.child_text("Code").unwrap_or_default().to_owned(),
        message: error.child_text("Message").unwrap_or_default().to_owned(),
        request_id: error.child_text("RequestId").map(str::to_owned),
        resource: error.child_text("Resource").map(str::to_owned),
    }
}

/// `text` with form-encoding undone: `+` is a space and `%XX` a byte. A
/// malformed escape stays as spelled and a result that is not UTF-8 leaves
/// the whole text as it stands, so a key S3 gave us is never silently
/// mangled.
fn decode_url(text: &str) -> String {
    if !text.contains(['%', '+']) {
        return text.to_owned();
    }
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if let Some(byte) = bytes.get(index + 1..index + 3).and_then(hex_byte) {
                decoded.push(byte);
                index += 3;
                continue;
            }
        }
        decoded.push(if bytes[index] == b'+' {
            b' '
        } else {
            bytes[index]
        });
        index += 1;
    }
    String::from_utf8(decoded).unwrap_or_else(|_| text.to_owned())
}

/// The byte two hex digits spell.
fn hex_byte(pair: &[u8]) -> Option<u8> {
    if pair.len() != 2 || !pair.iter().all(u8::is_ascii_hexdigit) {
        return None;
    }
    u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()
}

/// One scanned element: its local name, the character data directly inside
/// it (entities and CDATA resolved, whitespace kept, since a key may start
/// or end with a space), and its children in document order.
#[derive(Debug, Default)]
struct Element {
    /// The tag name without any namespace prefix.
    name: String,
    /// The element's own character data, children excluded.
    text: String,
    /// Child elements in document order.
    children: Vec<Element>,
}

impl Element {
    /// The first child named `name`.
    fn child(&self, name: &str) -> Option<&Element> {
        self.children.iter().find(|child| child.name == name)
    }

    /// Every child named `name`, in document order.
    fn children<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Element> + 'a {
        self.children.iter().filter(move |child| child.name == name)
    }

    /// The text of the first child named `name`.
    fn child_text(&self, name: &str) -> Option<&str> {
        self.child(name).map(|child| child.text.as_str())
    }

    /// The text of the first child named `name`, whose absence is an error.
    fn required(&self, name: &str) -> Result<&str, XmlError> {
        self.child_text(name)
            .ok_or_else(|| XmlError(format!("<{}> without <{name}>", self.name)))
    }
}

/// The root of `xml`, which must be named `expected`.
fn parse_root(xml: &[u8], expected: &str) -> Result<Element, XmlError> {
    let root = parse_document(xml)?;
    if root.name == expected {
        Ok(root)
    } else {
        Err(XmlError(format!(
            "expected <{expected}>, found <{}>",
            root.name
        )))
    }
}

/// The root element of `xml`: prolog and trailing comments allowed, a byte
/// order mark tolerated, anything else outside the root refused.
fn parse_document(xml: &[u8]) -> Result<Element, XmlError> {
    let text = std::str::from_utf8(xml).map_err(|error| XmlError(format!("not UTF-8: {error}")))?;
    let mut scanner = Scanner {
        text: text.strip_prefix('\u{FEFF}').unwrap_or(text),
        at: 0,
    };
    scanner.skip_misc()?;
    if !scanner.starts_with("<") {
        return Err(scanner.error("expected the root element"));
    }
    let root = scanner.element(0)?;
    scanner.skip_misc()?;
    if scanner.at < scanner.text.len() {
        return Err(scanner.error("content after the root element"));
    }
    Ok(root)
}

/// The cursor of one scan; `at` is always on a character boundary because
/// it only ever lands after ASCII markup.
struct Scanner<'a> {
    /// The whole document.
    text: &'a str,
    /// The byte offset of the next unread character.
    at: usize,
}

impl<'a> Scanner<'a> {
    /// What is left to scan.
    fn rest(&self) -> &'a str {
        &self.text[self.at..]
    }

    fn starts_with(&self, token: &str) -> bool {
        self.rest().starts_with(token)
    }

    /// An error naming where the scan stopped.
    fn error(&self, message: impl fmt::Display) -> XmlError {
        XmlError(format!("{message} at byte {}", self.at))
    }

    /// Advance past `token` or fail.
    fn expect(&mut self, token: &str) -> Result<(), XmlError> {
        if !self.starts_with(token) {
            return Err(self.error(format!("expected `{token}`")));
        }
        self.at += token.len();
        Ok(())
    }

    /// The text up to the next `marker`, which is consumed too.
    fn take_until(&mut self, marker: &str, what: &str) -> Result<&'a str, XmlError> {
        let rest = self.rest();
        let end = rest
            .find(marker)
            .ok_or_else(|| self.error(format!("unterminated {what}")))?;
        self.at += end + marker.len();
        Ok(&rest[..end])
    }

    fn skip_whitespace(&mut self) {
        let skipped = self
            .rest()
            .bytes()
            .take_while(|byte| is_space(*byte))
            .count();
        self.at += skipped;
    }

    /// Skip whitespace, comments, and processing instructions (the XML
    /// declaration is one), which may surround the root.
    fn skip_misc(&mut self) -> Result<(), XmlError> {
        loop {
            self.skip_whitespace();
            if self.starts_with("<?") {
                self.at += 2;
                self.take_until("?>", "processing instruction")?;
            } else if self.starts_with("<!--") {
                self.at += 4;
                self.take_until("-->", "comment")?;
            } else {
                return Ok(());
            }
        }
    }

    /// The name under the cursor: everything up to whitespace or markup.
    fn name(&mut self) -> Result<&'a str, XmlError> {
        let rest = self.rest();
        let end = rest
            .bytes()
            .position(|byte| {
                is_space(byte) || matches!(byte, b'/' | b'>' | b'<' | b'=' | b'"' | b'\'')
            })
            .unwrap_or(rest.len());
        if end == 0 {
            return Err(self.error("expected a name"));
        }
        self.at += end;
        Ok(&rest[..end])
    }

    /// Skip one `name="value"` pair. The value is skipped as a quoted unit so
    /// a `>` inside it does not end the tag.
    fn attribute(&mut self) -> Result<(), XmlError> {
        self.name()?;
        self.skip_whitespace();
        self.expect("=")?;
        self.skip_whitespace();
        let quote = match self.rest().as_bytes().first() {
            Some(b'"') => "\"",
            Some(b'\'') => "'",
            _ => return Err(self.error("expected a quoted attribute value")),
        };
        self.at += 1;
        self.take_until(quote, "attribute value")?;
        Ok(())
    }

    /// The element whose `<` is under the cursor, with everything inside it.
    fn element(&mut self, depth: usize) -> Result<Element, XmlError> {
        if depth > MAX_DEPTH {
            return Err(self.error(format!("elements nested deeper than {MAX_DEPTH}")));
        }
        self.at += 1;
        let full_name = self.name()?;
        let mut element = Element {
            name: full_name.rsplit(':').next().unwrap_or(full_name).to_owned(),
            ..Element::default()
        };
        loop {
            self.skip_whitespace();
            if self.starts_with("/>") {
                self.at += 2;
                return Ok(element);
            }
            if self.starts_with(">") {
                self.at += 1;
                break;
            }
            self.attribute()?;
        }
        loop {
            if self.starts_with("</") {
                self.at += 2;
                let closing = self.name()?;
                if closing != full_name {
                    return Err(self.error(format!("<{full_name}> closed by </{closing}>")));
                }
                self.skip_whitespace();
                self.expect(">")?;
                return Ok(element);
            }
            if self.starts_with("<![CDATA[") {
                self.at += 9;
                element
                    .text
                    .push_str(self.take_until("]]>", "CDATA section")?);
            } else if self.starts_with("<!--") {
                self.at += 4;
                self.take_until("-->", "comment")?;
            } else if self.starts_with("<?") {
                self.at += 2;
                self.take_until("?>", "processing instruction")?;
            } else if self.starts_with("<") {
                let child = self.element(depth + 1)?;
                element.children.push(child);
            } else if self.rest().is_empty() {
                return Err(self.error(format!("<{full_name}> never closed")));
            } else {
                let rest = self.rest();
                let end = rest.find('<').unwrap_or(rest.len());
                decode_entities(&rest[..end], &mut element.text)
                    .map_err(|message| self.error(message))?;
                self.at += end;
            }
        }
    }
}

/// XML's own whitespace set, narrower than ASCII's.
fn is_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\r' | b'\n')
}

/// Append `raw` character data to `out` with the five named entities and
/// numeric character references resolved. Anything else after `&` is the
/// error text.
fn decode_entities(raw: &str, out: &mut String) -> Result<(), String> {
    let mut rest = raw;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let end = after.find(';').ok_or_else(|| {
            format!(
                "unterminated reference `&{}`",
                after.chars().take(8).collect::<String>()
            )
        })?;
        let reference = &after[..end];
        let character = match reference {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => character_reference(reference),
        };
        out.push(character.ok_or_else(|| format!("unknown reference `&{reference};`"))?);
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Ok(())
}

/// The character `#NNN` or `#xHH` names, when it names one XML allows.
fn character_reference(reference: &str) -> Option<char> {
    let (digits, radix) = match reference.strip_prefix("#x") {
        Some(hex) => (hex, 16),
        None => (reference.strip_prefix('#')?, 10),
    };
    if digits.is_empty() || !digits.bytes().all(|byte| char::from(byte).is_digit(radix)) {
        return None;
    }
    u32::from_str_radix(digits, radix)
        .ok()
        .and_then(char::from_u32)
        .filter(|character| *character != '\0')
}

#[cfg(test)]
mod tests {
    use super::*;

    const XMLNS: &str = "http://s3.amazonaws.com/doc/2006-03-01/";

    fn listing(body: &str) -> String {
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<ListBucketResult xmlns=\"{XMLNS}\">\n<Name>bucket</Name>\n{body}\n</ListBucketResult>\n"
        )
    }

    fn keys(page: &ListPage) -> Vec<&str> {
        page.objects
            .iter()
            .map(|object| object.key.as_str())
            .collect()
    }

    #[test]
    fn a_plain_listing_yields_objects_in_document_order() {
        let xml = listing(
            "<Prefix></Prefix><KeyCount>2</KeyCount><MaxKeys>1000</MaxKeys><IsTruncated>false</IsTruncated>
             <Contents>
               <Key>my-image.jpg</Key>
               <LastModified>2009-10-12T17:50:30.000Z</LastModified>
               <ETag>\"fba9dede5f27731c9771645a39863328\"</ETag>
               <Size>434234</Size>
               <StorageClass>STANDARD</StorageClass>
             </Contents>
             <Contents>
               <Key>notes/</Key>
               <Size>0</Size>
             </Contents>",
        );
        let page = parse_list_objects(xml.as_bytes()).unwrap();
        assert_eq!(
            page.objects,
            vec![
                ObjectSummary {
                    key: "my-image.jpg".to_owned(),
                    size: 434_234,
                    etag: Some("\"fba9dede5f27731c9771645a39863328\"".to_owned()),
                    last_modified: Some("2009-10-12T17:50:30.000Z".to_owned()),
                },
                ObjectSummary {
                    key: "notes/".to_owned(),
                    size: 0,
                    etag: None,
                    last_modified: None,
                },
            ]
        );
        assert!(page.prefixes.is_empty());
        assert!(!page.is_truncated);
        assert_eq!(page.next_continuation_token, None);
    }

    #[test]
    fn common_prefixes_and_the_continuation_token_are_read() {
        let xml = listing(
            "<Prefix>photos/</Prefix><Delimiter>/</Delimiter><KeyCount>3</KeyCount><MaxKeys>3</MaxKeys>
             <IsTruncated>true</IsTruncated>
             <NextContinuationToken>1ueGcxLPRx1Tr/XYExHnhbYLgveDs2J/wm36Hy4vbOwM=</NextContinuationToken>
             <Contents><Key>photos/index.html</Key><Size>12</Size></Contents>
             <CommonPrefixes><Prefix>photos/2006/</Prefix></CommonPrefixes>
             <CommonPrefixes><Prefix>photos/2007/</Prefix></CommonPrefixes>",
        );
        let page = parse_list_objects(xml.as_bytes()).unwrap();
        assert_eq!(keys(&page), ["photos/index.html"]);
        assert_eq!(page.prefixes, ["photos/2006/", "photos/2007/"]);
        assert!(page.is_truncated);
        assert_eq!(
            page.next_continuation_token.as_deref(),
            Some("1ueGcxLPRx1Tr/XYExHnhbYLgveDs2J/wm36Hy4vbOwM=")
        );
    }

    #[test]
    fn an_empty_listing_is_an_empty_page() {
        let xml = listing(
            "<Prefix/><KeyCount>0</KeyCount><MaxKeys>1000</MaxKeys><IsTruncated>false</IsTruncated>",
        );
        assert_eq!(
            parse_list_objects(xml.as_bytes()).unwrap(),
            ListPage::default()
        );
    }

    #[test]
    fn url_encoding_type_decodes_keys_and_prefixes() {
        // EncodingType sits after the entries, where S3 puts it.
        let xml = listing(
            "<Contents><Key>photos%2F2006+January%2Fsample%281%29.jpg</Key><Size>1</Size></Contents>
             <Contents><Key>caf%C3%A9%2B%25.txt</Key><Size>2</Size></Contents>
             <CommonPrefixes><Prefix>a+b%2F</Prefix></CommonPrefixes>
             <EncodingType>url</EncodingType>",
        );
        let page = parse_list_objects(xml.as_bytes()).unwrap();
        assert_eq!(
            keys(&page),
            ["photos/2006 January/sample(1).jpg", "café+%.txt"]
        );
        assert_eq!(page.prefixes, ["a b/"]);
    }

    #[test]
    fn without_encoding_type_keys_stay_verbatim() {
        let xml = listing(
            "<Contents><Key>a+b%2F</Key><Size>1</Size></Contents>
             <Contents><Key>tom &amp; jerry.txt</Key><Size>2</Size></Contents>
             <Contents><Key>café/日本.txt</Key><Size>3</Size></Contents>
             <CommonPrefixes><Prefix>a+b%2F</Prefix></CommonPrefixes>",
        );
        let page = parse_list_objects(xml.as_bytes()).unwrap();
        assert_eq!(keys(&page), ["a+b%2F", "tom & jerry.txt", "café/日本.txt"]);
        assert_eq!(page.prefixes, ["a+b%2F"]);
    }

    #[test]
    fn url_decoding_leaves_malformed_escapes_and_non_utf8_alone() {
        assert_eq!(decode_url("100%25+%zz%"), "100% %zz%");
        assert_eq!(decode_url("%FF"), "%FF");
        assert_eq!(decode_url("plain"), "plain");
        assert_eq!(decode_url("%2"), "%2");
    }

    #[test]
    fn entities_and_character_references_decode() {
        let xml = listing(
            "<Contents><Key>&amp;&lt;&gt;&quot;&apos;&#65;&#x42;&#x1F600;&#x00e9;</Key><Size>1</Size></Contents>",
        );
        let page = parse_list_objects(xml.as_bytes()).unwrap();
        assert_eq!(keys(&page), ["&<>\"'AB😀é"]);
    }

    #[test]
    fn cdata_comments_and_processing_instructions_are_handled() {
        let xml = format!(
            "\u{FEFF}<?xml version=\"1.0\"?><!-- header --><?pi data?>
             <s3:ListBucketResult xmlns:s3=\"{XMLNS}\">
               <s3:Contents><!-- inside --><s3:Key><![CDATA[a<b&c]]>.txt</s3:Key><?x?><s3:Size> 7 </s3:Size></s3:Contents>
               <s3:IsTruncated>false</s3:IsTruncated>
             </s3:ListBucketResult><!-- trailer -->
             "
        );
        let page = parse_list_objects(xml.as_bytes()).unwrap();
        assert_eq!(keys(&page), ["a<b&c.txt"]);
        assert_eq!(page.objects[0].size, 7);
    }

    #[test]
    fn attributes_may_hold_the_closing_bracket() {
        let xml = format!(
            "<ListBucketResult xmlns=\"{XMLNS}\" note='a>b' other = \"c/>d\"><Contents><Key>k</Key><Size>1</Size></Contents></ListBucketResult>"
        );
        let page = parse_list_objects(xml.as_bytes()).unwrap();
        assert_eq!(keys(&page), ["k"]);
    }

    #[test]
    fn a_different_root_is_rejected() {
        let error = parse_list_objects(b"<Error><Code>NoSuchBucket</Code></Error>").unwrap_err();
        assert_eq!(error.0, "expected <ListBucketResult>, found <Error>");
    }

    #[test]
    fn contents_without_a_key_or_a_numeric_size_are_rejected() {
        let xml = listing("<Contents><Size>1</Size></Contents>");
        assert_eq!(
            parse_list_objects(xml.as_bytes()).unwrap_err().0,
            "<Contents> without <Key>"
        );
        let xml = listing("<Contents><Key>k</Key></Contents>");
        assert_eq!(
            parse_list_objects(xml.as_bytes()).unwrap_err().0,
            "<Contents> without <Size>"
        );
        let xml = listing("<Contents><Key>k</Key><Size>big</Size></Contents>");
        assert_eq!(
            parse_list_objects(xml.as_bytes()).unwrap_err().0,
            "<Size> is not a number: `big`"
        );
    }

    #[test]
    fn malformed_documents_are_rejected() {
        let malformed: [(&[u8], &str); 12] = [
            (b"", "expected the root element at byte 0"),
            (b"   ", "expected the root element at byte 3"),
            (b"<a><b></a>", "<b> closed by </a> at byte 9"),
            (b"<a>", "<a> never closed at byte 3"),
            (b"<a>&nope;</a>", "unknown reference `&nope;` at byte 3"),
            (b"<a>&amp</a>", "unterminated reference `&amp` at byte 3"),
            (b"<a>&#xD800;</a>", "unknown reference `&#xD800;` at byte 3"),
            (b"<a>&#0;</a>", "unknown reference `&#0;` at byte 3"),
            (b"<a>x</a>tail", "content after the root element at byte 8"),
            (b"<a><!-- open </a>", "unterminated comment at byte 7"),
            (
                b"<a x=1></a>",
                "expected a quoted attribute value at byte 5",
            ),
            (
                b"<a>\xFF</a>",
                "not UTF-8: invalid utf-8 sequence of 1 bytes from index 3",
            ),
        ];
        for (xml, message) in malformed {
            assert_eq!(parse_document(xml).unwrap_err().0, message);
        }
    }

    #[test]
    fn nesting_is_bounded() {
        let open = "<a>".repeat(40);
        let close = "</a>".repeat(40);
        let error = parse_document(format!("{open}{close}").as_bytes()).unwrap_err();
        assert!(error.0.starts_with("elements nested deeper than 32"));
        assert!(
            parse_document(format!("{}{}", "<a>".repeat(30), "</a>".repeat(30)).as_bytes()).is_ok()
        );
    }

    #[test]
    fn an_error_document_is_read_with_its_ids() {
        let xml = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>
             <Error xmlns=\"{XMLNS}\"><Code>NoSuchKey</Code><Message>The specified key does not exist.</Message>
             <Key>missing.txt</Key><RequestId>4442587FB7D0A2F9</RequestId><HostId>abc=</HostId></Error>"
        );
        assert_eq!(
            parse_error(xml.as_bytes()),
            Some(ErrorBody {
                code: "NoSuchKey".to_owned(),
                message: "The specified key does not exist.".to_owned(),
                request_id: Some("4442587FB7D0A2F9".to_owned()),
                resource: None,
            })
        );
        let xml = "<Error><Code>AccessDenied</Code><Message>Access Denied</Message><Resource>/bucket/key</Resource></Error>";
        assert_eq!(
            parse_error(xml.as_bytes()).unwrap().resource.as_deref(),
            Some("/bucket/key")
        );
        assert_eq!(parse_error(b"<Error/>").unwrap().code, "");
    }

    #[test]
    fn bytes_that_are_not_an_error_document_answer_none() {
        assert_eq!(parse_error(b""), None);
        assert_eq!(
            parse_error(b"<html><body>502 Bad Gateway</body></html>"),
            None
        );
        assert_eq!(parse_error(b"not xml at all"), None);
        assert_eq!(parse_error(listing("").as_bytes()), None);
        assert_eq!(parse_error(b"<Error><Code>Unclosed</Error>"), None);
    }

    #[test]
    fn the_upload_id_is_read() {
        let xml = format!(
            "<InitiateMultipartUploadResult xmlns=\"{XMLNS}\"><Bucket>b</Bucket><Key>k</Key>
             <UploadId>VXBsb2FkIElEIGZvciA2aWWpbmcncyBteS1tb3ZpZS5tMnRzIHVwbG9hZA</UploadId></InitiateMultipartUploadResult>"
        );
        assert_eq!(
            parse_upload_id(xml.as_bytes()).unwrap(),
            "VXBsb2FkIElEIGZvciA2aWWpbmcncyBteS1tb3ZpZS5tMnRzIHVwbG9hZA"
        );
        let missing = parse_upload_id(b"<InitiateMultipartUploadResult/>").unwrap_err();
        assert_eq!(
            missing.0,
            "<InitiateMultipartUploadResult> without <UploadId>"
        );
        let empty = parse_upload_id(
            b"<InitiateMultipartUploadResult><UploadId/></InitiateMultipartUploadResult>",
        );
        assert_eq!(empty.unwrap_err().0, "empty <UploadId>");
        assert!(parse_upload_id(b"<Error><Code>NoSuchBucket</Code></Error>").is_err());
    }

    #[test]
    fn a_completed_upload_answers_its_etag_even_behind_keepalive_padding() {
        let xml = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\n\n\n<CompleteMultipartUploadResult xmlns=\"{XMLNS}\">
             <Location>https://b.s3.amazonaws.com/k</Location><Bucket>b</Bucket><Key>k</Key>
             <ETag>\"3858f62230ac3c915f300c664312c11f-9\"</ETag></CompleteMultipartUploadResult>"
        );
        assert_eq!(
            parse_complete_multipart(xml.as_bytes()).unwrap().as_deref(),
            Some("\"3858f62230ac3c915f300c664312c11f-9\"")
        );
        assert_eq!(
            parse_complete_multipart(b"<CompleteMultipartUploadResult/>").unwrap(),
            None
        );
    }

    #[test]
    fn a_late_failure_in_a_200_body_is_an_error() {
        let xml = "\n\n<Error><Code>InternalError</Code><Message>We encountered an internal error. Please try again.</Message></Error>";
        let error = parse_complete_multipart(xml.as_bytes()).unwrap_err();
        assert_eq!(
            error.0,
            "InternalError: We encountered an internal error. Please try again."
        );
        assert_eq!(
            parse_complete_multipart(listing("").as_bytes())
                .unwrap_err()
                .0,
            "expected <CompleteMultipartUploadResult>, found <ListBucketResult>"
        );
    }

    #[test]
    fn delete_failures_are_read_and_deleted_entries_ignored() {
        let xml = format!(
            "<DeleteResult xmlns=\"{XMLNS}\"><Deleted><Key>a</Key></Deleted>
             <Error><Key>b &amp; c</Key><Code>AccessDenied</Code><Message>Access Denied</Message></Error>
             <Deleted><Key>d</Key></Deleted>
             <Error><Key>e</Key><Code>InternalError</Code><Message>Try again</Message></Error></DeleteResult>"
        );
        assert_eq!(
            parse_delete_result(xml.as_bytes()).unwrap(),
            vec![
                DeleteFailure {
                    key: "b & c".to_owned(),
                    code: "AccessDenied".to_owned(),
                    message: "Access Denied".to_owned(),
                },
                DeleteFailure {
                    key: "e".to_owned(),
                    code: "InternalError".to_owned(),
                    message: "Try again".to_owned(),
                },
            ]
        );
        assert!(parse_delete_result(b"<DeleteResult/>").unwrap().is_empty());
        assert_eq!(
            parse_delete_result(b"<DeleteResult><Error><Code>x</Code></Error></DeleteResult>")
                .unwrap_err()
                .0,
            "<Error> without <Key>"
        );
    }

    #[test]
    fn create_bucket_names_the_region() {
        assert_eq!(
            render_create_bucket("eu-west-1"),
            "<CreateBucketConfiguration><LocationConstraint>eu-west-1</LocationConstraint></CreateBucketConfiguration>"
        );
        assert!(render_create_bucket("<x>").contains("&lt;x&gt;"));
    }

    #[test]
    fn delete_objects_escapes_keys_and_honors_quiet() {
        let keys = ["a&b<c>\"d'e".to_owned(), "plain".to_owned()];
        assert_eq!(
            render_delete_objects(&keys, true),
            "<Delete><Quiet>true</Quiet><Object><Key>a&amp;b&lt;c&gt;&quot;d&apos;e</Key></Object><Object><Key>plain</Key></Object></Delete>"
        );
        assert_eq!(render_delete_objects(&[], false), "<Delete></Delete>");
    }

    #[test]
    fn complete_multipart_lists_parts_with_escaped_etags() {
        let parts = [(1, "\"a\"".to_owned()), (2, "\"b&c\"".to_owned())];
        assert_eq!(
            render_complete_multipart(&parts),
            "<CompleteMultipartUpload><Part><PartNumber>1</PartNumber><ETag>&quot;a&quot;</ETag></Part><Part><PartNumber>2</PartNumber><ETag>&quot;b&amp;c&quot;</ETag></Part></CompleteMultipartUpload>"
        );
    }

    #[test]
    fn escape_text_covers_the_five_characters() {
        assert_eq!(escape_text("&<>\"' ok é"), "&amp;&lt;&gt;&quot;&apos; ok é");
        assert_eq!(escape_text(""), "");
    }

    #[test]
    fn rendered_bodies_scan_back_to_the_same_text() {
        let keys = ["a&b<c>\"d'e/ф".to_owned(), " lead and trail ".to_owned()];
        let root = parse_document(render_delete_objects(&keys, true).as_bytes()).unwrap();
        let scanned: Vec<&str> = root
            .children("Object")
            .map(|object| object.child_text("Key").unwrap())
            .collect();
        assert_eq!(scanned, keys);
        assert_eq!(root.child_text("Quiet"), Some("true"));
        let root = parse_document(render_complete_multipart(&[(3, "\"e\"".to_owned())]).as_bytes())
            .unwrap();
        let part = root.child("Part").unwrap();
        assert_eq!(part.child_text("PartNumber"), Some("3"));
        assert_eq!(part.child_text("ETag"), Some("\"e\""));
    }
}
