//! Incremental YAML event decoding with explicit resource accounting.

use std::borrow::Cow;
use std::cell::RefCell;
use std::io::{BufReader, Read};
use std::rc::Rc;

use base64::Engine as _;
use saphyr_parser::{BufferedInput, Event, Parser, ScalarStyle, Tag as SaphyrTag};

use crate::text::wire::RawValue;
use crate::text::{Limits, input_too_large};
use crate::{Error, Result};

type EventParser<'a> = Parser<'a, BufferedInput<Box<dyn Iterator<Item = char> + 'a>>>;

pub(super) struct YamlParser<'a> {
    parser: EventParser<'a>,
    input: Rc<RefCell<InputState>>,
    limits: Limits,
    frames: Vec<Frame>,
    root: Option<RawValue>,
    anchors: Vec<Option<RawValue>>,
    expansions: Vec<usize>,
    nodes: usize,
    tagged_frames: usize,
    documents: usize,
    document_start: usize,
    in_document: bool,
    finished: bool,
}

impl<'a> YamlParser<'a> {
    pub(super) fn new<R: Read + 'a>(reader: R, limits: Limits) -> Self {
        let input = Rc::new(RefCell::new(InputState::default()));
        let characters = Utf8Chars::new(reader, limits.max_input_bytes(), Rc::clone(&input));
        let characters: Box<dyn Iterator<Item = char> + 'a> = Box::new(characters);
        Self {
            parser: Parser::new_from_iter(characters),
            input,
            limits,
            frames: Vec::new(),
            root: None,
            anchors: Vec::new(),
            expansions: Vec::new(),
            nodes: 0,
            tagged_frames: 0,
            documents: 0,
            document_start: 0,
            in_document: false,
            finished: false,
        }
    }

    pub(super) fn byte_offset(&self) -> usize {
        self.input.borrow().bytes
    }

    pub(super) const fn document_start(&self) -> usize {
        self.document_start
    }

    fn byte_position(&self, character_index: usize) -> usize {
        self.input.borrow().byte_position(character_index)
    }

    fn start_document(&mut self, position: usize) -> Result<()> {
        if self.documents >= self.limits.max_documents() {
            return Err(codec_error(position, "document limit exceeded"));
        }
        self.documents = self.documents.saturating_add(1);
        self.document_start = position;
        self.frames.clear();
        self.root = None;
        self.anchors.clear();
        self.expansions.clear();
        self.nodes = 0;
        self.tagged_frames = 0;
        self.in_document = true;
        Ok(())
    }

    fn observe_nodes(&mut self, count: usize, position: usize) -> Result<()> {
        self.nodes = self.nodes.saturating_add(count);
        if self.nodes > self.limits.max_nodes() {
            Err(codec_error(position, "decoded node limit exceeded"))
        } else {
            Ok(())
        }
    }

    fn observe_container_depth(&self, tagged: bool, position: usize) -> Result<()> {
        let depth = self
            .frames
            .len()
            .saturating_add(1)
            .saturating_add(self.tagged_frames)
            .saturating_add(usize::from(tagged));
        if depth > super::MAX_PARSER_DEPTH {
            Err(codec_error(
                position,
                "YAML nesting exceeds the parser hard limit of 384",
            ))
        } else if depth > self.limits.max_depth() {
            Err(codec_error(position, "nesting depth limit exceeded"))
        } else {
            Ok(())
        }
    }

    fn observe_tagged_scalar_depth(&self, position: usize) -> Result<()> {
        let depth = self
            .frames
            .len()
            .saturating_add(self.tagged_frames)
            .saturating_add(1);
        if depth > super::MAX_PARSER_DEPTH {
            Err(codec_error(
                position,
                "YAML nesting exceeds the parser hard limit of 384",
            ))
        } else if depth > self.limits.max_depth() {
            Err(codec_error(position, "nesting depth limit exceeded"))
        } else {
            Ok(())
        }
    }

    fn attach(&mut self, value: RawValue, position: usize) -> Result<()> {
        match self.frames.last_mut() {
            Some(Frame::Sequence { values, .. }) => values.push(normalize_merge_key(value)),
            Some(Frame::Mapping {
                entries,
                key,
                key_positions,
                ..
            }) => {
                if let Some((key, key_position)) = key.take() {
                    if matches!(&key, RawValue::YamlMergeKey) {
                        return Err(codec_error(position, "YAML merge keys are not supported"));
                    }
                    entries.push((key, normalize_merge_key(value)));
                    key_positions.push(key_position);
                } else {
                    *key = Some((value, position));
                }
            }
            None if self.root.is_none() => self.root = Some(normalize_merge_key(value)),
            None => {
                return Err(codec_error(
                    position,
                    "document contains more than one root value",
                ));
            }
        }
        Ok(())
    }

    fn remember_anchor(&mut self, anchor: usize, value: &RawValue) {
        if anchor == 0 {
            return;
        }
        if self.anchors.len() <= anchor {
            self.anchors.resize_with(anchor.saturating_add(1), || None);
        }
        self.anchors[anchor] = Some(value.clone());
    }

    fn alias(&mut self, anchor: usize, position: usize) -> Result<()> {
        let value = self
            .anchors
            .get(anchor)
            .and_then(Option::as_ref)
            .ok_or_else(|| codec_error(position, "unknown YAML anchor"))?;
        let (nodes, depth) = raw_stats(value);
        let parent_depth = self.frames.len().saturating_add(self.tagged_frames);
        let expanded_depth = parent_depth.saturating_add(depth);
        if expanded_depth > super::MAX_PARSER_DEPTH {
            return Err(codec_error(
                position,
                "YAML nesting exceeds the parser hard limit of 384",
            ));
        }
        if expanded_depth > self.limits.max_depth() {
            return Err(codec_error(position, "nesting depth limit exceeded"));
        }
        self.observe_nodes(nodes, position)?;
        if self.expansions.len() <= anchor {
            self.expansions.resize(anchor.saturating_add(1), 0);
        }
        self.expansions[anchor] = self.expansions[anchor].saturating_add(1);
        if self.expansions[anchor] > self.limits.max_nodes() {
            return Err(codec_error(position, "alias expansion limit exceeded"));
        }
        let value = self
            .anchors
            .get(anchor)
            .and_then(Option::as_ref)
            .cloned()
            .ok_or_else(|| codec_error(position, "unknown YAML anchor"))?;
        self.attach(value, position)
    }

    fn fail(&mut self, error: Error) -> Option<Result<RawValue>> {
        self.finished = true;
        Some(Err(error))
    }

    fn input_error(&mut self) -> Option<Result<RawValue>> {
        let error = self.input.borrow_mut().error.take()?;
        let position = error.position;
        let error = if error.limit {
            input_too_large("yaml", position)
        } else {
            codec_error(position, &error.reason)
        };
        self.fail(error)
    }

    fn finish_document(&mut self, position: usize) -> Result<RawValue> {
        if !self.frames.is_empty() {
            return Err(codec_error(position, "unterminated YAML container"));
        }
        self.in_document = false;
        Ok(self.root.take().unwrap_or(RawValue::Null))
    }
}

