//! JSON read in place.
//!
//! The boundary rules apply while the bytes are read: an integer never passes
//! through an `f64`, which is the corruption a host parser may already have
//! committed before Seam is called.
//!
//! Parsing records where each value sits rather than copying it out, so a
//! string is borrowed from the caller's buffer and a rejected document is never
//! materialised. [`Ref`] implements [`Input`], so the validator walks the bytes.
//!
//! This parses in order to validate. There is no encoder.

use std::borrow::Cow;

use crate::input::{Input, Kind};
use crate::limits::Limits;
use crate::value::{Int, Slot};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonError {
    pub line: usize,
    pub column: usize,
    pub message: String,
}

impl std::fmt::Display for JsonError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}: {}", self.line, self.column, self.message)
    }
}

impl std::error::Error for JsonError {}

#[derive(Debug, Clone, Copy)]
struct Span {
    start: u32,
    len: u32,
    escaped: bool,
}

#[derive(Debug, Clone, Copy)]
struct Member {
    key: Span,
    node: u32,
}

#[derive(Debug, Clone, Copy)]
enum Node {
    Null,
    Bool(bool),
    Int(Int),
    IntTooWide,
    Float(f64),
    Str(Span),
    Array { first: u32, len: u32 },
    Object { first: u32, len: u32 },
}

/// A parsed document: where every value lives, not a copy of it.
#[derive(Debug)]
pub struct Document<'a> {
    src: &'a [u8],
    nodes: Vec<Node>,
    items: Vec<u32>,
    members: Vec<Member>,
    root: u32,
}

impl<'a> Document<'a> {
    /// Parses one document, rejecting anything after it.
    pub fn parse(input: &'a [u8], limits: Limits) -> Result<Self, JsonError> {
        if u32::try_from(input.len()).is_err() {
            return Err(JsonError {
                line: 1,
                column: 1,
                message: "document larger than 4 GiB".to_string(),
            });
        }
        if std::str::from_utf8(input).is_err() {
            return Err(JsonError {
                line: 1,
                column: 1,
                message: "input is not valid UTF-8".to_string(),
            });
        }

        let mut p = Parser {
            src: input,
            pos: 0,
            limits,
            nodes: Vec::new(),
            items: Vec::new(),
            members: Vec::new(),
        };
        p.skip_ws();
        let root = p.value(0)?;
        p.skip_ws();
        if p.pos < p.src.len() {
            return Err(p.error("trailing characters after the document"));
        }
        Ok(Document {
            src: input,
            nodes: p.nodes,
            items: p.items,
            members: p.members,
            root,
        })
    }

    /// The whole document, ready to validate.
    #[must_use]
    pub fn root(&self) -> Ref<'a, '_> {
        Ref { doc: self, node: self.root }
    }

    fn bytes(&self, span: Span) -> &'a [u8] {
        let end = (span.start + span.len) as usize;
        self.src.get(span.start as usize..end).unwrap_or(&[])
    }

    fn text(&self, span: Span) -> Cow<'a, str> {
        // Checked once, for the whole input, before parsing began.
        let raw = std::str::from_utf8(self.bytes(span)).unwrap_or("");
        if span.escaped {
            Cow::Owned(unescape(raw))
        } else {
            Cow::Borrowed(raw)
        }
    }
}

/// One value inside a [`Document`].
#[derive(Clone, Copy)]
pub struct Ref<'a, 'd> {
    doc: &'d Document<'a>,
    node: u32,
}

impl<'a, 'd> Ref<'a, 'd> {
    fn node(&self) -> Node {
        self.doc
            .nodes
            .get(self.node as usize)
            .copied()
            .unwrap_or(Node::Null)
    }

    fn at(&self, node: u32) -> Ref<'a, 'd> {
        Ref { doc: self.doc, node }
    }

    fn members(&self) -> &'d [Member] {
        match self.node() {
            Node::Object { first, len } => self
                .doc
                .members
                .get(first as usize..(first + len) as usize)
                .unwrap_or(&[]),
            _ => &[],
        }
    }

    /// The text of a string value, borrowed unless it carried escapes.
    #[must_use]
    pub fn text(&self) -> Option<Cow<'a, str>> {
        match self.node() {
            Node::Str(span) => Some(self.doc.text(span)),
            _ => None,
        }
    }

    /// Members of an object, in the order they appeared.
    pub fn entries(&self) -> impl Iterator<Item = (Cow<'a, str>, Ref<'a, 'd>)> + '_ {
        self.members()
            .iter()
            .map(move |m| (self.doc.text(m.key), self.at(m.node)))
    }

    /// Elements of an array, in order.
    pub fn elements(&self) -> impl Iterator<Item = Ref<'a, 'd>> + '_ {
        let items = match self.node() {
            Node::Array { first, len } => self
                .doc
                .items
                .get(first as usize..(first + len) as usize)
                .unwrap_or(&[]),
            _ => &[],
        };
        items.iter().map(move |&n| self.at(n))
    }
}

