//! `.drm` schema parser: AST data structures, recursive-descent parser with
//! line recovery, local validation, and the byte entry point — verbatim from
//! `docs/algorithms/toolchain/schema-parser.md` §Data Structures, §Functions,
//! and §Invariants (I1, I2). One file yields ALL its errors (skip-to-newline
//! recovery); the vocabulary is closed. Semantic AST equality (I4) ignores
//! source spans: spans live on nodes for diagnostics only and `PartialEq`
//! skips them.
//! REQ: REQ-TOOL-052, REQ-TOOL-017, REQ-TOOL-032, REQ-TOOL-034, REQ-TOOL-031,
//! REQ-TOOL-033.

pub mod diag;
pub mod lex;
pub mod render;

#[cfg(feature = "spatial-grammar")]
pub mod spatial;

use std::collections::HashSet;

use lex::{Tok, TokSpanned};

pub use diag::{Diagnostic, Reason, Span};
pub use render::render_canonical;

/// The 11 canonical primitives (schema-parser.md §Data Structures).
/// REQ: REQ-TOOL-034.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Prim {
    /// `f32`.
    F32,
    /// `f64`.
    F64,
    /// `i8`.
    I8,
    /// `i16`.
    I16,
    /// `i32`.
    I32,
    /// `i64`.
    I64,
    /// `u8`.
    U8,
    /// `u16`.
    U16,
    /// `u32`.
    U32,
    /// `u64`.
    U64,
    /// `bool`.
    Bool,
}

impl Prim {
    fn from_name(s: &str) -> Option<Self> {
        Some(match s {
            "f32" => Self::F32,
            "f64" => Self::F64,
            "i8" => Self::I8,
            "i16" => Self::I16,
            "i32" => Self::I32,
            "i64" => Self::I64,
            "u8" => Self::U8,
            "u16" => Self::U16,
            "u32" => Self::U32,
            "u64" => Self::U64,
            "bool" => Self::Bool,
            _ => return None,
        })
    }

    /// Lowercase source spelling (canonical render).
    /// REQ: REQ-TOOL-054.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::F32 => "f32",
            Self::F64 => "f64",
            Self::I8 => "i8",
            Self::I16 => "i16",
            Self::I32 => "i32",
            Self::I64 => "i64",
            Self::U8 => "u8",
            Self::U16 => "u16",
            Self::U32 => "u32",
            Self::U64 => "u64",
            Self::Bool => "bool",
        }
    }
}

/// Field/topic-attribute types. Arrays nest at most one level over a prim.
/// REQ: REQ-TOOL-034.
#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    /// A canonical primitive.
    Prim(Prim),
    /// `string[N]`.
    String {
        /// Fixed capacity in bytes.
        max: u32,
    },
    /// `bytes[<=N]`.
    Bytes {
        /// Upper bound in bytes.
        max: u32,
    },
    /// `T[N]` over a prim.
    FixedArray {
        /// Element prim.
        elem: Prim,
        /// Exact length.
        n: u32,
    },
    /// `T[<=N]` over a prim.
    Bounded {
        /// Element prim.
        elem: Prim,
        /// Upper bound.
        max: u32,
    },
    /// `enum { … }`.
    Enum {
        /// Variant names, declaration order.
        variants: Vec<String>,
    },
}

/// Semantic annotation value (preserved through canonical render, I4).
/// REQ: REQ-TOOL-055.
#[derive(Debug, Clone, PartialEq)]
pub enum AnnotValue {
    /// Numeric value (canonicalized from the stored `f64`).
    Number(f64),
    /// Bare token, e.g. unit `m/s`; re-emitted verbatim.
    Token(String),
    /// Bracketed range `[a, b]`; interior comma is not an entry separator.
    Range(f64, f64),
    /// Quoted text `"…"`; re-emitted verbatim.
    Text(String),
}

/// One field annotation entry.
/// REQ: REQ-TOOL-055.
#[derive(Debug, Clone)]
pub struct Annotation {
    /// Entry key.
    pub key: String,
    /// Entry value.
    pub value: AnnotValue,
    /// Key span (diagnostics only; excluded from semantic equality).
    pub span: Span,
}

/// A field declaration.
/// REQ: REQ-TOOL-032.
#[derive(Debug, Clone)]
pub struct Field {
    /// Field name.
    pub name: String,
    /// Field type.
    pub ty: Type,
    /// Inline annotations, declaration order.
    pub annotations: Vec<Annotation>,
    /// Name span (diagnostics only; excluded from semantic equality).
    pub span: Span,
}

