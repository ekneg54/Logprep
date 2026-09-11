use std::sync::LazyLock;

use pyo3::prelude::*;
use pyo3::types::PyDict;
use regex::Regex;

use super::expression::{
    build_sigma_regex, build_wildcard_regex, normalize_regex, FilterExpressionInner,
    NumericBound, PyFilterExpression,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LuceneParseError(pub String);

impl std::fmt::Display for LuceneParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for LuceneParseError {}

// ═══════════════════════════════════════════════════════════
// Lucene-Escape-Protokoll (Port der Python-Helfer aus
// logprep/filter/lucene_filter.py — add/remove lucene escaping)
// ═══════════════════════════════════════════════════════════

static QUOTE_ESCAPING_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?:\\)+""#).unwrap());
static LAST_QUOTATION_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"((?:\\)+")$"#).unwrap());
static END_ESCAPING_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"((?:\\)+"[\s\)]+(?:AND|OR|NOT|$))"#).unwrap());
static FIND_UNESCAPING_QUOTE_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?:\\)*""#).unwrap());
static FIND_UNESCAPING_END_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:\\)*$").unwrap());

/// Interleaves split-Teile mit Ersetzungen (wie Python zip_longest(split, matches)).
fn interleave_split_matches(split: Vec<&str>, replacements: Vec<String>) -> String {
    let mut result = String::new();
    for (idx, part) in split.iter().enumerate() {
        result.push_str(part);
        if idx < replacements.len() {
            result.push_str(&replacements[idx]);
        }
    }
    result
}

/// Port von `LuceneFilter._make_uneven_double_quotes_escaping`:
/// verdoppelt Backslash-Läufe vor Anführungszeichen und fügt eines hinzu.
fn make_uneven_double_quotes_escaping(query_string: &str) -> String {
    let mut replacements = Vec::new();
    for m in QUOTE_ESCAPING_PATTERN.find_iter(query_string) {
        let text = m.as_str();
        let cnt_backslashes = text.len() - 1;
        if cnt_backslashes > 0 {
            replacements.push(format!("{}\\\"", "\\".repeat(2 * cnt_backslashes)));
        } else {
            replacements.push(text.to_string());
        }
    }
    let split = QUOTE_ESCAPING_PATTERN.split(query_string).collect();
    interleave_split_matches(split, replacements)
}

/// Port von `LuceneFilter._escape_ends_of_expressions`.
fn escape_ends_of_expressions(query_string: &str) -> String {
    // Python re.split mit Capturing Group: die Matches bleiben im Ergebnis.
    let mut parts: Vec<String> = Vec::new();
    let mut last_end = 0;
    for m in END_ESCAPING_PATTERN.find_iter(query_string) {
        parts.push(query_string[last_end..m.start()].to_string());
        parts.push(m.as_str().to_string());
        last_end = m.end();
    }
    parts.push(query_string[last_end..].to_string());

    let mut new_string = String::new();
    for part in parts.iter().filter(|p| !p.is_empty()) {
        if END_ESCAPING_PATTERN.is_match(part) && part.starts_with('\\') {
            let backslash_count = part.bytes().take_while(|&b| b == b'\\').count();
            new_string.push_str(&"\\".repeat(backslash_count * 2));
            new_string.push_str(&part[backslash_count..]);
        } else {
            new_string.push_str(part);
        }
    }
    if let Some(m) = LAST_QUOTATION_PATTERN.find(&new_string) {
        let cnt_backslashes = m.as_str().len() - 1;
        let new_end = format!("{}\"", "\\".repeat(cnt_backslashes * 2));
        new_string = format!("{}{}", &new_string[..m.start()], new_end);
    }
    new_string
}

/// Port von `LuceneFilter._add_lucene_escaping`.
fn add_lucene_escaping(query_string: &str) -> String {
    let s = make_uneven_double_quotes_escaping(query_string);
    escape_ends_of_expressions(&s)
}

/// Port von `LuceneTransformer._remove_escaping_from_end_of_expression`.
fn remove_escaping_from_end_of_expression(s: &str) -> String {
    let mut string = s.to_string();
    if let Some(m) = FIND_UNESCAPING_END_PATTERN.find(&string) {
        let cnt = m.as_str().len();
        string = format!("{}{}", &string[..m.start()], "\\".repeat(cnt / 2));
        if let Some(m2) = FIND_UNESCAPING_END_PATTERN.find(&string) {
            let cnt2 = m2.as_str().len();
            let new_escaping = "\\".repeat(cnt2.saturating_sub(1) / 2);
            string = format!("{}{}", &string[..m2.start()], new_escaping);
        }
    }
    string
}

/// Port von `LuceneTransformer._remove_uneven_double_quotes_escaping`.
fn remove_uneven_double_quotes_escaping(s: &str) -> String {
    let mut replacements = Vec::new();
    for m in FIND_UNESCAPING_QUOTE_PATTERN.find_iter(s) {
        let text = m.as_str();
        let cnt_backslashes = text.len() - 1;
        if cnt_backslashes >= 3 {
            let keep = (cnt_backslashes - 2) / 2;
            replacements.push(format!("{}\\\"", &text[..keep]));
        } else {
            replacements.push(text.to_string());
        }
    }
    let split = FIND_UNESCAPING_QUOTE_PATTERN.split(s).collect();
    interleave_split_matches(split, replacements)
}

/// Port von `LuceneTransformer._remove_one_escaping_from_quotes`.
fn remove_one_escaping_from_quotes(s: &str) -> String {
    let mut replacements = Vec::new();
    for m in FIND_UNESCAPING_QUOTE_PATTERN.find_iter(s) {
        let text = m.as_str();
        if text.len() > 1 {
            replacements.push(text[1..].to_string());
        } else {
            replacements.push(text.to_string());
        }
    }
    let split = FIND_UNESCAPING_QUOTE_PATTERN.split(s).collect();
    interleave_split_matches(split, replacements)
}

