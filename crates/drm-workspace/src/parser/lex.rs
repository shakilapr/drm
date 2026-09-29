//! Lexer for `.drm` topic definitions, verbatim from
//! `docs/algorithms/toolchain/schema-parser.md` §Grammar and §Data Structures
//! (`Tok`, `TokSpanned`). Two modes: normal (idents, ints, punctuation) and
//! annotation-value (entered at a `{` that does not follow `enum`; numbers,
//! quoted strings, and bare tokens are legal only there). `\r\n` is normalized
//! by the caller; `#` comments are skipped but newlines preserved.
//! REQ: REQ-TOOL-052, REQ-TOOL-053 (no panic on arbitrary bytes).
//! REQ: REQ-TOOL-032, REQ-TOOL-034, REQ-TOOL-031, REQ-TOOL-033.

use std::iter::Peekable;
use std::str::Chars;

use super::diag::{Diagnostic, Reason, Span};

/// Lexical token set (schema-parser.md §Data Structures).
/// REQ: REQ-TOOL-032.
#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    /// `[a-zA-Z_][a-zA-Z0-9_]*`.
    Ident(String),
    /// Unsigned integer, u32 range (overflow is diagnosed by the lexer).
    Int(u32),
    /// Signed decimal number (`-3.14`, `0.1`); finite decimals only.
    Number(f64),
    /// Annotation text value: `"display name"`.
    QuotedString(String),
    /// Annotation bare token, e.g. unit `m/s` — lexed in annotation-value mode
    /// until `,` or `}`, trimmed.
    BareAnnotationToken(String),
    /// `@` sigil (topic attributes).
    At,
    /// `:`.
    Colon,
    /// `[`.
    LBracket,
    /// `]`.
    RBracket,
    /// `{`.
    LBrace,
    /// `}`.
    RBrace,
    /// `<=`.
    Leq,
    /// `,`.
    Comma,
    /// Line break.
    Newline,
    /// End of input.
    Eof,
}

/// Token with its source span.
/// REQ: REQ-TOOL-017.
#[derive(Debug, Clone, PartialEq)]
pub struct TokSpanned {
    /// The token.
    pub tok: Tok,
    /// Span of the token's first character.
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Normal,
    Annotation,
}

struct Lexer<'t, 'd> {
    chars: Peekable<Chars<'t>>,
    diags: &'d mut Vec<Diagnostic>,
    line: u32,
    col: u32,
    offset: usize,
    mode: Mode,
    prev_enum: bool,
}

/// Lexes the whole (already `\r\n`-normalized, BOM-stripped) text into a token
/// stream ending with [`Tok::Eof`]. Stray characters are diagnosed and skipped;
/// the lexer never panics and always advances.
/// REQ: REQ-TOOL-053.
pub fn tokenize(text: &str, diags: &mut Vec<Diagnostic>) -> Vec<TokSpanned> {
    let mut lx = Lexer {
        chars: text.chars().peekable(),
        diags,
        line: 1,
        col: 1,
        offset: 0,
        mode: Mode::Normal,
        prev_enum: false,
    };
    let mut out = Vec::new();
    loop {
        let t = lx.next();
        let eof = t.tok == Tok::Eof;
        out.push(t);
        if eof {
            break;
        }
    }
    out
}

impl Lexer<'_, '_> {
    fn here(&self) -> Span {
        Span {
            line: self.line,
            col: self.col,
            offset: u32::try_from(self.offset).unwrap_or(u32::MAX),
        }
    }

    fn bump(&mut self) {
        if let Some(c) = self.chars.next() {
            self.offset += c.len_utf8();
            if c == '\n' {
                self.line += 1;
                self.col = 1;
            } else {
                self.col += 1;
            }
        }
    }

    fn diag_at(&mut self, span: Span, reason: Reason) {
        self.diags.push(Diagnostic { span, reason });
    }

    fn emit(&mut self, tok: Tok) -> TokSpanned {
        let span = self.here();
        self.prev_enum = matches!(&tok, Tok::Ident(s) if s == "enum");
        TokSpanned { tok, span }
    }