/// Topic attribute key set (closed; `@`-prefixed in source).
/// REQ: REQ-TOOL-031.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttrKey {
    /// `@history`.
    History,
    /// `@pool_size`.
    PoolSize,
    /// `@overflow`.
    Overflow,
    /// `@encoding`.
    Encoding,
}

/// Topic attribute value.
/// REQ: REQ-TOOL-031.
#[derive(Debug, Clone, PartialEq)]
pub enum AttrValue {
    /// `@history: latest`.
    Latest,
    /// `@history: <depth>`.
    Depth(u32),
    /// `@pool_size: <n>`.
    Size(u32),
    /// `@overflow: overwrite`.
    Overwrite,
    /// `@overflow: reject`.
    Reject,
    /// `@encoding: {type: P, scale: N}`.
    Encoding(EncodingSpec),
}

/// Explicit quantization for constrained buses.
/// REQ: REQ-TOOL-033.
#[derive(Debug, Clone, PartialEq)]
pub struct EncodingSpec {
    /// Quantized storage prim.
    pub ty: Prim,
    /// Quantum; must be finite and `> 0`.
    pub scale: f64,
}

/// One topic attribute.
/// REQ: REQ-TOOL-031.
#[derive(Debug, Clone)]
pub struct Attribute {
    /// Attribute key.
    pub key: AttrKey,
    /// Attribute value.
    pub value: AttrValue,
    /// Key span (diagnostics only; excluded from semantic equality).
    pub span: Span,
}

/// Parsed topic file: attributes then fields; no path, no transport knowledge.
/// Semantic equality (I4): attribute order is canonical (fixed key order —
/// `render_canonical` reorders), field order is declaration order; spans are
/// excluded.
/// REQ: REQ-TOOL-032.
#[derive(Debug, Clone, Default)]
pub struct TopicFile {
    /// Topic attributes, declaration order.
    pub attrs: Vec<Attribute>,
    /// Field declarations, declaration order.
    pub fields: Vec<Field>,
}

impl PartialEq for TopicFile {
    fn eq(&self, other: &Self) -> bool {
        self.attrs.len() == other.attrs.len()
            && self.fields == other.fields
            && [
                AttrKey::History,
                AttrKey::PoolSize,
                AttrKey::Overflow,
                AttrKey::Encoding,
            ]
            .iter()
            .all(|k| {
                self.attrs.iter().find(|a| a.key == *k) == other.attrs.iter().find(|a| a.key == *k)
            })
    }
}

impl PartialEq for Attribute {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key && self.value == other.value
    }
}

impl PartialEq for Field {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name && self.ty == other.ty && self.annotations == other.annotations
    }
}

impl PartialEq for Annotation {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key && self.value == other.value
    }
}

/// Parses raw bytes: UTF-8 is validated here (I3), one optional leading BOM is
/// consumed silently, any further U+FEFF is `invalid-utf8`.
/// REQ: REQ-TOOL-052.
///
/// # Errors
/// Every defect as [`Diagnostic`]s with closed-vocabulary reasons and spans.
pub fn parse_bytes(src: &[u8]) -> Result<TopicFile, Vec<Diagnostic>> {
    let text = std::str::from_utf8(src).map_err(|e| vec![invalid_utf8_at(src, e.valid_up_to())])?;
    let text = strip_single_bom(text)?;
    parse_text(text)
}

/// Parses normalized source text (single entry after UTF-8/BOM handling):
/// lex, parse with line recovery, then local validation. `Err` carries every
/// defect found.
/// REQ: REQ-TOOL-017.
///
/// # Errors
/// Every defect as [`Diagnostic`]s with closed-vocabulary reasons and spans.
pub fn parse_text(text: &str) -> Result<TopicFile, Vec<Diagnostic>> {
    let mut diags = Vec::new();
    let toks = lex::tokenize(text, &mut diags);
    let mut p = P {
        toks,
        pos: 0,
        file: TopicFile::default(),
        fields_seen: false,
    };
    p.run(&mut diags);
    let file = p.file;
    if file.attrs.is_empty() && file.fields.is_empty() {
        diags.push(Diagnostic {
            span: Span {
                line: 1,
                col: 1,
                offset: 0,
            },
            reason: Reason::EmptyFile,
        });
    }
    diags.extend(validate(&file));
    if diags.is_empty() {
        Ok(file)
    } else {
        Err(diags)
    }
}

