# Toolchain Diagnostics (v1)

Closed vocabulary shared toolchain-wide (source of truth:
[`docs/algorithms/toolchain/schema-parser.md`](../algorithms/toolchain/schema-parser.md)
§Diagnostics Vocabulary). Renderers never invent strings.

Message format:

```text
<file>:<line>:<col>: error: <reason>[: <detail>]
```

| Reason slug | Emitted by | Meaning |
|---|---|---|
| `unknown-type` | parser | type not among the 11 prims / `string` / `bytes` / `enum` |
| `unbounded-string` | parser | `string` without `[N]` capacity |
| `unbounded-bytes` | parser | `bytes` without `[<=N]` bound |
| `unbounded-array` | parser | array without a length (`T[]`, `T[<=]`) |
| `bad-attribute-value` | parser/validate | attribute value outside its closed set (`@history` depth 0, `@pool_size` 0, unknown words) |
| `unknown-attribute` | parser | `@`-attribute outside the four-key set |
| `duplicate-attribute` | parser | same `@`-attribute repeated |
| `attribute-after-field` | parser | `@`-attribute after the first field |
| `duplicate-field` | validate | field name declared twice |
| `duplicate-enum-variant` | validate | enum variant repeated |
| `enum-too-large` | validate | enum with > 256 variants |
| `enum-empty` | validate | enum with zero variants |
| `array-depth-exceeded` | parser | nesting deeper than one level, or array over non-prim |
| `size-overflow` | lexer/parser | numeric size outside the u32 grammar range |
| `missing-colon` | parser | `:` missing where the grammar requires one |
| `unterminated-enum` | parser | enum body never closed before line end |
| `trailing-garbage` | lexer/parser | tokens that cannot start any production |
| `empty-file` | parser | file with neither attributes nor fields |
| `invalid-utf8` | parser | source bytes not valid UTF-8, or stray U+FEFF |
| `annotation-malformed` | lexer/parser | annotation entry does not match the grammar |
| `encoding-invalid` | parser/validate | `@encoding` form malformed or `scale <= 0` |
| `reserved-field-name` | (later layer) | joins the set when its owner lands |
| `name-invalid` | workspace-model builder | topic-path defects (parser never sees the path) |
