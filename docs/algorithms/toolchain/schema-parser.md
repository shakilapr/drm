# Schema Parser Algorithms

Scope: lexing, parsing, validation and canonical rendering of `.drm` topic-definition files. Topic attributes use an `@` sigil so any ordinary identifier remains a legal field name. Implements architecture.md §Topic Definitions (grammar, structural bounds, enums, topic attributes). Owner: E2 (crate `drm-workspace::parser`, `drm-workspace::render`). Gate criteria map 1:1 to invariants I1–I4 below.

## REQ Bindings

| Provisional slug | Architecture anchor | Numeric ID |
|---|---|---|
| `REQ-TOOL-grammar-ebnf` | §Topic Definitions — file path defines topic name | `REQ-TOOL-032` |
| `REQ-TOOL-grammar-types` | §Structural Bounds — prims, `T[N]`, `T[<=N]`, `string[N]`, `bytes[<=N]`, enums | `REQ-TOOL-034` |
| `REQ-TOOL-grammar-attrs` | §Topic attributes — `@history`, `@pool_size`, `@overflow`; attrs precede fields | `REQ-TOOL-031` |
| `REQ-TOOL-grammar-encoding` | §Structural Bounds — explicit `@encoding` quantization for constrained buses | `REQ-TOOL-033` |
| `REQ-TOOL-parse-bytes` | parser entry takes raw bytes and validates UTF-8 internally | `REQ-TOOL-052` |
| `REQ-TOOL-render-semantic` | canonical render preserves the full semantic AST (incl. annotations) | `REQ-TOOL-055` |
| `REQ-TOOL-diag-location-reason` | Phase 2 gate — invalid schema fails with location + reason | `REQ-TOOL-017` |
| `REQ-TOOL-parser-no-panic` | Phase 2 gate — fuzz input never crashes/hangs | `REQ-TOOL-053` |
| `REQ-TOOL-render-deterministic` | Phase 2 gate — parser round-trip deterministic | `REQ-TOOL-054` |
| `REQ-TOOL-topic-path-ascii` | canonical topic paths are lowercase ASCII (cross-platform identity) | `REQ-TOOL-056` |

## Grammar (EBNF, normative for parser and renderer)

The canonical `.drm` grammar is single-line and indentation-insensitive. Topic attributes are
`@`-prefixed, must precede the first field, and may not be repeated or unknown. Field annotations
use inline braces. Topic paths are lowercase ASCII (`[a-z0-9_]+` segments joined by `/`), so
Linux (case-sensitive) and Windows (case-insensitive) filesystems resolve the same identity.

```ebnf
file := topic-attr* field* EOF
topic-attr := '@' attr-name ':' attr-value NEWLINE
attr-name := 'history' | 'pool_size' | 'overflow' | 'encoding'
attr-value := 'latest' | int | 'overwrite' | 'reject' | encoding-spec
encoding-spec := '{' 'type' ':' prim ',' 'scale' ':' number '}'
field := ident ':' type annotations? NEWLINE
type := prim
 | 'string' '[' int ']'
 | 'bytes' '[' '<=' int ']'
 | prim '[' int ']' ; fixed array of prim (depth ≤ 1)
 | prim '[' '<=' int ']' ; bounded variable array of prim (depth ≤ 1)
 | 'enum' '{' variant (',' variant)* '}'
variant := ident
ident := [a-zA-Z_][a-zA-Z0-9_]*
int := [0-9]+ ; u32 range; overflow ⇒ error
number := ['-'] int ['.' int] ; finite; NaN/inf rejected
annotations := '{' annot-entry (',' annot-entry)* '}' ; field semantic metadata — preserved in AST
annot-entry := ident ':' annot-value
annot-value := number | annot-token | '[' number ',' number ']' | '"' [^"]* '"'
annot-token := [^,}]+ ; bare token, e.g. unit `m/s`
```

Restrictions (rejected with dedicated diagnostics): unbounded `string`/`bytes`/arrays on the data plane; nesting depth > 1; enum > 256 variants; unknown/duplicate `@` attributes; attributes after the first field; empty enum; `scale <= 0`. Two further restrictions are enforced in their owning layer, not the parser: topic-path lowercase ASCII (workspace-model builder — the parser never sees the path) and `@encoding` only on constrained-bus topics (verify-engine — transport applicability is a route property).

## Data Structures

