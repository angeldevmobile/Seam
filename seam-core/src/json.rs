//! JSON in, [`Value`] out.
//!
//! Written here rather than delegated because the boundary rules have to apply
//! while the bytes are read, not after. An integer never passes through an
//! `f64`, which is the corruption the whole project exists to prevent and which
//! a host parser may already have committed before Seam is called.
//!
//! This parses in order to validate. It is not a serialisation library: there
//! is no encoder and no public tree to walk.

use std::collections::BTreeMap;
use std::fmt;

use crate::limits::Limits;
use crate::value::{Int, Value};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonError {
    pub line: usize,
    pub column: usize,
    pub message: String,
}

impl fmt::Display for JsonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}: {}", self.line, self.column, self.message)
    }
}

impl std::error::Error for JsonError {}

/// Parses one JSON document, rejecting anything after it.
pub fn parse(input: &[u8], limits: Limits) -> Result<Value, JsonError> {
    let src = std::str::from_utf8(input).map_err(|e| JsonError {
        line: 1,
        column: e.valid_up_to() + 1,
        message: "input is not valid UTF-8".to_string(),
    })?;

    let mut p = Parser { src: src.as_bytes(), pos: 0, limits };
    p.skip_ws();
    let value = p.value(0)?;
    p.skip_ws();
    if p.pos < p.src.len() {
        return Err(p.error("trailing characters after the document"));
    }
    Ok(value)
}