/// Port von `LuceneTransformer._remove_lucene_escaping`.
fn remove_lucene_escaping(s: &str) -> String {
    let s = remove_escaping_from_end_of_expression(s);
    let s = remove_uneven_double_quotes_escaping(&s);
    remove_one_escaping_from_quotes(&s)
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    #[allow(dead_code)]
    Field(String),
    Colon,
    LParen,
    RParen,
    LBracket,
    RBracket,
    LBrace,
    RBrace,
    To,
    And,
    Or,
    Not,
    Minus,
    Star,
    Pipe,
    Re,
    StringLit(String),
    RegexLit(String),
    Word { value: String, raw: String },
    Eof,
}

fn is_word_char(c: char) -> bool {
    !c.is_ascii_whitespace()
        && !matches!(c, ':' | '(' | ')' | '[' | ']' | '{' | '}' | '/' | '|')
}

fn is_word_escape(c: char) -> bool {
    c.is_ascii_whitespace()
        || matches!(c, ':' | '(' | ')' | '[' | ']' | '{' | '}' | '"' | '\\')
}

fn is_word_start(c: char) -> bool {
    !c.is_ascii_whitespace()
        && !matches!(c, ':' | '(' | ')' | '[' | ']' | '{' | '}' | '/' | '"')
}

struct Lexer {
    chars: Vec<char>,
    pos: usize,
}

