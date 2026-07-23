# gherkin-cargo-test

**The smallest honest Gherkin runner for Rust.** Two boring dependencies, no
proc macros, no async, no framework — it turns `.feature` files into real
`cargo test` tests (one per scenario, via [libtest-mimic]), and it treats
every silence as a bug. One file, ~1,400 lines, small enough to read in one
sitting or to vendor outright. The same file doubles as a **feature-file
linter** for projects whose runner is something else — see
[the linter role](#the-linter-role--under-someone-elses-runner).

This is the Rust sibling of [gherkin-node-test] — same author, same grammar,
same philosophy, ported guard-for-guard. The feature files are portable
between the two: Gherkin is the language-neutral control layer.

[libtest-mimic]: https://crates.io/crates/libtest-mimic
[gherkin-node-test]: https://github.com/bingh0/gherkin-node-test

```toml
[dev-dependencies]
gherkin-cargo-test = "0.4"   # or just vendor src/lib.rs; it's one file

[[test]]
name = "features"
harness = false
```

## Why another BDD tool

There is an excellent full Cucumber implementation for Rust —
[cucumber-rs](https://github.com/cucumber-rs/cucumber) — and if you want the
full standard, tag expressions, and living-documentation output, use it.

This crate exists for a different reason. It came out of an experiment in
**agent-driven development with strict BDD**: a workflow where a human writes
and owns the Gherkin feature files, and coding agents write essentially all of
the implementation. In that workflow the feature files aren't documentation —
they're the **control layer**. They're the one artifact the human actually
reads, audits, and carries between implementations (including across
*languages*: this crate parses its JS sibling's feature corpus unchanged).
Everything underneath is regenerable.

That inverts what matters in a test harness. When no human reads every line of
the code, the harness is the only witness — and the failure mode that kills
you is not a crash. It's a **false green**: a suite that says "all your
acceptance criteria hold" when some of them were never checked. Crashes get
fixed; silences compound.

False greens have specific, boring causes. Each one is a design decision here:

| How suites lie | What this runner does about it |
|---|---|
| The parser half-understands a construct and silently drops steps or table cells | Unsupported syntax is a **hard error with `file:line`** — doc strings, `Rule:`, ragged tables, a table row missing its closing `\|`, all of it. Never a best-effort parse. |
| A scenario with zero bound steps "passes" | Unbound scenarios register as *ignored* — and ignored is *reported as green*, so the runner **fails the suite** on any unbound step unless the feature is explicitly marked `.wip()`. Rewording one step can't silently un-test a feature. |
| A step matches two definitions and one silently wins | Ambiguity is **asserted against per feature**, as its own trial, for every step — even for `.wip()` features. |
| Step definitions collide across the suite's global namespace | There is no global namespace: **each feature gets its own registry and its own typed `World`**. An agent editing one feature structurally cannot break another's bindings — the type system enforces it. |
| A scaffolded step stub passes vacuously | Missing-step failures include a **paste-ready definition whose body panics** `pending`. You cannot paste your way to a false green. |
| A failing assertion leaks the temp dir / process it was about to clean up | `ctx.defer(f)` runs cleanup LIFO **even when a step panics**. The step failure outranks cleanup errors. |
| A typo'd `@skip` tag is silently inert | Misplaced tags, dangling tags, and near-miss tags (`@Skip`, `@ONLY`) are **loud errors**. `@only` itself is rejected loudly too — a silently inert (or silently *focusing*) `@only` is the worst tag failure mode; focus one scenario with `cargo test -- '<name>'` instead. gherkin-node-test (≥0.3) rejects it identically. |
| A step matcher that falls through silently | Steps are matched by a **registry with an unbound-step ratchet** — never by inline `if text.contains(…)` chains, where an unmatched step is a silent no-op. |

The same properties are exactly what a coding agent needs, because agents act
on error output: a located `file:line` error, a failure message containing the
snippet that fixes it, a ratchet that converts silent decay into a red test.
And Rust adds its own layer — the compiler checks every step definition's
types against the feature's `World` before anything runs.

Because scenarios compile into `cargo test` trials, there is no second
toolchain: one command runs unit tests and acceptance criteria together, with
name filtering (`cargo test -- 'Counter'`) and CI integration inherited from
Cargo itself. [cargo-nextest] works too.

[cargo-nextest]: https://nexte.st

## Quick start

```
features/
  counter.feature
tests/
  features.rs
```

```gherkin
# features/counter.feature
Feature: Counter
  Scenario: increment once
    Given a counter at 0
    When I add 5
    Then the counter is 5
```

```rust
// tests/features.rs      (harness = false in Cargo.toml, see above)
use gherkin_cargo_test::{Features, StepRegistry};

#[derive(Default)]
struct Counter {
    count: i64,
}

fn counter_steps(reg: &mut StepRegistry<Counter>) {
    reg.define(r"^a counter at (\d+)$", |ctx, args, _| {
        ctx.world.count = args[0].parse().unwrap();
    });
    reg.define(r"^I add (\d+)$", |ctx, args, _| {
        ctx.world.count += args[0].parse::<i64>().unwrap();
    });
    reg.define(r"^the counter is (\d+)$", |ctx, args, _| {
        assert_eq!(ctx.world.count, args[0].parse::<i64>().unwrap());
    });
}

fn main() {
    Features::new("features")
        .feature("counter", counter_steps)   // feature basename → step definer
        .run()
}
```

```sh
cargo test
```

Each scenario becomes one trial named `Feature :: Scenario`. A fresh `World`
(`W::default()`) is created per scenario; `Background` steps run before each
one. Alongside the scenarios, the runner registers the guard trials described
above (ambiguity, unbound steps, orphaned definer keys).

If a step is missing, the guard failure hands you the definition:

```
FAILED: counter :: step definitions are complete and unambiguous
  unbound steps would register as ignored (reported green); bind them or mark 'counter' as .wip():

  // I add 5
  reg.define(r#"^I add (\d+)$"#, |ctx, args, table| {
      panic!("pending: implement this step");
  });
```

## The binding ratchet

That guard failure is half of the design's central mechanism. The other half
is `.wip()` — together they form a **ratchet**: binding coverage (the fraction
of your feature files' steps wired to executable code) can move forward
freely, and can never slip backward silently.

The decay path the ratchet closes is induced by *normal editing*, not by bad
tests: reword one step in a `.feature` file and its regex no longer matches;
the scenario becomes unbound; the runner registers it as an ignored trial —
which `cargo test` **reports as green** — and a feature you believed was
tested is now tested by nothing, with no signal emitted. In a workflow where
feature files are edited constantly (by you or by an agent), that path would
be exercised weekly.

So the guard fails the suite on *any* unbound step, and `.wip(base)` is the
one sanctioned exception:

- **Bootstrapping**: mark a new feature `.wip()` and bind steps one at a
  time. Its unbound scenarios still *register* — visibly, as `[unbound]`
  ignored trials — they just don't fail the suite. Honest green, with the
  debt on display.
- **The click**: when the last step binds, remove the `.wip()` call. That's
  the pawl dropping into the next tooth — from this commit forward the
  feature cannot silently lose coverage again.
- **Backward motion is loud in exactly two ways**, both reviewable diffs:
  the suite goes red (with a paste-ready, panicking definition per missing
  step), or someone re-adds `.wip()` — a one-line, grep-able confession in
  the test file. There is no third path.

`.wip()` is therefore a **debt register**: `grep wip tests/features.rs` tells
you exactly which features are not yet fully enforced. It relaxes *only*
unbound-ness — ambiguity stays a hard error even for wip features ("not
fully bound yet" never means "allowed to be ambiguous").

Two companion rules seal the ratchet's other entrances: the orphan-definer
guard (renaming a `.feature` file can't silently strand its steps), and
skip-still-binds (`@skip` means "don't run", never "don't bind" — otherwise a
tag would be a hole in the ratchet).

The ratchet is also this crate's honest replacement for tag-based scenario
exclusion (cucumber's `excludeTags`): scenarios awaiting data or a pending
experiment simply stay unbound under a `.wip()` feature — visible as ignored,
never silently green, and demanded by the guard the moment the marker comes
off.

## N-version verification

Because the feature files are language-neutral and strictly separated from
step code, they support a workflow that used to be priced out of reach:
**independent implementations of the same spec, diffed against each other.**
Classic N-version programming meant paying two teams; with coding agents, a
second implementation of a pure kernel costs one prompt. The features are
the shared contract — this crate and its JS sibling
[gherkin-node-test](https://github.com/bingh0/gherkin-node-test) parse the
same dialect, so one `.feature` suite can drive both implementations
**verbatim**.

The mechanics, beyond running the same scenarios against both:

1. Drive both implementations with **identical generated inputs** — a
   deterministic PRNG implementable bit-for-bit in both languages (e.g.
   mulberry32: `wrapping_mul`/`^`/`>>` here ≡ `Math.imul`/`^`/`>>>` in JS),
   so both sides see the same doubles in the same order.
2. Compare a **checksum over every output** (not just pass/fail). Agreement
   to full float precision is the strongest correctness evidence available
   to someone who cannot read the code; disagreement localizes a bug to one
   side before any user ever sees it.
3. A behavioral divergence that **no scenario catches** is a spec gap with
   two witnesses — feed it back into the feature file.

When it's worth it: pure, deterministic kernels — parsers, numeric and
financial code, codecs, business rules — where subtle bugs (boundary
conditions, float behavior) would otherwise be silent; any port, where the
old implementation verifies the new one for free; anywhere the human
auditing the system reads only the features. When it isn't: I/O-heavy glue
and UI code, whose behavior *is* the environment rather than a function of
its inputs.

Proven in practice: a TypeScript signal-processing kernel and its
agent-written Rust port, bound to md5-identical feature files, matched to
six decimal places over thousands of PRNG-generated inputs — on the first
comparison.

This repo also eats its own cooking: `tools/parity` applies exactly this
discipline to the parser itself, diffing this crate against the JS sibling
over a shared corpus plus seeded fuzzing (canonical AST and lint-finding
dumps, compared byte-for-byte). It caught a real divergence no hand-written
test had — Rust's Unicode case folding matching the Kelvin sign `K` as `k`
where JS refuses — before any user did.

## Strictness surfaces feature-file bugs

The checksum workflow above is scoped to pure kernels. The *parser's*
strictness, though, pays off well outside that scope — including the I/O-heavy
CLIs the diff isn't aimed at. A worked example: an agent-driven port of a
stateful file tool — a folder organizer with moves, collision renames, undo,
and exit codes — re-driven through the same feature files.

The forensic detail is the sell. On first parse, `collisions.feature` was
**rejected at line 2**: its `Feature:` narrative began "When a file being
moved…", and the micro-grammar reads `When ` as a step keyword appearing before
any Scenario. pytest-bdd had run that same file green for the life of the
project. Porting the corpus also surfaced five real behaviors that no scenario
documented (trailing-dot classification, symlink skip, non-regular-file skip,
`--version`, corrupt-manifest undo) — so the contract came out **stronger than
the suite it was ported from**. The Python original and the Rust port then produced
byte-identical reports *and* filesystem trees across organize / dry-run / undo /
recursive / collisions.

The strict, simple parser doesn't just refuse bad input — it drives you to
better feature files.

## Supported grammar

| Construct | Notes |
|---|---|
| `Feature:` | exactly one per file, required |
| `Background:` | optional, at most one, must precede every `Scenario` |
| `Scenario:` | free-text title |
| `Scenario Outline:` | requires exactly one `Examples:` table |
| `Examples:` | a header row then ≥1 data row, `\|`-delimited |
| `<placeholder>` | substituted from the Examples columns — in step text **and** step data tables; every `<name>` must match a column |
| Steps | `Given` `When` `Then` `And` `But` `*`, followed by step text |
| Step data tables | `\|` rows after a step attach to it; the step closure receives an **`Option<&DataTable>`** as its last argument |
| Tags | `@skip` → trial ignored (steps must still bind); `@todo` → runs, failure tolerated; `@only` → **rejected loudly** (use `cargo test -- '<filter>'`); the three are **mutually exclusive — combining them is a parse error**; tags on `Feature:` apply to all its scenarios; any other tag is carried on `scenario.tags` with no runtime effect |
| `# comment` | ignored anywhere |
| Feature narrative | the `As a… / I want… / So that…` prose block is ignored |

Table cells honor the Gherkin escapes `\|` (literal pipe), `\\` (literal
backslash) and `\n` (newline); a backslash before any other character is
literal, so cells like `C:\Temp` need no escaping.

### Scenario Outline: one scenario, many values

When the same behavior should hold across a spread of inputs — the usual
suspects plus the extremes — write it once as a `Scenario Outline` and put the
values in the `Examples` table:

```gherkin
Scenario Outline: the counter reflects adding <amount>
  Given a counter at 0
  When I add <amount>
  Then the counter is <amount>

  Examples:
    | amount           |
    | 5                |
    | -3               |
    | 0.5              |
    | 9007199254740991 |
```

Each row expands into its own independent trial — fresh `World`, `Background`
re-run — named with the substituted title and a row suffix:
`Counter :: the counter reflects adding -3 [2]`. Placeholders substitute in
step text **and** step data tables; a `<name>` with no matching column is a
parse error, not a silent leak into the step text.

One honest caveat: expansion is text substitution, and in Rust a value passes
**two** gates — the step's regex decides what *binds*, and the step body's
`parse` decides what *survives*. The quick-start steps match `(\d+)` and parse
into an `i64`, so the `-3` and `0.5` rows won't bind until you widen the regex
to something like `(-?\d+(?:\.\d+)?)` — and the fractional row additionally
needs the parse (and the `World` field) to move to `f64`. Forgetting the regex
isn't a quiet pass: the unbound-step guard fails loudly and hands you the stub
for the exact expanded text. And an outline with a single `Examples` row gets
a [lint warning](#the-linter-role--under-someone-elses-runner) — that's a
plain `Scenario` with extra ceremony, and usually a missing case.

### Step matching and `DataTable`

Steps are matched by **regex source string** (`define`) or **exact literal**
(`define_exact`) — capture groups become the step's `args: &[String]`; parse
them in the step. There are no Cucumber Expressions (`{int}`, `{string}`);
write a regex.

A step with a data table receives `Some(&DataTable)` as its last argument,
API-compatible with cucumber so step code (and muscle memory) ports both ways:

```gherkin
Given these users
  | name | role  |
  | ada  | admin |
```

```rust
reg.define_exact("these users", |ctx, _, table| {
    let t = table.expect("step has a data table");
    t.raw();       // Vec<Vec<String>>, every row
    t.rows();      // rows minus the header
    t.hashes();    // Vec<HashMap>: [{name: "ada", role: "admin"}]
    t.rows_hash(); // two-column table → key → value map (panics otherwise)
    t.transpose(); // columns become rows → new DataTable
});
```

### Scenario-scoped cleanup: `ctx.defer(f)`

Cleanup runs after the scenario in reverse (LIFO) order — **including when a
step panicked**. The step failure, if any, outranks cleanup errors; if the
steps passed, the first cleanup error fails the scenario.

```rust
reg.define_exact("a scratch dir", |ctx, _, _| {
    let dir = mkdtemp();
    ctx.world.dir = Some(dir.clone());
    ctx.defer(move |_| { let _ = std::fs::remove_dir_all(&dir); });
});
```

## Deliberately unsupported — and rejected loudly

The design rule: **parse the supported subset correctly; reject everything
else with a `file:line` error; never parse a feature file vacuously.** Each of
these is a `GherkinSyntaxError` with the offending line number:

| Rejected | Why it's rejected, not ignored |
|---|---|
| Doc strings (`"""` / ` ``` `) | would be mis-read line-by-line as steps |
| Multiple `Examples:` per Outline | the 2nd header row would corrupt the expansion |
| `Examples:` with no data rows / no header | would expand to zero (vacuous) scenarios |
| Ragged table rows (Examples **or** step tables) | column misalignment would pass silently |
| A table row missing its closing `\|` | the trailing cell would be silently dropped |
| A table row with no preceding step | the data would silently belong to nothing |
| Unknown `<placeholder>` | almost always a typo; would leak `<name>` into a step |
| A `Scenario`/`Scenario Outline` with no steps | would run zero assertions and pass vacuously |
| A step *after* its `Examples:` table | malformed ordering; the step would mis-attach |
| Tags anywhere but immediately before `Feature:` / `Scenario:` / `Scenario Outline:` | a mis-placed `@skip` would silently not skip |
| A near-miss semantic tag (`@Skip`, `@SKIP`, `@Only`, …) | would be silently inert |
| Combined semantic tags (`@skip @todo` on one scenario) | every runner resolves the combination differently (this one would let `@skip` win silently); dialect parity with gherkin-node-test, which rejects these identically |
| `Rule:` (Gherkin 6) | grouping would be silently flattened |
| A step before any `Scenario`/`Background` | would be silently discarded |
| A 2nd `Feature:` / `Background:`, or `Background:` after a `Scenario` | ambiguous scope |

Two non-features by design, with no dedicated error: **Cucumber Expressions**
(write a regex) and **i18n** (English keywords only — a non-English keyword
reads as narrative; if that empties a scenario, the no-steps guard fires, so
it still can't pass vacuously).

## Deviations from gherkin-node-test

Same grammar, same guards; the differences are where Rust is genuinely
different, and each one is deliberate:

| Node | Rust | Why |
|---|---|---|
| dynamic `world` object | **typed `World` per feature** (`StepRegistry<W>`, `W::default()` per scenario) | the compiler now catches world-shape mistakes the JS version can't |
| `@todo` → node:test `todo` | runs; failure printed and **tolerated** (trial kind `todo`) | libtest has no todo concept; this preserves "runs but doesn't gate" |
| unbound scenario → node:test TODO | unbound scenario → **ignored trial** (kind `unbound`) whose body **fails with its reason** | same visibility, same ratchet: the binding guard fails the suite unless `.wip()`; and `cargo test -- --include-ignored` can't turn parked debt into a vacuous pass |
| zero dependencies | **two boring dependencies** (`regex`, `libtest-mimic`) | hand-rolling a regex engine or a test harness protocol would be its own foot-gun; zero-dep is a non-goal here |
| throws on parse error at load | parse error becomes a **failing trial** (`base :: parses`) | sibling features still report; the suite is red either way |
| lint findings carry plain strings (`rule`, `severity`); `filename` defaults to `<feature>` | **`LintRule` / `LintSeverity` enums** with `as_str()`; `filename` is explicit | the compiler exhaustiveness-checks rule matches that JS can't; the *string forms and finding text* are identical both sides, held together by `tools/parity` |

## The linter role — under someone else's runner

Everything above assumes this is your runner. It doesn't have to be:
`lint_feature(text, filename)` exposes the same loud dialect gate, plus
deterministic spec lints, as a **pure function** — text in, findings out. No
filesystem, no environment, no trial registration; directory walking stays in
*your* code. Use it to hold `.feature` files to this dialect and quality
floor in a repo whose executor is something else (cucumber-rs — or keep the
files portable to the node sibling's runtimes).

Findings carry `rule` / `severity` / `line` / `message`:

| Rule | Severity | Fires on |
|---|---|---|
| `dialect` | error | anything the rejection table above rejects — the exact `GherkinSyntaxError`, as a finding (the parser stops at the first violation, so it arrives alone) |
| `no-then` | warn | a scenario whose steps never resolve to `Then` — runs code, asserts nothing (`And`/`But`/`*` inherit the preceding primary keyword, across a `Background`) |
| `vague-then` | warn | a `Then`-resolved step containing *works · correctly · properly · as expected · handles · appropriate* — words that assert nothing checkable |
| `single-row-outline` | warn | a `Scenario Outline` with one `Examples` row — a scenario with extra ceremony, and usually a missing case |
| `near-miss-keyword` | warn | a silently dropped line that was almost certainly meant as syntax: a wrong-case step keyword inside a scenario or `Background` body (`when I add 5`, `GIVEN a counter`), or — anywhere — a construct header that isn't the one exact form the parser recognizes (`scenario: b`, `Scenario : b`, `SCENARIO OUTLINE: b`) |

`near-miss-keyword` is the counterpart to the near-miss *tag* the parser
already rejects outright (`@Skip`, `@Only`): keywords are exact, and anything
else on the line is narrative, dropped without a finding. A wrong-case step
keyword loses one requirement; a wrong-form `scenario:` is worse — the
scenario never exists and its steps silently merge into the *previous*
scenario, unseeable by the no-steps guard and `no-then` precisely because
there is no scenario to inspect. The rule reads the dropped lines off the
parse itself (`ParsedFeature.narrative`), so what the linter checks and what
the parser drops cannot drift apart. `Rule:` is exempt: its exact form is
itself a `dialect` error in this subset, so a near miss is not a rescue.

Severity is descriptive, not policy: the wip-style debt register (filter by
rule) belongs in your guard test. Finding **text** is identical to
gherkin-node-test 0.5.0's `lintFeature` — the two linters are held together
differentially by `tools/parity` (byte-for-byte finding streams over shared
corpora and fuzzing), so a feature corpus linted here means the same thing
linted there.

## When *not* to use this

- You want the full Gherkin standard, tag-expression filtering, i18n, or
  living-documentation reports → **cucumber-rs**. That's a platform; this is a
  file.
- You need async step functions → **cucumber-rs** (steps here are sync
  closures; blocking IO in tests is fine).
- You only need a Gherkin *parser* → the **`gherkin`** crate.

The niche here is exactly: Gherkin on `cargo test`, minimal and macro-free,
loud by construction.

## API

| Export | Purpose |
|---|---|
| `Features::new(dir).feature(base, definer).wip(base).run()` | **high-level runner**: discover every `.feature`, scoped registries, typed worlds, guard trials |
| `Features::build_trials()` | the same trials without running them (the guards are testable — see `tests/guards-proof.rs`) |
| `parse_feature(text, filename)` | parse → `ParsedFeature` (incl. `outlines: Vec<OutlineMeta>`); `Err(GherkinSyntaxError)` on unsupported/malformed syntax |
| `lint_feature(text, filename)` | **linter**: dialect gate + spec lints as `Vec<LintFinding>` — pure text-in/findings-out, for use under another runner |
| `StepRegistry<W>` | `.define(regex_src, fn)` / `.define_exact(text, fn)` / `.find(text)` / `.matching(text)` |
| `execute_steps(steps, &registry, world)` | run a flat step list against a world (LIFO `defer` cleanup included) |
| `check_bindings(&parsed, &registry, base, wip)` | the pure binding guard (ambiguity + unbound-step ratchet) |
| `Ctx<W>` | step context: `.world` + `.defer(f)` |
| `DataTable` | cucumber-compatible step table: `raw` / `rows` / `hashes` / `rows_hash` / `transpose` |
| `build_snippet(text)` | paste-ready step definition for an unbound step (body panics). One known edge: step text containing `"#` would break the emitted `r#"…"#` literal — write that regex by hand |
| `GherkinSyntaxError` | parse error; carries `.line`, displays as `file:line: message` |

There is also a corpus checker for evaluating an existing feature suite
against this grammar before porting a project:

```sh
cargo run --example parse -- path/to/features/*.feature
```

## Provenance

Ported guard-for-guard from [gherkin-node-test], which was extracted from
[ccr](https://github.com/bingh0/ccr) where it runs the acceptance layer of a
shipping CLI. The port is validated four ways: a conformance suite with a
rejection test for every guard above (`tests/conformance.rs`); an executed
proof that every runner guard actually *fires* — trials are built over fixture
features and run through libtest-mimic in-process, asserting pass/fail/ignored
counts (`tests/guards-proof.rs`); a real-world corpus check — the feature
suites of two shipping projects written for the JS sibling and for
vitest-cucumber (102 files, 507 scenarios) parse with zero rejections; and the
strongest of the four, because it's the only one where an independent
implementation vouches: the differential parity harness (`tools/parity`),
which holds this crate and the JS sibling to byte-identical parse trees, lint
findings, and error-message text over a curated corpus plus thousands of
seeded fuzz cases per run.

MIT © Bing Ho
