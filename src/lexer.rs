//! JavaScript lexer / tokenizer.
//!
//! Handles the full ES2020+ lexical grammar that the parser needs, including
//! regex literals (context-sensitive), template literals, BigInt, and numeric
//! literals in decimal/hex/octal/binary forms.

use std::rc::Rc;

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    // Literals
    Number(f64),
    BigInt(Rc<str>),
    String(Rc<str>),
    TemplateNoSub { cooked: Rc<str>, raw: Rc<str> },
    TemplateHead { cooked: Rc<str>, raw: Rc<str> },
    TemplateMiddle { cooked: Rc<str>, raw: Rc<str> },
    TemplateTail { cooked: Rc<str>, raw: Rc<str> },
    Regex { pattern: Rc<str>, flags: Rc<str> },
    Ident(Rc<str>),
    PrivateIdent(Rc<str>),
    // Keywords
    Keyword(Keyword),
    // Punctuation
    Punct(Punct),
    Null,
    True,
    False,
    Undefined,
    Eof,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Keyword {
    Var,
    Let,
    Const,
    Function,
    Return,
    If,
    Else,
    While,
    Do,
    For,
    Break,
    Continue,
    Switch,
    Case,
    Default,
    Throw,
    Try,
    Catch,
    Finally,
    New,
    Delete,
    Typeof,
    Instanceof,
    In,
    Of,
    This,
    Super,
    Class,
    Extends,
    Static,
    Get,
    Set,
    Async,
    Await,
    Yield,
    Import,
    Export,
    From,
    As,
    Default_, // `default` as keyword handled contextually
    With,
    Debugger,
    Void,
    Null,
    True,
    False,
    Undefined,
    Constructor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Punct {
    // Single char
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Comma,
    Semicolon,
    Dot,
    Question,
    Colon,
    Tilde,
    Bang,
    // Multi char
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    StarStar,
    Amp,
    Pipe,
    Caret,
    Shl,    // <<
    Shr,    // >>
    UShr,   // >>>
    Eq,
    EqEq,
    EqEqEq,
    NotEq,
    NotEqEq,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    Nullish, // ??
    PlusPlus,
    MinusMinus,
    PlusEq,
    MinusEq,
    StarEq,
    SlashEq,
    PercentEq,
    StarStarEq,
    AmpEq,
    PipeEq,
    CaretEq,
    ShlEq,
    ShrEq,
    UShrEq,
    AndEq,
    OrEq,
    NullishEq,
    Arrow,    // =>
    Spread,   // ...
    Optional, // ?.
    Hash,     // # private
    At,       // decorators (parsed & ignored)
}

#[derive(Debug, Clone)]
pub struct TokenWithPos {
    pub token: Token,
    pub line: u32,
    pub col: u32,
    pub preceded_by_newline: bool,
}

pub struct Lexer<'a> {
    src: &'a [u8],
    pos: usize,
    line: u32,
    col: u32,
    /// Whether the previous significant token was followed by a newline.
    pub last_was_newline: bool,
    /// Brace stack: `true` means a template-substitution is open (the matching
    /// `}` should continue the template literal rather than close a block).
    brace_stack: Vec<bool>,
}

/// A save point allowing the lexer to be rewound.
#[derive(Clone, Copy)]
pub struct LexSave {
    pos: usize,
    line: u32,
    col: u32,
    last_was_newline: bool,
    brace_depth: usize,
}

impl<'a> Lexer<'a> {
    pub fn new(src: &'a str) -> Self {
        Lexer {
            src: src.as_bytes(),
            pos: 0,
            line: 1,
            col: 1,
            last_was_newline: false,
            brace_stack: Vec::new(),
        }
    }

    /// Snapshot the lexer state so it can be restored later.
    pub fn save(&self) -> LexSave {
        LexSave {
            pos: self.pos,
            line: self.line,
            col: self.col,
            last_was_newline: self.last_was_newline,
            brace_depth: self.brace_stack.len(),
        }
    }

