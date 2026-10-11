// Copyright 2026 OpenC3, Inc.
// All Rights Reserved.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.
// See LICENSE.md for more details.

//! Splits a template into text and tags (ERB `trim_mode: "-"`), then tokenizes the code.

use crate::error::{Flow, RtError, SYNTAX_ERROR};

/// Deepest allowed nesting of string interpolation.
const MAX_INTERP_DEPTH: u32 = 64;

#[derive(Debug, Clone, PartialEq)]
pub enum StrPart {
    Lit(String),
    Code(Vec<Token>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    /// Integer literal magnitude (negative when written with a leading `-`).
    Int(i128),
    /// Integer literal too large to represent.
    BigInt,
    Float(f64),
    Str(Vec<StrPart>),
    DSym(Vec<StrPart>),
    Sym(String),
    Words(Vec<String>),
    Symbols(Vec<String>),
    Ident(String),
    Const(String),
    Label(String),
    /// A string literal used as a label: `"name": value`.
    StrLabel(String),
    Kw(&'static str),
    /// A reserved word ERBLite does not support (`def`, `class`, `self`, ...).
    Reserved(String),
    Op(&'static str),
    Newline,
    Text(String),
    OutStart,
    OutEnd,
    Eof,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub tok: Tok,
    pub line: u32,
    pub space_before: bool,
    /// A numeric literal written with a directly attached `-` (`-2`).
    pub neg_lit: bool,
}

pub fn syntax_error(msg: impl Into<String>, line: u32) -> Flow {
    Flow::Err(Box::new(RtError {
        kind: SYNTAX_ERROR,
        message: msg.into(),
        line: Some(line),
        filename: None,
    }))
}

const KEYWORDS: &[&str] = &[
    "if", "elsif", "else", "unless", "then", "end", "while", "until", "for", "in", "do", "case", "when", "break",
    "next", "nil", "true", "false", "and", "or", "not",
];

const RESERVED: &[&str] = &[
    "begin",
    "self",
    "def",
    "class",
    "module",
    "return",
    "yield",
    "rescue",
    "ensure",
    "super",
    "alias",
    "undef",
    "redo",
    "retry",
    "defined?",
    "BEGIN",
    "END",
    "__FILE__",
    "__LINE__",
    "__ENCODING__",
];

/// Operators, longest first.
const OPS: &[&str] = &[
    "**=", "<=>", "===", "...", "<<=", ">>=", "||=", "&&=", "**", "==", "!=", ">=", "<=", "&&", "||", "<<", ">>", "+=",
    "-=", "*=", "/=", "%=", "|=", "&=", "^=", "..", "::", "=>", "&.", "+", "-", "*", "/", "%", "=", "<", ">", "!", "&",
    "|", "^", "~", "?", ":", ",", ".", ";", "(", ")", "[", "]", "{", "}",
];

const SYM_OPS: &[&str] = &[
    "[]=", "[]", "<=>", "===", "==", "=~", "!=", "!~", "**", "+@", "-@", "<<", ">>", "<=", ">=", "+", "-", "*", "/",
    "%", "<", ">", "!", "&", "|", "^", "~",
];

#[derive(Debug)]
pub enum Segment {
    Text(String, u32),
    Code(String, u32),
    Output(String, u32),
}

/// Scans a template into text and tag segments, applying ERB `-` trimming.
pub fn scan_segments(src: &str) -> Result<Vec<Segment>, Flow> {
    let b = src.as_bytes();
    let mut segs = Vec::new();
    let mut text = String::new();
    let mut line: u32 = 1;
    let mut i = 0;
    let mut text_start = 0;
    while i < b.len() {
        // ERB's scanner anchors `^[ \t]*<%-` at line starts and at the start of each scan
        // step, i.e. right after a previous tag or `<%%`.
        let at_line_start = i == 0 || b[i - 1] == b'\n' || i == text_start;
        if at_line_start {
            let mut j = i;
            while j < b.len() && (b[j] == b' ' || b[j] == b'\t') {
                j += 1;
            }
            if b[j..].starts_with(b"<%-") {
                text.push_str(&src[text_start..i]);
                i = j + 3;
                let (seg_end, next, lines) = scan_tag(src, i, line)?;
                flush_text(&mut segs, &mut text, line);
                segs.push(Segment::Code(seg_end, line));
                line += lines;
                i = next;
                text_start = i;
                continue;
            }
        }
        if b[i] == b'<' && b[i..].starts_with(b"<%") {
            text.push_str(&src[text_start..i]);
            if b[i..].starts_with(b"<%%") {
                text.push_str("<%");
                i += 3;
                text_start = i;
                continue;
            }
            let (kind, skip) = match b.get(i + 2) {
                Some(b'=') => (1, 3),
                Some(b'#') => (2, 3),
                Some(b'-') => (0, 3),
                _ => (0, 2),
            };
            let tag_line = line;
            let (content, next, lines) = scan_tag(src, i + skip, line)?;
            flush_text(&mut segs, &mut text, line);
            match kind {
                1 => segs.push(Segment::Output(content, tag_line)),
                2 => {}
                _ => segs.push(Segment::Code(content, tag_line)),
            }
            line += lines;
            i = next;
            text_start = i;
            continue;
        }
        if b[i] == b'\n' {
            line += 1;
        }
        i += 1;
    }
    text.push_str(&src[text_start..]);
    flush_text(&mut segs, &mut text, line);
    Ok(segs)
}

/// Pushes pending text; `line` is the line where the text ends.
fn flush_text(segs: &mut Vec<Segment>, text: &mut String, line: u32) {
    if !text.is_empty() {
        let start = line.saturating_sub(text.matches('\n').count() as u32);
        segs.push(Segment::Text(std::mem::take(text), start));
    }
}

/// Scans tag content starting at `start`. Returns (content, index after the tag, newline count).
fn scan_tag(src: &str, start: usize, line: u32) -> Result<(String, usize, u32), Flow> {
    let b = src.as_bytes();
    let mut i = start;
    let mut content = String::new();
    let mut seg_start = start;
    let mut lines = 0;
    while i < b.len() {
        if b[i] == b'-' && b[i..].starts_with(b"-%>") {
            content.push_str(&src[seg_start..i]);
            i += 3;
            if b[i..].starts_with(b"\r\n") {
                i += 2;
                lines += 1;
            } else if b[i..].starts_with(b"\n") {
                i += 1;
                lines += 1;
            }
            return Ok((content, i, lines));
        }
        if b[i] == b'%' && b[i..].starts_with(b"%%>") {
            content.push_str(&src[seg_start..i]);
            content.push_str("%>");
            i += 3;
            seg_start = i;
            continue;
        }
        if b[i] == b'%' && b[i..].starts_with(b"%>") {
            content.push_str(&src[seg_start..i]);
            return Ok((content, i + 2, lines));
        }
        if b[i] == b'\n' {
            lines += 1;
        }
        i += 1;
    }
    Err(syntax_error("unterminated tag (missing %>)", line))
}

/// Tokenizes a whole template into one token stream.
pub fn tokenize_template(src: &str) -> Result<Vec<Token>, Flow> {
    let segs = scan_segments(src)?;
    let mut out = Vec::new();
    let mut last_line = 1;
    for seg in segs {
        match seg {
            Segment::Text(t, line) => {
                last_line = line + t.matches('\n').count() as u32;
                out.push(Token {
                    tok: Tok::Text(t),
                    line,
                    space_before: true,
                    neg_lit: false,
                });
            }
            Segment::Code(code, line) => {
                let mut lx = Lexer::new(&code, line);
                let toks = lx.lex_all()?;
                out.extend(toks);
                let end_line = line + code.matches('\n').count() as u32;
                out.push(Token {
                    tok: Tok::Newline,
                    line: end_line,
                    space_before: true,
                    neg_lit: false,
                });
                last_line = end_line;
            }
            Segment::Output(code, line) => {
                out.push(Token {
                    tok: Tok::OutStart,
                    line,
                    space_before: true,
                    neg_lit: false,
                });
                let mut lx = Lexer::new(&code, line);
                let toks = lx.lex_all()?;
                out.extend(toks);
                let end_line = line + code.matches('\n').count() as u32;
                out.push(Token {
                    tok: Tok::OutEnd,
                    line: end_line,
                    space_before: true,
                    neg_lit: false,
                });
                last_line = end_line;
            }
        }
    }
    out.push(Token {
        tok: Tok::Eof,
        line: last_line,
        space_before: true,
        neg_lit: false,
    });
    Ok(out)
}

struct Lexer {
    chars: Vec<char>,
    pos: usize,
    line: u32,
    depth: u32,
}

fn ends_value(t: &Token) -> bool {
    match &t.tok {
        Tok::Int(_) | Tok::BigInt | Tok::Float(_) | Tok::Str(_) | Tok::DSym(_) | Tok::Sym(_) => true,
        Tok::Words(_) | Tok::Symbols(_) | Tok::Ident(_) | Tok::Const(_) | Tok::Reserved(_) => true,
        Tok::Kw(k) => matches!(*k, "nil" | "true" | "false" | "end"),
        Tok::Op(o) => matches!(*o, ")" | "]" | "}"),
        _ => false,
    }
}

fn is_continuation(t: &Token) -> bool {
    match &t.tok {
        Tok::Op(o) => !matches!(*o, ")" | "]" | "}" | ";"),
        Tok::Kw(k) => matches!(*k, "and" | "or" | "not"),
        _ => false,
    }
}

fn is_ident_start(c: char) -> bool {
    c.is_ascii_lowercase() || c == '_' || (!c.is_ascii() && c.is_alphabetic())
}

fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || (!c.is_ascii() && c.is_alphanumeric())
}

impl Lexer {
    fn new(src: &str, line: u32) -> Lexer {
        Lexer {
            chars: src.chars().collect(),
            pos: 0,
            line,
            depth: 0,
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn peek_at(&self, n: usize) -> Option<char> {
        self.chars.get(self.pos + n).copied()
    }

    fn starts_with(&self, s: &str) -> bool {
        s.chars()
            .enumerate()
            .all(|(i, c)| self.chars.get(self.pos + i) == Some(&c))
    }

    fn err(&self, msg: impl Into<String>) -> Flow {
        syntax_error(msg, self.line)
    }

    fn lex_all(&mut self) -> Result<Vec<Token>, Flow> {
        self.lex_tokens(false)
    }

    /// Lexes tokens to the end of input, or (for interpolation) to the matching `}`.
    fn lex_tokens(&mut self, until_brace: bool) -> Result<Vec<Token>, Flow> {
        let mut toks: Vec<Token> = Vec::new();
        let mut brace_depth = 0u32;
        let mut space = false;
        loop {
            let Some(c) = self.peek() else {
                if until_brace {
                    return Err(self.err("unterminated string interpolation"));
                }
                break;
            };
            match c {
                ' ' | '\t' | '\r' | '\x0c' | '\x0b' => {
                    self.pos += 1;
                    space = true;
                    continue;
                }
                '\\' if self.peek_at(1) == Some('\n') => {
                    self.pos += 2;
                    self.line += 1;
                    space = true;
                    continue;
                }
                '#' => {
                    while let Some(c) = self.peek() {
                        if c == '\n' {
                            break;
                        }
                        self.pos += 1;
                    }
                    continue;
                }
                '\n' => {
                    self.pos += 1;
                    let line = self.line;
                    self.line += 1;
                    space = true;
                    let skip = match toks.last() {
                        None => true,
                        Some(t) => t.tok == Tok::Newline || is_continuation(t),
                    };
                    if !skip && !self.next_line_starts_with_dot() {
                        toks.push(Token {
                            tok: Tok::Newline,
                            line,
                            space_before: true,
                            neg_lit: false,
                        });
                    }
                    continue;
                }
                _ => {}
            }
            if until_brace && c == '}' && brace_depth == 0 {
                self.pos += 1;
                return Ok(toks);
            }
            let line = self.line;
            let prev_ends_value = toks.last().map(ends_value).unwrap_or(false);
            let mut neg_lit = false;
            let tok = if c.is_ascii_digit() {
                self.lex_number()?
            } else if c == '-' && !prev_ends_value && self.peek_at(1).is_some_and(|d| d.is_ascii_digit()) {
                self.pos += 1;
                neg_lit = true;
                match self.lex_number()? {
                    Tok::Int(v) => Tok::Int(-v),
                    Tok::Float(f) => Tok::Float(-f),
                    other => other,
                }
            } else if is_ident_start(c) {
                self.lex_ident()
            } else if c.is_ascii_uppercase() {
                let w = self.read_word();
                if self.peek() == Some(':') && self.peek_at(1) != Some(':') {
                    self.pos += 1;
                    Tok::Label(w)
                } else if RESERVED.contains(&w.as_str()) {
                    Tok::Reserved(w)
                } else {
                    Tok::Const(w)
                }
            } else if c == '"' || c == '\'' {
                self.pos += 1;
                let parts = if c == '"' {
                    self.lex_dstring('"')?
                } else {
                    vec![StrPart::Lit(self.lex_sstring()?)]
                };
                // "name": value is a symbol label
                match parts.as_slice() {
                    [StrPart::Lit(s)] if self.peek() == Some(':') && self.peek_at(1) != Some(':') => {
                        self.pos += 1;
                        Tok::StrLabel(s.clone())
                    }
                    _ => Tok::Str(parts),
                }
            } else if c == ':' && self.peek_at(1) != Some(':') {
                self.lex_colon()?
            } else if c == '%'
                && !prev_ends_value
                && matches!(self.peek_at(1), Some('w') | Some('i'))
                && matches!(self.peek_at(2), Some('[') | Some('(') | Some('{') | Some('<'))
            {
                self.lex_words()?
            } else if c == '@' || c == '$' || c == '`' {
                return Err(self.err(format!("unsupported syntax '{}'", c)));
            } else if self.starts_with("=~") || self.starts_with("!~") || self.starts_with("->") {
                return Err(self.err("unsupported operator"));
            } else {
                let Some(op) = OPS.iter().find(|op| self.starts_with(op)) else {
                    return Err(self.err(format!("unexpected character '{}'", c)));
                };
                self.pos += op.chars().count();
                match *op {
                    "{" => brace_depth += 1,
                    "}" => brace_depth = brace_depth.saturating_sub(1),
                    _ => {}
                }
                Tok::Op(op)
            };
            toks.push(Token {
                tok,
                line,
                space_before: space,
                neg_lit,
            });
            space = false;
        }
        Ok(toks)
    }

    /// After a newline: does the next non-blank line start with `.` (not `..`) or `&.`?
    fn next_line_starts_with_dot(&self) -> bool {
        let mut i = self.pos;
        loop {
            match self.chars.get(i) {
                Some(' ') | Some('\t') | Some('\r') | Some('\n') => i += 1,
                Some('#') => {
                    while let Some(&c) = self.chars.get(i) {
                        if c == '\n' {
                            break;
                        }
                        i += 1;
                    }
                }
                Some('.') => return self.chars.get(i + 1) != Some(&'.'),
                Some('&') => return self.chars.get(i + 1) == Some(&'.'),
                _ => return false,
            }
        }
    }

    fn read_word(&mut self) -> String {
        let mut w = String::new();
        while let Some(c) = self.peek() {
            if is_ident_char(c) {
                w.push(c);
                self.pos += 1;
            } else {
                break;
            }
        }
        w
    }

    fn lex_ident(&mut self) -> Tok {
        let mut w = self.read_word();
        if let Some(c @ ('?' | '!')) = self.peek() {
            if self.peek_at(1) != Some('=') || (c == '?' && self.peek_at(2) == Some('=')) {
                w.push(c);
                self.pos += 1;
            }
        }
        if self.peek() == Some(':') && self.peek_at(1) != Some(':') {
            self.pos += 1;
            return Tok::Label(w);
        }
        if let Some(k) = KEYWORDS.iter().find(|k| **k == w) {
            return Tok::Kw(k);
        }
        if RESERVED.contains(&w.as_str()) {
            return Tok::Reserved(w);
        }
        Tok::Ident(w)
    }

    fn read_digits(&mut self, radix: u32) -> Result<String, Flow> {
        let mut s = String::new();
        let mut last_us = false;
        while let Some(c) = self.peek() {
            if c.is_digit(radix) {
                s.push(c);
                last_us = false;
                self.pos += 1;
            } else if c == '_' && !s.is_empty() && !last_us && self.peek_at(1).is_some_and(|d| d.is_digit(radix)) {
                last_us = true;
                self.pos += 1;
            } else {
                break;
            }
        }
        if s.is_empty() {
            return Err(self.err("numeric literal without digits"));
        }
        Ok(s)
    }

    fn lex_number(&mut self) -> Result<Tok, Flow> {
        let radix = if self.peek() == Some('0') {
            match self.peek_at(1) {
                Some('x') | Some('X') => Some(16),
                Some('b') | Some('B') => Some(2),
                Some('o') | Some('O') => Some(8),
                Some('d') | Some('D') => Some(10),
                Some('0'..='9') | Some('_') => {
                    self.pos += 1;
                    if self.peek() == Some('_') {
                        self.pos += 1;
                    }
                    let digits = self.read_digits(10)?;
                    if digits.chars().any(|c| c == '8' || c == '9') {
                        return Err(self.err("Invalid octal digit"));
                    }
                    self.check_number_end()?;
                    return Ok(int_tok(&digits, 8));
                }
                _ => None,
            }
        } else {
            None
        };
        if let Some(r) = radix {
            self.pos += 2;
            let digits = self.read_digits(r)?;
            self.check_number_end()?;
            return Ok(int_tok(&digits, r));
        }
        let int_part = self.read_digits(10)?;
        let mut is_float = false;
        let mut text = int_part.clone();
        if self.peek() == Some('.') && self.peek_at(1).is_some_and(|c| c.is_ascii_digit()) {
            self.pos += 1;
            let frac = self.read_digits(10)?;
            text.push('.');
            text.push_str(&frac);
            is_float = true;
        }
        if matches!(self.peek(), Some('e') | Some('E')) {
            let sign_off = if matches!(self.peek_at(1), Some('+') | Some('-')) {
                2
            } else {
                1
            };
            if self.peek_at(sign_off).is_some_and(|c| c.is_ascii_digit()) {
                text.push('e');
                if sign_off == 2 {
                    text.push(self.peek_at(1).unwrap_or('+'));
                }
                self.pos += sign_off;
                let exp = self.read_digits(10)?;
                text.push_str(&exp);
                is_float = true;
            }
        }
        self.check_number_end()?;
        if is_float {
            let f: f64 = text.parse().map_err(|_| self.err("invalid float literal"))?;
            Ok(Tok::Float(f))
        } else {
            Ok(int_tok(&int_part, 10))
        }
    }

    fn check_number_end(&self) -> Result<(), Flow> {
        match self.peek() {
            Some(c) if is_ident_char(c) => Err(self.err("unexpected identifier after number")),
            _ => Ok(()),
        }
    }

    fn lex_sstring(&mut self) -> Result<String, Flow> {
        let mut s = String::new();
        loop {
            let Some(c) = self.peek() else {
                return Err(self.err("unterminated string meets end of file"));
            };
            self.pos += 1;
            match c {
                '\'' => return Ok(s),
                '\\' => match self.peek() {
                    Some('\\') => {
                        s.push('\\');
                        self.pos += 1;
                    }
                    Some('\'') => {
                        s.push('\'');
                        self.pos += 1;
                    }
                    _ => s.push('\\'),
                },
                '\n' => {
                    self.line += 1;
                    s.push('\n');
                }
                c => s.push(c),
            }
        }
    }

    fn lex_dstring(&mut self, close: char) -> Result<Vec<StrPart>, Flow> {
        let mut parts = Vec::new();
        let mut cur = String::new();
        loop {
            let Some(c) = self.peek() else {
                return Err(self.err("unterminated string meets end of file"));
            };
            self.pos += 1;
            if c == close {
                break;
            }
            match c {
                '\\' => self.lex_escape(&mut cur)?,
                '#' if self.peek() == Some('{') => {
                    self.pos += 1;
                    if self.depth >= MAX_INTERP_DEPTH {
                        return Err(self.err("string interpolation nested too deeply"));
                    }
                    self.depth += 1;
                    let toks = self.lex_tokens(true)?;
                    self.depth -= 1;
                    if !cur.is_empty() {
                        parts.push(StrPart::Lit(std::mem::take(&mut cur)));
                    }
                    parts.push(StrPart::Code(toks));
                }
                '#' if matches!(self.peek(), Some('@') | Some('$'))
                    && self
                        .peek_at(1)
                        .is_some_and(|c| is_ident_start(c) || c.is_ascii_uppercase() || c == '@') =>
                {
                    return Err(self.err("instance/global variable interpolation (#@x, #$x) is not supported"));
                }
                '\n' => {
                    self.line += 1;
                    cur.push('\n');
                }
                c => cur.push(c),
            }
        }
        if !cur.is_empty() || parts.is_empty() {
            parts.push(StrPart::Lit(cur));
        }
        Ok(parts)
    }

    fn read_hex(&mut self, max: usize) -> String {
        let mut h = String::new();
        while h.len() < max {
            match self.peek() {
                Some(c) if c.is_ascii_hexdigit() => {
                    h.push(c);
                    self.pos += 1;
                }
                _ => break,
            }
        }
        h
    }

    fn push_codepoint(&self, out: &mut String, cp: u32) -> Result<(), Flow> {
        match char::from_u32(cp) {
            Some(c) => {
                out.push(c);
                Ok(())
            }
            None => Err(self.err("invalid Unicode escape")),
        }
    }

    fn lex_escape(&mut self, out: &mut String) -> Result<(), Flow> {
        let Some(c) = self.peek() else {
            return Err(self.err("unterminated string meets end of file"));
        };
        self.pos += 1;
        match c {
            'n' => out.push('\n'),
            't' => out.push('\t'),
            'r' => out.push('\r'),
            's' => out.push(' '),
            'e' => out.push('\x1b'),
            'a' => out.push('\x07'),
            'b' => out.push('\x08'),
            'f' => out.push('\x0c'),
            'v' => out.push('\x0b'),
            '0'..='7' => {
                let mut v = c.to_digit(8).unwrap_or(0);
                for _ in 0..2 {
                    match self.peek().and_then(|d| d.to_digit(8)) {
                        Some(d) => {
                            v = v * 8 + d;
                            self.pos += 1;
                        }
                        None => break,
                    }
                }
                self.push_codepoint(out, v)?;
            }
            'x' => {
                let h = self.read_hex(2);
                if h.is_empty() {
                    return Err(self.err("invalid hex escape"));
                }
                let v = u32::from_str_radix(&h, 16).unwrap_or(0);
                self.push_codepoint(out, v)?;
            }
            'u' => {
                if self.peek() == Some('{') {
                    self.pos += 1;
                    loop {
                        while self.peek() == Some(' ') {
                            self.pos += 1;
                        }
                        if self.peek() == Some('}') {
                            self.pos += 1;
                            break;
                        }
                        let h = self.read_hex(6);
                        if h.is_empty() {
                            return Err(self.err("invalid Unicode escape"));
                        }
                        let v = u32::from_str_radix(&h, 16).unwrap_or(u32::MAX);
                        self.push_codepoint(out, v)?;
                    }
                } else {
                    let h = self.read_hex(4);
                    if h.len() != 4 {
                        return Err(self.err("invalid Unicode escape"));
                    }
                    let v = u32::from_str_radix(&h, 16).unwrap_or(u32::MAX);
                    self.push_codepoint(out, v)?;
                }
            }
            '\n' => {
                self.line += 1;
            }
            other => out.push(other),
        }
        Ok(())
    }

    fn lex_colon(&mut self) -> Result<Tok, Flow> {
        // self.peek() == ':'
        match self.peek_at(1) {
            Some('"') => {
                self.pos += 2;
                let parts = self.lex_dstring('"')?;
                Ok(match parts.as_slice() {
                    [StrPart::Lit(s)] => Tok::Sym(s.clone()),
                    _ => Tok::DSym(parts),
                })
            }
            Some('\'') => {
                self.pos += 2;
                Ok(Tok::Sym(self.lex_sstring()?))
            }
            Some('@') | Some('$')
                if self
                    .peek_at(2)
                    .is_some_and(|c| is_ident_start(c) || c.is_ascii_uppercase() || c == '@') =>
            {
                self.pos += 1;
                let mut w = String::new();
                while let Some(c @ ('@' | '$')) = self.peek() {
                    w.push(c);
                    self.pos += 1;
                }
                w.push_str(&self.read_word());
                Ok(Tok::Sym(w))
            }
            Some(c) if is_ident_start(c) || c.is_ascii_uppercase() => {
                self.pos += 1;
                let mut w = self.read_word();
                match self.peek() {
                    Some(c @ ('?' | '!')) if self.peek_at(1) != Some('=') => {
                        w.push(c);
                        self.pos += 1;
                    }
                    Some('=') if !matches!(self.peek_at(1), Some('=') | Some('>') | Some('~')) => {
                        w.push('=');
                        self.pos += 1;
                    }
                    _ => {}
                }
                Ok(Tok::Sym(w))
            }
            _ => {
                self.pos += 1;
                if let Some(op) = SYM_OPS.iter().find(|op| self.starts_with(op)) {
                    self.pos += op.chars().count();
                    return Ok(Tok::Sym(op.to_string()));
                }
                Ok(Tok::Op(":"))
            }
        }
    }

    fn lex_words(&mut self) -> Result<Tok, Flow> {
        let kind = self.peek_at(1);
        let open = self.peek_at(2).unwrap_or('[');
        let close = match open {
            '[' => ']',
            '(' => ')',
            '{' => '}',
            _ => '>',
        };
        self.pos += 3;
        let mut words = Vec::new();
        let mut cur = String::new();
        let mut depth = 0;
        loop {
            let Some(c) = self.peek() else {
                return Err(self.err("unterminated list meets end of file"));
            };
            self.pos += 1;
            if c == close && depth == 0 {
                break;
            }
            if c == open {
                depth += 1;
            } else if c == close {
                depth -= 1;
            }
            if c.is_whitespace() {
                if c == '\n' {
                    self.line += 1;
                }
                if !cur.is_empty() {
                    words.push(std::mem::take(&mut cur));
                }
            } else if c == '\\' && self.peek().is_some() {
                let n = self.peek().unwrap_or('\\');
                self.pos += 1;
                cur.push(n);
            } else {
                cur.push(c);
            }
        }
        if !cur.is_empty() {
            words.push(cur);
        }
        Ok(if kind == Some('i') {
            Tok::Symbols(words)
        } else {
            Tok::Words(words)
        })
    }
}

fn int_tok(digits: &str, radix: u32) -> Tok {
    let mut v: i128 = 0;
    for c in digits.chars() {
        let d = c.to_digit(radix).unwrap_or(0) as i128;
        match v.checked_mul(radix as i128).and_then(|x| x.checked_add(d)) {
            Some(n) if n <= (i64::MAX as i128) + 1 => v = n,
            _ => return Tok::BigInt,
        }
    }
    Tok::Int(v)
}