impl Input for Ref<'_, '_> {
    type Child<'x>
        = Self
    where
        Self: 'x;

    fn kind(&self) -> Kind {
        match self.node() {
            Node::Null => Kind::Null,
            Node::Bool(_) => Kind::Bool,
            Node::Int(_) => Kind::Int,
            Node::IntTooWide => Kind::IntegerTooWide,
            Node::Float(_) => Kind::Float,
            Node::Str(_) => Kind::String,
            Node::Array { .. } => Kind::Array,
            Node::Object { .. } => Kind::Object,
        }
    }

    fn as_bool(&self) -> Option<bool> {
        match self.node() {
            Node::Bool(b) => Some(b),
            _ => None,
        }
    }

    fn as_int(&self) -> Option<Int> {
        match self.node() {
            Node::Int(n) => Some(n),
            _ => None,
        }
    }

    fn as_f64(&self) -> Option<f64> {
        match self.node() {
            Node::Float(f) => Some(f),
            Node::Int(n) => Some(n.as_i128() as f64),
            _ => None,
        }
    }

    fn as_str(&self) -> Option<Cow<'_, str>> {
        self.text()
    }

    fn len(&self) -> usize {
        match self.node() {
            Node::Array { len, .. } | Node::Object { len, .. } => len as usize,
            _ => 0,
        }
    }

    fn item(&self, index: usize) -> Option<Self::Child<'_>> {
        let Node::Array { first, len } = self.node() else {
            return None;
        };
        if index >= len as usize {
            return None;
        }
        self.doc
            .items
            .get(first as usize + index)
            .map(|&n| self.at(n))
    }

    fn slot(&self, key: &str) -> Slot<Self::Child<'_>> {
        // Linear over the members: objects at a boundary have a handful of
        // keys, and comparing byte slices beats hashing at that size.
        for m in self.members() {
            let matches = if m.key.escaped {
                self.doc.text(m.key) == key
            } else {
                self.doc.bytes(m.key) == key.as_bytes()
            };
            if matches {
                let child = self.at(m.node);
                return match child.node() {
                    Node::Null => Slot::Null,
                    _ => Slot::Present(child),
                };
            }
        }
        Slot::Absent
    }

    fn each_key(&self, f: &mut dyn FnMut(&str)) {
        for m in self.members() {
            f(&self.doc.text(m.key));
        }
    }
}

fn unescape(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('"') => out.push('"'),
            Some('\\') => out.push('\\'),
            Some('/') => out.push('/'),
            Some('b') => out.push('\u{0008}'),
            Some('f') => out.push('\u{000C}'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some('u') => {
                let first = hex4(&mut chars);
                let scalar = if (0xD800..=0xDBFF).contains(&first) {
                    // The parser accepted this, so the pair is well formed.
                    chars.next();
                    chars.next();
                    let second = hex4(&mut chars);
                    0x10000 + ((first - 0xD800) << 10) + (second - 0xDC00)
                } else {
                    first
                };
                out.push(char::from_u32(scalar).unwrap_or('\u{FFFD}'));
            }
            _ => {}
        }
    }
    out
}

fn hex4(chars: &mut std::str::Chars<'_>) -> u32 {
    let mut n = 0;
    for _ in 0..4 {
        n = n * 16 + chars.next().and_then(|c| c.to_digit(16)).unwrap_or(0);
    }
    n
}

struct Parser<'a> {
    src: &'a [u8],
    pos: usize,
    limits: Limits,
    nodes: Vec<Node>,
    items: Vec<u32>,
    members: Vec<Member>,
}

