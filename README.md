# gherkin-cargo-test

**The smallest honest Gherkin runner for Rust.** Two boring dependencies, no
proc macros, no async, no framework — it turns `.feature` files into real
`cargo test` tests (one per scenario, via [libtest-mimic]), and it treats
every silence as a bug. One file, ~2,600 lines, small enough to read in one
sitting or to vendor outright. The same file doubles as a **feature-file
linter** for projects whose runner is something else, with a `gct-lint`
command to run it — see
[the linter role](#the-linter-role--under-someone-elses-runner).

This is the Rust sibling of [gherkin-node-test] — same author, same grammar,
same philosophy, ported guard-for-guard. The feature files are portable
between the two: Gherkin is the language-neutral control layer.

[libtest-mimic]: https://crates.io/crates/libtest-mimic
[gherkin-node-test]: https://github.com/bingh0/gherkin-node-test

```toml
[dev-dependencies]
gherkin-cargo-test = "0.10"   # or just vendor src/lib.rs; it's one file

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
Cargo itself. [cargo-nextest] works too — with one scoped exception: the
opt-in [run manifest](#the-run-manifest) needs a single-process run and is
refused loudly under nextest (one red trial naming the fix; everything else
runs normally).

[cargo-nextest]: https://nexte.st

The guards above are the mechanical floor. How a suite lies *above* that
floor — green and hollow (`cino:`, completion in name only) versus never
written at all (`blind:`) — is mapped once, with an address, a tell, a
catch and dated specimens, in the JS sibling's
[failure ladder](https://github.com/bingh0/gherkin-node-test/blob/main/docs/failure-ladder.md); this crate adds no copy.

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

## Running one scenario by name

`cargo test -- "<filter>"` runs the trials whose name contains the filter.
The name is the one `--list` prints, kind label included, so `"[skip]"`
selects the skipped trials. `--skip "<pattern>"` leaves out the trials whose
name contains the pattern. libtest answers a mistyped filter with
`ok. 0 passed` and exit 0 — green, having observed nothing. This runner does
not:

- **A filter or `--skip` under which no scenario ran fails the run**, and the
  message names the filter, the skip patterns and the feature directory. Exit
  status is 101. `--ignored` and `--include-ignored` change nothing here:
  under them a parked scenario really runs, and one that ran counts.
- **The runner's own check trials don't count.** A filter that matches only
  `counter :: step definitions are complete and unambiguous` passes that
  check and is still refused: no scenario was observed.
- **A filter matching only parked scenarios says so.** If everything it
  matched is `@skip` or unbound and stays parked, the message reads "matched
  only ignored scenarios"; nothing ran.
- **A failed trial outranks the refusal.** The run is already red, and
  libtest's failure says why.
- **Not refused: `--list`, and cargo-nextest.** `--list` runs nothing; a
  selection that lists nothing prints a note and fails nothing. nextest runs
  every trial in its own process with `--exact`, so there "this process ran
  no scenario" is the normal case; the exemption needs a nextest variable
  and `--exact` both.
- **One filter per run.** A second positional filter is refused before any
  trial runs, naming every filter in the order given, with exit status 2.
  After a bare `--` every argument is a filter. libtest would OR them; this
  harness takes one, so run them one at a time.

The guard is per test target, because one process sees only its own trials.
`cargo test -- "<filter>"` hands the filter to every test target, so with two
runner targets the one that does not hold the scenario is refused, and cargo
stops there. Name the target:

```sh
cargo test --test design -- "<filter>"
```

## Running the suite

This section is for a project starting on this runner, and for the build agent
that works in it. It assumes nothing about the machine: every number that
depends on hardware is read from the machine, never written into the
repository. Except where it names a flag or a file of this crate, it describes
practice, not runner features.

### The loop

- **While working**, run one scenario or one feature: one name filter, with the
  test target named (`cargo test --test <target> -- "<Feature> :: <Scenario>"`,
  or a substring of it). The filter rules are in
  [Running one scenario by name](#running-one-scenario-by-name).
- **Before handing work back**, run the whole suite, once.
- **Run long commands in the foreground with a time limit** (for example
  `timeout`). Do not start a background run and then loop, sleeping and
  checking: with a limit in the foreground, a run that died is found at once.
- **If the project has a mutation check**, run during a stage only the mutants
  for what changed, each against its named target scenario. Before a release,
  run the whole table once. This crate has no mutation runner; the project's own
  scripts do it.

### What a faster suite is for

Mutation proves one thing: that a scenario's bindings observe something. Doctor
the world, and the scenario goes red. It says nothing about three other ways a
green suite lies:

- an assertion whose ground truth the build itself wrote;
- a code path the scenarios never drive;
- input written in a foreign system's name, with no recording of that system.

Spend the time a faster suite saves on those. Assert what the system on the far
side observes. Replay a recorded foreign input through the installed entry
point. Count an "every" or "never" claim against the foreign system's own list
or record. A report that ran only mutation says it checked one thing; say so.
The [failure ladder](https://github.com/bingh0/gherkin-node-test/blob/main/docs/failure-ladder.md)
maps these.

### Harness habits that prevent false reds

These apply to projects whose scenarios drive real software: processes,
sockets, files. All of them are in the project's own harness code.

- **Give every test layer its own `HOME`, temporary folder and tool-state
  folder, from the first commit.** A unit layer that kept the real home folder
  once wrote into its owner's real configuration.
- **Keep anything the run must keep or share outside any folder the harness
  makes private:** logs, reports, a lock folder, the cargo target folder
  itself. A harness that gives each run a private `/tmp` makes a build folder
  under `/tmp` vanish, and the re-run test binary is then not found.
- **Use one time factor for waits**, set per machine and multiplied into the
  project's default; 1.0 means no change. A wait for something to happen may
  scale up or down. A check that something does *not* happen within a window
  must never scale below the length its author wrote. Better, wait for a later
  event that can only follow the forbidden one, then assert the forbidden one
  was not seen.
- **Write a fixture executable through a child process** (`cp`, for example),
  when a test process writes an executable and then runs it. If a thread holds
  the file open for writing while another thread forks, the exec fails with
  "Text file busy". This is for fixtures only: a product that itself writes and
  runs a program has the same race, and its own code must stay under test.
- **Read a cap on a scarce resource from an environment variable, with a
  default, and print it at the start of the run.** The resource is live
  sessions or containers, and the cap lives in the harness. A cap set once in a
  constant and never measured again set one project's suite time for weeks.

### Waiting

`gherkin_cargo_test::harness` is the one way to wait, so a project writes no
waiting code of its own.

- `wait_for(what, base, predicate)` and `wait_for_some(what, base, probe)`
  poll until the thing happens, up to `patience(base)`: `base` × the project
  default (`Features::patience_default(f)`, 1.0 when not set) × the machine's
  `ABDD_TIME_SCALE`. They return at the first poll that sees it. Polls are up
  to 100 ms apart, so the return can lag the event by that much, and a
  condition that is true only between two polls is missed: the predicate
  should observe something that stays true once it happens (a record, a file,
  a counter). A predicate is never interrupted, so one that takes longer than
  the limit makes the wait overrun by that much; the expiry message reports
  the real elapsed time. A limit too large to be a deadline fails at once with
  `cause: harness`.
- A check that something does *not* happen runs its whole window, so the
  window never shrinks: `absence_window(base)` is `base` × `max(1, ABDD_TIME_SCALE)`,
  and the project default does not apply. `absent_for(what, base, forbidden)`
  watches for that window. Prefer `absent_until(what, forbidden, sentinel_what,
  base, sentinel)`: it waits for a later event that can only follow the
  forbidden one, then checks. `forbidden` must read something that stays
  (a record, a file, a counter); a poll can miss a transient.
- `harness_error(message)` fails a step as the harness's fault: a fixture that
  would not start, a port it could not bind.
- Settings come from `ABDD_TIME_SCALE` (a multiplier; 1.0 when unset, and
  anything but a number from 0.01 to 1000 is an error) and, only when
  `ABDD_MACHINE_ENV` names one by an absolute path, a settings file. An empty
  or blank value in the environment counts as unset. The file may be a
  general env file: only lines that begin with `ABDD_` (after an optional
  `export `) are read, and no message ever quotes any other line. On those
  lines a value is quoted (`'…'` or `"…"`, which may hold `#`) or runs to the
  first `#`; anything else is an error naming the file, line and name. The
  environment wins over the file. Nothing is read from a home folder. The
  project default (`patience_default`) must also be from 0.01 to 1000.
- A step that fails because a wait ran out, or through `harness_error`, gets
  `  cause: patience` or `  cause: harness` after its role line. The mark is
  matched to the failure's message text, so a step that catches the panic and
  panics again with the identical text keeps the cause. Like the
  role, it is information only: it changes no verdict. Call these functions
  on the step's own thread; a wait that expires on a helper thread loses the
  cause.
- When a project default is set, `ABDD_TIME_SCALE` has a value, or
  `ABDD_MACHINE_ENV` names a file (even one that sets nothing), `run()` prints
  one line to stderr first, for example
  `gherkin-cargo-test: patience x3 (default 3 x machine 1); ABDD_TIME_SCALE unset`.
  With none of these, a run prints nothing new. A bad setting stops any
  project's run there, before any trial, with exit status 2, whether or not
  the project uses `harness`.

### Debug-build hot spots

Tests run debug builds. In the project's `Cargo.toml`,

```toml
[profile.dev.package."*"]
opt-level = 3
```

optimises every dependency and leaves the project's own crates as they are. It
also covers the regex matching this runner does for every step. Measured on one
project (539 trials, 1,389 step definitions) on one machine:

| | Default | Dependencies at opt-level 3 |
|---|---:|---:|
| Clean build | 11.1 s | 23.3 s |
| Rebuild after a step-file edit | 0.71 s | 0.71 s |
| Runner start-up (`--list`) | 0.61 s | 0.12 s |

For one hot crate, `[profile.dev.package.<crate>] opt-level = 3` works too.
Either way the hot function must not be generic (`&mut dyn Read`, not
`impl Read`): a generic function is compiled in the crate that calls it, at
that crate's opt-level, and the setting does nothing (measured once: 2.39 s
unchanged, against 0.23 s after the fix). Code in the project's own crate is
not covered; move it into a crate of its own. Set `opt-level` only. Never turn
off `debug-assertions` or `overflow-checks`: they change what is tested.