    fn next(&mut self) -> TokSpanned {
        loop {
            let Some(&ch) = self.chars.peek() else {
                return self.emit(Tok::Eof);
            };
            match ch {
                '\n' => {
                    self.bump();
                    return self.emit(Tok::Newline);
                }
                ' ' | '\t' | '\r' => {
                    self.bump();
                }
                '#' => {
                    while let Some(&c) = self.chars.peek() {
                        if c == '\n' {
                            break;
                        }
                        self.bump();
                    }
                }
                '@' => {
                    self.bump();
                    return self.emit(Tok::At);
                }
                ':' => {
                    self.bump();
                    return self.emit(Tok::Colon);
                }
                '[' => {
                    self.bump();
                    return self.emit(Tok::LBracket);
                }
                ']' => {
                    self.bump();
                    return self.emit(Tok::RBracket);
                }
                ',' => {
                    self.bump();
                    return self.emit(Tok::Comma);
                }
                '{' => {
                    self.bump();
                    if self.mode == Mode::Normal && !self.prev_enum {
                        // Field-annotation or encoding-spec brace: values inside
                        // switch to annotation-value mode (enum bodies stay normal).
                        self.mode = Mode::Annotation;
                    }
                    return self.emit(Tok::LBrace);
                }
                '}' => {
                    self.bump();
                    self.mode = Mode::Normal;
                    return self.emit(Tok::RBrace);
                }
                '<' => {
                    let start = self.here();
                    self.bump();
                    if self.chars.peek() == Some(&'=') {
                        self.bump();
                        return self.emit_at(start, Tok::Leq);
                    }
                    self.diag_at(start, Reason::TrailingGarbage);
                }
                '"' if self.mode == Mode::Annotation => {
                    return self.lex_quoted();
                }
                '"' => {
                    self.bump();
                    let start = self.here();
                    self.diag_at(start, Reason::TrailingGarbage);
                }
                '0'..='9' | '-' => {
                    return self.lex_number();
                }
                c if c.is_ascii_alphabetic() || c == '_' => {
                    return self.lex_ident_or_bare();
                }
                _ => {
                    self.bump();
                    let start = self.here();
                    let reason = if self.mode == Mode::Annotation {
                        Reason::AnnotationMalformed
                    } else {
                        Reason::TrailingGarbage
                    };
                    self.diag_at(start, reason);
                }
            }
        }
    }

    fn emit_at(&mut self, span: Span, tok: Tok) -> TokSpanned {
        self.prev_enum = matches!(&tok, Tok::Ident(s) if s == "enum");
        TokSpanned { tok, span }
    }

    fn lex_quoted(&mut self) -> TokSpanned {
        let start = self.here();
        self.bump();
        let mut s = String::new();
        let mut closed = false;
        while let Some(&c) = self.chars.peek() {
            if c == '"' {
                self.bump();
                closed = true;
                break;
            }
            if c == '\n' {
                break;
            }
            s.push(c);
            self.bump();
        }
        if !closed {
            // Unterminated quote: the whole value is malformed (grammar allows
            // `[^"]*` only between two quotes).
            self.diag_at(start, Reason::AnnotationMalformed);
        }
        self.emit_at(start, Tok::QuotedString(s))
    }

    fn lex_number(&mut self) -> TokSpanned {
        let start = self.here();
        let mut s = String::new();
        if self.chars.peek() == Some(&'-') {
            let digit_follows = self
                .chars
                .clone()
                .nth(1)
                .is_some_and(|c| c.is_ascii_digit());
            if !digit_follows {
                self.bump();
                let reason = if self.mode == Mode::Annotation {
                    Reason::AnnotationMalformed
                } else {
                    Reason::TrailingGarbage
                };
                self.diag_at(start, reason);
                return self.emit_at(start, Tok::Int(0));
            }
            s.push('-');
            self.bump();
        }
        while let Some(&c) = self.chars.peek() {
            if c.is_ascii_digit() {
                s.push(c);
                self.bump();
            } else {
                break;
            }
        }
        if self.chars.peek() == Some(&'.')
            && self
                .chars
                .clone()
                .nth(1)
                .is_some_and(|c| c.is_ascii_digit())
        {
            s.push('.');
            self.bump();
            while let Some(&c) = self.chars.peek() {
                if c.is_ascii_digit() {
                    s.push(c);
                    self.bump();
                } else {
                    break;
                }
            }
            // Finite decimal lexing can still overflow f64 on pathological
            // digit runs (`inf`), which the grammar rejects.
            let v = s.parse::<f64>().unwrap_or(f64::INFINITY);
            if !v.is_finite() {
                self.diag_at(start, Reason::SizeOverflow);
            }
            return self.emit_at(start, Tok::Number(v));
        }
        if s.starts_with('-') {
            // Negative integer: still a grammar `number`, stored as f64.
            let v = s.parse::<f64>().unwrap_or(f64::NEG_INFINITY);
            if !v.is_finite() {
                self.diag_at(start, Reason::SizeOverflow);
            }
            return self.emit_at(start, Tok::Number(v));
        }
        if let Ok(v) = s.parse::<u32>() {
            self.emit_at(start, Tok::Int(v))
        } else {
            self.diag_at(start, Reason::SizeOverflow);
            self.emit_at(start, Tok::Int(u32::MAX))
        }
    }