impl Iterator for YamlParser<'_> {
    type Item = Result<RawValue>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }
        loop {
            let event = self.parser.next();
            if let Some(error) = self.input_error() {
                return Some(error);
            }
            let Some(event) = event else {
                self.finished = true;
                if self.in_document {
                    return Some(self.finish_document(self.byte_offset()));
                }
                return None;
            };
            let (event, span) = match event {
                Ok(event) => event,
                Err(error) => {
                    let position = self.byte_position(error.marker().index());
                    let reason = if error.info() == "recursion limit exceeded"
                        && self.limits.max_depth() > super::MAX_FLOW_DEPTH
                    {
                        "YAML flow nesting exceeds the parser hard limit of 255"
                    } else {
                        error.info()
                    };
                    return self.fail(codec_error(position, reason));
                }
            };
            let position = self.byte_position(span.start.index());
            let result = match event {
                Event::StreamStart | Event::Nothing => continue,
                Event::StreamEnd => {
                    self.finished = true;
                    if self.in_document {
                        return Some(self.finish_document(position));
                    }
                    return None;
                }
                Event::DocumentStart(_) => self.start_document(position),
                Event::DocumentEnd => {
                    let value = self.finish_document(position);
                    if value.is_err() {
                        self.finished = true;
                    }
                    return Some(value);
                }
                Event::Scalar(value, style, anchor, tag) => {
                    if !self.in_document {
                        if let Err(error) = self.start_document(position) {
                            return self.fail(error);
                        }
                    }
                    let custom = custom_tag(tag.as_deref());
                    let tag_position = tag_position(tag.as_deref(), position);
                    if custom.is_some() {
                        if let Err(error) = self.observe_tagged_scalar_depth(position) {
                            return self.fail(error);
                        }
                    }
                    if let Err(error) =
                        self.observe_nodes(1 + usize::from(custom.is_some()), position)
                    {
                        return self.fail(error);
                    }
                    parse_scalar(value, style, tag.as_deref(), position, tag_position).and_then(
                        |value| {
                            self.remember_anchor(anchor, &value);
                            self.attach(value, position)
                        },
                    )
                }
                Event::SequenceStart(anchor, tag) => {
                    if !self.in_document {
                        if let Err(error) = self.start_document(position) {
                            return self.fail(error);
                        }
                    }
                    let tag_position = tag_position(tag.as_deref(), position);
                    let tag = container_tag(tag.as_deref(), "seq", position);
                    match tag {
                        Ok(tag) => self
                            .observe_container_depth(tag.is_some(), position)
                            .and_then(|()| {
                                self.observe_nodes(1 + usize::from(tag.is_some()), position)
                            })
                            .map(|()| {
                                self.tagged_frames = self
                                    .tagged_frames
                                    .saturating_add(usize::from(tag.is_some()));
                                self.frames.push(Frame::Sequence {
                                    values: Vec::new(),
                                    anchor,
                                    tag,
                                    position: tag_position,
                                });
                            }),
                        Err(error) => Err(error),
                    }
                }
                Event::MappingStart(anchor, tag) => {
                    if !self.in_document {
                        if let Err(error) = self.start_document(position) {
                            return self.fail(error);
                        }
                    }
                    let tag_position = tag_position(tag.as_deref(), position);
                    let tag = container_tag(tag.as_deref(), "map", position);
                    match tag {
                        Ok(tag) => self
                            .observe_container_depth(tag.is_some(), position)
                            .and_then(|()| {
                                self.observe_nodes(1 + usize::from(tag.is_some()), position)
                            })
                            .map(|()| {
                                self.tagged_frames = self
                                    .tagged_frames
                                    .saturating_add(usize::from(tag.is_some()));
                                self.frames.push(Frame::Mapping {
                                    entries: Vec::new(),
                                    key: None,
                                    key_positions: Vec::new(),
                                    anchor,
                                    tag,
                                    position: tag_position,
                                });
                            }),
                        Err(error) => Err(error),
                    }
                }
                Event::SequenceEnd => match self.pop_frame() {
                    Some(Frame::Sequence {
                        values,
                        anchor,
                        tag,
                        position: tag_position,
                    }) => {
                        let value = wrap_tag(RawValue::Sequence(values), tag, tag_position);
                        self.remember_anchor(anchor, &value);
                        self.attach(value, position)
                    }
                    _ => Err(codec_error(position, "unexpected YAML sequence end")),
                },
                Event::MappingEnd => match self.pop_frame() {
                    Some(Frame::Mapping {
                        entries,
                        key: None,
                        key_positions,
                        anchor,
                        tag,
                        position: tag_position,
                    }) => {
                        let value = wrap_tag(
                            RawValue::YamlMapping(entries, key_positions),
                            tag,
                            tag_position,
                        );
                        self.remember_anchor(anchor, &value);
                        self.attach(value, position)
                    }
                    Some(Frame::Mapping { .. }) => {
                        Err(codec_error(position, "YAML mapping is missing a value"))
                    }
                    _ => Err(codec_error(position, "unexpected YAML mapping end")),
                },
                Event::Alias(anchor) => self.alias(anchor, position),
            };
            if let Err(error) = result {
                return self.fail(error);
            }
        }
    }
}