/// Local validation only — everything decidable from [`TopicFile`] alone:
/// duplicate fields, enum shape, attribute ranges, encoding scale.
/// REQ: REQ-TOOL-031, REQ-TOOL-033, REQ-TOOL-034.
#[must_use]
pub fn validate(f: &TopicFile) -> Vec<Diagnostic> {
    let mut d = Vec::new();
    let mut names = HashSet::new();
    for field in &f.fields {
        if !names.insert(field.name.as_str()) {
            d.push(Diagnostic {
                span: field.span,
                reason: Reason::DuplicateField,
            });
        }
        if let Type::Enum { variants } = &field.ty {
            if variants.is_empty() {
                d.push(Diagnostic {
                    span: field.span,
                    reason: Reason::EnumEmpty,
                });
                continue;
            }
            if variants.len() > 256 {
                d.push(Diagnostic {
                    span: field.span,
                    reason: Reason::EnumTooLarge,
                });
            }
            let mut seen = HashSet::new();
            for v in variants {
                if !seen.insert(v.as_str()) {
                    d.push(Diagnostic {
                        span: field.span,
                        reason: Reason::DuplicateEnumVariant,
                    });
                    break;
                }
            }
        }
    }
    for a in &f.attrs {
        match &a.value {
            AttrValue::Depth(0) | AttrValue::Size(0) => d.push(Diagnostic {
                span: a.span,
                reason: Reason::BadAttributeValue,
            }),
            AttrValue::Encoding(spec) if spec.scale <= 0.0 || !spec.scale.is_finite() => {
                d.push(Diagnostic {
                    span: a.span,
                    reason: Reason::EncodingInvalid,
                });
            }
            _ => {}
        }
    }
    d
}

fn invalid_utf8_at(src: &[u8], upto: usize) -> Diagnostic {
    let mut line = 1u32;
    let mut line_start = 0usize;
    for (i, b) in src[..upto].iter().enumerate() {
        if *b == b'\n' {
            line += 1;
            line_start = i + 1;
        }
    }
    Diagnostic {
        span: Span {
            line,
            col: u32::try_from(upto - line_start + 1).unwrap_or(1),
            offset: u32::try_from(upto).unwrap_or(u32::MAX),
        },
        reason: Reason::InvalidUtf8,
    }
}

fn strip_single_bom(text: &str) -> Result<&str, Vec<Diagnostic>> {
    let Some(stripped) = text.strip_prefix('\u{FEFF}') else {
        return Ok(text);
    };
    if let Some((i, _)) = stripped.char_indices().find(|&(_, c)| c == '\u{FEFF}') {
        return Err(vec![Diagnostic {
            span: Span {
                line: 1,
                col: 1,
                offset: u32::try_from(3 + i).unwrap_or(u32::MAX),
            },
            reason: Reason::InvalidUtf8,
        }]);
    }
    Ok(stripped)
}

fn push(diags: &mut Vec<Diagnostic>, span: Span, reason: Reason) {
    diags.push(Diagnostic { span, reason });
}

struct P {
    toks: Vec<TokSpanned>,
    pos: usize,
    file: TopicFile,
    fields_seen: bool,
}

impl P {
    fn peek(&self) -> &TokSpanned {
        &self.toks[self.pos.min(self.toks.len() - 1)]
    }

    fn next(&mut self) -> TokSpanned {
        let t = self.toks[self.pos.min(self.toks.len() - 1)].clone();
        if self.pos < self.toks.len() - 1 {
            self.pos += 1;
        }
        t
    }

    fn run(&mut self, diags: &mut Vec<Diagnostic>) {
        loop {
            match &self.peek().tok {
                Tok::Eof => break,
                Tok::Newline => {
                    self.next();
                }
                Tok::At => self.parse_attr(diags),
                Tok::Ident(_) => {
                    self.fields_seen = true;
                    self.parse_field(diags);
                }
                _ => {
                    let span = self.peek().span;
                    push(diags, span, Reason::TrailingGarbage);
                    self.skip_line();
                }
            }
        }
    }

    fn skip_line(&mut self) {
        loop {
            match &self.peek().tok {
                Tok::Newline => {
                    self.next();
                    return;
                }
                Tok::Eof => return,
                _ => {
                    self.next();
                }
            }
        }
    }