### Lint from the first commit

Run `gct-lint --strict` in CI, installed at the same version as the runner the
project pins, so findings never pile up. The command is described in
[From the command line](#from-the-command-line-gct-lint).

One rule goes with it. When strict lint flags a vague Then, the fix is a
stronger binding, or a recorded ruling that narrows the promise. Rewording the
step down to what the binding already proves narrows the contract quietly.

### How many runs at once

A suite that drives real software is heavy in memory and often mostly waits.
How many runs fit at once is a property of the machine, not of the repository,
so do not put a count in the repository. Read it from the machine's
environment, count every agent's runs together, and when in doubt run one at a
time. The names shared with gherkin-node-test are `ABDD_HEAVY_JOBS` (heavy runs
at once, across all agents) and `ABDD_TIME_SCALE` (the time factor for waits,
above). They are conventions for the project's harness and scripts to read. Every
`ABDD_` name, what it means and who reads it is listed in one place, the
[machine settings](https://github.com/bingh0/gherkin-node-test/blob/main/docs/machine-settings.md)
page in the sibling's repository. Too many at once makes tests fail from load, and those failures look
exactly like real ones. Measured on one project: the same 24 mutation checks
at 6 at once instead of 4 produced 14 extra failing scenarios, most of them
one ordinary assertion. This runner reads `ABDD_TIME_SCALE` (and
`ABDD_MACHINE_ENV`, see [Waiting](#waiting)); it does not read or enforce
`ABDD_HEAVY_JOBS`.

### Raising a limit

Raise a limit (threads, a harness cap, runs at once) only after repeated green
whole runs at the new value, under the load it will really see. Write the
measurement where the number lives.

### A long run started as a service

Where a service manager starts the run (for example `systemd-run --user` on
Linux), the run gets the service's limits, not the shell's. On one machine the
default of 1,024 open files killed a long run in its first minutes. Do one
short dry run under the same limits first.

### A block for the project's AGENTS.md

A build agent starting a project copies this block into the project's own
`AGENTS.md` (or `CLAUDE.md`) itself, fills in the target names, and keeps it
short; this README stays the source for the reasons.

```markdown
## Running tests
- One filter per run, and name the target:
  `cargo test --test <contract-target> -- "<Feature> :: <Scenario>"`
  (`<design-target>` likewise).
- A filter or `--skip` that ran no scenario fails the run. A red there means
  the selection is wrong, not the code.
- Run the whole suite once before handing work back.
- Run long commands in the foreground with a time limit, never in the
  background with a sleep loop.
- Give each test layer its own HOME and temporary folder.
- Wait only through `harness::wait_for`, `absent_until` and `absence_window`
  (`ABDD_TIME_SCALE`, 1.0 when unset). A window that checks something does not
  happen never shrinks below its written length.
- Start no more heavy runs at once than the machine's limit (`ABDD_HEAVY_JOBS`),
  counted across all agents. When it is unset, run one at a time.
- Run `gct-lint --strict` before committing a `.feature` change.
- Fix a vague Then with a stronger binding or a recorded ruling. Never reword
  it down to what the binding already proves.
- Raise a limit only after repeated green whole runs at the new value.
- Mutation is one check. A report says which checks it ran.
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

### Scenario-scoped wip

`.wip(base)` holds a *whole* feature open — the right grain while
bootstrapping, but too coarse for a feature that is 10/12 bound with two
scenarios waiting on an interface that doesn't exist yet. Wip-ing the whole
feature would relax the ratchet for the ten bound scenarios too. So the
register has a second grain:

```rust
Features::new("features")
    .feature("smoothing", smoothing_steps)   // 10/12 bound
    .wip_scenarios("smoothing", [
        "resumes after a gap",               // by SOURCE title — an outline's
        "streams the tail <mode>",           //   title covers every expanded row
    ])
    .run()
```

The named scenarios register as ignored exactly like whole-feature wip; every
*other* scenario in the feature keeps the full can't-silently-lose-a-binding
guarantee. Two properties keep the finer grain honest:

- **Scenario wip means "expected-unbound", never "skip".** A listed scenario
  whose steps all happen to be bound runs normally — this lever cannot
  suppress executable code. To pend it, leave its distinguishing step
  unbound; there always is one (the step touching the unbuilt interface).
- **Both wip shapes are ratcheted against rot.** An entry whose feature — or
  scenario — has become fully bound *fails the suite* until the entry is
  removed: an allowlist that could go stale silently would hold the ratchet
  open for nothing. Likewise an entry naming a feature or scenario that no
  longer exists fails loudly; renaming can't strand debt off the register.

Under a wip'd feature, the ignored trials you see are exactly the ones the
register declares — a reviewer can tell "intentionally pending" from
"someone broke a binding" by grepping the entry.

Three companion rules seal the ratchet's other entrances: the orphan-definer
guard (renaming a `.feature` file can't silently strand its steps),
skip-still-binds (`@skip` means "don't run", never "don't bind" — otherwise a
tag would be a hole in the ratchet), and the unused-definition guard, the
ratchet's dual: a definition no scenario consumes is a failing trial naming
the pattern and its feature. The pair holds the ledger taut from both sides —
the ratchet catches scenarios without definitions, the guard catches
definitions without scenarios; the guard carries no register of its own,
because the sanctioned way to keep a definition is to write the scenario it
serves and hold *that* in the wip register.

The ratchet is also this crate's honest replacement for tag-based scenario
exclusion (cucumber's `excludeTags`): scenarios awaiting data or a pending
experiment simply stay unbound under a `.wip()` feature — or, finer, under a
`.wip_scenarios()` entry naming exactly them — visible as ignored, never
silently green, and demanded by the guard the moment the marker comes off.

## The run manifest

Everything above makes a run that *happens* honest. It cannot make a run
happen: a feature directory nobody points a runner at is green by absence —
spec present, enforcement quietly missing, invisible in any diff of the
feature files. The fix is one written sentence per run: **the runner writes
down what ran, so a reader can notice what didn't.**

```rust
Features::new("features")
    .feature("counter", counter_steps)
    .manifest("features/run-manifest.ndjson")   // opt-in, one per feature dir
    .run()
```

Every registered scenario becomes one NDJSON row — sorted, fixed key order,
**byte-compatible with the node sibling's manifest** (same escaping, same
code-point sort, same five statuses):

```json
{"file":"counter.feature","title":"increment once","status":"passed"}
{"file":"counter.feature","title":"resumes after a gap","status":"unbound"}
{"file":"counter.feature","title":"streams the tail live [1]","status":"skipped"}
```

Commit the manifest. A tool (or a reviewer) joining it against the
`.feature` files in the tree sees the silent class structurally: a feature
file with no rows in any manifest is coverage that isn't running.

The contract, held deliberately small (`ManifestRecorder` in the source
carries the full rules):

- **Rows are `{file, title, status}` and nothing else.** No timestamps, no
  durations — the bytes change only when results change; *when* comes from
  the commit that touched it. Identity is path + title (outline rows land
  individually, `title [n]`); there is no invented ID scheme. `file` is
  recorded **relative to the manifest file's own directory** — the account
  must not change with the machine or checkout path that produced it, so an
  absolute feature-dir argument never leaks machine paths into committed
  bytes, and downstream joins stay portable.
- **Statuses are run-mode-independent.** `passed`/`failed` come from
  execution; `skipped` (@skip) and `unbound` come from *registration* — a
  normal run and an `--include-ignored` run write identical bytes, even
  though the latter executes parked bodies. `todo` comes from execution
  too, safely: the @todo inversion runs on every mode, so the row is
  deterministic — `todo` means *declared and still failing*, and a stale
  @todo (one that passes) writes `failed`, so a red run's account explains
  the red.
- **Every account opens with `{"run-manifest":1}`** — an in-band schema
  declaration, so the file explains itself with no other context. The key
  names the format, not the tool: the node sibling writes byte-identical
  accounts. The version bumps only on a change that would mislead a
  version-1 reader.
- **A partial run never writes.** The file is written exactly once, when
  every scenario's outcome has been observed. A name-filtered, `--list`, or
  `--ignored` run leaves the previous manifest untouched. An empty feature
  directory is refused outright (one failing trial naming the path — a
  zero-scenario run must be loud, not a green file). A feature that fails to
  read or parse disables the writer entirely: an account omitting its rows
  would be absence-shaped.
- **cargo-nextest is refused loudly.** The manifest needs one process to
  observe the full run; nextest runs one test per process, so no process
  ever can — silently never writing would be the exact class the manifest
  exists to expose. Detected via nextest's env vars, the refusal is one
  registered failing trial naming the fix (run the manifest lane under
  `cargo test --test features`); every scenario still runs. Honest limit: an
  unknown process-per-test runner without those env vars falls back to
  never-writing — write-on-complete at least guarantees it never writes a
  partial account.
- **The runner stays stateless.** Nothing is read back, nothing gates on
  the manifest, registration is unchanged. Failure history and change dating
  are derivable by downstream tooling from the committed manifest's git
  history.

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
| Tags | `@skip` → trial ignored (steps must still bind); `@todo` → inverted: runs, expected to fail (failure printed, gates nothing) — a *passing* @todo fails the run naming the stale tag; `@only` → **rejected loudly** (use `cargo test -- '<filter>'`); the three are **mutually exclusive — combining them is a parse error**; tags on `Feature:` apply to all its scenarios; any other tag is carried on `scenario.tags` with no runtime effect |
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
| A `Feature:` with no scenarios | a header plus narrative registers nothing and reads as a passing file — the same vacuous pass, one level up. When a construct near miss emptied the file (`scenario: s`), the error names that line |
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
| opt-in typed world since 0.10.0 (`StepRegistry<W>`, type-level: the world is still born `{}`, its shape accreted by steps) | **typed `World` per feature** (`StepRegistry<W>`, `W::default()` per scenario) | the deviation narrowed at 0.10.0: node types the world too, checked only by the consumer's own tsc. What remains deliberate is construction and enforcement — Rust builds `W::default()` per scenario and the compiler enforces the shape at build |
| `@todo` → plain test, inverted (xfail) | same inversion, with a `[todo]` trial kind label | since 0.9.0 the siblings share the inversion (runs everywhere; failure printed, gates nothing; a *passing* @todo is red naming the stale tag). The label is the one ratified asymmetry: libtest-mimic supports kind markers, the JS runtimes have none for plain tests |
| invoke-time **args-consumption guard**: the callback signature must declare exactly what the sentence produces (captures + table), defaults refused, rest-form exempt | **no guard — the class is unrepresentable**: `args: &[String]` is a slice by construction and the table is an explicit `Option<&DataTable>` parameter; a Rust signature cannot under- or over-declare what arrives | accepted asymmetry, ratified 2026-08-24 (node gh#4 item (a)): the guard exists because JavaScript signatures can lie about consumption; Rust's cannot, so there is nothing to refuse |
| `lintStepDefinitionSource` — step-source lint (unearned-absence on literal-needle negations, rest-signature sightings, statement-attached sanction markers with stale-sighting) | **no lint — but not because the class is unrepresentable**: the shipped forms are JavaScript assertion dialects, and Rust closes two species by construction (`Option` must be handled; worlds are typed structs). A reader that falls back to a default (`unwrap_or_default()`, `unwrap_or("")`) reopens the first: a missing key reads `""`, and every `!=` against it passes. And `assert!(!out.contains("wrong needle"))` rots in Rust exactly as in JS; clippy does not sight needles | accepted asymmetry, re-measured 2026-11-07 when its revisit condition (errata 2026-08-25: a Rust step corpus that accumulates absence assertions) was met. Two in-house Rust corpora (884 definitions) give 27 literal-needle default fires (3.2% and 2.8%), or 45 counting a project `assert_lacks(text, "needle", …)` helper, which a port would see only through node's `config.rules`. One fire found a needle with no source and no positive control, an external UI string, but on a redundant `Then`. Its load-bearing copy sat in a readiness predicate (`if … && !text.contains("…")`) that an assertion-dialect rule cannot see. The other silent absences found by reading had earned needles: a `!= "host"` over an inspect reader that defaults to `""`, a negated event that cannot occur, a name prefix no step ever observes. A substring earnedness test clears all three. What they lack is a positive control: a check that the thing can appear. No text rule sees that, so the doctrine is language-neutral and lives in the node sibling's [assertion asymmetry](https://github.com/bingh0/gherkin-node-test/blob/main/docs/workflow.md#the-assertion-asymmetry), not in a lint. A Rust default rule still has no data to earn admission. The row reopens on a corpus where literal-needle fires find unearned absences that matter |
| unbound scenario → node:test TODO | unbound scenario → **ignored trial** (kind `unbound`) whose body **fails with its reason** | same visibility, same ratchet: the binding guard fails the suite unless `.wip()`; and `cargo test -- --include-ignored` can't turn parked debt into a vacuous pass |
| zero dependencies | **two boring dependencies** (`regex`, `libtest-mimic`) | hand-rolling a regex engine or a test harness protocol would be its own foot-gun; zero-dep is a non-goal here |
| throws on parse error at load | parse error becomes a **failing trial** (`base :: parses`) | sibling features still report; the suite is red either way |
| lint findings carry plain strings (`rule`, `severity`); `filename` defaults to `<feature>` | **`LintRule` / `LintSeverity` enums** with `as_str()`; `filename` is explicit | the compiler exhaustiveness-checks rule matches that JS can't; the *string forms and finding text* are identical both sides, held together by `tools/parity` |
| no command: the linter is `lintFeature` plus the README's guard test (recursive walk, `ALLOW` register, strict bit) | **`gct-lint`**, contract agreed with the node sibling 2026-11-08: recursive sorted walk skipping hidden dirs, `target/` and `node_modules/`; paths printed as typed, walked ones joined with `/`; `path:line: severity[rule]: message` on stdout, summary on stderr; exit 1 on any error, unreadable path or empty directory; `--strict` the one bit; no allow list | gct's beta testers asked for a command; gnt's have not, and its 0.4.0 record keeps the library off the filesystem, which the command leaves untouched. Reopens when a gnt user asks: gnt would then ship the same contract, so `tools/parity` can diff the two commands' stdout byte for byte |
| `bindRunner(testFn)` + a `gherkin-node-test/vitest` adapter entry | **no analog** | node-only by nature: it rebinds registration onto a foreign JS runner's `test()`. The dialect and lint surface stay in lockstep; runner *hosting* is each sibling's own business |
| manifest refuses a RE-INVOKED body (vitest retry/repeats re-run the same registered fn) | **no code** — libtest-mimic invokes each trial closure at most once, so the hazard is structurally unreachable | same doctrine, different fence: the cross-process cousin (nextest's one-test-per-process model) is what gets refused here, at build time via nextest's env vars |
| manifest disabled by a parse error **at load** (validate-everything-first throws before registration) | manifest disabled by the parse failure **trial** (`base :: parses`; recorder disables itself, siblings still run) | same end — a run that failed validation never writes — reached through each sibling's own parse-failure channel |
| name-filter guard (0.12.0): per test **file** (one `runFeatures` call per file) on node, bun and Deno; a selection under which no scenario ran is red, and the runner's own checks don't count. Under vitest it is not covered (no hook runs when every test is filtered out). The refusal names the filter text on node only (bun and Deno cannot read it). A skip pattern as a selection, the separate "only wip-held" message and the refusal of a second name filter are node-only. bun does not refuse a zero-match file when another file matched. node's `--test-only` with no pattern can run no scenario and stay green: a stated gap | same doctrine, per test **target** (one process): the refusal names the filter, any `--skip` patterns and the feature directory, and says to pick the target; exit status 101. **A second positional filter is refused before any trial, naming each; exit status 2.** `--ignored` and `--include-ignored` are not exempt (parked bodies really run there, and one that ran counts). Exempt: `--list` (a note only), and cargo-nextest when `--exact` is also given (it runs one trial per process) | each sibling can only see its own process or file, and each runtime owns its own filter flag, so the scope and the wording differ. Refusal text is runner-specific and not parity-compared; the shared scenarios are in gnt's `features/honest-run.feature` and proven here in `tests/guards-proof.rs` |
| role of a failing step (0.12.0): one stderr line just before the failure propagates; what the step threw is rethrown untouched. Printed for any failure inside the step loop, an undefined step and an args-consumption refusal included, because that line is the only place gnt names the failing step. A cleanup-only failure prints `cleanup failed`. The token `role: Given\|When\|Then\|unresolved` and the resolution of And, But and `*` are shared | **a `  role: ...` line inside the failure text**, after the whole panic message and before the `in <file>` line; the first line stays byte-for-byte as 0.11.1 printed it. **None for an undefined step (its text already names the step), the ambiguity refusal, or a cleanup-only failure** | information only on both sides: no tool should count or discard a failure by it. The two differ where each runner already did, or did not, name the step |
| no analog in 0.12.0 | **the `harness` module**: `patience`, `wait_for`, `wait_for_some`, `absence_window`, `absent_for`, `absent_until`, `harness_error`, `Features::patience_default`, reading `ABDD_TIME_SCALE` and an optional file named by `ABDD_MACHINE_ENV`; a `cause: patience` / `cause: harness` line after the role line and one header line on stderr | harness helpers, not Gherkin: projects on this runner each wrote their own waiting code and got the same things wrong. gnt would ship them as a `./harness` export and use the same two cause words; until then this row stands |

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
| `no-scenarios` | error | a `Feature:` header with no scenarios under it — the file enforces nothing and reads as passing. The same parser refusal as `dialect`, under its own rule name: "add a scenario or delete the file" is a different remedy than "fix this line" |
| `no-then` | warn | a scenario whose steps never resolve to `Then` — runs code, asserts nothing (`And`/`But`/`*` inherit the preceding primary keyword, across a `Background`) |
| `vague-then` | warn | a `Then`-resolved step containing *works · correctly · properly · as expected · handles · appropriate* — words that assert nothing checkable. The finding also names the trap in the fix: rewording the step down to what its binding proves narrows the contract, so either strengthen the binding or record the narrowing as a ruling. |
| `single-row-outline` | warn | a `Scenario Outline` with one `Examples` row — a scenario with extra ceremony, and usually a missing case |
| `near-miss-keyword` | warn | a silently dropped line that was almost certainly meant as syntax: a wrong-case step keyword inside a scenario or `Background` body (`when I add 5`, `GIVEN a counter`), or — anywhere — a construct header that isn't the one exact form the parser recognizes (`scenario: b`, `Scenario : b`, `SCENARIO OUTLINE: b`). Prose trips it too when a Feature description wraps a sentence so that a line begins with a construct word and a colon (`…would pass every contract` / `scenario: the wrong session moved, …`); rewrap the line. |
| `dropped-prose` | warn | any *other* line inside a scenario, outline, or `Background` body that the parser dropped as narrative — and any non-tag, non-comment line *above* the `Feature:` header. The floor under `near-miss-keyword`: no dropped line goes unaccounted. The `Feature` narrative (below the header) stays exempt |
| `duplicate-title` | error | a `Scenario` or `Scenario Outline` title already used earlier in the file — compared pre-expansion, because two outlines sharing a title expand to **byte-identical** trial names (the `[n]` suffix indexes rows within one outline, not across outlines); a post-expansion backstop catches source-distinct collisions (`adds 1 [1]`). An error because the runner refuses it too: a failing trial (the `@only` mechanism, additive — both copies still run), since a duplicated title silently breaks `cargo test -- '<substring>'` focus |
| `unused-column` | warn | an `Examples` column no `<placeholder>` in the outline's title, steps, or step tables ever references — a case someone wrote down that no assertion consumes. Reported at the header row's line; a warn because a leading label column kept for the human reader is legitimate style |
| `strict-tag` | error (strict only) | a `@skip` or `@only` tag, under `lint_feature_strict`. Reviewed output carries no silent debt or focus. `@todo` is deliberately exempt — the stale-@todo inversion polices it at *run* time |

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

**Strict mode** is one bit: `lint_feature_strict(text, filename)` promotes
every warning to an error — same rule, same line, same message, nothing
removed or reworded — and adds the strict-only `strict-tag` rule. A
strict-clean file is clean in default mode by construction; there is no
relaxed mode. New rules enter only through the four admission tests of the
node sibling's `docs/lint-admission.md`, which binds this crate in lockstep.

Severity is descriptive, not policy: the wip-style debt register (filter by
rule) belongs in your guard test. Finding **text** is identical to
gherkin-node-test 0.9.0's `lintFeature` — the two linters are held together
differentially by `tools/parity` (byte-for-byte finding streams over shared
corpora and fuzzing), so a feature corpus linted here means the same thing
linted there. That is also why the `duplicate-title` message says
"name-filter selection" rather than naming either runner's flag: the message
is shared verbatim across siblings whose flags differ.

### From the command line: `gct-lint`

The crate also installs a command that runs the linter over your files:

```
cargo install gherkin-cargo-test
gct-lint                      # lints ./features
gct-lint features/ extra.feature
gct-lint --strict features/   # warnings become errors, plus strict-tag
```

It prints one line per finding on stdout, in the same form as the guard test
below:

```
features/counter.feature:34: warn[no-then]: Scenario "skipped scenarios do not run" has no Then step — it runs code but asserts nothing
```

A summary (`gct-lint: 3 files, 0 errors, 1 warning`) goes to stderr, so
stdout holds only findings.

- **Paths.** Each path is a `.feature` file or a directory. Directories are
  walked recursively in sorted order, skipping hidden directories,
  `target/` and `node_modules/`. Symlinked directories are not followed. A
  file you name is linted whatever its extension and printed exactly as
  typed. A walked file is printed as the argument you typed, then `/`, then
  its path below it, with no other normalization (`./features` stays
  `./features`).
- **Exit status.** `0` when no finding is an error, so warnings alone pass.
  `1` on any error finding, an unreadable path, or a directory with no
  `.feature` files: a typo'd path never reads as a pass. `2` on an unknown
  option.
- **Strict.** `--strict` is the one bit that makes warnings fail. It runs
  `lint_feature_strict`, nothing else changes.
- **One dialect line per file.** The parser stops at the first `dialect` or
  `no-scenarios` fault, so a broken file prints one line, not every fault in
  it.
- **No allow list.** Known debt belongs in a guard test's `ALLOW` register,
  next to the test that enforces it.

The command is a thin shell over `lint_feature`: it adds file handling and
an exit code, never different findings. The library function still never
touches the filesystem. The node sibling has no command yet, but this
contract was agreed with it so that a later `gnt-lint` would print the same
stdout for the same tree.

### A guard test to paste

To make `cargo test` itself fail on lint findings, with a register for known
debt, paste this into `tests/feature_lint.rs`. It needs nothing beyond this
crate:

```rust
// tests/feature_lint.rs — every .feature file in FEATURES lints clean.
use gherkin_cargo_test::lint_feature; // or lint_feature_strict
use std::{fs, path::Path};

const FEATURES: &str = "features"; // relative to the crate root

// Known debt, by (file, rule) — e.g. ("features/legacy.feature", "no-then").
// An entry that no longer fires fails the test, so the list can only shrink.
const ALLOW: &[(&str, &str)] = &[];

#[test]
fn feature_files_lint_clean() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files: Vec<_> = fs::read_dir(root.join(FEATURES))
        .expect("read the features directory")
        .map(|e| e.expect("read a directory entry").path())
        .filter(|p| p.extension().is_some_and(|x| x == "feature"))
        .collect();
    files.sort();
    assert!(!files.is_empty(), "no .feature files in {FEATURES}/");

    let (mut problems, mut used) = (Vec::new(), Vec::new());
    for path in &files {
        let rel = path.strip_prefix(root).unwrap_or(path);
        let name = rel.display().to_string().replace('\\', "/"); // keys match on Windows too
        let text = fs::read_to_string(path).expect("read a .feature file");
        for f in lint_feature(&text, &name) {
            let (sev, rule) = (f.severity.as_str(), f.rule.as_str());
            if ALLOW.contains(&(name.as_str(), rule)) {
                used.push((name.clone(), rule));
            } else {
                problems.push(format!("{name}:{}: {sev}[{rule}]: {}", f.line, f.message));
            }
        }
    }
    for (file, rule) in ALLOW {
        if !used.iter().any(|(f, r)| f == file && r == rule) {
            problems.push(format!(
                "stale ALLOW entry ({file:?}, {rule:?}) — delete it"
            ));
        }
    }
    assert!(problems.is_empty(), "\n{}\n", problems.join("\n"));
}
```

Run it with `cargo test --test feature_lint`. A failure lists every finding
as `file:line: severity[rule]: message`:

```
features/counter.feature:34: warn[no-then]: Scenario "skipped scenarios do not run" has no Then step — it runs code but asserts nothing
```

Every finding fails the test, warnings included. A warning you have decided
to keep goes in `ALLOW`. The list works per file, like the runner's
`.wip()`. Swap in `lint_feature_strict` to also reject committed `@skip` and
`@only` tags. The walk is flat, like the runner's discovery: for a second
directory (a `features/design/` tier, say), copy the test and change
`FEATURES`.

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
| `Features::new(dir).feature(base, definer).wip(base).run()` | **high-level runner**: discover every `.feature`, scoped registries, typed worlds, guard trials; `.manifest(path)` opts into the [run manifest](#the-run-manifest) |
| `Features::build_trials()` | the same trials without running them (the guards are testable — see `tests/guards-proof.rs`) |
| `Features::run_with(&Arguments)` | run under constructed `libtest_mimic::Arguments` and return a `RunReport` instead of exiting; applies the [name-filter guard](#running-one-scenario-by-name) (what `run()` does after parsing `argv`) |
| `RunReport` | `{ conclusion, refusal, note }`: libtest-mimic's counts, the filter guard's refusal (`Some` means fail the run whatever the counts), and a non-failing note (a `--list` filter that lists nothing) |
| `parse_feature(text, filename)` | parse → `ParsedFeature` (incl. `outlines: Vec<OutlineMeta>`); `Err(GherkinSyntaxError)` on unsupported/malformed syntax |
| `lint_feature(text, filename)` | **linter**: dialect gate + spec lints as `Vec<LintFinding>` — pure text-in/findings-out, for use under another runner |
| `gct-lint [--strict] [PATH]...` | **command**: the linter over files and directories, one `path:line: severity[rule]: message` line per finding; exits 1 on any error finding (see [From the command line](#from-the-command-line-gct-lint)) |
| `StepRegistry<W>` | `.define(regex_src, fn)` / `.define_exact(text, fn)` / `.find(text)` / `.matching(text)` |
| `execute_steps(steps, &registry, world)` | run a flat step list against a world (LIFO `defer` cleanup included) |
| `check_bindings(&parsed, &registry, base, wip)` | the pure binding guard (ambiguity + unbound-step ratchet) |
| `Ctx<W>` | step context: `.world` + `.defer(f)` |
| `DataTable` | cucumber-compatible step table: `raw` / `rows` / `hashes` / `rows_hash` / `transpose` |
| `build_snippet(text)` | paste-ready step definition for an unbound step (body panics). One known edge: step text containing `"#` would break the emitted `r#"…"#` literal — write that regex by hand |
| `GherkinSyntaxError` | parse error; carries `.line`, displays as `file:line: message` |
| `Features::patience_default(f)` | the project's default patience factor (0.01 to 1000; set once per process) |
| `harness::patience(base)` / `harness::absence_window(base)` | the limits: `base × default × ABDD_TIME_SCALE`, and `base × max(1, ABDD_TIME_SCALE)` (see [Waiting](#waiting)) |
| `harness::wait_for(what, base, predicate)` / `harness::wait_for_some(what, base, probe) -> T` | poll (up to 100 ms apart) until a poll sees it; on expiry fail with `cause: patience` |
| `harness::absent_for(what, base, forbidden)` / `harness::absent_until(what, forbidden, sentinel_what, base, sentinel)` | absence checks: a whole window, or until a later event |
| `harness::harness_error(message) -> !` | fail a step as the harness's fault, with `cause: harness` |

There is also a corpus checker for evaluating an existing feature suite
against this grammar before porting a project:

```sh
cargo run --example parse -- path/to/features/*.feature
```

A failing step's failure (from `execute_steps`, and so from every scenario
trial) starts with ``step `<Keyword> <text>` (feature line <N>) failed: <panic message>``
and then names the step's role on a line of its own, `  role: Given`,
`  role: When` or `  role: Then`. `And`, `But` and `*` take the role of the
nearest earlier Given/When/Then in the executed list (Background steps first);
with none, the line reads `  role: unresolved`. A step that failed through a
`harness` wait running out, or through `harness_error`, then gets
`  cause: patience` or `  cause: harness`; the mark is matched by message text, so another failure gets none unless it repeats a caught marked panic's exact text.
A cleanup failure has no step and no role line. The role is information for whoever reads a red. No tool
should count or discard a failure by it: a real catch can fail in a When (the
step asserts on the action's result), and a load-caused failure can look like
an ordinary Then assertion.

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
