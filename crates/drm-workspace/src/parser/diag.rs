//! Parser diagnostics: closed vocabulary, spans, and message format, verbatim
//! from `docs/algorithms/toolchain/schema-parser.md` §Diagnostics Vocabulary and
//! §Data Structures (`Span`). The renderer never invents strings — every
//! diagnostic names a [`Reason`] variant.
//! REQ: REQ-TOOL-017 (invalid schema fails with location + reason).

use std::fmt;

/// Source location of a defect: 1-based line/column into the normalized text
/// (`\r\n` → `\n`, one optional leading BOM consumed) plus the byte offset into
/// that normalized text.
/// REQ: REQ-TOOL-017.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    /// 1-based line.
    pub line: u32,
    /// 1-based column (Unicode scalar counts, matching the rendered text).
    pub col: u32,
    /// Byte offset into the normalized source.
    pub offset: u32,
}

/// Closed diagnostic vocabulary (schema-parser.md). Variants owned by later
/// layers (`reserved-field-name`, `name-invalid`) join when their owners land.
/// REQ: REQ-TOOL-017.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// Type is not one of the 11 prims, `string`, `bytes`, or `enum`.
    UnknownType,
    /// `string` without a `[N]` capacity.
    UnboundedString,
    /// `bytes` without a `[<=N]` bound.
    UnboundedBytes,
    /// Array without an exact length (`T[]`).
    UnboundedArray,
    /// Attribute value outside the grammar's closed value set.
    BadAttributeValue,
    /// `@`-attribute not in the four-key set.
    UnknownAttribute,
    /// Same `@`-attribute key repeated.
    DuplicateAttribute,
    /// `@`-attribute appearing after the first field.
    AttributeAfterField,
    /// Field name declared twice.
    DuplicateField,
    /// Enum variant repeated.
    DuplicateEnumVariant,
    /// Enum with more than 256 variants.
    EnumTooLarge,
    /// Enum with zero variants.
    EnumEmpty,
    /// Array nesting deeper than one level or array over a non-prim.
    ArrayDepthExceeded,
    /// Numeric size outside the u32 grammar range.
    SizeOverflow,
    /// `:` missing where the grammar requires one.
    MissingColon,
    /// Enum body never closed before line end.
    UnterminatedEnum,
    /// Tokens that cannot start any production.
    TrailingGarbage,
    /// File with neither attributes nor fields.
    EmptyFile,
    /// Source bytes are not valid UTF-8 (or a stray U+FEFF).
    InvalidUtf8,
    /// Annotation entry does not match `annot-entry`/`annot-value`.
    AnnotationMalformed,
    /// `@encoding` form malformed or `scale <= 0`.
    EncodingInvalid,
}

impl Reason {
    /// Vocabulary slug used in rendered messages.
    /// REQ: REQ-TOOL-017.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UnknownType => "unknown-type",
            Self::UnboundedString => "unbounded-string",
            Self::UnboundedBytes => "unbounded-bytes",
            Self::UnboundedArray => "unbounded-array",
            Self::BadAttributeValue => "bad-attribute-value",
            Self::UnknownAttribute => "unknown-attribute",
            Self::DuplicateAttribute => "duplicate-attribute",
            Self::AttributeAfterField => "attribute-after-field",
            Self::DuplicateField => "duplicate-field",
            Self::DuplicateEnumVariant => "duplicate-enum-variant",
            Self::EnumTooLarge => "enum-too-large",
            Self::EnumEmpty => "enum-empty",
            Self::ArrayDepthExceeded => "array-depth-exceeded",
            Self::SizeOverflow => "size-overflow",
            Self::MissingColon => "missing-colon",
            Self::UnterminatedEnum => "unterminated-enum",
            Self::TrailingGarbage => "trailing-garbage",
            Self::EmptyFile => "empty-file",
            Self::InvalidUtf8 => "invalid-utf8",
            Self::AnnotationMalformed => "annotation-malformed",
            Self::EncodingInvalid => "encoding-invalid",
        }
    }
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One parse/validation failure: span + closed-vocabulary reason.
/// REQ: REQ-TOOL-017.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Diagnostic {
    /// Defect location.
    pub span: Span,
    /// Closed-vocabulary reason.
    pub reason: Reason,
}

impl Diagnostic {
    /// Renders the toolchain message format
    /// `<file>:<line>:<col>: error: <reason>[: <detail>]`.
    /// REQ: REQ-TOOL-017.
    #[must_use]
    pub fn format(&self, file: &str) -> String {
        format!(
            "{file}:{}:{}: error: {}",
            self.span.line, self.span.col, self.reason
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{Diagnostic, Reason, Span};

    /// REQ: REQ-TOOL-017 — message format `<file>:<line>:<col>: error: <reason>`.
    #[test]
    fn message_format() {
        let d = Diagnostic {
            span: Span {
                line: 3,
                col: 7,
                offset: 40,
            },
            reason: Reason::MissingColon,
        };
        assert_eq!(d.format("a.drm"), "a.drm:3:7: error: missing-colon");
        assert_eq!(Reason::UnknownType.as_str(), "unknown-type");
    }
}