impl Lexer {
    fn new(input: &str) -> Self {
        Self {
            chars: input.chars().collect(),
            pos: 0,
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn advance(&mut self) -> Option<char> {
        let c = self.chars.get(self.pos).copied();
        if c.is_some() {
            self.pos += 1;
        }
        c
    }

    fn skip_whitespace(&mut self) {
        while let Some(c) = self.peek() {
            if c.is_ascii_whitespace() {
                self.advance();
            } else {
                break;
            }
        }
    }

    fn read_string_lit(&mut self) -> Result<String, LuceneParseError> {
        let mut value = String::new();
        loop {
            let c = self.advance();
            match c {
                Some('"') => return Ok(value),
                Some('\\') => {
                    let mut slash_count = 1;
                    while self.peek() == Some('\\') {
                        self.advance();
                        slash_count += 1;
                    }
                    for _ in 0..slash_count {
                        value.push('\\');
                    }
                    match self.peek() {
                        Some('"') => {
                            if slash_count % 2 == 1 {
                                value.push('"');
                                self.advance();
                            } else {
                                self.advance();
                                return Ok(value);
                            }
                        }
                        Some(next) => {
                            value.push(next);
                            self.advance();
                        }
                        None => {
                            return Err(LuceneParseError(
                                "Unterminated escape in quoted string".to_string(),
                            ))
                        }
                    }
                }
                Some(c) => value.push(c),
                None => {
                    return Err(LuceneParseError("Unterminated quoted string".to_string()))
                }
            }
        }
    }

    fn read_regex_lit(&mut self) -> Result<String, LuceneParseError> {
        let mut value = String::new();
        loop {
            match self.advance() {
                Some('/') => return Ok(value),
                Some('\\') => {
                    value.push('\\');
                    if let Some(c) = self.advance() {
                        value.push(c);
                    }
                }
                Some(c) => value.push(c),
                None => {
                    return Err(LuceneParseError("Unterminated regex pattern".to_string()))
                }
            }
        }
    }

    fn read_word_with_escape(&mut self) -> (String, String) {
        let mut value = String::new();
        let mut raw = String::new();
        while let Some(c) = self.peek() {
            if c == '\\' {
                self.advance();
                raw.push('\\');
                if let Some(next) = self.advance() {
                    raw.push(next);
                    if is_word_escape(next) {
                        value.push(next);
                    } else {
                        value.push('\\');
                        value.push(next);
                    }
                }
            } else if is_word_char(c) {
                self.advance();
                raw.push(c);
                value.push(c);
            } else {
                break;
            }
        }
        (value, raw)
    }

    fn tokenize(&mut self) -> Result<Vec<Token>, LuceneParseError> {
        let mut tokens = Vec::new();
        loop {
            self.skip_whitespace();
            let c = match self.peek() {
                Some(c) => c,
                None => {
                    tokens.push(Token::Eof);
                    break;
                }
            };
            match c {
                ':' => {
                    self.advance();
                    tokens.push(Token::Colon);
                }
                '(' => {
                    self.advance();
                    tokens.push(Token::LParen);
                }
                ')' => {
                    self.advance();
                    tokens.push(Token::RParen);
                }
                '[' => {
                    self.advance();
                    tokens.push(Token::LBracket);
                }
                ']' => {
                    self.advance();
                    tokens.push(Token::RBracket);
                }
                '{' => {
                    self.advance();
                    tokens.push(Token::LBrace);
                }
                '}' => {
                    self.advance();
                    tokens.push(Token::RBrace);
                }
                '"' => {
                    self.advance();
                    let s = self.read_string_lit()?;
                    tokens.push(Token::StringLit(s));
                }
                '/' => {
                    self.advance();
                    let p = self.read_regex_lit()?;
                    tokens.push(Token::RegexLit(p));
                }
                '-' => {
                    self.advance();
                    tokens.push(Token::Minus);
                }
                '|' => {
                    self.advance();
                    tokens.push(Token::Pipe);
                }
                _ if is_word_start(c) => {
                    let (value, raw) = self.read_word_with_escape();
                    match value.as_str() {
                        "AND" => tokens.push(Token::And),
                        "OR" => tokens.push(Token::Or),
                        "NOT" => tokens.push(Token::Not),
                        "TO" => tokens.push(Token::To),
                        "*" => tokens.push(Token::Star),
                        "re" => tokens.push(Token::Re),
                        _ => {
                            // Ein Wort direkt vor '(' ist nicht erlaubt — die
                            // Klammer muss escaped werden (wie bei luqum).
                            if self.peek() == Some('(') {
                                return Err(LuceneParseError(
                                    "Illegal character '(' - expression not escaped correctly"
                                        .to_string(),
                                ));
                            }
                            tokens.push(Token::Word { value, raw })
                        }
                    }
                }
                _ => {
                    self.advance();
                }
            }
        }
        Ok(tokens)
    }
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    field_group_key: Option<Vec<String>>,
    special_fields: SpecialFields,
}

/// Eine geparste Range-Grenze: Wert (None = offene Grenze `*`),
/// Originaltext (für Fehlermeldungen) und ob sie quotet war.
struct RangeBoundary {
    value: Option<String>,
    raw: String,
    quoted: bool,
}

#[derive(Debug, Clone, Default)]
struct SpecialFields {
    regex_fields: Vec<String>,
    sigma_fields: Vec<String>,
    regex_all: bool,
    sigma_all: bool,
}

impl SpecialFields {
    fn from_pydict(dict: &Bound<'_, PyDict>) -> Result<Self, LuceneParseError> {
        let mut sf = SpecialFields::default();
        if let Ok(Some(rf)) = dict.get_item("regex_fields") {
            if rf.extract::<bool>().unwrap_or(false) {
                sf.regex_all = true;
            } else if let Ok(list) = rf.extract::<Vec<String>>() {
                sf.regex_fields = list;
            }
        }
        if let Ok(Some(sf_item)) = dict.get_item("sigma_fields") {
            if sf_item.extract::<bool>().unwrap_or(false) {
                sf.sigma_all = true;
            } else if let Ok(list) = sf_item.extract::<Vec<String>>() {
                sf.sigma_fields = list;
            }
        }
        Ok(sf)
    }
}

impl Parser {
    fn new(tokens: Vec<Token>, special_fields: SpecialFields) -> Self {
        Self {
            tokens,
            pos: 0,
            field_group_key: None,
            special_fields,
        }
    }

    fn peek(&self) -> &Token {
        self.tokens.get(self.pos).unwrap_or(&Token::Eof)
    }

    fn advance(&mut self) -> Token {
        let tok = self.tokens.get(self.pos).cloned().unwrap_or(Token::Eof);
        if self.pos < self.tokens.len() {
            self.pos += 1;
        }
        tok
    }

    fn expect(&mut self, expected: &Token) -> Result<(), LuceneParseError> {
        let tok = self.advance();
        if &tok == expected {
            Ok(())
        } else {
            Err(LuceneParseError(format!(
                "Expected {:?}, got {:?}",
                expected, tok
            )))
        }
    }

    fn parse(&mut self) -> Result<FilterExpressionInner, LuceneParseError> {
        let expr = self.parse_or()?;
        match self.peek() {
            Token::Eof => Ok(expr),
            tok => Err(LuceneParseError(format!(
                "Unexpected trailing token: {:?}",
                tok
            ))),
        }
    }

    fn parse_or(&mut self) -> Result<FilterExpressionInner, LuceneParseError> {
        let mut left = self.parse_and()?;
        while matches!(self.peek(), Token::Or) {
            self.advance();
            let right = self.parse_and()?;
            match &mut left {
                FilterExpressionInner::Or { children } => {
                    children.push(right);
                }
                _ => {
                    left = FilterExpressionInner::Or {
                        children: vec![left, right],
                    };
                }
            }
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<FilterExpressionInner, LuceneParseError> {
        let mut left = self.parse_not()?;
        while matches!(self.peek(), Token::And) {
            self.advance();
            let right = self.parse_not()?;
            match &mut left {
                FilterExpressionInner::And { children } => {
                    children.push(right);
                }
                _ => {
                    left = FilterExpressionInner::And {
                        children: vec![left, right],
                    };
                }
            }
        }
        Ok(left)
    }

    fn parse_not(&mut self) -> Result<FilterExpressionInner, LuceneParseError> {
        if matches!(self.peek(), Token::Not) {
            self.advance();
            let child = self.parse_not()?;
            Ok(FilterExpressionInner::Not {
                child: Box::new(child),
            })
        } else {
            self.parse_atom()
        }
    }

    fn parse_atom(&mut self) -> Result<FilterExpressionInner, LuceneParseError> {
        match self.peek().clone() {
            Token::LParen => {
                self.advance();
                let expr = self.parse_or()?;
                self.expect(&Token::RParen)?;
                Ok(expr)
            }
            Token::Star => {
                self.advance();
                Ok(FilterExpressionInner::Always { value: true })
            }
            Token::Minus => {
                self.advance();
                let child = self.parse_not()?;
                Ok(FilterExpressionInner::Not {
                    child: Box::new(child),
                })
            }
            Token::LBracket | Token::LBrace => {
                if let Some(fg_key) = &self.field_group_key.clone() {
                    self.parse_range(fg_key, "")
                } else {
                    Err(LuceneParseError(
                        "Expected field name before '[' or '{'".to_string(),
                    ))
                }
            }
            Token::Word { .. } | Token::RegexLit(_) | Token::StringLit(_) => {
                let saved = self.field_group_key.clone();
                let expr = self.parse_search_field_or_value()?;
                self.field_group_key = saved;
                Ok(expr)
            }
            Token::Eof => Err(LuceneParseError(
                "unexpected end of expression".to_string(),
            )),
            _ => Err(LuceneParseError(format!(
                "Unexpected token: {:?}",
                self.peek()
            ))),
        }
    }

    fn apply_field_group_context(
        &self,
        expr: &FilterExpressionInner,
        key: &[String],
    ) -> Result<FilterExpressionInner, LuceneParseError> {
        match expr {
            FilterExpressionInner::Or { children } | FilterExpressionInner::And { children } => {
                let mut new_children = Vec::new();
                for child in children {
                    new_children.push(self.fill_leaf_keys(child, key)?);
                }
                if matches!(expr, FilterExpressionInner::Or { .. }) {
                    Ok(FilterExpressionInner::Or {
                        children: new_children,
                    })
                } else {
                    Ok(FilterExpressionInner::And {
                        children: new_children,
                    })
                }
            }
            _ => self.fill_leaf_keys(expr, key),
        }
    }

    fn fill_leaf_keys(
        &self,
        expr: &FilterExpressionInner,
        key: &[String],
    ) -> Result<FilterExpressionInner, LuceneParseError> {
        match expr {
            FilterExpressionInner::String { expected, .. } => {
                Ok(self.create_string_expr(key, expected, expected, false))
            }
            FilterExpressionInner::Wildcard { expected, .. } => {
                let regex =
                    build_wildcard_regex(expected).map_err(|e| LuceneParseError(e))?;
                Ok(FilterExpressionInner::Wildcard {
                    key: key.to_vec(),
                    expected: expected.clone(),
                    regex,
                })
            }
            FilterExpressionInner::Sigma { expected, .. } => {
                let regex =
                    build_sigma_regex(expected).map_err(|e| LuceneParseError(e))?;
                Ok(FilterExpressionInner::Sigma {
                    key: key.to_vec(),
                    expected: expected.clone(),
                    regex,
                })
            }
            FilterExpressionInner::Regex { pattern, .. } => {
                let normalized = normalize_regex(pattern.as_str());
                let compiled = fancy_regex::Regex::new(&normalized)
                    .map_err(|e| LuceneParseError(format!("Invalid regex: {}", e)))?;
                Ok(FilterExpressionInner::Regex {
                    key: key.to_vec(),
                    pattern: compiled,
                })
            }
            FilterExpressionInner::Null { .. } => Ok(FilterExpressionInner::Null {
                key: key.to_vec(),
            }),
            FilterExpressionInner::Not { child } => {
                let filled = self.fill_leaf_keys(child, key)?;
                Ok(FilterExpressionInner::Not {
                    child: Box::new(filled),
                })
            }
            FilterExpressionInner::And { children } => {
                let mut new_children = Vec::new();
                for child in children {
                    new_children.push(self.fill_leaf_keys(child, key)?);
                }
                Ok(FilterExpressionInner::And {
                    children: new_children,
                })
            }
            FilterExpressionInner::Or { children } => {
                let mut new_children = Vec::new();
                for child in children {
                    new_children.push(self.fill_leaf_keys(child, key)?);
                }
                Ok(FilterExpressionInner::Or {
                    children: new_children,
                })
            }
            other => Ok(other.clone()),
        }
    }

    fn parse_search_field_or_value(
        &mut self,
    ) -> Result<FilterExpressionInner, LuceneParseError> {
        let tok = self.peek().clone();

        match &tok {
            Token::Word { value, raw } => {
                let value = value.clone();
                let raw = raw.clone();
                self.advance();

                if matches!(self.peek(), Token::Pipe) {
                    self.advance();
                    if matches!(self.peek(), Token::Re) {
                        self.advance();
                    }
                    if matches!(self.peek(), Token::Colon) {
                        self.expect(&Token::Colon)?;
                        return self.parse_regex_value(&split_dotted_field(&value));
                    }
                }

                if matches!(self.peek(), Token::Colon) {
                    return self.parse_field_value(&value);
                }

                if self.field_group_key.is_some() {
                    return self.parse_value_atom(&value, &raw, false);
                }

                if value == "*" {
                    return Ok(FilterExpressionInner::Always { value: true });
                }

                let key = split_dotted_field(&value);
                Ok(FilterExpressionInner::Exists { key })
            }
            Token::RegexLit(pattern) => {
                let pattern = pattern.clone();
                self.advance();

                if matches!(self.peek(), Token::Colon) {
                    self.expect(&Token::Colon)?;
                    let key = split_dotted_field(&pattern);
                    return self.parse_regex_value(&key);
                }

                if self.field_group_key.is_some() {
                    let normalized = normalize_regex(&pattern);
                    let compiled = fancy_regex::Regex::new(&normalized)
                        .map_err(|e| LuceneParseError(format!("Invalid regex: {}", e)))?;
                    if let Some(fg_key) = &self.field_group_key.clone() {
                        return Ok(FilterExpressionInner::Regex {
                            key: fg_key.clone(),
                            pattern: compiled,
                        });
                    }
                    return Ok(FilterExpressionInner::Regex {
                        key: vec![],
                        pattern: compiled,
                    });
                }

                let normalized = normalize_regex(&pattern);
                let compiled = fancy_regex::Regex::new(&normalized)
                    .map_err(|e| LuceneParseError(format!("Invalid regex: {}", e)))?;
                Ok(FilterExpressionInner::Regex {
                    key: vec![],
                    pattern: compiled,
                })
            }
            Token::StringLit(s) => {
                let s = s.clone();
                self.advance();

                if matches!(self.peek(), Token::Colon) {
                    return self.parse_field_value_str(&s);
                }

                if self.field_group_key.is_some() {
                    return self.parse_value_atom(&s, &s, true);
                }

                Ok(FilterExpressionInner::String {
                    key: vec![],
                    expected: s,
                })
            }
            _ => Err(LuceneParseError(format!(
                "Unexpected token in atom: {:?}",
                tok
            ))),
        }
    }

    fn parse_regex_value(
        &mut self,
        key: &[String],
    ) -> Result<FilterExpressionInner, LuceneParseError> {
        let val_tok = self.peek().clone();
        match &val_tok {
            Token::StringLit(s) => {
                let s = s.clone();
                self.advance();
                let normalized = normalize_regex(&s);
                let compiled = fancy_regex::Regex::new(&normalized)
                    .map_err(|e| LuceneParseError(format!("Invalid regex: {}", e)))?;
                Ok(FilterExpressionInner::Regex {
                    key: key.to_vec(),
                    pattern: compiled,
                })
            }
            Token::RegexLit(p) => {
                let p = p.clone();
                self.advance();
                let normalized = normalize_regex(&p);
                let compiled = fancy_regex::Regex::new(&normalized)
                    .map_err(|e| LuceneParseError(format!("Invalid regex: {}", e)))?;
                Ok(FilterExpressionInner::Regex {
                    key: key.to_vec(),
                    pattern: compiled,
                })
            }
            Token::Word { raw, .. } => {
                let raw = raw.clone();
                self.advance();
                let normalized = normalize_regex(&raw);
                let compiled = fancy_regex::Regex::new(&normalized)
                    .map_err(|e| LuceneParseError(format!("Invalid regex: {}", e)))?;
                Ok(FilterExpressionInner::Regex {
                    key: key.to_vec(),
                    pattern: compiled,
                })
            }
            Token::LParen => self.parse_field_group_regex(key),
            _ => Err(LuceneParseError(format!(
                "Unexpected regex value token: {:?}",
                val_tok
            ))),
        }
    }

    fn parse_field_value(
        &mut self,
        field_word: &str,
    ) -> Result<FilterExpressionInner, LuceneParseError> {
        self.expect(&Token::Colon)?;
        let key = split_dotted_field(field_word);
        self.parse_value_after_colon(&key)
    }

    fn parse_field_value_str(
        &mut self,
        field_word: &str,
    ) -> Result<FilterExpressionInner, LuceneParseError> {
        self.expect(&Token::Colon)?;
        let key = split_dotted_field(field_word);
        self.parse_value_after_colon(&key)
    }

    fn parse_value_after_colon(
        &mut self,
        key: &[String],
    ) -> Result<FilterExpressionInner, LuceneParseError> {
        let val_tok = self.peek().clone();
        match &val_tok {
            Token::LParen => self.parse_field_group(key),
            Token::StringLit(s) => {
                let s = s.clone();
                self.advance();
                Ok(self.create_string_expr(key, &s, &s, true))
            }
            Token::RegexLit(p) => {
                let p = p.clone();
                self.advance();
                let normalized = normalize_regex(&p);
                let compiled = fancy_regex::Regex::new(&normalized)
                    .map_err(|e| LuceneParseError(format!("Invalid regex: {}", e)))?;
                Ok(FilterExpressionInner::Regex {
                    key: key.to_vec(),
                    pattern: compiled,
                })
            }
            Token::Word { value, raw } => {
                let value = value.clone();
                let raw = raw.clone();
                self.advance();
                if matches!(self.peek(), Token::LBracket)
                    || matches!(self.peek(), Token::LBrace)
                {
                    return self.parse_range(key, &value);
                }
                if value == "null" {
                    return Ok(FilterExpressionInner::Null { key: key.to_vec() });
                }
                Ok(self.create_string_expr(key, &value, &raw, false))
            }
            Token::Minus => {
                self.advance();
                match self.peek().clone() {
                    Token::Word { value, raw } => {
                        let value = value.clone();
                        let raw = raw.clone();
                        self.advance();
                        let negated = format!("-{}", value);
                        let negated_raw = format!("-{}", raw);
                        if matches!(self.peek(), Token::LBracket)
                            || matches!(self.peek(), Token::LBrace)
                        {
                            return self.parse_range(key, &negated);
                        }
                        Ok(self.create_string_expr(key, &negated, &negated_raw, false))
                    }
                    _ => Err(LuceneParseError(
                        "Expected value after '-'".to_string(),
                    )),
                }
            }
            Token::Star => {
                self.advance();
                Ok(FilterExpressionInner::Always { value: true })
            }
            Token::LBracket | Token::LBrace => self.parse_range(key, ""),
            Token::Eof => Err(LuceneParseError(
                "unexpected end of expression".to_string(),
            )),
            _ => Err(LuceneParseError(format!(
                "Unexpected value token: {:?}",
                val_tok
            ))),
        }
    }

    fn parse_field_group(
        &mut self,
        key: &[String],
    ) -> Result<FilterExpressionInner, LuceneParseError> {
        let saved = self.field_group_key.clone();
        self.field_group_key = Some(key.to_vec());
        self.advance();
        let expr = self.parse_or()?;
        self.expect(&Token::RParen)?;
        let result = if self.field_group_key.is_some() {
            self.apply_field_group_context(&expr, key)?
        } else {
            expr
        };
        self.field_group_key = saved;
        Ok(result)
    }

    fn parse_field_group_regex(
        &mut self,
        key: &[String],
    ) -> Result<FilterExpressionInner, LuceneParseError> {
        let saved = self.field_group_key.clone();
        self.field_group_key = Some(key.to_vec());
        self.advance();
        let expr = self.parse_or()?;
        self.expect(&Token::RParen)?;
        let result = self.apply_field_group_context(&expr, key)?;
        self.field_group_key = saved;
        Ok(result)
    }

    fn parse_value_atom(
        &mut self,
        word: &str,
        raw_word: &str,
        is_quoted: bool,
    ) -> Result<FilterExpressionInner, LuceneParseError> {
        if let Some(fg_key) = &self.field_group_key.clone() {
            Ok(self.create_string_expr(fg_key, word, raw_word, is_quoted))
        } else if is_quoted {
            Ok(FilterExpressionInner::String {
                key: vec![],
                expected: remove_lucene_escaping(word),
            })
        } else {
            Ok(FilterExpressionInner::String {
                key: vec![],
                expected: word.to_string(),
            })
        }
    }

    fn create_string_expr(
        &self,
        key: &[String],
        value: &str,
        raw_value: &str,
        is_quoted: bool,
    ) -> FilterExpressionInner {
        let dotted = dotted_field_list(key);
        let cleaned_value;
        let cleaned_raw;
        let (value, raw_value) = if is_quoted {
            cleaned_value = remove_lucene_escaping(value);
            cleaned_raw = remove_lucene_escaping(raw_value);
            (cleaned_value.as_str(), cleaned_raw.as_str())
        } else {
            (value, raw_value)
        };
        if is_quoted && self.special_fields.regex_all {
            let normalized = normalize_regex(raw_value);
            if let Ok(compiled) = fancy_regex::Regex::new(&normalized) {
                return FilterExpressionInner::Regex {
                    key: key.to_vec(),
                    pattern: compiled,
                };
            }
        }
        if is_quoted && !self.special_fields.regex_fields.is_empty() {
            if self.special_fields.regex_fields.contains(&dotted) {
                let normalized = normalize_regex(raw_value);
                if let Ok(compiled) = fancy_regex::Regex::new(&normalized) {
                    return FilterExpressionInner::Regex {
                        key: key.to_vec(),
                        pattern: compiled,
                    };
                }
            }
        }
        if is_quoted && self.special_fields.sigma_all {
            if let Ok(regex) = build_sigma_regex(value) {
                return FilterExpressionInner::Sigma {
                    key: key.to_vec(),
                    expected: value.to_string(),
                    regex,
                };
            }
        }
        if is_quoted && !self.special_fields.sigma_fields.is_empty() {
            if self.special_fields.sigma_fields.contains(&dotted) {
                if let Ok(regex) = build_sigma_regex(value) {
                    return FilterExpressionInner::Sigma {
                        key: key.to_vec(),
                        expected: value.to_string(),
                        regex,
                    };
                }
            }
        }
        if !is_quoted && !self.special_fields.regex_fields.is_empty() {
            if self.special_fields.regex_fields.contains(&dotted) {
                let normalized = normalize_regex(raw_value);
                if let Ok(compiled) = fancy_regex::Regex::new(&normalized) {
                    return FilterExpressionInner::Regex {
                        key: key.to_vec(),
                        pattern: compiled,
                    };
                }
            }
        }
        if !is_quoted && value == "null" {
            return FilterExpressionInner::Null {
                key: key.to_vec(),
            };
        }
        if !is_quoted && (value.contains('*') || value.contains('?')) {
            if let Ok(regex) = build_wildcard_regex(value) {
                return FilterExpressionInner::Wildcard {
                    key: key.to_vec(),
                    expected: value.to_string(),
                    regex,
                };
            }
        }
        FilterExpressionInner::String {
            key: key.to_vec(),
            expected: value.to_string(),
        }
    }

    fn parse_range(
        &mut self,
        key: &[String],
        first_boundary: &str,
    ) -> Result<FilterExpressionInner, LuceneParseError> {
        let bracket_tok = if first_boundary.is_empty() {
            self.advance()
        } else {
            self.peek().clone()
        };

        let incl_low = matches!(bracket_tok, Token::LBracket);

        let lower = if first_boundary.is_empty() {
            self.parse_range_boundary()?
        } else {
            RangeBoundary {
                value: Some(first_boundary.to_string()),
                raw: first_boundary.to_string(),
                quoted: false,
            }
        };

        let tok = self.advance();
        if !matches!(tok, Token::To) {
            return Err(LuceneParseError(format!("Expected TO, got {:?}", tok)));
        }

        let upper = self.parse_range_boundary()?;

        let close_tok = self.advance();
        let incl_high = matches!(close_tok, Token::RBracket);

        if !incl_high && !matches!(close_tok, Token::RBrace) {
            return Err(LuceneParseError(format!(
                "Expected ] or }}, got {:?}",
                close_tok
            )));
        }

        let raw_range = format!(
            "{}{} TO {}{}",
            if incl_low { "[" } else { "{" },
            lower.raw,
            upper.raw,
            if incl_high { "]" } else { "}" },
        );

        // [* TO *] übersetzt sich zu "Feld existiert"
        if lower.value.is_none() && upper.value.is_none() {
            return Ok(FilterExpressionInner::Exists { key: key.to_vec() });
        }

        let reversed_error = || {
            LuceneParseError(format!(
                "The lower range boundary must not exceed the upper range boundary: \"{}\"",
                raw_range
            ))
        };

        // Ein quotete Grenze erzwingt den lexikographischen String-Vergleich
        let quoted = lower.quoted || upper.quoted;

        if !quoted {
            let lower_num = lower.value.as_ref().and_then(|s| NumericBound::parse(s));
            let upper_num = upper.value.as_ref().and_then(|s| NumericBound::parse(s));
            let all_numeric = lower.value.is_none() || lower_num.is_some();
            let all_numeric = all_numeric && (upper.value.is_none() || upper_num.is_some());
            if all_numeric {
                if let (Some(lo), Some(hi)) = (&lower_num, &upper_num) {
                    if lo.compare(hi) == std::cmp::Ordering::Greater {
                        return Err(reversed_error());
                    }
                }
                return Ok(FilterExpressionInner::NumericRange {
                    key: key.to_vec(),
                    lower: lower_num,
                    upper: upper_num,
                    incl_low,
                    incl_high,
                });
            }
        }

        if let (Some(lo), Some(hi)) = (&lower.value, &upper.value) {
            if lo > hi {
                return Err(reversed_error());
            }
        }

        Ok(FilterExpressionInner::StringRange {
            key: key.to_vec(),
            lower: lower.value,
            upper: upper.value,
            incl_low,
            incl_high,
        })
    }

    fn parse_range_boundary(&mut self) -> Result<RangeBoundary, LuceneParseError> {
        let (negative, mut value, mut raw, quoted) = match self.peek().clone() {
            Token::Word { value, raw } => {
                self.advance();
                (false, value, raw, false)
            }
            Token::StringLit(s) => {
                self.advance();
                // raw rekonstruiert die ursprüngliche quotete Form
                return Ok(RangeBoundary {
                    raw: format!("\"{}\"", s),
                    value: Some(s),
                    quoted: true,
                });
            }
            Token::Star => {
                self.advance();
                // unquotetes '*' ist eine offene Grenze
                return Ok(RangeBoundary {
                    value: None,
                    raw: "*".to_string(),
                    quoted: false,
                });
            }
            Token::Minus => {
                self.advance();
                match self.peek().clone() {
                    Token::Word { value, raw } => {
                        self.advance();
                        (true, value, raw, false)
                    }
                    _ => {
                        return Err(LuceneParseError(
                            "Expected value after '-' in range".to_string(),
                        ))
                    }
                }
            }
            _ => {
                return Err(LuceneParseError(format!(
                    "Invalid range boundary: {:?}",
                    self.peek()
                )))
            }
        };

        // Doppelpunkte innerhalb unquoteter Grenzen (z.B. ISO-8601-Timestamps)
        // werden wieder zusammengesetzt.
        while matches!(self.peek(), Token::Colon) {
            self.advance();
            match self.peek().clone() {
                Token::Word { value: v, raw: r } => {
                    self.advance();
                    value.push(':');
                    value.push_str(&v);
                    raw.push(':');
                    raw.push_str(&r);
                }
                _ => {
                    return Err(LuceneParseError(
                        "Invalid range boundary after ':'".to_string(),
                    ))
                }
            }
        }

        if negative {
            value = format!("-{}", value);
            raw = format!("-{}", raw);
        }

        // Unquotete Grenzen mit '+' (z.B. ISO-8601-Offsets) müssen — wie bei
        // luqum — quotet werden.
        if value.contains('+') {
            return Err(LuceneParseError(
                "Illegal character '+' in range boundary - expression not escaped correctly"
                    .to_string(),
            ));
        }

        Ok(RangeBoundary {
            value: Some(value),
            raw,
            quoted,
        })
    }
}

fn split_dotted_field(field: &str) -> Vec<String> {
    crate::field::get_dotted_field_list(field)
}

fn dotted_field_list(key: &[String]) -> String {
    key.iter()
        .map(|k| k.replace('.', "\\."))
        .collect::<Vec<_>>()
        .join(".")
}

pub fn parse_lucene_query_inner(
    query_string: &str,
    special_fields: Option<&Bound<'_, PyDict>>,
) -> Result<FilterExpressionInner, LuceneParseError> {
    let sf = if let Some(dict) = special_fields {
        SpecialFields::from_pydict(dict)?
    } else {
        SpecialFields::default()
    };
    let escaped = add_lucene_escaping(query_string);
    let mut lexer = Lexer::new(&escaped);
    let tokens = lexer.tokenize()?;
    let mut parser = Parser::new(tokens, sf);
    parser.parse()
}

#[pyfunction]
#[pyo3(signature = (query_string, special_fields=None))]
pub fn parse_lucene_query(
    _py: Python<'_>,
    query_string: &str,
    special_fields: Option<&Bound<'_, PyDict>>,
) -> PyResult<PyFilterExpression> {
    let sf = if let Some(dict) = special_fields {
        SpecialFields::from_pydict(dict)
            .map_err(|e| pyo3::exceptions::PyValueError::new_err(e.0))?
    } else {
        SpecialFields::default()
    };
    let escaped = add_lucene_escaping(query_string);
    let mut lexer = Lexer::new(&escaped);
    let tokens = lexer
        .tokenize()
        .map_err(|e| pyo3::exceptions::PyValueError::new_err(e.0))?;
    let mut parser = Parser::new(tokens, sf);
    let inner = parser
        .parse()
        .map_err(|e| pyo3::exceptions::PyValueError::new_err(e.0))?;
    Ok(PyFilterExpression { inner })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse(query: &str) -> FilterExpressionInner {
        let mut lexer = Lexer::new(query);
        let tokens = lexer.tokenize().unwrap();
        let mut parser = Parser::new(tokens, SpecialFields::default());
        parser.parse().unwrap()
    }

    fn try_parse(query: &str) -> Result<FilterExpressionInner, LuceneParseError> {
        let mut lexer = Lexer::new(query);
        let tokens = lexer.tokenize()?;
        let mut parser = Parser::new(tokens, SpecialFields::default());
        parser.parse()
    }

    #[test]
    fn simple_string_query() {
        let expr = parse(r#"key: "value""#);
        assert_eq!(
            expr,
            FilterExpressionInner::String {
                key: vec!["key".into()],
                expected: "value".into(),
            }
        );
    }

    #[test]
    fn and_query() {
        let expr = parse(r#"key: "value" AND key2: "value2""#);
        match expr {
            FilterExpressionInner::And { children } => assert_eq!(children.len(), 2),
            _ => panic!("Expected And"),
        }
    }

    #[test]
    fn or_query() {
        let expr = parse(r#"key: "value" OR key2: "value2""#);
        match expr {
            FilterExpressionInner::Or { children } => assert_eq!(children.len(), 2),
            _ => panic!("Expected Or"),
        }
    }

    #[test]
    fn not_query() {
        let expr = parse("NOT foo");
        assert_eq!(
            expr,
            FilterExpressionInner::Not {
                child: Box::new(FilterExpressionInner::Exists {
                    key: vec!["foo".into()]
                })
            }
        );
    }

    #[test]
    fn exists_query() {
        let expr = parse("foo");
        assert_eq!(
            expr,
            FilterExpressionInner::Exists {
                key: vec!["foo".into()]
            }
        );
    }

    #[test]
    fn match_all() {
        assert_eq!(parse("*"), FilterExpressionInner::Always { value: true });
    }

    #[test]
    fn null_query() {
        assert_eq!(
            parse("null_key: null"),
            FilterExpressionInner::Null {
                key: vec!["null_key".into()]
            }
        );
    }

    #[test]
    fn string_not_null() {
        assert_eq!(
            parse(r#"null_key: "null""#),
            FilterExpressionInner::String {
                key: vec!["null_key".into()],
                expected: "null".into(),
            }
        );
    }

    #[test]
    fn dotted_field() {
        assert_eq!(
            parse("something.null_key: null"),
            FilterExpressionInner::Null {
                key: vec!["something".into(), "null_key".into()]
            }
        );
    }

    #[test]
    fn integer_range() {
        assert_eq!(
            parse("key:[18 TO 65]"),
            FilterExpressionInner::NumericRange {
                key: vec!["key".into()],
                lower: Some(NumericBound::Int("18".into())),
                upper: Some(NumericBound::Int("65".into())),
                incl_low: true,
                incl_high: true,
            }
        );
    }

    #[test]
    fn float_range() {
        assert_eq!(
            parse("key:[0.1 TO 8.5]"),
            FilterExpressionInner::NumericRange {
                key: vec!["key".into()],
                lower: Some(NumericBound::Float(0.1)),
                upper: Some(NumericBound::Float(8.5)),
                incl_low: true,
                incl_high: true,
            }
        );
    }

    #[test]
    fn exclusive_range() {
        assert_eq!(
            parse("key:{18 TO 65}"),
            FilterExpressionInner::NumericRange {
                key: vec!["key".into()],
                lower: Some(NumericBound::Int("18".into())),
                upper: Some(NumericBound::Int("65".into())),
                incl_low: false,
                incl_high: false,
            }
        );
    }

    #[test]
    fn mixed_range() {
        assert_eq!(
            parse("key:[18 TO 65}"),
            FilterExpressionInner::NumericRange {
                key: vec!["key".into()],
                lower: Some(NumericBound::Int("18".into())),
                upper: Some(NumericBound::Int("65".into())),
                incl_low: true,
                incl_high: false,
            }
        );
    }

    #[test]
    fn string_range() {
        assert_eq!(
            parse("key:[alpha TO zulu]"),
            FilterExpressionInner::StringRange {
                key: vec!["key".into()],
                lower: Some("alpha".into()),
                upper: Some("zulu".into()),
                incl_low: true,
                incl_high: true,
            }
        );
    }

    #[test]
    fn regex_query() {
        let expr = parse(r#"regex_key: /.*value.*/"#);
        assert!(
            matches!(expr, FilterExpressionInner::Regex { key, .. } if key == vec!["regex_key".to_string()])
        );
    }

    #[test]
    fn field_group_or() {
        let expr = parse(r#"key: ("value" OR "value2")"#);
        match &expr {
            FilterExpressionInner::Or { children } => {
                assert_eq!(children.len(), 2);
                for child in children {
                    match child {
                        FilterExpressionInner::String { key, .. } => {
                            assert_eq!(key, &vec!["key".to_string()]);
                        }
                        _ => panic!("Expected String in Or"),
                    }
                }
            }
            _ => panic!("Expected Or"),
        }
    }

    #[test]
    fn escaped_field_name() {
        let expr = parse(r#"a\ key\(: "value""#);
        assert_eq!(
            expr,
            FilterExpressionInner::String {
                key: vec!["a key(".into()],
                expected: "value".into(),
            }
        );
    }

    #[test]
    fn negative_range_boundary() {
        assert_eq!(
            parse("key:[-10 TO -1]"),
            FilterExpressionInner::NumericRange {
                key: vec!["key".into()],
                lower: Some(NumericBound::Int("-10".into())),
                upper: Some(NumericBound::Int("-1".into())),
                incl_low: true,
                incl_high: true,
            }
        );
    }

    #[test]
    fn quoted_string_value() {
        assert_eq!(
            parse(r#"key: "hello world""#),
            FilterExpressionInner::String {
                key: vec!["key".into()],
                expected: "hello world".into(),
            }
        );
    }

    #[test]
    fn bare_word_value() {
        assert_eq!(
            parse("key: value"),
            FilterExpressionInner::String {
                key: vec!["key".into()],
                expected: "value".into(),
            }
        );
    }

    #[test]
    fn paren_group() {
        assert_eq!(
            parse(r#"(key: "value")"#),
            FilterExpressionInner::String {
                key: vec!["key".into()],
                expected: "value".into(),
            }
        );
    }

    #[test]
    fn complex_query() {
        let expr = parse(r#"(title:"foo bar" AND body:"quick fox") OR title:fox"#);
        match expr {
            FilterExpressionInner::Or { children } => {
                assert_eq!(children.len(), 2);
                match &children[0] {
                    FilterExpressionInner::And { children: ac } => assert_eq!(ac.len(), 2),
                    _ => panic!("Expected And"),
                }
            }
            _ => panic!("Expected Or"),
        }
    }

    #[test]
    fn field_group_with_and() {
        let expr = parse(r#"key: ("value1" AND "value2")"#);
        match &expr {
            FilterExpressionInner::And { children } => {
                assert_eq!(children.len(), 2);
                for child in children {
                    match child {
                        FilterExpressionInner::String { key, .. } => {
                            assert_eq!(key, &vec!["key".to_string()]);
                        }
                        _ => panic!("Expected String in And"),
                    }
                }
            }
            _ => panic!("Expected And"),
        }
    }

    #[test]
    fn range_without_field_raises() {
        let result = try_parse("[0 TO 10]");
        assert!(result.is_err(), "Expected error for range without field");
    }

    #[test]
    fn open_range_is_unbounded_numeric_range() {
        assert_eq!(
            parse("key:[* TO 10]"),
            FilterExpressionInner::NumericRange {
                key: vec!["key".into()],
                lower: None,
                upper: Some(NumericBound::Int("10".into())),
                incl_low: true,
                incl_high: true,
            }
        );
    }

    #[test]
    fn fully_open_range_is_exists() {
        assert_eq!(
            parse("key:[* TO *]"),
            FilterExpressionInner::Exists {
                key: vec!["key".into()],
            }
        );
    }

    #[test]
    fn reversed_range_raises() {
        let result = try_parse("key:[10 TO 0]");
        assert!(result.is_err(), "Expected error for reversed range");
    }

    #[test]
    fn pipe_re_modifier() {
        let expr = parse(r#"key|re: ".*value.*""#);
        assert!(
            matches!(expr, FilterExpressionInner::Regex { key, .. } if key == vec!["key".to_string()])
        );
    }

    #[test]
    fn test_match_string() {
        let expr = parse(r#"key: "value""#);
        assert!(expr.matches(&json!({"key": "value"})));
        assert!(!expr.matches(&json!({"key": "wrong"})));
        assert!(!expr.matches(&json!({})));
    }

    #[test]
    fn test_match_and() {
        let expr = parse(r#"a: "1" AND b: "2""#);
        assert!(expr.matches(&json!({"a": "1", "b": "2"})));
        assert!(!expr.matches(&json!({"a": "1"})));
    }

    #[test]
    fn test_match_or() {
        let expr = parse(r#"a: "1" OR b: "2""#);
        assert!(expr.matches(&json!({"a": "1"})));
        assert!(expr.matches(&json!({"b": "2"})));
        assert!(!expr.matches(&json!({})));
    }

    #[test]
    fn test_match_not() {
        let expr = parse("NOT foo");
        assert!(expr.matches(&json!({"bar": "1"})));
        assert!(!expr.matches(&json!({"foo": "1"})));
    }

    #[test]
    fn test_match_exists() {
        let expr = parse("foo");
        assert!(expr.matches(&json!({"foo": "bar"})));
        assert!(!expr.matches(&json!({})));
    }

    #[test]
    fn test_match_integer_range() {
        let expr = parse("age:[18 TO 65]");
        assert!(expr.matches(&json!({"age": 25})));
        assert!(expr.matches(&json!({"age": 18})));
        assert!(expr.matches(&json!({"age": 65})));
        assert!(!expr.matches(&json!({"age": 10})));
    }

    #[test]
    fn test_match_float_range() {
        let expr = parse("temp:[0.0 TO 100.0]");
        assert!(expr.matches(&json!({"temp": 50.0})));
        assert!(!expr.matches(&json!({"temp": -1.0})));
    }

    #[test]
    fn test_match_string_range() {
        let expr = parse("name:[a TO m]");
        assert!(expr.matches(&json!({"name": "hello"})));
        assert!(!expr.matches(&json!({"name": "world"})));
    }

    #[test]
    fn test_match_null() {
        let expr = parse("field: null");
        assert!(expr.matches(&json!({"field": null})));
        assert!(!expr.matches(&json!({"field": "value"})));
    }
}