struct Parser<'a> {
    src: &'a [u8],
    pos: usize,
    limits: Limits,
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

    fn value(&mut self, depth: usize) -> Result<Value, JsonError> {
        if depth > self.limits.max_depth {
            return Err(self.error(format!(
                "nesting deeper than the limit of {}",
                self.limits.max_depth
            )));
        }
        match self.peek() {
            Some(b'{') => self.object(depth),
            Some(b'[') => self.array(depth),
            Some(b'"') => self.string().map(Value::String),
            Some(b't') => self.literal("true", Value::Bool(true)),
            Some(b'f') => self.literal("false", Value::Bool(false)),
            Some(b'n') => self.literal("null", Value::Null),
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(b) => Err(self.error(format!("unexpected `{}`", b as char))),
            None => Err(self.error("unexpected end of input")),
        }
    }

    fn literal(&mut self, word: &str, value: Value) -> Result<Value, JsonError> {
        if self.src[self.pos..].starts_with(word.as_bytes()) {
            self.pos += word.len();
            Ok(value)
        } else {
            Err(self.error(format!("expected `{word}`")))
        }
    }

    fn object(&mut self, depth: usize) -> Result<Value, JsonError> {
        self.expect(b'{')?;
        let mut map = BTreeMap::new();
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(Value::Object(map));
        }
        loop {
            self.skip_ws();
            let key = self.string()?;
            self.skip_ws();
            self.expect(b':')?;
            self.skip_ws();
            let value = self.value(depth + 1)?;
            // Last one wins, as every mainstream parser does.
            map.insert(key, value);
            if map.len() > self.limits.max_object_keys {
                return Err(self.error(format!(
                    "more than {} keys in one object",
                    self.limits.max_object_keys
                )));
            }
            self.skip_ws();
            match self.bump() {
                Some(b',') => {}
                Some(b'}') => return Ok(Value::Object(map)),
                _ => return Err(self.error("expected `,` or `}`")),
            }
        }
    }

    fn array(&mut self, depth: usize) -> Result<Value, JsonError> {
        self.expect(b'[')?;
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(Value::Array(items));
        }
        loop {
            self.skip_ws();
            items.push(self.value(depth + 1)?);
            if items.len() > self.limits.max_items {
                return Err(self.error(format!(
                    "more than {} items in one array",
                    self.limits.max_items
                )));
            }
            self.skip_ws();
            match self.bump() {
                Some(b',') => {}
                Some(b']') => return Ok(Value::Array(items)),
                _ => return Err(self.error("expected `,` or `]`")),
            }
        }
    }

    fn string(&mut self) -> Result<String, JsonError> {
        self.expect(b'"')?;
        let mut out = String::new();
        loop {
            let start = self.pos;
            while !matches!(self.peek(), Some(b'"' | b'\\') | None) {
                self.pos += 1;
            }
            // The run since the last escape is already valid UTF-8, because the
            // whole input was checked before parsing began.
            match std::str::from_utf8(&self.src[start..self.pos]) {
                Ok(chunk) => out.push_str(chunk),
                Err(_) => return Err(self.error("invalid UTF-8 in string")),
            }
            if out.len() > self.limits.max_string_bytes {
                return Err(self.error(format!(
                    "string longer than {} bytes",
                    self.limits.max_string_bytes
                )));
            }
            match self.bump() {
                Some(b'"') => return Ok(out),
                Some(b'\\') => self.escape(&mut out)?,
                _ => return Err(self.error("unterminated string")),
            }
        }
    }

    fn escape(&mut self, out: &mut String) -> Result<(), JsonError> {
        let c = match self.bump() {
            Some(b'"') => '"',
            Some(b'\\') => '\\',
            Some(b'/') => '/',
            Some(b'b') => '\u{0008}',
            Some(b'f') => '\u{000C}',
            Some(b'n') => '\n',
            Some(b'r') => '\r',
            Some(b't') => '\t',
            Some(b'u') => return self.unicode_escape(out),
            _ => return Err(self.error("unknown escape")),
        };
        out.push(c);
        Ok(())
    }

    fn unicode_escape(&mut self, out: &mut String) -> Result<(), JsonError> {
        let first = self.hex4()?;
        let scalar = match first {
            // A high surrogate is only meaningful paired with a low one.
            0xD800..=0xDBFF => {
                if self.bump() != Some(b'\\') || self.bump() != Some(b'u') {
                    return Err(self.error("lone high surrogate"));
                }
                let second = self.hex4()?;
                if !(0xDC00..=0xDFFF).contains(&second) {
                    return Err(self.error("high surrogate not followed by a low one"));
                }
                0x10000 + ((first - 0xD800) << 10) + (second - 0xDC00)
            }
            0xDC00..=0xDFFF => return Err(self.error("lone low surrogate")),
            other => other,
        };
        match char::from_u32(scalar) {
            Some(c) => {
                out.push(c);
                Ok(())
            }
            None => Err(self.error("escape is not a Unicode scalar value")),
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

    fn number(&mut self) -> Result<Value, JsonError> {
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

        let text = match std::str::from_utf8(&self.src[start..self.pos]) {
            Ok(t) => t,
            Err(_) => return Err(self.error("invalid number")),
        };

        if fractional {
            return match text.parse::<f64>() {
                Ok(f) => Ok(Value::Float(f)),
                Err(_) => Err(self.error("number does not fit an f64")),
            };
        }

        // The integer never touches an f64. This is the line the project is
        // about: a host parser that widens here has already lost the value.
        if let Ok(n) = text.parse::<i64>() {
            return Ok(Value::Int(Int::Signed(n)));
        }
        if let Ok(n) = text.parse::<u64>() {
            return Ok(Value::Int(Int::Unsigned(n)));
        }
        Ok(Value::IntTooWide)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(src: &str) -> Value {
        parse(src.as_bytes(), Limits::PERMISSIVE).expect("should parse")
    }

    fn err(src: &str) -> JsonError {
        parse(src.as_bytes(), Limits::PERMISSIVE).expect_err("should not parse")
    }

    #[test]
    fn the_boundary_integer_survives() {
        // The value JavaScript's JSON.parse corrupts to ...992.
        assert_eq!(
            ok("9007199254740993"),
            Value::Int(Int::Signed(9_007_199_254_740_993))
        );
        assert_eq!(
            ok("18446744073709551615"),
            Value::Int(Int::Unsigned(u64::MAX))
        );
        assert_eq!(
            ok("-9223372036854775808"),
            Value::Int(Int::Signed(i64::MIN))
        );
    }

    #[test]
    fn an_integer_past_64_bits_is_kept_as_a_fact_not_a_number() {
        assert_eq!(ok("18446744073709551616"), Value::IntTooWide);
        assert_eq!(ok("-9223372036854775809"), Value::IntTooWide);
    }

    #[test]
    fn floats_stay_floats() {
        assert_eq!(ok("1.5"), Value::Float(1.5));
        assert_eq!(ok("1e3"), Value::Float(1000.0));
        assert_eq!(ok("-0.0"), Value::Float(-0.0));
    }

    #[test]
    fn literals_and_containers() {
        assert_eq!(ok("null"), Value::Null);
        assert_eq!(ok("true"), Value::Bool(true));
        assert_eq!(ok("[]"), Value::Array(vec![]));
        assert_eq!(ok("{}"), Value::Object(BTreeMap::new()));
        assert_eq!(
            ok(r#"{"a":[1,null]}"#),
            Value::Object(BTreeMap::from([(
                "a".to_string(),
                Value::Array(vec![Value::Int(Int::Signed(1)), Value::Null])
            )]))
        );
    }

    #[test]
    fn strings_and_escapes() {
        assert_eq!(ok(r#""hi""#), Value::String("hi".into()));
        assert_eq!(ok(r#""a\"b""#), Value::String("a\"b".into()));
        assert_eq!(ok(r#""ñ""#), Value::String("ñ".into()));
        assert_eq!(ok(r#""😀""#), Value::String("😀".into()));
        assert_eq!(ok(r#""ñ😀""#), Value::String("ñ😀".into()));
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
        assert!(err("{} {}").message.contains("trailing"));
        assert!(err("").message.contains("end of input"));
    }

    #[test]
    fn invalid_utf8_is_rejected() {
        let e = parse(&[b'"', 0xFF, b'"'], Limits::PERMISSIVE).expect_err("should fail");
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
        assert!(parse(deep.as_bytes(), limits)
            .expect_err("should fail")
            .message
            .contains("nesting"));

        let limits = Limits { max_items: 2, ..Limits::DEFAULT };
        assert!(parse(b"[1,2,3]", limits)
            .expect_err("should fail")
            .message
            .contains("items"));

        let limits = Limits { max_string_bytes: 3, ..Limits::DEFAULT };
        assert!(parse(br#""abcd""#, limits)
            .expect_err("should fail")
            .message
            .contains("longer than"));
    }

    #[test]
    fn a_hostile_document_does_not_exhaust_the_stack() {
        let deep = "[".repeat(100_000);
        assert!(parse(deep.as_bytes(), Limits::DEFAULT).is_err());
    }
}