    fn expect_line_end(&mut self, diags: &mut Vec<Diagnostic>) {
        match &self.peek().tok {
            Tok::Newline => {
                self.next();
            }
            Tok::Eof => {}
            _ => {
                let span = self.peek().span;
                push(diags, span, Reason::TrailingGarbage);
                self.skip_line();
            }
        }
    }

    fn parse_attr(&mut self, diags: &mut Vec<Diagnostic>) {
        let at = self.next();
        let key_tok = self.next();
        let key_span = key_tok.span;
        let key = if let Tok::Ident(s) = &key_tok.tok {
            s.clone()
        } else {
            push(diags, key_span, Reason::MissingColon);
            self.skip_line();
            return;
        };
        let Some(attr_key) = (match key.as_str() {
            "history" => Some(AttrKey::History),
            "pool_size" => Some(AttrKey::PoolSize),
            "overflow" => Some(AttrKey::Overflow),
            "encoding" => Some(AttrKey::Encoding),
            _ => None,
        }) else {
            push(diags, key_span, Reason::UnknownAttribute);
            self.skip_line();
            return;
        };
        if self.file.attrs.iter().any(|a| a.key == attr_key) {
            push(diags, key_span, Reason::DuplicateAttribute);
            self.skip_line();
            return;
        }
        if self.fields_seen {
            push(diags, at.span, Reason::AttributeAfterField);
        }
        let colon = self.next();
        if colon.tok != Tok::Colon {
            push(diags, colon.span, Reason::MissingColon);
            self.skip_line();
            return;
        }
        let value = match attr_key {
            AttrKey::History => {
                let t = self.next();
                match t.tok {
                    Tok::Ident(s) if s == "latest" => AttrValue::Latest,
                    Tok::Int(n) => AttrValue::Depth(n),
                    _ => {
                        push(diags, t.span, Reason::BadAttributeValue);
                        self.skip_line();
                        return;
                    }
                }
            }
            AttrKey::PoolSize => {
                let t = self.next();
                if let Tok::Int(n) = t.tok {
                    AttrValue::Size(n)
                } else {
                    push(diags, t.span, Reason::BadAttributeValue);
                    self.skip_line();
                    return;
                }
            }
            AttrKey::Overflow => {
                let t = self.next();
                if let Tok::Ident(s) = &t.tok {
                    if s == "overwrite" {
                        AttrValue::Overwrite
                    } else if s == "reject" {
                        AttrValue::Reject
                    } else {
                        push(diags, t.span, Reason::BadAttributeValue);
                        self.skip_line();
                        return;
                    }
                } else {
                    push(diags, t.span, Reason::BadAttributeValue);
                    self.skip_line();
                    return;
                }
            }
            AttrKey::Encoding => match self.parse_encoding(diags) {
                Some(spec) => AttrValue::Encoding(spec),
                None => return,
            },
        };
        self.expect_line_end(diags);
        self.file.attrs.push(Attribute {
            key: attr_key,
            value,
            span: key_span,
        });
    }

    fn parse_encoding(&mut self, diags: &mut Vec<Diagnostic>) -> Option<EncodingSpec> {
        let fail = |diags: &mut Vec<Diagnostic>, span: Span| {
            push(diags, span, Reason::EncodingInvalid);
        };
        let brace = self.next();
        if brace.tok != Tok::LBrace {
            fail(diags, brace.span);
            self.skip_line();
            return None;
        }
        let ty_key = self.next();
        if !matches!(&ty_key.tok, Tok::Ident(s) if s == "type") {
            fail(diags, ty_key.span);
            self.skip_line();
            return None;
        }
        let c1 = self.next();
        if c1.tok != Tok::Colon {
            fail(diags, c1.span);
            self.skip_line();
            return None;
        }
        let prim_tok = self.next();
        let ty = if let Tok::Ident(s) | Tok::BareAnnotationToken(s) = &prim_tok.tok {
            Prim::from_name(s)
        } else {
            None
        };
        let Some(ty) = ty else {
            fail(diags, prim_tok.span);
            self.skip_line();
            return None;
        };
        let comma = self.next();
        if comma.tok != Tok::Comma {
            fail(diags, comma.span);
            self.skip_line();
            return None;
        }
        let scale_key = self.next();
        if !matches!(&scale_key.tok, Tok::Ident(s) if s == "scale") {
            fail(diags, scale_key.span);
            self.skip_line();
            return None;
        }
        let c2 = self.next();
        if c2.tok != Tok::Colon {
            fail(diags, c2.span);
            self.skip_line();
            return None;
        }
        let scale_tok = self.next();
        let scale = match scale_tok.tok {
            Tok::Number(v) => v,
            Tok::Int(n) => f64::from(n),
            _ => {
                fail(diags, scale_tok.span);
                self.skip_line();
                return None;
            }
        };
        let close = self.next();
        if close.tok != Tok::RBrace {
            fail(diags, close.span);
            self.skip_line();
            return None;
        }
        Some(EncodingSpec { ty, scale })
    }

