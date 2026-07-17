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

Status 2026-07-16: 59 curated + 8,000 fuzz cases (4 seeds), zero divergence —
node 0.4.0 (`24f5a76`) vs cargo 0.2.0 working tree. This harness is the seed
of the `gherkin-x-test` conformance-corpus extraction (bdd-v2-plan §4): when
that repo exists, the curated corpus in `corpus.js` becomes its first accept/
reject cases and this directory shrinks to an adapter.

Known AST asymmetries (not dialect divergences — the dump format omits them):
node 0.4.0 carries `ParsedFeature.outlines` ({name, line, rows}, powering
`lintFeature`'s single-row-outline rule) and `ParsedFeature.file`; this crate
does not yet. Mirror both when porting the linter role here.