impl YamlParser<'_> {
    fn pop_frame(&mut self) -> Option<Frame> {
        let frame = self.frames.pop()?;
        self.tagged_frames = self
            .tagged_frames
            .saturating_sub(usize::from(frame.tag().is_some()));
        Some(frame)
    }
}

enum Frame {
    Sequence {
        values: Vec<RawValue>,
        anchor: usize,
        tag: Option<String>,
        position: usize,
    },
    Mapping {
        entries: Vec<(RawValue, RawValue)>,
        key: Option<(RawValue, usize)>,
        key_positions: Vec<usize>,
        anchor: usize,
        tag: Option<String>,
        position: usize,
    },
}

impl Frame {
    fn tag(&self) -> Option<&str> {
        match self {
            Self::Sequence { tag, .. } | Self::Mapping { tag, .. } => tag.as_deref(),
        }
    }
}

fn wrap_tag(value: RawValue, tag: Option<String>, _position: usize) -> RawValue {
    match tag {
        Some(_) => RawValue::YamlTagged(Box::new(value)),
        None => value,
    }
}

fn normalize_merge_key(value: RawValue) -> RawValue {
    if matches!(&value, RawValue::YamlMergeKey) {
        RawValue::String("<<".to_owned())
    } else {
        value
    }
}

fn container_tag(
    tag: Option<&SaphyrTag>,
    expected: &str,
    position: usize,
) -> Result<Option<String>> {
    let Some(tag) = tag else {
        return Ok(None);
    };
    if tag.is_yaml_core_schema() {
        if tag.suffix == expected {
            return Ok(None);
        }
        return Err(codec_error(
            position,
            "YAML core tag does not match container",
        ));
    }
    Ok(custom_tag(Some(tag)))
}