    fn parse_field(&mut self, diags: &mut Vec<Diagnostic>) {
        let name_tok = self.next();
        let name_span = name_tok.span;
        let name = if let Tok::Ident(s) = &name_tok.tok {
            s.clone()
        } else {
            push(diags, name_span, Reason::UnknownType);
            self.skip_line();
            return;
        };
        let colon = self.next();
        if colon.tok != Tok::Colon {
            push(diags, colon.span, Reason::MissingColon);
            self.skip_line();
            return;
        }
        let Some(ty) = self.parse_type(diags) else {
            return;
        };
        let annotations = if self.peek().tok == Tok::LBrace {
            match self.parse_annotations(diags) {
                Some(a) => a,
                None => return,
            }
        } else {
            Vec::new()
        };
        self.expect_line_end(diags);
        self.file.fields.push(Field {
            name,
            ty,
            annotations,
            span: name_span,
        });
    }

    fn parse_type(&mut self, diags: &mut Vec<Diagnostic>) -> Option<Type> {
        let t = self.next();
        let Tok::Ident(s) = t.tok else {
            push(diags, t.span, Reason::UnknownType);
            self.skip_line();
            return None;
        };
        if s == "enum" {
            return self.parse_enum_body(diags, t.span);
        }
        if s == "string" || s == "bytes" {
            let ty = self.parse_sized(diags, &s, t.span)?;
            if self.peek().tok == Tok::LBracket {
                push(diags, self.peek().span, Reason::ArrayDepthExceeded);
                self.skip_line();
                return None;
            }
            return Some(ty);
        }
        let Some(prim) = Prim::from_name(&s) else {
            push(diags, t.span, Reason::UnknownType);
            self.skip_line();
            return None;
        };
        if self.peek().tok != Tok::LBracket {
            return Some(Type::Prim(prim));
        }
        self.next();
        let size_tok = self.next();
        let (bounded, n) = match size_tok.tok {
            Tok::Int(n) => (false, n),
            Tok::Leq => {
                let n_tok = self.next();
                if let Tok::Int(n) = n_tok.tok {
                    (true, n)
                } else if n_tok.tok == Tok::RBracket {
                    push(diags, size_tok.span, Reason::UnboundedArray);
                    self.skip_line();
                    return None;
                } else {
                    push(diags, n_tok.span, Reason::SizeOverflow);
                    self.skip_line();
                    return None;
                }
            }
            Tok::RBracket => {
                push(diags, size_tok.span, Reason::UnboundedArray);
                self.skip_line();
                return None;
            }
            _ => {
                push(diags, size_tok.span, Reason::SizeOverflow);
                self.skip_line();
                return None;
            }
        };
        let close = self.next();
        if close.tok != Tok::RBracket {
            push(diags, close.span, Reason::TrailingGarbage);
            self.skip_line();
            return None;
        }
        if self.peek().tok == Tok::LBracket {
            push(diags, self.peek().span, Reason::ArrayDepthExceeded);
            self.skip_line();
            return None;
        }
        Some(if bounded {
            Type::Bounded { elem: prim, max: n }
        } else {
            Type::FixedArray { elem: prim, n }
        })
    }

    /// `string[N]` / `bytes[<=N]`; bare or wrongly-bounded forms are diagnosed
    /// as the respective unbounded-* restriction.
    fn parse_sized(&mut self, diags: &mut Vec<Diagnostic>, s: &str, at: Span) -> Option<Type> {
        let unbounded = if s == "string" {
            Reason::UnboundedString
        } else {
            Reason::UnboundedBytes
        };
        let bracket = self.next();
        if bracket.tok != Tok::LBracket {
            push(diags, at, unbounded);
            self.skip_line();
            return None;
        }
        let want_leq = s == "bytes";
        let size_tok = self.next();
        let (leq, n) = match size_tok.tok {
            Tok::Leq => {
                let n_tok = self.next();
                if let Tok::Int(n) = n_tok.tok {
                    (true, n)
                } else if n_tok.tok == Tok::RBracket {
                    push(diags, size_tok.span, unbounded);
                    self.skip_line();
                    return None;
                } else {
                    push(diags, n_tok.span, Reason::SizeOverflow);
                    self.skip_line();
                    return None;
                }
            }
            Tok::Int(n) => (false, n),
            Tok::RBracket => {
                push(diags, size_tok.span, unbounded);
                self.skip_line();
                return None;
            }
            _ => {
                push(diags, size_tok.span, Reason::SizeOverflow);
                self.skip_line();
                return None;
            }
        };
        let close = self.next();
        if close.tok != Tok::RBracket {
            push(diags, close.span, unbounded);
            self.skip_line();
            return None;
        }
        if want_leq != leq {
            push(diags, size_tok.span, unbounded);
            self.skip_line();
            return None;
        }
        Some(if s == "string" {
            Type::String { max: n }
        } else {
            Type::Bytes { max: n }
        })
    }

