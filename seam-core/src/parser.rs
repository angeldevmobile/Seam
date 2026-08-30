//! The `.seam` front end.
//!
//! ```text
//! file        := declaration*
//! declaration := "schema" ident "{" field* "}"
//! field       := ident ":" "optional"? type rule*
//! type        := base "?"?
//! base        := ident | "[" type "]" | enum
//! enum        := "enum" "{" value ("," value)* ","? "}"
//! value       := ident | string
//! rule        := "@" ident "(" args ")"
//! ```

use crate::schema::{Field, IntType, IntWidth, ObjectType, Presence, Rule, Schema, Type};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub line: usize,
    pub column: usize,
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}: {}", self.line, self.column, self.message)
    }
}

impl std::error::Error for ParseError {}

pub fn parse(source: &str) -> Result<Schema, ParseError> {
    let tokens = lex(source)?;
    let mut p = Parser { toks: tokens, pos: 0, refs: Vec::new() };
    let schema = p.file()?;
    p.resolve(&schema)?;
    Ok(schema)
}

// ---------------------------------------------------------------- lexer

#[derive(Debug, Clone, PartialEq, Eq)]
enum Tok {
    Ident(String),
    Str(String),
    Int(i128),
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    LParen,
    RParen,
    Colon,
    Question,
    At,
    Comma,
    RangeIncl,
    Eof,
}

impl Tok {
    fn describe(&self) -> String {
        match self {
            Tok::Ident(s) => format!("`{s}`"),
            Tok::Str(s) => format!("string \"{s}\""),
            Tok::Int(n) => format!("`{n}`"),
            Tok::LBrace => "`{`".into(),
            Tok::RBrace => "`}`".into(),
            Tok::LBracket => "`[`".into(),
            Tok::RBracket => "`]`".into(),
            Tok::LParen => "`(`".into(),
            Tok::RParen => "`)`".into(),
            Tok::Colon => "`:`".into(),
            Tok::Question => "`?`".into(),
            Tok::At => "`@`".into(),
            Tok::Comma => "`,`".into(),
            Tok::RangeIncl => "`..=`".into(),
            Tok::Eof => "end of input".into(),
        }
    }
}

#[derive(Debug, Clone)]
struct Token {
    tok: Tok,
    line: usize,
    column: usize,
}

fn lex(src: &str) -> Result<Vec<Token>, ParseError> {
    let chars: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    let mut line = 1;
    let mut col = 1;

    while let Some(&c) = chars.get(i) {
        if c == '\n' {
            i += 1;
            line += 1;
            col = 1;
            continue;
        }
        if c.is_whitespace() {
            i += 1;
            col += 1;
            continue;
        }
        if c == '/' && chars.get(i + 1) == Some(&'/') {
            while matches!(chars.get(i), Some(&ch) if ch != '\n') {
                i += 1;
                col += 1;
            }
            continue;
        }

        let (tl, tc) = (line, col);

        if c == '.' && chars.get(i + 1) == Some(&'.') && chars.get(i + 2) == Some(&'=') {
            out.push(Token { tok: Tok::RangeIncl, line: tl, column: tc });
            i += 3;
            col += 3;
            continue;
        }

        let symbol = match c {
            '{' => Some(Tok::LBrace),
            '}' => Some(Tok::RBrace),
            '[' => Some(Tok::LBracket),
            ']' => Some(Tok::RBracket),
            '(' => Some(Tok::LParen),
            ')' => Some(Tok::RParen),
            ':' => Some(Tok::Colon),
            '?' => Some(Tok::Question),
            '@' => Some(Tok::At),
            ',' => Some(Tok::Comma),
            _ => None,
        };
        if let Some(tok) = symbol {
            out.push(Token { tok, line: tl, column: tc });
            i += 1;
            col += 1;
            continue;
        }

        if c == '"' {
            i += 1;
            col += 1;
            let mut s = String::new();
            loop {
                match chars.get(i) {
                    None | Some('\n') => {
                        return Err(ParseError {
                            line: tl,
                            column: tc,
                            message: "unterminated string".into(),
                        })
                    }
                    Some('"') => {
                        i += 1;
                        col += 1;
                        break;
                    }
                    Some(&ch) => {
                        s.push(ch);
                        i += 1;
                        col += 1;
                    }
                }
            }
            out.push(Token { tok: Tok::Str(s), line: tl, column: tc });
            continue;
        }

        let negative = c == '-' && matches!(chars.get(i + 1), Some(ch) if ch.is_ascii_digit());
        if c.is_ascii_digit() || negative {
            let mut s = String::new();
            if negative {
                s.push('-');
                i += 1;
                col += 1;
            }
            while let Some(&ch) = chars.get(i) {
                if ch.is_ascii_digit() {
                    s.push(ch);
                } else if ch != '_' {
                    break;
                }
                i += 1;
                col += 1;
            }
            let n = s.parse::<i128>().map_err(|_| ParseError {
                line: tl,
                column: tc,
                message: format!("`{s}` does not fit a 128-bit integer"),
            })?;
            out.push(Token { tok: Tok::Int(n), line: tl, column: tc });
            continue;
        }

        if c.is_alphabetic() || c == '_' {
            let mut s = String::new();
            while let Some(&ch) = chars.get(i) {
                if ch.is_alphanumeric() || ch == '_' {
                    s.push(ch);
                    i += 1;
                    col += 1;
                } else {
                    break;
                }
            }
            out.push(Token { tok: Tok::Ident(s), line: tl, column: tc });
            continue;
        }

        return Err(ParseError {
            line: tl,
            column: tc,
            message: format!("unexpected character `{c}`"),
        });
    }

    out.push(Token { tok: Tok::Eof, line, column: col });
    Ok(out)
}