fn custom_tag(tag: Option<&SaphyrTag>) -> Option<String> {
    let tag = tag?;
    if tag.is_yaml_core_schema() {
        return None;
    }
    if tag.handle == "!" {
        Some(tag.suffix.clone())
    } else {
        Some(tag.to_string().trim_start_matches('!').to_owned())
    }
}

fn tag_position(tag: Option<&SaphyrTag>, value_position: usize) -> usize {
    tag.filter(|tag| !tag.is_yaml_core_schema())
        .map_or(value_position, |tag| {
            value_position.saturating_sub(tag.to_string().len().saturating_add(1))
        })
}

fn parse_scalar(
    value: Cow<'_, str>,
    style: ScalarStyle,
    tag: Option<&SaphyrTag>,
    position: usize,
    tag_position: usize,
) -> Result<RawValue> {
    let value = value.into_owned();
    let merge_key = matches!(style, ScalarStyle::Plain) && tag.is_none() && value == "<<";
    let custom = custom_tag(tag);
    let parsed = if let Some(tag) = tag.filter(|tag| tag.is_yaml_core_schema()) {
        match tag.suffix.as_str() {
            "str" | "timestamp" => RawValue::String(value),
            "null" => RawValue::Null,
            "bool" => parse_bool(&value)
                .map(RawValue::Bool)
                .ok_or_else(|| codec_error(position, "invalid YAML boolean"))?,
            "int" => parse_integer(&value, position)
                .transpose()?
                .ok_or_else(|| codec_error(position, "invalid YAML integer"))?,
            "float" => RawValue::Float(
                parse_float(&value, position)
                    .transpose()?
                    .ok_or_else(|| codec_error(position, "invalid YAML float"))?,
            ),
            "binary" => RawValue::Bytes(
                base64::engine::general_purpose::STANDARD
                    .decode(compact_binary(&value).as_ref())
                    .map_err(|_| codec_error(position, "invalid YAML binary scalar"))?,
            ),
            _ => return Err(codec_error(position, "unsupported YAML core scalar tag")),
        }
    } else if matches!(style, ScalarStyle::Plain) {
        parse_plain(&value, position)?
    } else {
        RawValue::String(value)
    };
    if merge_key {
        Ok(RawValue::YamlMergeKey)
    } else {
        Ok(wrap_tag(parsed, custom, tag_position))
    }
}

