# Differential parity harness — gherkin-cargo-test vs gherkin-node-test

The dialect is an implicit spec duplicated across two parsers. Hand-ported
test suites can drift in tandem with the implementation they check; this
harness can't: it runs the SAME `.feature` corpus through both parsers and
compares canonical AST dumps **byte-for-byte**. Disagreement is the finding —
no case carries a hand-written expectation.

The rust side is `examples/dump.rs`; the node side is `dump-node.js` (same
format, documented in dump.rs). Point `GNT_PATH` at a gherkin-node-test
checkout (defaults to `~/Documents/gherkin-node-test/index.js`).

```sh
cargo build --example dump          # once, from the repo root
node tools/parity/run-parity.js     # curated corpus: grammar + every rejection-matrix row
node tools/parity/fuzz.js 2000 999  # hostile fuzz (reject-heavy), deterministic by seed
node tools/parity/fuzz-valid.js 2000 715  # valid-biased fuzz (accept-path: expansion, escapes, tags)
```

Every run is reproducible: the fuzzers use a seeded xorshift PRNG, and any
divergence prints both dumps plus the offending generated file (kept in
`fuzz-out/` / `fuzz-valid-out/`).

Both dumpers also take `--lint`: the finding stream (`FINDING rule severity
line message`) is compared the same way, so `lint_feature` here and
`lintFeature` in node are held to IDENTICAL finding text — rules, lines, and
message wording. The fuzzers compare both streams per generated file.

Status 2026-07-16: AST parity — 59 curated + 8,000 fuzz cases (4 seeds), zero
divergence, node 0.4.0 (`24f5a76`) vs cargo 0.2.0 (`6c14113`). Lint parity —
64 curated cases (128 case-modes, including the banned-word matrix and the
Unicode-folding hostiles) + 6,000 fuzz cases with thousands of non-empty
finding streams, zero divergence, node 0.4.0 vs cargo 0.4.0. The sibling
versions now track the shared dialect+linter surface in lockstep — pinning
the same version on both sides pins one de-facto dialect+lint version. This
harness is the seed of the `gherkin-x-test`
conformance-corpus extraction (bdd-v2-plan §4): when that repo exists, the
curated corpus in `corpus.js` becomes its first accept/reject cases and this
directory shrinks to an adapter.

Status 2026-07-22 (0.5.0): both dumps gained `NARRATIVE <line> <in_body>
<text>` records — the parser-side narrative capture is itself part of the
parity surface now, since `near-miss-keyword` reads findings off it. Fuzzer
pools gained wrong-case step keywords and wrong-form construct headers (plus
quiet lookalikes: plurals, `rule:`, `example:`, glued `scenarioutline:`).
128 curated case-modes + 8,000 fuzz cases (2 seeds per fuzzer), zero
divergence, 919 fuzz files producing 1,227 byte-identical near-miss findings,
node 0.5.0 (`576f974`) vs cargo 0.5.0.

Remaining AST asymmetry (not a dialect divergence — the dump format omits
it): node carries `ParsedFeature.file`; this crate does not. `outlines`
landed here with the linter port, `narrative` with 0.5.0.

Status 2026-07-23 (0.6.0): both dumps gained OUTLINE / OHEADER /
OPLACEHOLDERS records — OutlineMeta's new header, header_line, and
placeholders fields (the parse data unused-column reads) are byte-compared
per outline. The fuzz-valid generator now deliberately produces the 0.6.0
shapes: reused titles (duplicate-title, including the cross-construct case),
a zero-scenario variant with an optional near-miss construct line (the
no-scenarios dialect error, hint text included), and a plain scenario
colliding with an outline row's expanded name (the post-expansion backstop).
Run: 144 curated case-modes + 16,000 fuzz cases (4 reject-heavy + 4
valid-biased seeds), zero divergence, node 0.6.0 vs cargo 0.6.0. One
3,000-case sweep of the valid fuzzer alone carried 964 unused-column, 578
duplicate-title (301 via the backstop), 236 no-scenarios (87 with the
near-miss hint) findings — all identical both sides.

Known accepted asymmetry (recorded 2026-07-23, class pre-dates 0.5.0): the
parsers' LINE TRIMS differ on JS-vs-Rust whitespace — Rust's char-level trim
strips NEL (U+0085) and keeps U+FEFF; JS's String.trim does the reverse. A
NEL- or BOM-edged line can therefore parse as a step on one side and
narrative on the other, and from 0.6.0 the no-scenarios dialect message can
differ in hint presence or quoted feature title on such files. The js_ws
pinning covers the near-miss scans only, deliberately: re-pinning the trim
itself would touch every line of every parse for a character class no real
corpus contains. Neither fuzzer alphabet includes NEL/U+FEFF, so parity runs
stay green; this note is the honest record that the class exists.