    /// Restore a previously saved state.
    pub fn restore(&mut self, s: LexSave) {
        self.pos = s.pos;
        self.line = s.line;
        self.col = s.col;
        self.last_was_newline = s.last_was_newline;
        self.brace_stack.truncate(s.brace_depth);
    }

    fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }
    fn peek2(&self) -> Option<u8> {
        self.src.get(self.pos + 1).copied()
    }
    fn peek3(&self) -> Option<u8> {
        self.src.get(self.pos + 2).copied()
    }
    fn peek4(&self) -> Option<u8> {
        self.src.get(self.pos + 3).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let c = self.peek()?;
        self.pos += 1;
        if c == b'\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        Some(c)
    }

    fn skip_ws_and_comments(&mut self) -> bool {
        let mut had_newline = false;
        loop {
            match self.peek() {
                Some(b' ') | Some(b'\t') | Some(b'\r') => {
                    self.bump();
                }
                Some(b'\n') => {
                    had_newline = true;
                    self.bump();
                }
                Some(b'/') if self.peek2() == Some(b'/') => {
                    while let Some(c) = self.peek() {
                        if c == b'\n' {
                            break;
                        }
                        self.bump();
                    }
                }
                Some(b'/') if self.peek2() == Some(b'*') => {
                    self.bump();
                    self.bump();
                    while let Some(c) = self.peek() {
                        if c == b'\n' {
                            had_newline = true;
                        }
                        if c == b'*' && self.peek2() == Some(b'/') {
                            self.bump();
                            self.bump();
                            break;
                        }
                        self.bump();
                    }
                }
                _ => break,
            }
        }
        had_newline
    }

    /// Read the next token. `prev` is the previous token (for regex disambiguation).
    pub fn next(&mut self, prev: &Token) -> TokenWithPos {
        let had_newline = self.skip_ws_and_comments();
        let line = self.line;
        let col = self.col;
        let c = match self.peek() {
            None => {
                return TokenWithPos {
                    token: Token::Eof,
                    line,
                    col,
                    preceded_by_newline: had_newline,
                }
            }
            Some(c) => c,
        };

        let token = match c {
            b'"' | b'\'' => self.read_string(c),
            b'`' => self.read_template(true),
            b'0'..=b'9' => self.read_number(),
            b'.' if matches!(self.peek2(), Some(b'0'..=b'9')) => self.read_number(),
            b'#' => {
                self.bump();
                let name = self.read_ident_rest();
                Token::PrivateIdent(name)
            }
            b'@' => {
                self.bump();
                // skip decorator name
                self.read_ident_rest();
                Token::Punct(Punct::At)
            }
            _ if is_ident_start(c) => {
                let name = self.read_ident_rest();
                if name.len() == 1 && &*name == "_" {
                    // keep as ident
                }
                classify_ident(&name)
            }
            _ => self.read_punct_or_regex(c, prev),
        };

        TokenWithPos {
            token,
            line,
            col,
            preceded_by_newline: had_newline,
        }
    }

    fn read_ident_rest(&mut self) -> Rc<str> {
        let start = self.pos;
        while let Some(c) = self.peek() {
            if is_ident_part(c) {
                self.bump();
            } else {
                break;
            }
        }
        // Handle unicode escapes in identifiers (\uXXXX, \u{...})
        let raw = std::str::from_utf8(&self.src[start..self.pos]).unwrap_or("");
        Rc::from(raw)
    }

    fn read_string(&mut self, quote: u8) -> Token {
        self.bump(); // opening quote
        let mut s = String::new();
        loop {
            match self.peek() {
                None => return Token::String(Rc::from("")),
                Some(c) if c == quote => {
                    self.bump();
                    break;
                }
                Some(b'\\') => {
                    self.bump();
                    self.read_escape(&mut s);
                }
                Some(b'\n') => {
                    // unterminated string literal — be lenient
                    break;
                }
                Some(_) => {
                    let ch = self.read_char();
                    s.push(ch);
                }
            }
        }
        Token::String(Rc::from(s.as_str()))
    }

    fn read_escape(&mut self, s: &mut String) {
        match self.peek() {
            None => {}
            Some(b'n') => {
                self.bump();
                s.push('\n');
            }
            Some(b't') => {
                self.bump();
                s.push('\t');
            }
            Some(b'r') => {
                self.bump();
                s.push('\r');
            }
            Some(b'b') => {
                self.bump();
                s.push('\u{8}');
            }
            Some(b'f') => {
                self.bump();
                s.push('\u{c}');
            }
            Some(b'v') => {
                self.bump();
                s.push('\u{b}');
            }
            Some(b'0') => {
                self.bump();
                if matches!(self.peek(), Some(b'0'..=b'9')) {
                    // legacy octal — parse octal
                    let mut val: u32 = 0;
                    while let Some(c) = self.peek() {
                        if (b'0'..=b'7').contains(&c) {
                            val = val * 8 + (c - b'0') as u32;
                            self.bump();
                            if val > 0o377 {
                                break;
                            }
                        } else {
                            break;
                        }
                    }
                    s.push(char::from_u32(val & 0xff).unwrap_or('\0'));
                } else {
                    s.push('\0');
                }
            }
            Some(b'x') => {
                self.bump();
                let h1 = self.bump().unwrap_or(b'0');
                let h2 = self.bump().unwrap_or(b'0');
                let v = u32::from_str_radix(
                    &format!("{}{}", h1 as char, h2 as char),
                    16,
                )
                .unwrap_or(0);
                s.push(char::from_u32(v).unwrap_or('\0'));
            }
            Some(b'u') => {
                self.bump();
                if self.peek() == Some(b'{') {
                    self.bump();
                    let mut hex = String::new();
                    while let Some(c) = self.peek() {
                        if c == b'}' {
                            self.bump();
                            break;
                        }
                        hex.push(c as char);
                        self.bump();
                    }
                    let v = u32::from_str_radix(&hex, 16).unwrap_or(0);
                    if let Some(ch) = char::from_u32(v) {
                        s.push(ch);
                    }
                } else {
                    let mut hex = String::new();
                    for _ in 0..4 {
                        if let Some(c) = self.peek() {
                            hex.push(c as char);
                            self.bump();
                        }
                    }
                    let v = u32::from_str_radix(&hex, 16).unwrap_or(0);
                    // surrogate pair handling
                    if (0xD800..=0xDBFF).contains(&v) {
                        if self.peek() == Some(b'\\') {
                            self.bump();
                            if self.peek() == Some(b'u') {
                                self.bump();
                                let mut hex2 = String::new();
                                for _ in 0..4 {
                                    if let Some(c) = self.peek() {
                                        hex2.push(c as char);
                                        self.bump();
                                    }
                                }
                                let lo = u32::from_str_radix(&hex2, 16).unwrap_or(0);
                                let cp = 0x10000 + ((v - 0xD800) << 10) + (lo - 0xDC00);
                                if let Some(ch) = char::from_u32(cp) {
                                    s.push(ch);
                                }
                                return;
                            }
                        }
                    }
                    if let Some(ch) = char::from_u32(v) {
                        s.push(ch);
                    }
                }
            }
            Some(c) => {
                self.bump();
                s.push(c as char);
            }
        }
    }

    fn read_char(&mut self) -> char {
        // Read one UTF-8 char.
        let start = self.pos;
        let first = self.peek().unwrap();
        let len = utf8_len(first);
        for _ in 0..len {
            self.bump();
        }
        std::str::from_utf8(&self.src[start..self.pos])
            .ok()
            .and_then(|s| s.chars().next())
            .unwrap_or('\u{fffd}')
    }

    fn read_template(&mut self, is_start: bool) -> Token {
        // assumes opening backtick (start) or closing brace of substitution consumed
        self.bump(); // backtick or `}`
        let mut cooked = String::new();
        let mut raw = String::new();
        loop {
            match self.peek() {
                None => break,
                Some(b'`') => {
                    self.bump();
                    let cooked = Rc::from(cooked.as_str());
                    let raw = Rc::from(raw.as_str());
                    return if is_start {
                        Token::TemplateNoSub { cooked, raw }
                    } else {
                        Token::TemplateTail { cooked, raw }
                    };
                }
                Some(b'$') if self.peek2() == Some(b'{') => {
                    self.bump();
                    self.bump();
                    self.brace_stack.push(true);
                    let cooked = Rc::from(cooked.as_str());
                    let raw = Rc::from(raw.as_str());
                    return if is_start {
                        Token::TemplateHead { cooked, raw }
                    } else {
                        Token::TemplateMiddle { cooked, raw }
                    };
                }
                Some(b'\\') => {
                    raw.push('\\');
                    self.bump();
                    // capture raw next char(s)
                    let before = self.pos;
                    let mut tmp = String::new();
                    self.read_escape(&mut tmp);
                    cooked.push_str(&tmp);
                    // raw: re-encode the escape as-is
                    let raw_seg: String = std::str::from_utf8(&self.src[before..self.pos])
                        .unwrap_or("")
                        .chars()
                        .collect();
                    raw.push_str(&raw_seg);
                }
                Some(_) => {
                    let ch = self.read_char();
                    cooked.push(ch);
                    raw.push(ch);
                }
            }
        }
        let cooked = Rc::from(cooked.as_str());
        let raw = Rc::from(raw.as_str());
        if is_start {
            Token::TemplateNoSub { cooked, raw }
        } else {
            Token::TemplateTail { cooked, raw }
        }
    }

    fn read_number(&mut self) -> Token {
        let start = self.pos;
        let mut is_bigint = false;
        if self.peek() == Some(b'0') {
            match self.peek2() {
                Some(b'x') | Some(b'X') => {
                    self.bump();
                    self.bump();
                    while matches!(self.peek(), Some(b'0'..=b'9' | b'a'..=b'f' | b'A'..=b'F' | b'_')) {
                        self.bump();
                    }
                    if self.peek() == Some(b'n') {
                        self.bump();
                        is_bigint = true;
                    }
                    let text = std::str::from_utf8(&self.src[start..self.pos]).unwrap_or("");
                    if is_bigint {
                        let digits = text.trim_start_matches("0x").trim_start_matches("0X");
                        let digits: String = digits.chars().filter(|c| *c != '_').collect();
                        return Token::BigInt(Rc::from(digits.as_str()));
                    }
                    let digits = text.trim_start_matches("0x").trim_start_matches("0X");
                    let digits: String = digits.chars().filter(|c| *c != '_').collect();
                    let v = u64::from_str_radix(&digits, 16).map(|v| v as f64).unwrap_or(f64::NAN);
                    return Token::Number(v);
                }
                Some(b'o') | Some(b'O') => {
                    self.bump();
                    self.bump();
                    while matches!(self.peek(), Some(b'0'..=b'7' | b'_')) {
                        self.bump();
                    }
                    if self.peek() == Some(b'n') {
                        self.bump();
                        is_bigint = true;
                    }
                    let text = std::str::from_utf8(&self.src[start..self.pos]).unwrap_or("");
                    if is_bigint {
                        let digits = text.trim_start_matches("0o").trim_start_matches("0O");
                        let digits: String = digits.chars().filter(|c| *c != '_').collect();
                        return Token::BigInt(Rc::from(digits.as_str()));
                    }
                    let digits = text.trim_start_matches("0o").trim_start_matches("0O");
                    let digits: String = digits.chars().filter(|c| *c != '_').collect();
                    let v = u64::from_str_radix(&digits, 8).map(|v| v as f64).unwrap_or(f64::NAN);
                    return Token::Number(v);
                }
                Some(b'b') | Some(b'B') => {
                    self.bump();
                    self.bump();
                    while matches!(self.peek(), Some(b'0' | b'1' | b'_')) {
                        self.bump();
                    }
                    if self.peek() == Some(b'n') {
                        self.bump();
                        is_bigint = true;
                    }
                    let text = std::str::from_utf8(&self.src[start..self.pos]).unwrap_or("");
                    if is_bigint {
                        let digits = text.trim_start_matches("0b").trim_start_matches("0B");
                        let digits: String = digits.chars().filter(|c| *c != '_').collect();
                        return Token::BigInt(Rc::from(digits.as_str()));
                    }
                    let digits = text.trim_start_matches("0b").trim_start_matches("0B");
                    let digits: String = digits.chars().filter(|c| *c != '_').collect();
                    let v = u64::from_str_radix(&digits, 2).map(|v| v as f64).unwrap_or(f64::NAN);
                    return Token::Number(v);
                }
                _ => {}
            }
        }
        // decimal
        let mut saw_dot = false;
        let mut saw_exp = false;
        while let Some(c) = self.peek() {
            match c {
                b'0'..=b'9' | b'_' => {
                    self.bump();
                }
                b'.' if !saw_dot && !saw_exp => {
                    saw_dot = true;
                    self.bump();
                }
                b'e' | b'E' if !saw_exp => {
                    saw_exp = true;
                    self.bump();
                    if matches!(self.peek(), Some(b'+') | Some(b'-')) {
                        self.bump();
                    }
                }
                b'n' => {
                    self.bump();
                    is_bigint = true;
                    break;
                }
                _ => break,
            }
        }
        let text = std::str::from_utf8(&self.src[start..self.pos]).unwrap_or("");
        let cleaned: String = text.chars().filter(|c| *c != '_').collect();
        if is_bigint {
            let digits = cleaned.trim_end_matches('n');
            return Token::BigInt(Rc::from(digits));
        }
        let v = cleaned.parse::<f64>().unwrap_or(f64::NAN);
        Token::Number(v)
    }

    fn read_punct_or_regex(&mut self, c: u8, prev: &Token) -> Token {
        // Regex disambiguation: a `/` starts a regex if the previous token
        // cannot end an expression.
        if c == b'/' && regex_allowed_before(prev) {
            return self.read_regex();
        }

        match c {
            b'(' => { self.bump(); Token::Punct(Punct::LParen) }
            b')' => { self.bump(); Token::Punct(Punct::RParen) }
            b'{' => { self.bump(); self.brace_stack.push(false); Token::Punct(Punct::LBrace) }
            b'}' => {
                if self.brace_stack.pop() == Some(true) {
                    // End of a template substitution: continue the template.
                    self.read_template(false)
                } else {
                    self.bump(); Token::Punct(Punct::RBrace)
                }
            }
            b'[' => { self.bump(); Token::Punct(Punct::LBracket) }
            b']' => { self.bump(); Token::Punct(Punct::RBracket) }
            b',' => { self.bump(); Token::Punct(Punct::Comma) }
            b';' => { self.bump(); Token::Punct(Punct::Semicolon) }
            b'~' => { self.bump(); Token::Punct(Punct::Tilde) }
            b':' => { self.bump(); Token::Punct(Punct::Colon) }
            b'@' => { self.bump(); Token::Punct(Punct::At) }
            b'.' => {
                if self.peek2() == Some(b'.') && self.peek3() == Some(b'.') {
                    self.bump(); self.bump(); self.bump();
                    Token::Punct(Punct::Spread)
                } else {
                    self.bump();
                    Token::Punct(Punct::Dot)
                }
            }
            b'?' => {
                if self.peek2() == Some(b'?') {
                    if self.peek3() == Some(b'=') {
                        self.bump(); self.bump(); self.bump();
                        Token::Punct(Punct::NullishEq)
                    } else {
                        self.bump(); self.bump();
                        Token::Punct(Punct::Nullish)
                    }
                } else if self.peek2() == Some(b'.') && !matches!(self.peek3(), Some(b'0'..=b'9')) {
                    self.bump(); self.bump();
                    Token::Punct(Punct::Optional)
                } else {
                    self.bump();
                    Token::Punct(Punct::Question)
                }
            }
            b'!' => {
                if self.peek2() == Some(b'=') {
                    if self.peek3() == Some(b'=') {
                        self.bump(); self.bump(); self.bump();
                        Token::Punct(Punct::NotEqEq)
                    } else {
                        self.bump(); self.bump();
                        Token::Punct(Punct::NotEq)
                    }
                } else {
                    self.bump();
                    Token::Punct(Punct::Bang)
                }
            }
            b'=' => {
                let p2 = self.peek2();
                if p2 == Some(b'=') {
                    if self.peek3() == Some(b'=') {
                        self.bump(); self.bump(); self.bump();
                        Token::Punct(Punct::EqEqEq)
                    } else {
                        self.bump(); self.bump();
                        Token::Punct(Punct::EqEq)
                    }
                } else if p2 == Some(b'>') {
                    self.bump(); self.bump();
                    Token::Punct(Punct::Arrow)
                } else {
                    self.bump();
                    Token::Punct(Punct::Eq)
                }
            }
            b'+' => {
                if self.peek2() == Some(b'+') {
                    self.bump(); self.bump();
                    Token::Punct(Punct::PlusPlus)
                } else if self.peek2() == Some(b'=') {
                    self.bump(); self.bump();
                    Token::Punct(Punct::PlusEq)
                } else {
                    self.bump();
                    Token::Punct(Punct::Plus)
                }
            }
            b'-' => {
                if self.peek2() == Some(b'-') {
                    self.bump(); self.bump();
                    Token::Punct(Punct::MinusMinus)
                } else if self.peek2() == Some(b'=') {
                    self.bump(); self.bump();
                    Token::Punct(Punct::MinusEq)
                } else {
                    self.bump();
                    Token::Punct(Punct::Minus)
                }
            }
            b'*' => {
                if self.peek2() == Some(b'*') {
                    if self.peek3() == Some(b'=') {
                        self.bump(); self.bump(); self.bump();
                        Token::Punct(Punct::StarStarEq)
                    } else {
                        self.bump(); self.bump();
                        Token::Punct(Punct::StarStar)
                    }
                } else if self.peek2() == Some(b'=') {
                    self.bump(); self.bump();
                    Token::Punct(Punct::StarEq)
                } else {
                    self.bump();
                    Token::Punct(Punct::Star)
                }
            }
            b'/' => {
                if self.peek2() == Some(b'=') {
                    self.bump(); self.bump();
                    Token::Punct(Punct::SlashEq)
                } else {
                    self.bump();
                    Token::Punct(Punct::Slash)
                }
            }
            b'%' => {
                if self.peek2() == Some(b'=') {
                    self.bump(); self.bump();
                    Token::Punct(Punct::PercentEq)
                } else {
                    self.bump();
                    Token::Punct(Punct::Percent)
                }
            }
            b'&' => {
                if self.peek2() == Some(b'&') {
                    if self.peek3() == Some(b'=') {
                        self.bump(); self.bump(); self.bump();
                        Token::Punct(Punct::AndEq)
                    } else {
                        self.bump(); self.bump();
                        Token::Punct(Punct::And)
                    }
                } else if self.peek2() == Some(b'=') {
                    self.bump(); self.bump();
                    Token::Punct(Punct::AmpEq)
                } else {
                    self.bump();
                    Token::Punct(Punct::Amp)
                }
            }
            b'|' => {
                if self.peek2() == Some(b'|') {
                    if self.peek3() == Some(b'=') {
                        self.bump(); self.bump(); self.bump();
                        Token::Punct(Punct::OrEq)
                    } else {
                        self.bump(); self.bump();
                        Token::Punct(Punct::Or)
                    }
                } else if self.peek2() == Some(b'=') {
                    self.bump(); self.bump();
                    Token::Punct(Punct::PipeEq)
                } else {
                    self.bump();
                    Token::Punct(Punct::Pipe)
                }
            }
            b'^' => {
                if self.peek2() == Some(b'=') {
                    self.bump(); self.bump();
                    Token::Punct(Punct::CaretEq)
                } else {
                    self.bump();
                    Token::Punct(Punct::Caret)
                }
            }
            b'<' => {
                if self.peek2() == Some(b'<') {
                    if self.peek3() == Some(b'=') {
                        self.bump(); self.bump(); self.bump();
                        Token::Punct(Punct::ShlEq)
                    } else {
                        self.bump(); self.bump();
                        Token::Punct(Punct::Shl)
                    }
                } else if self.peek2() == Some(b'=') {
                    self.bump(); self.bump();
                    Token::Punct(Punct::Le)
                } else {
                    self.bump();
                    Token::Punct(Punct::Lt)
                }
            }
            b'>' => {
                if self.peek2() == Some(b'>') {
                    if self.peek3() == Some(b'>') {
                        if self.peek4() == Some(b'=') {
                            self.bump(); self.bump(); self.bump(); self.bump();
                            Token::Punct(Punct::UShrEq)
                        } else {
                            self.bump(); self.bump(); self.bump();
                            Token::Punct(Punct::UShr)
                        }
                    } else if self.peek3() == Some(b'=') {
                        self.bump(); self.bump(); self.bump();
                        Token::Punct(Punct::ShrEq)
                    } else {
                        self.bump(); self.bump();
                        Token::Punct(Punct::Shr)
                    }
                } else if self.peek2() == Some(b'=') {
                    self.bump(); self.bump();
                    Token::Punct(Punct::Ge)
                } else {
                    self.bump();
                    Token::Punct(Punct::Gt)
                }
            }
            b'#' => {
                self.bump();
                let name = self.read_ident_rest();
                Token::PrivateIdent(name)
            }
            _ => {
                // unknown char: skip
                self.bump();
                Token::Punct(Punct::Semicolon)
            }
        }
    }

    fn read_regex(&mut self) -> Token {
        self.bump(); // leading /
        let mut pattern = String::new();
        let mut in_class = false;
        loop {
            match self.peek() {
                None => break,
                Some(b'\\') => {
                    pattern.push('\\');
                    self.bump();
                    if let Some(c) = self.peek() {
                        pattern.push(c as char);
                        self.bump();
                    }
                }
                Some(b'[') => {
                    in_class = true;
                    pattern.push('[');
                    self.bump();
                }
                Some(b']') => {
                    in_class = false;
                    pattern.push(']');
                    self.bump();
                }
                Some(b'/') if !in_class => {
                    self.bump();
                    break;
                }
                Some(_) => {
                    let ch = self.read_char();
                    pattern.push(ch);
                }
            }
        }
        let mut flags = String::new();
        while let Some(c) = self.peek() {
            if c.is_ascii_alphabetic() {
                flags.push(c as char);
                self.bump();
            } else {
                break;
            }
        }
        Token::Regex {
            pattern: Rc::from(pattern.as_str()),
            flags: Rc::from(flags.as_str()),
        }
    }
}