    fn lex_ident_or_bare(&mut self) -> TokSpanned {
        let start = self.here();
        let mut s = String::new();
        while let Some(&c) = self.chars.peek() {
            if c.is_ascii_alphanumeric() || c == '_' {
                s.push(c);
                self.bump();
            } else {
                break;
            }
        }
        if self.mode == Mode::Annotation {
            // `ident ':'` is the next annotation key; anything else is a bare
            // annotation token spanning to the next `,` / `}` / newline.
            if self.chars.peek() == Some(&':') {
                return self.emit_at(start, Tok::Ident(s));
            }
            while let Some(&c) = self.chars.peek() {
                if c == ',' || c == '}' || c == '\n' {
                    break;
                }
                s.push(c);
                self.bump();
            }
            return self.emit_at(start, Tok::BareAnnotationToken(s.trim().to_string()));
        }
        self.emit_at(start, Tok::Ident(s))
    }
}

#[cfg(test)]
mod tests {
    use super::super::diag::Reason;
    use super::{Tok, tokenize};

    /// REQ: REQ-TOOL-032 — token forms for the canonical grammar.
    #[test]
    fn normal_mode_tokens() {
        let mut d = Vec::new();
        let t = tokenize("@pool_size: 4\nspeed: f32[3] {unit: m/s}\n", &mut d);
        assert!(d.is_empty(), "{d:?}");
        let kinds: Vec<&Tok> = t.iter().map(|ts| &ts.tok).collect();
        assert_eq!(
            kinds,
            vec![
                &Tok::At,
                &Tok::Ident("pool_size".into()),
                &Tok::Colon,
                &Tok::Int(4),
                &Tok::Newline,
                &Tok::Ident("speed".into()),
                &Tok::Colon,
                &Tok::Ident("f32".into()),
                &Tok::LBracket,
                &Tok::Int(3),
                &Tok::RBracket,
                &Tok::LBrace,
                &Tok::Ident("unit".into()),
                &Tok::Colon,
                &Tok::BareAnnotationToken("m/s".into()),
                &Tok::RBrace,
                &Tok::Newline,
                &Tok::Eof,
            ]
        );
    }

    /// REQ: REQ-TOOL-033 — encoding spec lexes: prim arrives as a bare token,
    /// scale as a number.
    #[test]
    fn encoding_spec_tokens() {
        let mut d = Vec::new();
        let t = tokenize("@encoding: {type: i16, scale: 0.01}\n", &mut d);
        assert!(d.is_empty(), "{d:?}");
        let kinds: Vec<&Tok> = t.iter().map(|ts| &ts.tok).collect();
        assert!(kinds.contains(&&Tok::BareAnnotationToken("i16".into())));
        assert!(kinds.contains(&&Tok::Number(0.01)));
    }

    /// REQ: REQ-TOOL-034 — enum bodies stay in normal mode; a following
    /// annotation brace re-enters annotation mode.
    #[test]
    fn enum_then_annotation() {
        let mut d = Vec::new();
        let t = tokenize("s: enum { a, b } v: u8 {gain: 2}\n", &mut d);
        assert!(d.is_empty(), "{d:?}");
        let kinds: Vec<&Tok> = t.iter().map(|ts| &ts.tok).collect();
        assert!(kinds.contains(&&Tok::Ident("a".into())));
        assert!(kinds.contains(&&Tok::Ident("b".into())));
        assert!(kinds.contains(&&Tok::Int(2)));
    }

    /// REQ: REQ-TOOL-053 — stray bytes are diagnosed, never panic; int overflow
    /// is `size-overflow`.
    #[test]
    fn stray_and_overflow() {
        let mut d = Vec::new();
        let _ = tokenize("a: u8 \u{7f} ??\n", &mut d);
        assert!(d.iter().any(|x| x.reason == Reason::TrailingGarbage));

        let mut d = Vec::new();
        let t = tokenize("n: u8[99999999999]\n", &mut d);
        assert!(d.iter().any(|x| x.reason == Reason::SizeOverflow));
        assert!(t.iter().any(|ts| ts.tok == Tok::Int(u32::MAX)));
    }

    /// REQ: REQ-TOOL-053 — unterminated quote is `annotation-malformed`; CRLF
    /// text is caller-normalized, lone `\r` is skipped as whitespace.
    #[test]
    fn quote_and_control() {
        let mut d = Vec::new();
        let _ = tokenize("a: u8 {label: \"open\n", &mut d);
        assert!(d.iter().any(|x| x.reason == Reason::AnnotationMalformed));
    }
}