// --------------------------------------------------------------- parser

struct Parser {
    toks: Vec<Token>,
    pos: usize,
    /// Type references, checked once the whole file is known so that a schema
    /// may refer to one declared later.
    refs: Vec<(String, usize, usize)>,
}

impl Parser {
    fn cur(&self) -> &Token {
        match self.toks.get(self.pos) {
            Some(t) => t,
            // `lex` always appends Eof, and `bump` never moves past it.
            None => match self.toks.last() {
                Some(t) => t,
                None => &EOF,
            },
        }
    }

    fn at(&self, t: &Tok) -> bool {
        &self.cur().tok == t
    }

    fn at_keyword(&self, kw: &str) -> bool {
        matches!(&self.cur().tok, Tok::Ident(s) if s == kw)
    }

    fn bump(&mut self) -> Token {
        let t = self.cur().clone();
        if !matches!(t.tok, Tok::Eof) {
            self.pos += 1;
        }
        t
    }

    fn eat(&mut self, t: &Tok) -> bool {
        if self.at(t) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn eat_keyword(&mut self, kw: &str) -> bool {
        if self.at_keyword(kw) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn error<T>(&self, message: String) -> Result<T, ParseError> {
        let t = self.cur();
        Err(ParseError { line: t.line, column: t.column, message })
    }

    fn expect(&mut self, t: &Tok) -> Result<(), ParseError> {
        if self.eat(t) {
            Ok(())
        } else {
            let found = self.cur().tok.describe();
            self.error(format!("expected {}, found {found}", t.describe()))
        }
    }

    fn expect_ident(&mut self, what: &str) -> Result<String, ParseError> {
        match &self.cur().tok {
            Tok::Ident(s) => {
                let s = s.clone();
                self.bump();
                Ok(s)
            }
            other => {
                let found = other.describe();
                self.error(format!("expected {what}, found {found}"))
            }
        }
    }

    fn expect_int(&mut self) -> Result<i128, ParseError> {
        match self.cur().tok {
            Tok::Int(n) => {
                self.bump();
                Ok(n)
            }
            ref other => {
                let found = other.describe();
                self.error(format!("expected a number, found {found}"))
            }
        }
    }

    fn file(&mut self) -> Result<Schema, ParseError> {
        let mut schema = Schema::default();
        while !self.at(&Tok::Eof) {
            if !self.at_keyword("schema") {
                let found = self.cur().tok.describe();
                return self.error(format!("expected `schema`, found {found}"));
            }
            let (name, ty) = self.declaration()?;
            if schema.types.contains_key(&name) {
                return self.error(format!("`{name}` is declared more than once"));
            }
            schema.types.insert(name, ty);
        }
        Ok(schema)
    }

    fn declaration(&mut self) -> Result<(String, ObjectType), ParseError> {
        self.bump(); // `schema`
        let name = self.expect_ident("a schema name")?;
        self.expect(&Tok::LBrace)?;

        let mut fields: Vec<Field> = Vec::new();
        while !self.at(&Tok::RBrace) && !self.at(&Tok::Eof) {
            let field = self.field()?;
            if fields.iter().any(|f| f.name == field.name) {
                return self.error(format!("`{name}` declares `{}` more than once", field.name));
            }
            fields.push(field);
        }
        self.expect(&Tok::RBrace)?;

        Ok((
            name.clone(),
            ObjectType { name, fields, deny_unknown_fields: true },
        ))
    }

    fn field(&mut self) -> Result<Field, ParseError> {
        let name = self.expect_ident("a field name")?;
        self.expect(&Tok::Colon)?;
        let optional = self.eat_keyword("optional");
        let (ty, nullable) = self.ty()?;

        let mut rules = Vec::new();
        while self.at(&Tok::At) {
            rules.push(self.rule()?);
        }

        Ok(Field { name, ty, presence: Presence { optional, nullable }, rules })
    }

    /// Returns the type and whether a `?` suffix marked it nullable.
    fn ty(&mut self) -> Result<(Type, bool), ParseError> {
        let base = if self.eat(&Tok::LBracket) {
            let (inner, inner_nullable) = self.ty()?;
            if inner_nullable {
                return self.error(
                    "nullable array items are not supported; \
                     nullability applies to a field, not to an element"
                        .into(),
                );
            }
            self.expect(&Tok::RBracket)?;
            Type::Array(Box::new(inner))
        } else if self.at_keyword("enum") {
            self.enumeration()?
        } else {
            let t = self.cur().clone();
            let name = self.expect_ident("a type")?;
            match builtin(&name) {
                Some(ty) => ty,
                None => {
                    self.refs.push((name.clone(), t.line, t.column));
                    Type::Ref(name)
                }
            }
        };

        let nullable = self.eat(&Tok::Question);
        Ok((base, nullable))
    }

    fn enumeration(&mut self) -> Result<Type, ParseError> {
        self.bump(); // `enum`
        self.expect(&Tok::LBrace)?;

        let mut values: Vec<String> = Vec::new();
        loop {
            if self.at(&Tok::RBrace) {
                break;
            }
            let value = match &self.cur().tok {
                Tok::Ident(s) | Tok::Str(s) => s.clone(),
                other => {
                    let found = other.describe();
                    return self.error(format!("expected an enum value, found {found}"));
                }
            };
            self.bump();
            if values.contains(&value) {
                return self.error(format!("`{value}` is listed twice"));
            }
            values.push(value);
            if !self.eat(&Tok::Comma) {
                break;
            }
        }
        self.expect(&Tok::RBrace)?;

        if values.is_empty() {
            return self.error("an enum needs at least one value".into());
        }
        Ok(Type::Enum(values))
    }

    fn rule(&mut self) -> Result<Rule, ParseError> {
        self.expect(&Tok::At)?;
        let at = self.cur().clone();
        let name = self.expect_ident("a rule name")?;
        self.expect(&Tok::LParen)?;

        let rule = match name.as_str() {
            "min_len" => Rule::MinLen(self.count(&name)?),
            "max_len" => Rule::MaxLen(self.count(&name)?),
            "min_items" => Rule::MinItems(self.count(&name)?),
            "max_items" => Rule::MaxItems(self.count(&name)?),
            "range" => {
                let min = self.expect_int()?;
                self.expect(&Tok::RangeIncl)?;
                let max = self.expect_int()?;
                if min > max {
                    return self.error(format!("`range({min}..={max})` is empty"));
                }
                Rule::Range { min, max }
            }
            _ => {
                return Err(ParseError {
                    line: at.line,
                    column: at.column,
                    message: format!("unknown rule `@{name}`"),
                })
            }
        };

        self.expect(&Tok::RParen)?;
        Ok(rule)
    }

    fn count(&mut self, rule: &str) -> Result<usize, ParseError> {
        let at = self.cur().clone();
        let n = self.expect_int()?;
        usize::try_from(n).map_err(|_| ParseError {
            line: at.line,
            column: at.column,
            message: format!("`@{rule}` needs a non-negative number, found {n}"),
        })
    }

    fn resolve(&self, schema: &Schema) -> Result<(), ParseError> {
        for (name, line, column) in &self.refs {
            if !schema.types.contains_key(name) {
                return Err(ParseError {
                    line: *line,
                    column: *column,
                    message: format!("unknown type `{name}`"),
                });
            }
        }
        Ok(())
    }
}

static EOF: Token = Token { tok: Tok::Eof, line: 1, column: 1 };

fn builtin(name: &str) -> Option<Type> {
    let int = |width, signed| Some(Type::Int(IntType { width, signed }));
    match name {
        "String" => Some(Type::String),
        "bool" => Some(Type::Bool),
        "f64" => Some(Type::Float),
        "Date" => Some(Type::Date),
        "DateTime" => Some(Type::DateTime),
        "i8" => int(IntWidth::W8, true),
        "i16" => int(IntWidth::W16, true),
        "i32" => int(IntWidth::W32, true),
        "i64" => int(IntWidth::W64, true),
        "u8" => int(IntWidth::W8, false),
        "u16" => int(IntWidth::W16, false),
        "u32" => int(IntWidth::W32, false),
        "u64" => int(IntWidth::W64, false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const USER: &str = r#"
schema User {
  id:          u64
  name:        String            @min_len(3) @max_len(64)
  age:         u32               @range(18..=120)
  plan:        enum { free, pro, enterprise }
  tags:        [String]          @max_items(10)

  nickname:    String?           // present, may be null
  bio:         optional String   // may be absent
  avatar:      optional String?  // may be absent OR null
}
"#;

    fn user() -> ObjectType {
        let schema = parse(USER).expect("USER should parse");
        schema.get("User").cloned().expect("User should exist")
    }

    fn field(name: &str) -> Field {
        user().field(name).cloned().expect("field should exist")
    }

    #[test]
    fn parses_the_readme_schema() {
        let u = user();
        assert_eq!(u.name, "User");
        assert_eq!(u.fields.len(), 8);
        assert!(u.deny_unknown_fields);
    }

    #[test]
    fn fields_keep_declaration_order() {
        let names: Vec<_> = user().fields.iter().map(|f| f.name.clone()).collect();
        assert_eq!(
            names,
            ["id", "name", "age", "plan", "tags", "nickname", "bio", "avatar"]
        );
    }

    #[test]
    fn the_four_presence_states_round_trip() {
        assert_eq!(field("id").presence, Presence::required());
        assert_eq!(field("nickname").presence, Presence::nullable());
        assert_eq!(field("bio").presence, Presence::optional());
        assert_eq!(field("avatar").presence, Presence::optional_nullable());
    }

    #[test]
    fn integer_width_and_signedness_survive() {
        assert_eq!(
            field("id").ty,
            Type::Int(IntType { width: IntWidth::W64, signed: false })
        );
        assert_eq!(
            field("age").ty,
            Type::Int(IntType { width: IntWidth::W32, signed: false })
        );
    }

    #[test]
    fn enums_arrays_and_rules_parse() {
        assert_eq!(
            field("plan").ty,
            Type::Enum(vec!["free".into(), "pro".into(), "enterprise".into()])
        );
        assert_eq!(field("tags").ty, Type::Array(Box::new(Type::String)));
        assert_eq!(field("name").rules, vec![Rule::MinLen(3), Rule::MaxLen(64)]);
        assert_eq!(field("age").rules, vec![Rule::Range { min: 18, max: 120 }]);
        assert_eq!(field("tags").rules, vec![Rule::MaxItems(10)]);
    }

    #[test]
    fn comments_are_ignored() {
        let s = parse("// leading\nschema A { x: u8 } // trailing\n").expect("should parse");
        assert!(s.get("A").is_some());
    }

    #[test]
    fn a_type_may_refer_to_one_declared_later() {
        let s = parse("schema A { b: B }\nschema B { x: u8 }").expect("should parse");
        assert_eq!(
            s.get("A").and_then(|a| a.field("b")).map(|f| f.ty.clone()),
            Some(Type::Ref("B".into()))
        );
    }

    fn err(src: &str) -> ParseError {
        parse(src).expect_err("should not parse")
    }

    #[test]
    fn an_unknown_type_is_reported_where_it_is_used() {
        let e = err("schema A { b: Nope }");
        assert_eq!(e.message, "unknown type `Nope`");
        assert_eq!((e.line, e.column), (1, 15));
    }

    #[test]
    fn unknown_rules_are_rejected_rather_than_ignored() {
        assert_eq!(
            err("schema A { x: u8 @nope(1) }").message,
            "unknown rule `@nope`"
        );
    }

    #[test]
    fn structural_mistakes_point_at_the_right_token() {
        assert_eq!(err("schema A { x u8 }").message, "expected `:`, found `u8`");
        assert_eq!(
            err("schema A {").message,
            "expected `}`, found end of input"
        );
        assert_eq!(err("A { }").message, "expected `schema`, found `A`");
    }

    #[test]
    fn duplicates_are_caught() {
        assert!(err("schema A { x: u8\n x: u8 }")
            .message
            .contains("more than once"));
        assert!(err("schema A { x: u8 }\nschema A { y: u8 }")
            .message
            .contains("more than once"));
        assert!(err("schema A { x: enum { a, a } }")
            .message
            .contains("twice"));
    }

    #[test]
    fn nullable_array_items_are_rejected_with_a_reason() {
        assert!(err("schema A { x: [String?] }")
            .message
            .contains("nullable array items"));
    }

    #[test]
    fn rule_arguments_are_checked() {
        assert!(err("schema A { x: String @min_len(-1) }")
            .message
            .contains("non-negative"));
        assert!(err("schema A { x: u8 @range(10..=1) }")
            .message
            .contains("empty"));
    }

    #[test]
    fn quoted_enum_values_allow_characters_idents_cannot_hold() {
        let s =
            parse(r#"schema A { r: enum { "us-east-1", "eu-west-2" } }"#).expect("should parse");
        assert_eq!(
            s.get("A").and_then(|a| a.field("r")).map(|f| f.ty.clone()),
            Some(Type::Enum(vec!["us-east-1".into(), "eu-west-2".into()]))
        );
    }
}