fn parse_plain(value: &str, position: usize) -> Result<RawValue> {
    let value = value.trim();
    if value.is_empty() || value == "~" || value.eq_ignore_ascii_case("null") {
        return Ok(RawValue::Null);
    }
    if let Some(value) = parse_bool(value) {
        return Ok(RawValue::Bool(value));
    }
    if let Some(value) = parse_integer(value, position).transpose()? {
        return Ok(value);
    }
    if let Some(value) = parse_plain_float(value, position).transpose()? {
        return Ok(RawValue::Float(value));
    }
    Ok(RawValue::String(value.to_owned()))
}

fn parse_bool(value: &str) -> Option<bool> {
    if value.eq_ignore_ascii_case("true")
        || value.eq_ignore_ascii_case("yes")
        || value.eq_ignore_ascii_case("on")
    {
        Some(true)
    } else if value.eq_ignore_ascii_case("false")
        || value.eq_ignore_ascii_case("no")
        || value.eq_ignore_ascii_case("off")
    {
        Some(false)
    } else {
        None
    }
}

fn parse_integer(value: &str, position: usize) -> Option<Result<RawValue>> {
    let (negative, unsigned) = value.strip_prefix('-').map_or_else(
        || (false, value.strip_prefix('+').unwrap_or(value)),
        |value| (true, value),
    );
    let (radix, digits) = if let Some(value) = unsigned
        .strip_prefix("0x")
        .or_else(|| unsigned.strip_prefix("0X"))
    {
        (16, value)
    } else if let Some(value) = unsigned
        .strip_prefix("0o")
        .or_else(|| unsigned.strip_prefix("0O"))
    {
        (8, value)
    } else if let Some(value) = unsigned
        .strip_prefix("0b")
        .or_else(|| unsigned.strip_prefix("0B"))
    {
        (2, value)
    } else {
        (10, unsigned)
    };
    let prefixed = radix != 10;
    let valid = valid_digit_sequence(digits, radix);
    if !valid {
        return prefixed.then(|| Err(codec_error(position, "invalid YAML integer")));
    }
    let normalized;
    let digits = if digits.as_bytes().contains(&b'_') {
        normalized = digits.replace('_', "");
        normalized.as_str()
    } else {
        digits
    };
    Some(if negative {
        u128::from_str_radix(digits, radix)
            .ok()
            .and_then(|magnitude| {
                if magnitude == (1_u128 << 127) {
                    Some(i128::MIN)
                } else {
                    i128::try_from(magnitude).ok().and_then(i128::checked_neg)
                }
            })
            .map(|value| i64::try_from(value).map_or(RawValue::I128(value), RawValue::I64))
            .ok_or_else(|| {
                codec_error(position, "YAML integer is outside the signed 128-bit range")
            })
    } else {
        u128::from_str_radix(digits, radix)
            .map(|value| u64::try_from(value).map_or(RawValue::U128(value), RawValue::U64))
            .map_err(|_| {
                codec_error(
                    position,
                    "YAML integer is outside the unsigned 128-bit range",
                )
            })
    })
}

fn parse_plain_float(value: &str, position: usize) -> Option<Result<f64>> {
    // The core float regex also matches a bare integer spelling such as `1`, but
    // plain resolution offers every scalar to the integer resolver first, so
    // claiming that spelling here would turn every untagged integer into a
    // float. Only an explicit `!!float` tag resolves it as a float, and that
    // path calls `parse_float` without this restriction.
    if !value.contains(['.', 'e', 'E']) {
        return None;
    }
    parse_float(value, position)
}