impl Parser<'_> {
    fn error(&self, message: impl Into<String>) -> JsonError {
        let mut line = 1;
        let mut column = 1;
        for &b in self.src.iter().take(self.pos) {
            if b == b'\n' {
                line += 1;
                column = 1;
            } else {
                column += 1;
            }
        }
        JsonError { line, column, message: message.into() }
    }

    fn push(&mut self, node: Node) -> u32 {
        self.nodes.push(node);
        (self.nodes.len() - 1) as u32
    }

    fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let b = self.peek()?;
        self.pos += 1;
        Some(b)
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.pos += 1;
        }
    }

    fn expect(&mut self, byte: u8) -> Result<(), JsonError> {
        if self.peek() == Some(byte) {
            self.pos += 1;
            Ok(())
        } else {
            Err(self.error(format!("expected `{}`", byte as char)))
        }
    }

    fn value(&mut self, depth: usize) -> Result<u32, JsonError> {
        if depth > self.limits.max_depth {
            return Err(self.error(format!(
                "nesting deeper than the limit of {}",
                self.limits.max_depth
            )));
        }
        match self.peek() {
            Some(b'{') => self.object(depth),
            Some(b'[') => self.array(depth),
            Some(b'"') => {
                let span = self.string()?;
                Ok(self.push(Node::Str(span)))
            }
            Some(b't') => self.literal("true", Node::Bool(true)),
            Some(b'f') => self.literal("false", Node::Bool(false)),
            Some(b'n') => self.literal("null", Node::Null),
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(b) => Err(self.error(format!("unexpected `{}`", b as char))),
            None => Err(self.error("unexpected end of input")),
        }
    }

    fn literal(&mut self, word: &str, node: Node) -> Result<u32, JsonError> {
        if self.src[self.pos..].starts_with(word.as_bytes()) {
            self.pos += word.len();
            Ok(self.push(node))
        } else {
            Err(self.error(format!("expected `{word}`")))
        }
    }

    fn object(&mut self, depth: usize) -> Result<u32, JsonError> {
        self.expect(b'{')?;
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(self.push(Node::Object { first: 0, len: 0 }));
        }
        let mut found: Vec<Member> = Vec::new();
        loop {
            self.skip_ws();
            let key = self.string()?;
            self.skip_ws();
            self.expect(b':')?;
            self.skip_ws();
            let node = self.value(depth + 1)?;

            // Last one wins, as every mainstream parser does.
            let existing = found.iter().position(|m| {
                let a = self
                    .src
                    .get(m.key.start as usize..(m.key.start + m.key.len) as usize);
                let b = self
                    .src
                    .get(key.start as usize..(key.start + key.len) as usize);
                a == b
            });
            match existing {
                Some(i) => found[i].node = node,
                None => found.push(Member { key, node }),
            }
            if found.len() > self.limits.max_object_keys {
                return Err(self.error(format!(
                    "more than {} keys in one object",
                    self.limits.max_object_keys
                )));
            }

            self.skip_ws();
            match self.bump() {
                Some(b',') => {}
                Some(b'}') => {
                    let first = self.members.len() as u32;
                    let len = found.len() as u32;
                    self.members.extend_from_slice(&found);
                    return Ok(self.push(Node::Object { first, len }));
                }
                _ => return Err(self.error("expected `,` or `}`")),
            }
        }
    }

    fn array(&mut self, depth: usize) -> Result<u32, JsonError> {
        self.expect(b'[')?;
        self.skip_ws();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(self.push(Node::Array { first: 0, len: 0 }));
        }
        let mut found: Vec<u32> = Vec::new();
        loop {
            self.skip_ws();
            found.push(self.value(depth + 1)?);
            if found.len() > self.limits.max_items {
                return Err(self.error(format!(
                    "more than {} items in one array",
                    self.limits.max_items
                )));
            }
            self.skip_ws();
            match self.bump() {
                Some(b',') => {}
                Some(b']') => {
                    let first = self.items.len() as u32;
                    let len = found.len() as u32;
                    self.items.extend_from_slice(&found);
                    return Ok(self.push(Node::Array { first, len }));
                }
                _ => return Err(self.error("expected `,` or `]`")),
            }
        }
    }

    /// Records where the string is instead of copying it out.
    fn string(&mut self) -> Result<Span, JsonError> {
        self.expect(b'"')?;
        let start = self.pos;
        let mut escaped = false;
        loop {
            match self.bump() {
                Some(b'"') => {
                    let len = (self.pos - 1 - start) as u32;
                    if len as usize > self.limits.max_string_bytes {
                        return Err(self.error(format!(
                            "string longer than {} bytes",
                            self.limits.max_string_bytes
                        )));
                    }
                    return Ok(Span { start: start as u32, len, escaped });
                }
                Some(b'\\') => {
                    escaped = true;
                    self.check_escape()?;
                }
                Some(_) => {}
                None => return Err(self.error("unterminated string")),
            }
        }
    }

    fn check_escape(&mut self) -> Result<(), JsonError> {
        match self.bump() {
            Some(b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't') => Ok(()),
            Some(b'u') => match self.hex4()? {
                0xD800..=0xDBFF => {
                    if self.bump() != Some(b'\\') || self.bump() != Some(b'u') {
                        return Err(self.error("lone high surrogate"));
                    }
                    if !(0xDC00..=0xDFFF).contains(&self.hex4()?) {
                        return Err(self.error("high surrogate not followed by a low one"));
                    }
                    Ok(())
                }
                0xDC00..=0xDFFF => Err(self.error("lone low surrogate")),
                _ => Ok(()),
            },
            _ => Err(self.error("unknown escape")),
        }
    }

    fn hex4(&mut self) -> Result<u32, JsonError> {
        let mut n = 0;
        for _ in 0..4 {
            let d = match self.bump() {
                Some(b @ b'0'..=b'9') => u32::from(b - b'0'),
                Some(b @ b'a'..=b'f') => u32::from(b - b'a') + 10,
                Some(b @ b'A'..=b'F') => u32::from(b - b'A') + 10,
                _ => return Err(self.error("expected four hex digits")),
            };
            n = n * 16 + d;
        }
        Ok(n)
    }

    fn number(&mut self) -> Result<u32, JsonError> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        match self.peek() {
            // A leading zero may not be followed by another digit.
            Some(b'0') => self.pos += 1,
            Some(b'1'..=b'9') => {
                while matches!(self.peek(), Some(b'0'..=b'9')) {
                    self.pos += 1;
                }
            }
            _ => return Err(self.error("expected a digit")),
        }

        let mut fractional = false;
        if self.peek() == Some(b'.') {
            fractional = true;
            self.pos += 1;
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(self.error("expected a digit after `.`"));
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            fractional = true;
            self.pos += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(self.error("expected a digit in the exponent"));
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
        }

        let text = std::str::from_utf8(&self.src[start..self.pos]).unwrap_or("");
        if fractional {
            return match text.parse::<f64>() {
                Ok(f) => Ok(self.push(Node::Float(f))),
                Err(_) => Err(self.error("number does not fit an f64")),
            };
        }
        // The integer never touches an f64. This is the line the project is
        // about: a host parser that widens here has already lost the value.
        if let Ok(n) = text.parse::<i64>() {
            return Ok(self.push(Node::Int(Int::Signed(n))));
        }
        if let Ok(n) = text.parse::<u64>() {
            return Ok(self.push(Node::Int(Int::Unsigned(n))));
        }
        Ok(self.push(Node::IntTooWide))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(src: &str) -> Document<'_> {
        Document::parse(src.as_bytes(), Limits::PERMISSIVE).expect("should parse")
    }

    fn err(src: &str) -> JsonError {
        Document::parse(src.as_bytes(), Limits::PERMISSIVE).expect_err("should not parse")
    }

    fn present<'a, 'd>(r: &Ref<'a, 'd>, key: &str) -> Ref<'a, 'd> {
        match r.slot(key) {
            Slot::Present(v) => v,
            _ => panic!("expected a value at `{key}`"),
        }
    }

    #[test]
    fn the_boundary_integer_survives() {
        // The value JavaScript's JSON.parse corrupts to ...992.
        assert_eq!(
            doc("9007199254740993").root().as_int(),
            Some(Int::Signed(9_007_199_254_740_993))
        );
        assert_eq!(
            doc("18446744073709551615").root().as_int(),
            Some(Int::Unsigned(u64::MAX))
        );
        assert_eq!(
            doc("-9223372036854775808").root().as_int(),
            Some(Int::Signed(i64::MIN))
        );
    }

    #[test]
    fn an_integer_past_64_bits_is_a_kind_not_a_number() {
        assert_eq!(
            doc("18446744073709551616").root().kind(),
            Kind::IntegerTooWide
        );
        assert_eq!(
            doc("-9223372036854775809").root().kind(),
            Kind::IntegerTooWide
        );
    }

    #[test]
    fn a_plain_string_is_borrowed_not_copied() {
        let d = doc(r#""hello""#);
        assert!(matches!(d.root().text(), Some(Cow::Borrowed("hello"))));
    }

    #[test]
    fn an_escaped_string_is_decoded_on_demand() {
        assert_eq!(doc(r#""a\"b""#).root().text().as_deref(), Some("a\"b"));
        assert_eq!(doc(r#""a\tb""#).root().text().as_deref(), Some("a\tb"));
        assert_eq!(doc(r#""ñ""#).root().text().as_deref(), Some("ñ"));
        assert_eq!(doc(r#""😀""#).root().text().as_deref(), Some("😀"));
    }

    #[test]
    fn utf8_passes_through_untouched() {
        assert_eq!(doc(r#""ñ😀""#).root().text().as_deref(), Some("ñ😀"));
    }

    #[test]
    fn objects_and_arrays_are_walkable() {
        let d = doc(r#"{"a": [1, null], "b": true}"#);
        let root = d.root();
        assert_eq!(root.kind(), Kind::Object);
        assert_eq!(root.len(), 2);

        let a = present(&root, "a");
        assert_eq!(a.kind(), Kind::Array);
        assert_eq!(a.len(), 2);
        assert_eq!(a.item(0).and_then(|v| v.as_int()), Some(Int::Signed(1)));
        assert_eq!(a.item(1).map(|v| v.kind()), Some(Kind::Null));
        assert!(a.item(2).is_none());

        assert_eq!(present(&root, "b").as_bool(), Some(true));
        assert!(matches!(root.slot("missing"), Slot::Absent));
    }

    #[test]
    fn a_null_member_is_null_and_a_missing_one_is_absent() {
        let d = doc(r#"{"n": null}"#);
        assert!(matches!(d.root().slot("n"), Slot::Null));
        assert!(matches!(d.root().slot("gone"), Slot::Absent));
    }

    #[test]
    fn keys_are_visited_in_order() {
        let d = doc(r#"{"b": 1, "a": 2, "c": 3}"#);
        let mut seen = Vec::new();
        d.root().each_key(&mut |k| seen.push(k.to_string()));
        assert_eq!(seen, ["b", "a", "c"]);
    }

    #[test]
    fn a_duplicated_key_keeps_the_last() {
        let d = doc(r#"{"a": 1, "a": 2}"#);
        assert_eq!(d.root().len(), 1);
        assert_eq!(present(&d.root(), "a").as_int(), Some(Int::Signed(2)));
    }

    #[test]
    fn an_escaped_key_still_matches() {
        let d = doc(r#"{"abc": 1}"#);
        assert_eq!(present(&d.root(), "abc").as_int(), Some(Int::Signed(1)));
    }

    #[test]
    fn nesting_is_walkable() {
        let d = doc(r#"{"a": {"b": {"c": 7}}}"#);
        let mut here = d.root();
        for key in ["a", "b", "c"] {
            here = present(&here, key);
        }
        assert_eq!(here.as_int(), Some(Int::Signed(7)));
    }

    #[test]
    fn floats_stay_floats() {
        assert_eq!(doc("1.5").root().as_f64(), Some(1.5));
        assert_eq!(doc("1e3").root().as_f64(), Some(1000.0));
        assert_eq!(doc("1").root().as_f64(), Some(1.0));
    }

    #[test]
    fn malformed_input_is_rejected() {
        assert!(err("01").message.contains("trailing"));
        assert!(err("+1").message.contains("unexpected"));
        assert!(err(".5").message.contains("unexpected"));
        assert!(err("5.").message.contains("after `.`"));
        assert!(err("1e").message.contains("exponent"));
        assert!(err(r#"{"a":1,}"#).message.contains("expected"));
        assert!(err("[1,]").message.contains("unexpected"));
        assert!(err(r#""unterminated"#).message.contains("unterminated"));
        assert!(err(r#""\ud800""#).message.contains("surrogate"));
        assert!(err(r#""\q""#).message.contains("escape"));
        assert!(err("{} {}").message.contains("trailing"));
        assert!(err("").message.contains("end of input"));
    }

    #[test]
    fn invalid_utf8_is_rejected() {
        let e = Document::parse(&[b'"', 0xFF, b'"'], Limits::PERMISSIVE).expect_err("should fail");
        assert!(e.message.contains("UTF-8"));
    }

    #[test]
    fn errors_carry_a_position() {
        let e = err("{\n  \"a\": xyz\n}");
        assert_eq!((e.line, e.column), (2, 8));
    }

    #[test]
    fn limits_are_enforced_while_reading() {
        let deep = "[".repeat(40) + &"]".repeat(40);
        let limits = Limits { max_depth: 8, ..Limits::DEFAULT };
        assert!(Document::parse(deep.as_bytes(), limits)
            .expect_err("should fail")
            .message
            .contains("nesting"));

        let limits = Limits { max_items: 2, ..Limits::DEFAULT };
        assert!(Document::parse(b"[1,2,3]", limits)
            .expect_err("should fail")
            .message
            .contains("items"));

        let limits = Limits { max_string_bytes: 3, ..Limits::DEFAULT };
        assert!(Document::parse(br#""abcd""#, limits)
            .expect_err("should fail")
            .message
            .contains("longer than"));
    }

    #[test]
    fn a_hostile_document_does_not_exhaust_the_stack() {
        let deep = "[".repeat(100_000);
        assert!(Document::parse(deep.as_bytes(), Limits::DEFAULT).is_err());
    }
}
