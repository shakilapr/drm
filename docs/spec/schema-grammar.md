# .drm Schema Grammar (v1)

Normative EBNF lives in [`docs/algorithms/toolchain/schema-parser.md`](../algorithms/toolchain/schema-parser.md) §Grammar — this spec restates it for toolchain implementers and pins the diagnostics contract. Source of truth: the algorithm doc; divergence here is a bug.

## Production summary

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
 | prim '[' int ']'          ; fixed array of prim (depth ≤ 1)
 | prim '[' '<=' int ']'     ; bounded variable array of prim (depth ≤ 1)
 | 'enum' '{' variant (',' variant)* '}'
variant := ident
ident := [a-zA-Z_][a-zA-Z0-9_]*
int := [0-9]+                ; u32 range; overflow ⇒ error
number := ['-'] int ['.' int] ; finite; NaN/inf rejected
annotations := '{' annot-entry (',' annot-entry)* '}'
annot-entry := ident ':' annot-value
annot-value := number | annot-token | '[' number ',' number ']' | '"' [^"]* '"'
annot-token := [^,}]+        ; bare token, e.g. unit `m/s`
```

## Restrictions (dedicated diagnostics)

- Unbounded `string` / `bytes` / arrays are rejected on the data plane.
- Nesting depth > 1 (`array-depth-exceeded`); arrays only over prims.
- Enum: > 256 variants (`enum-too-large`), empty (`enum-empty`).
- `@` attributes: unknown, duplicate, or after the first field are rejected.
- `scale <= 0` (`encoding-invalid`).
- Enforced in their owning layers, not the parser: topic-path lowercase ASCII
  (workspace-model builder, REQ-TOOL-056) and `@encoding` only on
  constrained-bus topics (verify engine — transport applicability is a route
  property).

## Lexer contract

- Input is raw bytes; UTF-8 is validated inside `parse_bytes` (`invalid-utf8`).
- One optional leading UTF-8 BOM is consumed silently; any further U+FEFF is
  `invalid-utf8`.
- `\r\n` is normalized to `\n` before lexing; `#` comments run to end of line
  and the newline itself is preserved.
- Two modes: normal, and annotation-value (entered at a `{` that does not
  follow `enum`; enum bodies stay normal). Annotation-value mode additionally
  admits numbers, quoted strings, and bare tokens (`[^,}]+`, trimmed); an
  identifier followed by `:` is the next annotation key.
- Spans are 1-based line/column into the normalized text plus byte offset.

## Diagnostics

Closed vocabulary and message format:
[`docs/spec/DIAGNOSTICS.md`](DIAGNOSTICS.md). One file yields ALL its errors:
on any defect the parser records the diagnostic, skips to the next newline,
and continues. No panic paths on arbitrary bytes (REQ-TOOL-053).

## Corpus

`tests/fixtures/schema/{valid,invalid}/` — 30 + 30 files named
`NNN_{valid,invalid}_<desc>.drm` with unique `NNN`; invalid files carry a
`# reason: <slug>` first line (exception: the empty-file fixture).
Validation: `cargo test -p drm-workspace --test fixtures_valid` (hygiene) and
`cargo test -p drm-workspace --test parser_corpus` (I1/I2/I4).