    fn parse_enum_body(&mut self, diags: &mut Vec<Diagnostic>, at: Span) -> Option<Type> {
        let open = self.next();
        if open.tok != Tok::LBrace {
            push(diags, at, Reason::UnknownType);
            self.skip_line();
            return None;
        }
        let mut variants = Vec::new();
        loop {
            match self.next().tok {
                Tok::RBrace => return Some(Type::Enum { variants }),
                Tok::Newline | Tok::Eof => {
                    push(diags, open.span, Reason::UnterminatedEnum);
                    self.skip_line();
                    return None;
                }
                Tok::Comma => {}
                Tok::Ident(v) => variants.push(v),
                _ => {
                    push(diags, open.span, Reason::TrailingGarbage);
                    self.skip_line();
                    return None;
                }
            }
        }
    }

    fn parse_annotations(&mut self, diags: &mut Vec<Diagnostic>) -> Option<Vec<Annotation>> {
        let open = self.next();
        let mut out = Vec::new();
        loop {
            match self.next().tok {
                Tok::RBrace => return Some(out),
                Tok::Ident(key) => {
                    let key_span = self.toks[self.pos.saturating_sub(1)].span;
                    let colon = self.next();
                    if colon.tok != Tok::Colon {
                        push(diags, colon.span, Reason::AnnotationMalformed);
                        self.skip_line();
                        return None;
                    }
                    let value = self.parse_annot_value(diags, open.span)?;
                    out.push(Annotation {
                        key,
                        value,
                        span: key_span,
                    });
                    match self.next().tok {
                        Tok::Comma => {}
                        Tok::RBrace => return Some(out),
                        _ => {
                            push(diags, open.span, Reason::AnnotationMalformed);
                            self.skip_line();
                            return None;
                        }
                    }
                }
                _ => {
                    push(diags, open.span, Reason::AnnotationMalformed);
                    self.skip_line();
                    return None;
                }
            }
        }
    }

    fn parse_annot_value(&mut self, diags: &mut Vec<Diagnostic>, ctx: Span) -> Option<AnnotValue> {
        let t = self.next();
        match t.tok {
            Tok::LBracket => {
                let a = self.next();
                let av = match a.tok {
                    Tok::Number(v) => v,
                    Tok::Int(n) => f64::from(n),
                    _ => {
                        push(diags, t.span, Reason::AnnotationMalformed);
                        self.skip_line();
                        return None;
                    }
                };
                let comma = self.next();
                if comma.tok != Tok::Comma {
                    push(diags, t.span, Reason::AnnotationMalformed);
                    self.skip_line();
                    return None;
                }
                let b = self.next();
                let bv = match b.tok {
                    Tok::Number(v) => v,
                    Tok::Int(n) => f64::from(n),
                    _ => {
                        push(diags, t.span, Reason::AnnotationMalformed);
                        self.skip_line();
                        return None;
                    }
                };
                let close = self.next();
                if close.tok != Tok::RBracket {
                    push(diags, t.span, Reason::AnnotationMalformed);
                    self.skip_line();
                    return None;
                }
                Some(AnnotValue::Range(av, bv))
            }
            Tok::Number(v) => Some(AnnotValue::Number(v)),
            Tok::Int(n) => Some(AnnotValue::Number(f64::from(n))),
            Tok::QuotedString(s) => Some(AnnotValue::Text(s)),
            Tok::BareAnnotationToken(s) => Some(AnnotValue::Token(s)),
            _ => {
                push(diags, ctx, Reason::AnnotationMalformed);
                self.skip_line();
                None
            }
        }
    }
}