fn parse_float(value: &str, position: usize) -> Option<Result<f64>> {
    if value.eq_ignore_ascii_case(".nan")
        || value.eq_ignore_ascii_case("+.nan")
        || value.eq_ignore_ascii_case("-.nan")
    {
        return Some(Ok(f64::NAN));
    }
    if value.eq_ignore_ascii_case(".inf") || value.eq_ignore_ascii_case("+.inf") {
        return Some(Ok(f64::INFINITY));
    }
    if value.eq_ignore_ascii_case("-.inf") {
        return Some(Ok(f64::NEG_INFINITY));
    }
    // Those three are the only non-finite spellings YAML has, yet the Rust float
    // parser also accepts `inf`, `infinity` and `nan`, which the core float
    // regex does not. The exponent marker is the one letter that regex allows,
    // so rejecting every other letter keeps those spellings out and leaves an
    // infinity after parsing meaning one thing only: a finite spelling whose
    // magnitude does not fit an f64.
    if value
        .bytes()
        .any(|byte| byte.is_ascii_alphabetic() && !matches!(byte, b'e' | b'E'))
    {
        return None;
    }
    if !valid_numeric_underscores(value) {
        return None;
    }
    let normalized;
    let value = if value.as_bytes().contains(&b'_') {
        normalized = value.replace('_', "");
        normalized.as_str()
    } else {
        value
    };
    let parsed: f64 = value.parse().ok()?;
    // Losing the magnitude of a finite spelling is an error in every codec here.
    // A spelling that instead rounds down to zero is ordinary IEEE-754 rounding
    // that any conforming producer may emit, so it is accepted and keeps the
    // sign the producer wrote.
    Some(if parsed.is_finite() {
        Ok(parsed)
    } else {
        Err(codec_error(
            position,
            "YAML float is outside the finite f64 range",
        ))
    })
}

fn compact_binary(value: &str) -> Cow<'_, [u8]> {
    if value.bytes().any(|byte| byte.is_ascii_whitespace()) {
        Cow::Owned(
            value
                .bytes()
                .filter(|byte| !byte.is_ascii_whitespace())
                .collect(),
        )
    } else {
        Cow::Borrowed(value.as_bytes())
    }
}

fn valid_digit_sequence(value: &str, radix: u32) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty()
        && bytes.iter().enumerate().all(|(index, byte)| {
            is_radix_digit(*byte, radix)
                || (*byte == b'_'
                    && index != 0
                    && index + 1 < bytes.len()
                    && is_radix_digit(bytes[index - 1], radix)
                    && is_radix_digit(bytes[index + 1], radix))
        })
}

fn valid_numeric_underscores(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.iter().enumerate().all(|(index, byte)| {
        *byte != b'_'
            || (index != 0
                && index + 1 < bytes.len()
                && bytes[index - 1].is_ascii_digit()
                && bytes[index + 1].is_ascii_digit())
    })
}

fn is_radix_digit(byte: u8, radix: u32) -> bool {
    match radix {
        2 => matches!(byte, b'0' | b'1'),
        8 => matches!(byte, b'0'..=b'7'),
        10 => byte.is_ascii_digit(),
        16 => byte.is_ascii_hexdigit(),
        _ => false,
    }
}

fn raw_stats(value: &RawValue) -> (usize, usize) {
    match value {
        RawValue::Sequence(values) => values.iter().fold((1, 1), |(nodes, depth), value| {
            let (child_nodes, child_depth) = raw_stats(value);
            (
                nodes.saturating_add(child_nodes),
                depth.max(child_depth.saturating_add(1)),
            )
        }),
        RawValue::Mapping(entries) | RawValue::YamlMapping(entries, _) => {
            entries.iter().fold((1, 1), |(nodes, depth), (key, value)| {
                let (key_nodes, key_depth) = raw_stats(key);
                let (value_nodes, value_depth) = raw_stats(value);
                (
                    nodes.saturating_add(key_nodes).saturating_add(value_nodes),
                    depth
                        .max(key_depth.saturating_add(1))
                        .max(value_depth.saturating_add(1)),
                )
            })
        }
        RawValue::YamlTagged(value) => raw_stats(value),
        RawValue::YamlMergeKey => (1, 0),
        _ => (1, 0),
    }
}

