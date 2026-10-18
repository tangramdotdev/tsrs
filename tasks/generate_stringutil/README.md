# Unicode casing table generator

Run from the repository with the pinned TypeScript checkout in `ts-ref/`:

```sh
cargo run -p tsrs_generate_stringutil -- --check
cargo run -p tsrs_generate_stringutil -- --write
cargo run -p tsrs_generate_stringutil -- --compact target/compact_casing.rs
```

The default is `--check`, which reports stale output without changing it. `--write` regenerates and formats
`crates/tsrs_core/src/stringutil/js_case_generated.rs`. `tools/gen-check.sh` runs this task with the other generators.

The source is `ts-ref/tsc/internal/stringutil/js_case_generated.go`, including its Unicode 15.1 casing properties
and final-sigma condition. Generation rejects malformed or unsorted mappings and invalid Unicode scalars.

`--compact PATH` writes the experimental representation proposed in [#543](https://github.com/oxc-project/tsrs/issues/543)
to the specified file. Each 16-byte record contains three `u16` byte offsets and three `u8` byte lengths into a
deduplicated UTF-8 pool, plus the condition. Generation rejects overflowing offsets or lengths and verifies every
string's round trip through the pool. The candidate accessors return checked byte slices; a compiler experiment
must append them with `js_case.rs`'s existing `push_bytes` helper in place of the three `builder.push_str` calls.

The default output retains the current string-reference layout: the compact candidate reduced binary size but
regressed non-ASCII casing in local measurements. See [the measurements](../../notes/perf-casing-table.md) before
revisiting it. No decompression, runtime cache or additional runtime allocation is involved.