```rust
pub struct Span { pub line: u32, pub col: u32, pub offset: u32 } // into source bytes

pub enum Tok {
 Ident(String), Int(u32),
 Number(f64), // '-3.14', '0.1' — signed decimal (grammar `number`); finite; NaN/inf rejected
 QuotedString(String), // annotation text: "display name"
 BareAnnotationToken(String), // annotation bare token: unit `m/s` — lexed in annotation-value mode until ',' or '}'
 At, Colon, LBracket, RBracket, LBrace, RBrace, Leq, Comma, Newline, Eof,
}
// Lexer: two modes. Normal mode: idents, ints, punctuation. Annotation-value mode (after
// `ident ':'` inside `{...}`): '[' number ',' number ']' (range) | number | quoted string |
// bare token ([^,}]+, trimmed) — matches grammar `annot-value` exactly, including the bracketed
// range whose interior comma is not an entry separator. No grammar production lacks a lexical form.

pub struct TokSpanned { pub tok: Tok, pub span: Span }

pub struct TopicFile { pub attrs: Vec<Attribute>, pub fields: Vec<Field> } // no path, no transport knowledge

pub struct Attribute { pub key: AttrKey, pub value: AttrValue, pub span: Span }
pub enum AttrKey { History, PoolSize, Overflow, Encoding } // @-prefixed in source
pub enum AttrValue { Latest, Depth(u32), Size(u32), Overwrite, Reject, Encoding(EncodingSpec) }
pub struct EncodingSpec { pub ty: Prim, pub scale: f64 } // explicit quantization (constrained bus)

pub struct Field { pub name: String, pub ty: Type, pub annotations: Vec<Annotation>, pub span: Span }
pub struct Annotation { pub key: String, pub value: AnnotValue, pub span: Span }
pub enum AnnotValue { Number(f64), Token(String), Range(f64, f64), Text(String) }

pub enum Type {
 Prim(Prim), // 11 canonical primitives
 String { max: u32 }, // string[N]
 Bytes { max: u32 }, // bytes[<=N]
 FixedArray { elem: Prim, n: u32 }, // T[N]
 Bounded { elem: Prim, max: u32 }, // T[<=N]
 Enum { variants: Vec<String> },
}
pub enum Prim { F32, F64, I8, I16, I32, I64, U8, U16, U32, U64, Bool } // 11 incl. f64
```

## Invariants

- **I1 ValidParse**: every `tests/fixtures/schema/valid/*.drm` parses to `Ok(TopicFile)`. Test: `parser_corpus_valid`.
- **I2 InvalidDiagnosed**: every invalid fixture fails with `Diagnostic { span, reason }` where `reason ∈ DIAG_VOCABULARY` and span points at the defect. Test: `parser_corpus_invalid`.
- **I3 NoPanic**: any byte input (fuzz corpus + arbitrary bytes, incl. invalid UTF-8) yields `Ok` or `Err` — never panic, never hang (libFuzzer `-timeout=5`). Test: `tests/fuzz/schema` (raw-byte corpus).
- **I4 SemanticRoundTrip**: `parse_bytes(render_canonical(parse_bytes(x))) == parse_bytes(x)` — **semantic AST equality**, not byte-for-byte source preservation. Canonical render is idempotent and preserves attributes, annotations, and field order. Test: `roundtrip` (proptest, ≥ 10 000 cases + 30 fixed fixtures).

## Diagnostics Vocabulary (closed set; renderer never invents strings)

`unknown-type · unbounded-string · unbounded-bytes · unbounded-array · bad-attribute-value ·
unknown-attribute · duplicate-attribute · attribute-after-field · reserved-field-name ·
duplicate-field · duplicate-enum-variant · enum-too-large · enum-empty · array-depth-exceeded ·
size-overflow · name-invalid · missing-colon · unterminated-enum · trailing-garbage · empty-file ·
invalid-utf8 · annotation-malformed · encoding-invalid`

(`name-invalid` is emitted by the workspace-model builder for path defects — closed vocabulary is shared toolchain-wide.)

Message format: `<file>:<line>:<col>: error: <reason>[: <detail>]`

## Functions