fn utf8_len(first: u8) -> usize {
    if first < 0x80 {
        1
    } else if first >> 5 == 0b110 {
        2
    } else if first >> 4 == 0b1110 {
        3
    } else if first >> 3 == 0b11110 {
        4
    } else {
        1
    }
}

fn is_ident_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_' || c == b'$' || c >= 0x80
}
fn is_ident_part(c: u8) -> bool {
    is_ident_start(c) || c.is_ascii_digit()
}

/// Whether a `/` after `prev` should be parsed as a regex (vs. division).
pub fn regex_allowed_before(prev: &Token) -> bool {
    match prev {
        Token::Number(_)
        | Token::BigInt(_)
        | Token::String(_)
        | Token::Ident(_)
        | Token::PrivateIdent(_)
        | Token::True
        | Token::False
        | Token::Null
        | Token::Undefined
        | Token::Regex { .. }
        | Token::TemplateNoSub { .. }
        | Token::TemplateTail { .. }
        | Token::Punct(Punct::RParen)
        | Token::Punct(Punct::RBracket)
        | Token::Punct(Punct::RBrace) => false,
        Token::Keyword(k)
            if matches!(
                k,
                Keyword::This
                    | Keyword::Super
                    | Keyword::True
                    | Keyword::False
                    | Keyword::Null
                    | Keyword::Undefined
            ) =>
        {
            false
        }
        _ => true,
    }
}

