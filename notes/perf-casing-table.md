# Unicode casing table compaction (2026-10-18)

For [#543](https://github.com/oxc-project/tsrs/issues/543), retain the shipped string-reference representation.
Compact offsets reduced binary size, but both checked string slices and checked byte slices regressed casing
workloads. The issue requires no performance regression. The generator now lives in `tasks/generate_stringutil`;
`--compact PATH` preserves the byte-slice candidate for further experiments without enabling it in the compiler.
Revisit only with evidence that the lookup and bounds-check cost can be removed without unchecked slicing.

Measurements: macOS arm64, Rust 1.99.1, baseline `34743839`, release profile (fat LTO, one codegen unit, no PGO),
five alternating before/after pairs. `/usr/bin/time -l` supplies wall time, instructions retired and peak RSS.
These are local screening measurements, not the deterministic Linux instruction gate or a published wall-time result.

## Representation and correctness

All 2,927 mappings, their 8,781 string fields and conditions match the original generated table. All Unicode scalar
inputs produce identical lower/upper output. Existing contextual sigma and WTF-8 unit cases pass. The pool has
8,517 bytes; records shrink from 64 to 16 bytes (187,328 to 46,832 bytes). Keys remain sorted and use binary search.

The generator validates scalar escapes, sorted unique keys, `u16` offsets, `u8` byte lengths, UTF-8 boundaries and
every pooled string's round trip. The compact candidate uses checked byte slicing and the existing `push_bytes`
helper; it adds no decompression, runtime cache, runtime allocation or unsafe block. The default generated file is
byte-identical to the previous Python generator's output except for its generator banner.

## Casing workloads

The probe compiled both versions of `js_case.rs` with their respective generated tables into one optimized binary,
using the same `tsrs_core` rune helpers. Each process selected one version, then ran 200,000 iterations of both
lowercase and uppercase conversion through `black_box`. Each input below was repeated 16 times:

- ASCII: `HelloWorld path/to/TypeScriptCompiler.ts`
- Unicode: `Straße İSPANYOL ﬁ Ω 𐐀 𞤀 hello`
- Sigma: `ΟΣ ΟΣΑ ʕΣ ͅΣ ΣͅA ⅠΣ`

The first candidate returned checked `&str` slices. Unicode instructions rose 9.24% (36.899 to 40.308 billion),
wall 2.93 to 3.27 s, and peak RSS 2,146,304 to 2,195,456 bytes. Sigma instructions rose 6.21%, wall 2.40 to 2.64 s,
and peak RSS 2,113,536 to 2,162,688 bytes. ASCII was unchanged within noise.

Returning checked byte slices removes the repeated UTF-8 boundary checks, but still regresses (medians):

| Input | Instructions before → after, billions | Change | Wall before → after, s | Peak RSS before → after, bytes |
| --- | ---: | ---: | ---: | ---: |
| ASCII | 3.38956 → 3.38948 | -0.002% | 0.12 → 0.12 | 2,080,768 → 2,080,768 |
| Unicode | 36.90075 → 37.91135 | +2.739% | 2.92 → 3.01 | 2,146,304 → 2,195,456 |
| Sigma | 30.90950 → 31.47264 | +1.822% | 2.40 → 2.45 | 2,129,920 → 2,162,688 |

The byte-slice candidate was rejected at this point; no whole-compiler performance claim is made for it.

## Whole-compiler screening of the first, string-slice candidate

The public Compiler and Compiler-Unions workloads use the `bench/projects.json` suite revision
`41f652ab2df5077b1115f73eaeefc2fe9f674132`. All measured diagnostics and exit statuses match byte for byte.
Single-threaded runs also set `RAYON_NUM_THREADS=1`; default runs leave the checker count unchanged.

| Project / mode | Instructions before → after, billions | Wall before → after, s | Peak RSS before → after, bytes |
| --- | ---: | ---: | ---: |
| Compiler / single | 1.72100 → 1.71865 | 0.14 → 0.14 | 60,129,280 → 59,932,672 |
| Compiler / default | 2.02817 → 2.02749 | 0.06 → 0.06 | 82,329,600 → 82,083,840 |
| Compiler-Unions / single | 3.21325 → 3.21216 | 0.22 → 0.22 | 62,865,408 → 62,668,800 |
| Compiler-Unions / default | 3.88574 → 3.88540 | 0.10 → 0.10 | 87,818,240 → 87,638,016 |

Stripped CLI size: 21,242,000 → 21,081,584 bytes (-160,416). Deterministic gzip level 9 (`mtime=0`):
8,354,936 → 8,353,125 bytes (-1,811). These sizes are for the string-slice candidate, not the later byte-slice probe.
The small whole-compiler movements do not compensate for the casing regressions.

## Reproduction and checks

```sh
cargo run -p tsrs_generate_stringutil -- --check
cargo run -p tsrs_generate_stringutil -- --compact target/compact_casing.rs
cargo test -p tsrs_generate_stringutil -p tsrs_core
cargo clippy --all-targets --config 'build.warnings="deny"'
tools/lint/source.py
```

To integrate a compact experiment, use its generated table and replace the three `builder.push_str(mapping.*)`
calls in `js_case.rs` with `push_bytes(&mut builder, mapping.*())`. Preserve the original binary first and build
each compiler with `cargo build --release -p tsrs_cli`. Measure both with:

```sh
/usr/bin/time -l <binary> -p <suite>/cases/solutions/Compiler --noEmit --incremental false --pretty false
RAYON_NUM_THREADS=1 /usr/bin/time -l <binary> -p <suite>/cases/solutions/Compiler --noEmit --incremental false --pretty false --singleThreaded
```

Repeat for Compiler-Unions, alternating the two binaries. The experimental compiler passed 131 core unit cases
(one ignored) and matched the existing conformance baseline: 13,458 passes, two code-only differences, two failures,
no crashes or timeouts. All generators ran successfully; the new task checks freshness without rewriting files.