```rust
fn parse_bytes(src: &[u8]) -> Result<TopicFile, Vec<Diagnostic>> {
 let text = std::str::from_utf8(src)
 .map_err(|e| vec![Diagnostic::invalid_utf8(e.valid_up_to())])?; // I3: UTF-8 checked here; Vec per signature
 parse_text(text)
}
// lex: Normalizes \r\n → \n; skips '#' comments but preserves newline tokens; emits `At` for '@';
// switches to annotation-value mode inside field annotations (see Tok).
// parse_text: recursive descent. On error record Diagnostic, skip to next Newline, continue —
// one file yields ALL its errors (error recovery).
// Attribute pass first: '@'-prefixed attrs must precede fields (attribute-after-field otherwise);
// unknown attr ⇒ unknown-attribute; repeated attr ⇒ duplicate-attribute.
// Type parsing enforces: arrays only over prim (array-depth-exceeded); 'string'/'bytes' require
// '[' int ']' / '[<= int]' (bare ⇒ unbounded-string/bytes); int > u32::MAX ⇒ size-overflow.

fn validate(f: &TopicFile) -> Result<(), Vec<Diagnostic>>
// LOCAL checks only — everything decidable from TopicFile alone:
// duplicate-field (name set); duplicate-enum-variant; enum-empty; enum > 256 variants
// (enum-too-large); @history ∈ {latest, 1..=u32}; @pool_size ≥ 1; @overflow ∈ {overwrite, reject};
// @encoding: known encoding form, scale > 0 (encoding-invalid).
// NOT here (layering — TopicFile has neither path nor transport knowledge):
//   topic-path lowercase ASCII ⇒ workspace-model builder (path fixed at file discovery; REQ-TOOL-056 enforced there)
//   @encoding only on constrained-bus topics ⇒ verify-engine V-RT family (transport applicability is a route property)

fn render_canonical(f: &TopicFile) -> String
// Deterministic: attrs first in fixed order (History, PoolSize, Overflow, Encoding), then fields in
// declaration order, each with its annotations in declaration order (numbers rendered canonically
// from the stored f64 — idempotent normalization, e.g. `0.10` → `0.1`; quoted/bare annotation
// values re-emitted verbatim). render_canonical(parse(x)) is a
// pure function of the AST ⇒ I4 semantic round-trip holds.
```

## Protocols

*Parser round-trip (determinism + semantic preservation, exercised by proptest):*

1. `t = parse_bytes(x)` (reject invalid).
2. `s1 = render_canonical(t)`.
3. `t2 = parse_bytes(s1)`; assert `t2 == t` (semantic AST equality, including annotations).
4. `s2 = render_canonical(t2)`; assert `s1 == s2` byte-identical (render idempotent).

## Edge Cases & Errors

| Input | Behavior |
|---|---|
| Empty file | `empty-file` diagnostic (a topic must declare ≥ 1 field or ≥ 1 attr) |
| Non-UTF8 bytes | `invalid-utf8` at first offending byte (detected in `parse_bytes`) |
| BOM | consumed silently if exactly one UTF-8 BOM at start; else `invalid-utf8` |
| CRLF | normalized to LF before lexing (I4 stability) |
| Int overflow | `size-overflow`, parse continues via recovery |
| Attribute after first field | `attribute-after-field` |
| Duplicate / unknown `@attr` | `duplicate-attribute` / `unknown-attribute` |
| Field named `history` | allowed — attrs use `@history`; no reserved field names collide |
| `@encoding` on a non-constrained-bus topic | valid parse; `verify` fails (transport applicability — route property, not local syntax) |
| `@encoding` malformed / `scale <= 0` | `encoding-invalid` at parse |
| Uppercase path segment | workspace-model builder error (`name-invalid`; parser has no path — lowercase ASCII topic identity) |
| 10k-char identifier / 10k fields | accepted within memory bound; fuzz seeds cover |

All parse/validation errors are `Err(Vec<Diagnostic>)` — no panic paths. Mapping to `drm-types::Status`: parser layer is toolchain-internal; only surface CLI exit codes (non-zero on any Diagnostic).

## Test Matrix

| Case | Expected |
|---|---|
| `001_valid_prims.drm` … (30 fixtures) | `Ok`, spans correct |
| `031_invalid_missing_colon.drm` … (30 fixtures) | `Err` with documented reason + span |
| attribute fixtures (`@history`, dup, unknown, after-field) | exact diagnostics |
| annotation fixtures (`unit: m/s` bare token, `range: [-3.14, 3.14]` negative/decimal numbers, `description: "display name"` quoted text) | exact lexemes; preserved through canonical round-trip (I4) |
| encoding fixture (`@encoding: {type: i16, scale: 0.01}`) | parses; scale preserved |
| raw-byte fuzz corpus incl. invalid UTF-8 | no panic/hang; valid-prefix accepted (I3) |
| proptest round-trip ≥ 10 000 | I4 semantic equality holds |
| blind spot-check ×10 (test lead) | messages name actual defect |