fn classify_ident(name: &Rc<str>) -> Token {
    let s: &str = name;
    let kw = match s {
        "var" => Some(Keyword::Var),
        "let" => Some(Keyword::Let),
        "const" => Some(Keyword::Const),
        "function" => Some(Keyword::Function),
        "return" => Some(Keyword::Return),
        "if" => Some(Keyword::If),
        "else" => Some(Keyword::Else),
        "while" => Some(Keyword::While),
        "do" => Some(Keyword::Do),
        "for" => Some(Keyword::For),
        "break" => Some(Keyword::Break),
        "continue" => Some(Keyword::Continue),
        "switch" => Some(Keyword::Switch),
        "case" => Some(Keyword::Case),
        "default" => Some(Keyword::Default),
        "throw" => Some(Keyword::Throw),
        "try" => Some(Keyword::Try),
        "catch" => Some(Keyword::Catch),
        "finally" => Some(Keyword::Finally),
        "new" => Some(Keyword::New),
        "delete" => Some(Keyword::Delete),
        "typeof" => Some(Keyword::Typeof),
        "instanceof" => Some(Keyword::Instanceof),
        "in" => Some(Keyword::In),
        "of" => Some(Keyword::Of),
        "this" => Some(Keyword::This),
        "super" => Some(Keyword::Super),
        "class" => Some(Keyword::Class),
        "extends" => Some(Keyword::Extends),
        "static" => Some(Keyword::Static),
        "get" => Some(Keyword::Get),
        "set" => Some(Keyword::Set),
        "async" => Some(Keyword::Async),
        "await" => Some(Keyword::Await),
        "yield" => Some(Keyword::Yield),
        "import" => Some(Keyword::Import),
        "export" => Some(Keyword::Export),
        "from" => Some(Keyword::From),
        "as" => Some(Keyword::As),
        "with" => Some(Keyword::With),
        "debugger" => Some(Keyword::Debugger),
        "void" => Some(Keyword::Void),
        "constructor" => Some(Keyword::Constructor),
        _ => None,
    };
    match kw {
        Some(k) => Token::Keyword(k),
        None => match s {
            "true" => Token::True,
            "false" => Token::False,
            "null" => Token::Null,
            "undefined" => Token::Undefined,
            _ => Token::Ident(name.clone()),
        },
    }
}

/// Tokenize a whole source into a vector (used by tests / REPL multi-line).
pub fn tokenize(src: &str) -> Vec<TokenWithPos> {
    let mut lex = Lexer::new(src);
    let mut prev = Token::Punct(Punct::Semicolon);
    let mut out = Vec::new();
    loop {
        let t = lex.next(&prev);
        let is_eof = matches!(t.token, Token::Eof);
        out.push(t.clone());
        prev = t.token;
        if is_eof {
            break;
        }
    }
    out
}