fn codec_error(position: usize, reason: &str) -> Error {
    Error::Codec {
        format: "yaml",
        position,
        reason: reason.into(),
    }
}

#[derive(Default)]
struct InputState {
    bytes: usize,
    characters: usize,
    extra_bytes: usize,
    multibyte: Vec<(usize, usize)>,
    error: Option<InputError>,
}

impl InputState {
    fn byte_position(&self, character_index: usize) -> usize {
        let index = self
            .multibyte
            .partition_point(|(characters, _)| *characters <= character_index);
        let extra = index
            .checked_sub(1)
            .and_then(|index| self.multibyte.get(index))
            .map_or(0, |(_, extra)| *extra);
        character_index.saturating_add(extra).min(self.bytes)
    }
}

struct InputError {
    position: usize,
    reason: String,
    limit: bool,
}

struct Utf8Chars<R: Read> {
    reader: BufReader<R>,
    limit: usize,
    checked_end: bool,
    state: Rc<RefCell<InputState>>,
}

impl<R: Read> Utf8Chars<R> {
    fn new(reader: R, limit: usize, state: Rc<RefCell<InputState>>) -> Self {
        Self {
            reader: BufReader::new(reader),
            limit,
            checked_end: false,
            state,
        }
    }

    fn record_error(&self, position: usize, reason: impl Into<String>, limit: bool) {
        let mut state = self.state.borrow_mut();
        if state.error.is_none() {
            state.error = Some(InputError {
                position,
                reason: reason.into(),
                limit,
            });
        }
    }
}

impl<R: Read> Iterator for Utf8Chars<R> {
    type Item = char;

    fn next(&mut self) -> Option<Self::Item> {
        if self.checked_end || self.state.borrow().error.is_some() {
            return None;
        }
        let bytes = self.state.borrow().bytes;
        if bytes >= self.limit {
            let mut sentinel = [0_u8; 1];
            return match self.reader.read(&mut sentinel) {
                Ok(0) => {
                    self.checked_end = true;
                    None
                }
                Ok(_) => {
                    self.record_error(self.limit, "input byte limit exceeded", true);
                    None
                }
                Err(error) => {
                    self.record_error(bytes, error.to_string(), false);
                    None
                }
            };
        }

        let mut encoded = [0_u8; 4];
        match self.reader.read(&mut encoded[..1]) {
            Ok(0) => {
                self.checked_end = true;
                return None;
            }
            Ok(_) => {}
            Err(error) => {
                self.record_error(bytes, error.to_string(), false);
                return None;
            }
        }
        let length = match encoded[0] {
            0x00..=0x7f => 1,
            0xc2..=0xdf => 2,
            0xe0..=0xef => 3,
            0xf0..=0xf4 => 4,
            _ => {
                self.record_error(bytes, "invalid UTF-8 leading byte", false);
                return None;
            }
        };
        if bytes.saturating_add(length) > self.limit {
            self.record_error(self.limit, "input byte limit exceeded", true);
            return None;
        }
        if length > 1 {
            if let Err(error) = self.reader.read_exact(&mut encoded[1..length]) {
                self.record_error(bytes, error.to_string(), false);
                return None;
            }
        }
        let character = match std::str::from_utf8(&encoded[..length]) {
            Ok(value) => value.chars().next(),
            Err(error) => {
                self.record_error(bytes, error.to_string(), false);
                return None;
            }
        }?;
        let mut state = self.state.borrow_mut();
        state.bytes = state.bytes.saturating_add(length);
        state.characters = state.characters.saturating_add(1);
        if length > 1 {
            state.extra_bytes = state.extra_bytes.saturating_add(length - 1);
            let characters = state.characters;
            let extra_bytes = state.extra_bytes;
            state.multibyte.push((characters, extra_bytes));
        }
        Some(character)
    }
}
