// Executed proof of every runner guard — the Rust analog of gherkin-node-test's
// subprocess tests. Each case builds a Features runner over a fixture dir,
// runs it through libtest-mimic IN-PROCESS, and asserts the Conclusion counts:
// guards must actually fire (not merely exist), @skip must not run, @todo must
// not gate, a parse error must fail loudly without silencing sibling features,
// and deferred cleanup must run even when a step fails.
//
// `Features::run()` itself (what it prints, how it exits) is proven through a
// real process: the binary re-executes itself in child mode (see
// `child_mode`) and the parent asserts on the child's exit code and output.
//
// harness = false: main() prints one line per case and exits nonzero if any
// expectation does not hold.

use std::process::exit;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use gherkin_cargo_test::{harness, Features, RunReport, StepRegistry};
use libtest_mimic::{Arguments, Conclusion};

#[derive(Default)]
struct World {
    value: i64,
}

fn value_steps(reg: &mut StepRegistry<World>) {
    reg.define(r"^a value of (\d+)$", |ctx, args, _| {
        ctx.world.value = args[0].parse().expect("integer");
    });
    reg.define(r"^the value is (\d+)$", |ctx, args, _| {
        assert_eq!(ctx.world.value, args[0].parse::<i64>().expect("integer"));
    });
}

fn run(f: Features) -> Conclusion {
    let args = Arguments {
        test_threads: Some(1),
        ..Default::default()
    };
    libtest_mimic::run(&args, f.build_trials())
}

/// Like `run`, but with `--include-ignored`: parked (ignored) trials execute.
fn run_include_ignored(f: Features) -> Conclusion {
    let args = Arguments {
        test_threads: Some(1),
        include_ignored: true,
        ..Default::default()
    };
    libtest_mimic::run(&args, f.build_trials())
}

static DEFER_RAN: AtomicBool = AtomicBool::new(false);

fn check(
    failures: &mut Vec<String>,
    name: &str,
    c: Conclusion,
    passed: u64,
    failed: u64,
    ignored: u64,
) {
    let got = (c.num_passed, c.num_failed, c.num_ignored);
    if got == (passed, failed, ignored) {
        println!("guard-proof: {name} ok {got:?}");
    } else {
        failures.push(format!(
            "{name}: expected (passed, failed, ignored) = ({passed}, {failed}, {ignored}), got {got:?}"
        ));
    }
}

/// Arguments for one filter case: a name filter, single-threaded.
fn filtered(filter: &str) -> Arguments {
    Arguments {
        filter: Some(filter.to_string()),
        test_threads: Some(1),
        ..Default::default()
    }
}

/// Run through `Features::run_with`, which applies the filter guard.
fn guarded(f: Features, args: &Arguments) -> RunReport {
    f.run_with(args)
}

/// One printed line per case; every failed expectation is collected.
fn expect(failures: &mut Vec<String>, case: &str, what: &str, ok: bool) {
    if ok {
        println!("guard-proof: {case} ok: {what}");
    } else {
        failures.push(format!("{case}: expected {what}"));
    }
}

fn expect_counts(failures: &mut Vec<String>, case: &str, r: &RunReport, p: u64, f: u64, i: u64) {
    let got = (
        r.conclusion.num_passed,
        r.conclusion.num_failed,
        r.conclusion.num_ignored,
    );
    expect(
        failures,
        case,
        &format!("(passed, failed, ignored) = ({p}, {f}, {i}), got {got:?}"),
        got == (p, f, i),
    );
}

/// Sets an environment variable for a scope and restores the previous value
/// on drop, even if a case panics.
struct EnvGuard {
    key: &'static str,
    old: Option<std::ffi::OsString>,
}

impl EnvGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let old = std::env::var_os(key);
        std::env::set_var(key, value);
        EnvGuard { key, old }
    }

    fn remove(key: &'static str) -> Self {
        let old = std::env::var_os(key);
        std::env::remove_var(key);
        EnvGuard { key, old }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match &self.old {
            Some(v) => std::env::set_var(self.key, v),
            None => std::env::remove_var(self.key),
        }
    }
}

fn good() -> Features {
    Features::new("tests/fixtures/good").feature("counter", value_steps)
}

fn skip_fixture() -> Features {
    Features::new("tests/fixtures/skip").feature("skip", |reg: &mut StepRegistry<World>| {
        reg.define_exact("a step that panics", |_, _, _| {
            panic!("@skip scenario executed");
        });
    })
}

fn wip_unbound() -> Features {
    Features::new("tests/fixtures/unbound")
        .feature("pipeline", |reg: &mut StepRegistry<World>| {
            reg.define_exact("a bound step", |_, _, _| {});
        })
        .wip("pipeline")
}

fn failing_scenario() -> Features {
    Features::new("tests/fixtures/defer").feature("defer", |reg: &mut StepRegistry<World>| {
        reg.define_exact("cleanup is registered", |_, _, _| {});
        reg.define_exact("this step fails on purpose", |_, _, _| {
            panic!("intentional failure");
        });
    })
}

/// The @skip fixture with a definer that PASSES: a parked body that, once
/// selected by `--ignored` / `--include-ignored`, runs green. Its outcome is
/// decided by the counter alone — no failure can stand in for it.
fn skip_passing() -> Features {
    Features::new("tests/fixtures/skip").feature("skip", |reg: &mut StepRegistry<World>| {
        reg.define_exact("a step that panics", |_, _, _| {});
    })
}

fn unbound_no_wip() -> Features {
    Features::new("tests/fixtures/unbound").feature("pipeline", |reg: &mut StepRegistry<World>| {
        reg.define_exact("a bound step", |_, _, _| {});
    })
}

fn todo_fixture() -> Features {
    Features::new("tests/fixtures/todo").feature("todo", |reg: &mut StepRegistry<World>| {
        value_steps(reg);
        reg.define_exact("a step that panics", |_, _, _| {
            panic!("intentional @todo failure");
        });
    })
}

fn ambiguous_fixture() -> Features {
    Features::new("tests/fixtures/ambiguous").feature("amb", |reg: &mut StepRegistry<World>| {
        reg.define(r"^a doubly matched step$", |_, _, _| {});
        reg.define(r"doubly matched", |_, _, _| {});
    })
}

/// Arguments with `--skip` patterns and no filter.
fn skipping(patterns: &[&str]) -> Arguments {
    Arguments {
        skip: patterns.iter().map(|s| s.to_string()).collect(),
        test_threads: Some(1),
        ..Default::default()
    }
}

/// A refusal is present and starts with `prefix` (the subject sentence that
/// names what was given), and contains each of `parts`.
fn expect_refusal(
    failures: &mut Vec<String>,
    case: &str,
    r: &RunReport,
    prefix: &str,
    parts: &[&str],
) {
    let msg = r.refusal.clone().unwrap_or_default();
    expect(
        failures,
        case,
        &format!("a refusal starting {prefix:?}, got {:?}", r.refusal),
        msg.starts_with(prefix),
    );
    for part in parts {
        expect(
            failures,
            case,
            &format!("the refusal contains {part:?}, got {msg:?}"),
            msg.contains(part),
        );
    }
}

fn expect_no_refusal(failures: &mut Vec<String>, case: &str, r: &RunReport) {
    expect(
        failures,
        case,
        &format!("no refusal, got {:?}", r.refusal),
        r.refusal.is_none(),
    );
}

/// A feature file's path as the runner shows it: the directory exactly as
/// given to `Features::new`, then the platform separator, then the file name
/// (`Path::join`, so `\` on Windows).
fn feature_path(dir: &str, file: &str) -> String {
    std::path::Path::new(dir).join(file).display().to_string()
}

const GOOD_DIR: &str = "tests/fixtures/good";
const CHECK_TRIAL: &str = "counter :: step definitions are complete and unambiguous";

/// The filter guard: a selection (name filter and/or `--skip`) under which no
/// scenario ran fails the run.
fn filter_guard_cases(failures: &mut Vec<String>) {
    // This binary may itself run under nextest; the cases below set the
    // variables they test and nothing else may leak in.
    let _clear1 = EnvGuard::remove("NEXTEST");
    let _clear2 = EnvGuard::remove("NEXTEST_EXECUTION_MODE");

    // No selection: nothing to guard, nothing to say.
    let r = guarded(
        good(),
        &Arguments {
            test_threads: Some(1),
            ..Default::default()
        },
    );
    expect_counts(failures, "filter: none", &r, 3, 0, 0);
    expect_no_refusal(failures, "filter: none", &r);
    expect(failures, "filter: none", "no note", r.note.is_none());

    // No selection and every scenario parked: no scenario ran, but nothing
    // was selected, so there is nothing to refuse.
    for (name, build) in [
        ("wip-unbound", wip_unbound as fn() -> Features),
        ("skip", skip_fixture as fn() -> Features),
    ] {
        let case = format!("filter: none, all parked ({name})");
        let r = guarded(
            build(),
            &Arguments {
                test_threads: Some(1),
                ..Default::default()
            },
        );
        expect(
            failures,
            &case,
            "the scenario stayed parked and nothing failed",
            r.conclusion.num_ignored == 1 && r.conclusion.num_failed == 0,
        );
        expect_no_refusal(failures, &case, &r);
    }

    // A filter matching the scenario: it runs, no refusal.
    let r = guarded(good(), &filtered("bound and passing"));
    expect_counts(failures, "filter: matches scenario", &r, 1, 0, 0);
    expect_no_refusal(failures, "filter: matches scenario", &r);

    // A filter matching only a @todo scenario: its body runs (the declared
    // failure is tolerated), so that is a run.
    let r = guarded(todo_fixture(), &filtered("fails but is tolerated"));
    expect_counts(failures, "filter: only @todo", &r, 1, 0, 0);
    expect_no_refusal(failures, "filter: only @todo", &r);

    // A mistyped filter: nothing ran, refused, naming filter and directory.
    let r = guarded(good(), &filtered("Countr typo"));
    expect_counts(failures, "filter: typo", &r, 0, 0, 0);
    expect_refusal(
        failures,
        "filter: typo",
        &r,
        "gherkin-cargo-test: the filter \"Countr typo\" ran no scenario in tests/fixtures/good;",
        &["cargo test --test <target> -- \"Countr typo\""],
    );

    // The case the guard exists for: the filter matches only the runner's own
    // check trials. They pass, and the run is refused all the same.
    let r = guarded(good(), &filtered("step definitions"));
    expect_counts(failures, "filter: checks only", &r, 1, 0, 0);
    expect_refusal(
        failures,
        "filter: checks only",
        &r,
        "gherkin-cargo-test: the filter \"step definitions\" ran no scenario in tests/fixtures/good;",
        &[],
    );

    // A real failure outranks the refusal: a filter matching only a FAILING
    // check trial (no definer, so the binding guard fails) ran no scenario,
    // yet the run is red for a named reason — no refusal on top.
    let r = guarded(Features::new(GOOD_DIR), &filtered("step definitions"));
    expect_counts(failures, "filter: failing check only", &r, 0, 1, 0);
    expect_no_refusal(failures, "filter: failing check only", &r);

    // An ambiguous scenario is a scenario trial: it runs and fails.
    let r = guarded(ambiguous_fixture(), &filtered("two definitions"));
    expect_counts(failures, "filter: only ambiguous", &r, 0, 1, 0);
    expect_no_refusal(failures, "filter: only ambiguous", &r);

    // Parked scenarios: @skip, unbound under the wip register, and unbound
    // with no wip entry (the filter leaves the failing binding guard out).
    for (name, build, filter, dir) in [
        (
            "skip",
            skip_fixture as fn() -> Features,
            "would explode",
            "tests/fixtures/skip",
        ),
        (
            "wip-unbound",
            wip_unbound as fn() -> Features,
            "one step is not bound",
            "tests/fixtures/unbound",
        ),
        (
            "unbound, no wip",
            unbound_no_wip as fn() -> Features,
            "one step is not bound",
            "tests/fixtures/unbound",
        ),
    ] {
        let case = format!("filter: only ignored ({name})");
        let r = guarded(build(), &filtered(filter));
        expect_counts(failures, &case, &r, 0, 0, 1);
        expect_refusal(
            failures,
            &case,
            &r,
            &format!(
                "gherkin-cargo-test: the filter \"{filter}\" matched only ignored scenarios in {dir}; nothing ran"
            ),
            &[],
        );
    }

    // A filter that matches nothing, over fixtures whose only scenario is
    // parked, without --ignored: the parked scenario was not selected, so
    // the wording is "ran no scenario", never "matched only ignored". The
    // ignored-only wording must come from what the filter matched, not from
    // parked scenarios existing.
    for (name, build, dir) in [
        (
            "skip",
            skip_fixture as fn() -> Features,
            "tests/fixtures/skip",
        ),
        (
            "wip-unbound",
            wip_unbound as fn() -> Features,
            "tests/fixtures/unbound",
        ),
    ] {
        let case = format!("filter: zzz over a parked-only fixture ({name})");
        let r = guarded(build(), &filtered("zzz"));
        expect_counts(failures, &case, &r, 0, 0, 0);
        expect_refusal(
            failures,
            &case,
            &r,
            &format!("gherkin-cargo-test: the filter \"zzz\" ran no scenario in {dir};"),
            &[],
        );
        expect(
            failures,
            &case,
            &format!("no \"matched only ignored\" wording, got {:?}", r.refusal),
            !r.refusal
                .as_deref()
                .unwrap_or_default()
                .contains("matched only ignored"),
        );
    }

    // The kind label is part of the name the filter sees: "[skip]" selects
    // the parked trial.
    let r = guarded(skip_fixture(), &filtered("[skip]"));
    expect_counts(failures, "filter: kind label", &r, 0, 0, 1);
    expect_refusal(
        failures,
        "filter: kind label",
        &r,
        "gherkin-cargo-test: the filter \"[skip]\" matched only ignored scenarios",
        &[],
    );

    // --ignored / --include-ignored are no exemption. A parked body that is
    // selected really runs, and counts: no refusal. The definer passes, so
    // only the counter decides this outcome.
    for (flag, ignored, include_ignored) in
        [("ignored", true, false), ("include-ignored", false, true)]
    {
        let with = |filter: &str| Arguments {
            ignored,
            include_ignored,
            ..filtered(filter)
        };
        let case = format!("filter: --{flag} runs the parked scenario");
        let r = guarded(skip_passing(), &with("would explode"));
        expect(
            failures,
            &case,
            "the parked scenario ran and passed (num_ignored == 0, num_failed == 0, num_passed >= 1)",
            r.conclusion.num_ignored == 0
                && r.conclusion.num_failed == 0
                && r.conclusion.num_passed >= 1,
        );
        expect_no_refusal(failures, &case, &r);

        // ...and a filter that runs nothing is refused, as "ran no scenario":
        // nothing was left parked.
        let case = format!("filter: --{flag} zzz");
        let r = guarded(skip_passing(), &with("zzz"));
        expect_counts(failures, &case, &r, 0, 0, 0);
        expect_refusal(
            failures,
            &case,
            &r,
            "gherkin-cargo-test: the filter \"zzz\" ran no scenario in tests/fixtures/skip;",
            &[],
        );
    }
    // --ignored with a filter matching only a plain scenario: --ignored runs
    // only ignored trials, so nothing ran — refused.
    let r = guarded(
        good(),
        &Arguments {
            ignored: true,
            ..filtered("bound and passing")
        },
    );
    expect_counts(failures, "filter: --ignored, plain scenario", &r, 0, 0, 0);
    expect_refusal(
        failures,
        "filter: --ignored, plain scenario",
        &r,
        "gherkin-cargo-test: the filter \"bound and passing\" ran no scenario in tests/fixtures/good;",
        &[],
    );

    // --skip is a selection too.
    let r = guarded(good(), &skipping(&[" :: "]));
    expect_counts(failures, "skip: deselects every scenario", &r, 1, 0, 0);
    expect_refusal(
        failures,
        "skip: deselects every scenario",
        &r,
        "gherkin-cargo-test: the selection --skip \" :: \" ran no scenario in tests/fixtures/good;",
        &["cargo test --test <target> -- --skip \" :: \""],
    );
    let r = guarded(good(), &skipping(&["step definers", "step definitions"]));
    expect_counts(failures, "skip: leaves the scenario", &r, 1, 0, 0);
    expect_no_refusal(failures, "skip: leaves the scenario", &r);
    // Two skips, both named in order.
    let r = guarded(good(), &skipping(&["Good", "step"]));
    expect_refusal(
        failures,
        "skip: two patterns",
        &r,
        "gherkin-cargo-test: the selection --skip \"Good\" --skip \"step\" ran no scenario in tests/fixtures/good;",
        &[],
    );
    // A filter AND a skip: both named.
    let r = guarded(
        good(),
        &Arguments {
            skip: vec!["passing".into()],
            ..filtered("Good")
        },
    );
    expect_counts(failures, "skip: with a filter", &r, 0, 0, 0);
    expect_refusal(
        failures,
        "skip: with a filter",
        &r,
        "gherkin-cargo-test: the filter \"Good\" with --skip \"passing\" ran no scenario in tests/fixtures/good;",
        &["cargo test --test <target> -- \"Good\" --skip \"passing\""],
    );
    // A skip that leaves only parked scenarios.
    let r = guarded(skip_fixture(), &skipping(&["step def"]));
    expect_counts(failures, "skip: only ignored left", &r, 0, 0, 1);
    expect_refusal(
        failures,
        "skip: only ignored left",
        &r,
        "gherkin-cargo-test: the selection --skip \"step def\" matched only ignored scenarios in tests/fixtures/skip; nothing ran",
        &[],
    );

    // --list: a selection that lists nothing gets a note, never a refusal.
    let r = guarded(
        good(),
        &Arguments {
            list: true,
            ..filtered("zzz")
        },
    );
    expect(
        failures,
        "filter: --list lists nothing",
        &format!("a note naming the filter, got {:?}", r.note),
        r.note.as_deref()
            == Some("gherkin-cargo-test: the filter \"zzz\" lists nothing in tests/fixtures/good"),
    );
    expect_no_refusal(failures, "filter: --list lists nothing", &r);
    let r = guarded(
        good(),
        &Arguments {
            list: true,
            ..skipping(&[""])
        },
    );
    expect(
        failures,
        "skip: --list lists nothing",
        &format!("a note naming the skip, got {:?}", r.note),
        r.note.as_deref()
            == Some("gherkin-cargo-test: the selection --skip \"\" lists nothing in tests/fixtures/good"),
    );
    expect_no_refusal(failures, "skip: --list lists nothing", &r);
    let r = guarded(
        good(),
        &Arguments {
            list: true,
            ..filtered("bound and passing")
        },
    );
    expect(
        failures,
        "filter: --list lists something",
        "no note",
        r.note.is_none(),
    );
    expect_no_refusal(failures, "filter: --list lists something", &r);

    // --exact with the full trial name ("{feature} :: {scenario}").
    let r = guarded(
        good(),
        &Arguments {
            exact: true,
            ..filtered("Good :: bound and passing")
        },
    );
    expect_counts(failures, "filter: --exact", &r, 1, 0, 0);
    expect_no_refusal(failures, "filter: --exact", &r);
    // And a non-matching exact name still refuses (so the case above is not vacuous).
    let r = guarded(
        good(),
        &Arguments {
            exact: true,
            ..filtered("bound and passing")
        },
    );
    expect(
        failures,
        "filter: --exact partial name",
        "a refusal (exact needs the full name)",
        r.refusal.is_some(),
    );

    // A failing scenario under a matching filter: red by counts, and no
    // filter refusal, because a scenario did run.
    let r = guarded(failing_scenario(), &filtered("cleanup runs even"));
    expect_counts(failures, "filter: failing scenario", &r, 0, 1, 0);
    expect_no_refusal(failures, "filter: failing scenario", &r);

    // nextest: one trial per process, always with --exact, so "no scenario
    // ran" is normal there. Each variable alone, with --exact, exempts an
    // exact filter naming only a (passing) check trial...
    let exact_check = Arguments {
        exact: true,
        ..filtered(CHECK_TRIAL)
    };
    for (var, value) in [
        ("NEXTEST", "1"),
        ("NEXTEST_EXECUTION_MODE", "process-per-test"),
    ] {
        {
            let _set = EnvGuard::set(var, value);
            let r = guarded(good(), &exact_check);
            let case = format!("filter: {var} with --exact");
            expect_counts(failures, &case, &r, 1, 0, 0);
            expect_no_refusal(failures, &case, &r);

            // ...and WITHOUT --exact a leaked variable exempts nothing.
            let r = guarded(good(), &filtered("step definitions"));
            let case = format!("filter: {var} without --exact");
            expect_counts(failures, &case, &r, 1, 0, 0);
            expect_refusal(
                failures,
                &case,
                &r,
                "gherkin-cargo-test: the filter \"step definitions\" ran no scenario",
                &[],
            );
        }
    }
    // Restored: the same exact filter is refused once the variables are unset.
    let r = guarded(good(), &exact_check);
    expect(
        failures,
        "filter: nextest restored",
        "refusal is back once the variables are unset",
        r.refusal.is_some(),
    );
}

// --- Through a real process -------------------------------------------------
//
// `Features::run()` prints and exits, so it is proven by re-executing this
// binary as a child. Child mode needs BOTH the case name and a token file the
// parent created for this run (`<case>:<token path>`); a variable leaked into
// an ordinary run names a token file that no longer exists, and the binary
// fails loudly instead of quietly proving nothing.

const CHILD_VAR: &str = "GCT_GUARD_PROOF_CHILD";

/// If this process is a child, run its fixture through `Features::run()`
/// (which never returns).
fn child_mode() {
    let Some(value) = std::env::var_os(CHILD_VAR) else {
        return;
    };
    let value = value.to_string_lossy().into_owned();
    let (case, token) = value.split_once(':').unwrap_or((value.as_str(), ""));
    if token.is_empty() || !std::path::Path::new(token).is_file() {
        eprintln!(
            "guard-proof: {CHILD_VAR}={value:?} is set but no parent issued it (its token file is missing); unset {CHILD_VAR} and run again"
        );
        exit(3);
    }
    let features = match case {
        "good" => good(),
        "failing" => failing_scenario(),
        "rolediff" => Features::new("tests/fixtures/rolediff").feature("rolediff", value_steps),
        "good-default" => good().patience_default(3.0),
        "good-default-tenth" => good().patience_default(0.1),
        "limits-default" => {
            // The public limits under a project default of 3, printed for the
            // parent; no run.
            let _ = good().patience_default(3.0);
            let base = Duration::from_millis(50);
            println!(
                "patience {:?} absence_window {:?}",
                harness::patience(base),
                harness::absence_window(base)
            );
            exit(0);
        }
        "patience" => Features::new("tests/fixtures/patience").feature("patience", patience_steps),
        other => {
            eprintln!("guard-proof: unknown child case {other:?}");
            exit(3);
        }
    };
    features.run()
}

struct ChildOutput {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

fn run_child(token: &std::path::Path, case: &str, args: &[&str]) -> ChildOutput {
    run_child_env(token, case, args, &[])
}

/// The child's command, in a controlled environment: every inherited
/// `ABDD_*` and `NEXTEST*` variable removed (the machine running the proof
/// may have an `ABDD_` configuration), then `env` set.
fn child_command(
    token: &std::path::Path,
    case: &str,
    args: &[&str],
    env: &[(&str, &str)],
) -> std::process::Command {
    let mut cmd = std::process::Command::new(std::env::current_exe().expect("current_exe"));
    cmd.args(args)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .env(CHILD_VAR, format!("{case}:{}", token.display()));
    for (k, _) in std::env::vars_os() {
        let k = k.to_string_lossy();
        if k.starts_with("ABDD_") || k.starts_with("NEXTEST") {
            cmd.env_remove(k.as_ref());
        }
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd
}

fn run_child_env(
    token: &std::path::Path,
    case: &str,
    args: &[&str],
    env: &[(&str, &str)],
) -> ChildOutput {
    let out = child_command(token, case, args, env)
        .output()
        .expect("spawn the child");
    ChildOutput {
        code: out.status.code(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

/// The child's stdout and stderr interleaved into one file, so the order of
/// the two streams can be asserted. Returns (exit code, combined text).
fn run_child_combined(
    token: &std::path::Path,
    case: &str,
    env: &[(&str, &str)],
) -> (Option<i32>, String) {
    let path = token.with_extension(format!("{case}.combined"));
    let file = std::fs::File::create(&path).expect("create the combined output file");
    let status = child_command(token, case, &[], env)
        .stdout(file.try_clone().expect("clone the output file"))
        .stderr(file)
        .status()
        .expect("spawn the child");
    let text = std::fs::read_to_string(&path).expect("read the combined output");
    let _ = std::fs::remove_file(&path);
    (status.code(), text)
}

/// `expect`, with the child's output added to the failure only.
fn expect_shown(failures: &mut Vec<String>, case: &str, shown: &str, what: &str, ok: bool) {
    if ok {
        println!("guard-proof: {case} ok: {what}");
    } else {
        failures.push(format!("{case}: expected {what}; {shown}"));
    }
}

/// What a failing step's failure looks like in the real runner's output: the
/// first line as 0.11.1 formatted it (built from its parts, not copied), the
/// assertion's own message intact and in order, the role line after it all,
/// and the trial's status line still ending in FAILED.
fn role_line_cases(failures: &mut Vec<String>, token: &std::path::Path) {
    let out = run_child(token, "rolediff", &[]);
    let shown = format!("stdout {:?}, stderr {:?}", out.stdout, out.stderr);
    let case = "process: step failure role";
    // 0.11.1's first line: keyword, text, line, then the panic message.
    let (kw, text, line) = ("Then", "the value is 2", 4);
    let first = format!(
        "step `{kw} {text}` (feature line {line}) failed: assertion `left == right` failed"
    );
    let at = out.stdout.find(&first);
    expect_shown(failures, case, &shown, "the exact first line", at.is_some());
    let Some(at) = at else { return };
    let block = &out.stdout[at..];
    // The assert_eq message, line by line, in order.
    let mut pos = first.len();
    for part in ["\n  left: 1", "\n right: 2"] {
        let found = block[pos..].find(part);
        expect_shown(
            failures,
            case,
            &shown,
            &format!("message part {part:?} in order"),
            found.is_some(),
        );
        let Some(f) = found else { return };
        pos += f + part.len();
    }
    let role_at = block[pos..].find("\n  role: Then");
    expect_shown(
        failures,
        case,
        &shown,
        "the role line after the whole message",
        role_at.is_some(),
    );
    let Some(role_at) = role_at else { return };
    pos += role_at;
    let in_at = block[pos..].find(&format!(
        "\n  in {}",
        feature_path("tests/fixtures/rolediff", "rolediff.feature")
    ));
    expect_shown(
        failures,
        case,
        &shown,
        "the `  in <file>` line after the role line",
        in_at.is_some(),
    );
    expect_shown(
        failures,
        case,
        &shown,
        "a FAILED status line ending the trial line",
        out.stdout
            .lines()
            .any(|l| l.starts_with("test ") && l.contains("Rolediff") && l.ends_with("... FAILED")),
    );
    expect_shown(
        failures,
        case,
        &shown,
        "exit code 101",
        out.code == Some(101),
    );
}

fn patience_steps(reg: &mut StepRegistry<World>) {
    value_steps(reg);
    reg.define_exact("the light turns green", |_, _, _| {
        harness::wait_for("the light", Duration::from_millis(50), || false);
    });
}

/// libtest's timing, the one part of a run's output that varies.
fn normalize_timing(s: &str) -> String {
    let mut out = String::new();
    for line in s.split_inclusive('\n') {
        match line.find("; finished in ") {
            Some(at) => {
                out.push_str(&line[..at]);
                out.push_str("; finished in <t>s\n");
            }
            None => out.push_str(line),
        }
    }
    out
}

/// Variables set for one child, after the inherited `ABDD_*` are removed.
type ChildEnv<'a> = Vec<(&'a str, &'a str)>;

/// `harness`: the run header, settings errors, a scaled wait's expiry, and
/// a project that uses none of it printing exactly what it printed before.
fn waiting_cases(failures: &mut Vec<String>, token: &std::path::Path) {
    // A project using none of the module, with no ABDD_ setting: the output
    // of the `good` fixture as captured from this binary BEFORE the module
    // existed (0.11.1 plus the uncommitted role work), timing normalized.
    let case = "process: no settings, no header";
    // One thread: libtest prints trials in completion order, so the order
    // is fixed only when they run one at a time.
    let one_thread: &[&str] = &["--test-threads", "1"];
    let out = run_child(token, "good", one_thread);
    let shown = format!("stdout {:?}, stderr {:?}", out.stdout, out.stderr);
    let before = "\nrunning 3 tests\n\
test step definers and wip entries map only to existing feature files ... ok\n\
test counter :: step definitions are complete and unambiguous         ... ok\n\
test Good :: bound and passing                                        ... ok\n\
\n\
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in <t>s\n\
\n";
    expect_shown(
        failures,
        case,
        &shown,
        "stdout byte-identical to before (timing normalized)",
        normalize_timing(&out.stdout) == before,
    );
    expect_shown(
        failures,
        case,
        &shown,
        "an empty stderr",
        out.stderr.is_empty(),
    );
    expect_shown(failures, case, &shown, "exit code 0", out.code == Some(0));

    // Header cases: one line, on stderr, before the trials.
    let dir = std::env::temp_dir().join(format!("gct-guard-proof-env-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create the settings dir");
    // Lines that are not ABDD_ lines, malformed, with unbalanced quotes, or
    // holding a secret: skipped unread, and never in any output.
    const MARKER: &str = "sk-MARKER-0f1e2d";
    let others = format!(
        "API_KEY={MARKER}\nPASSWORD=\"{MARKER}\nthis is {MARKER} not a setting\nmy-var='{MARKER}\"\n"
    );
    let write = |name: &str, text: &str| {
        let path = dir.join(name);
        std::fs::write(&path, text).expect("write a settings file");
        path.display().to_string()
    };
    let good_file = write(
        "machine.env",
        &format!(
            "# a machine\nABDD_HEAVY_JOBS=4   # not read by the runner\nABDD_TIME_SCALE = \"2.5\"\n{others}"
        ),
    );
    let bad_file = write(
        "bad.env",
        &format!("{others}ABDD_TIME_SCALE=2\n\nABDD_BROKEN is not a setting\n{others}"),
    );
    let export_file = write("export.env", "export ABDD_TIME_SCALE=2\n");
    let bom_file = write("bom.env", "\u{feff}ABDD_TIME_SCALE=2\r\nOTHER=1\r\n");
    let hash_file = write(
        "hash.env",
        "ABDD_NOTE=\"slow # box = yes\" # a quoted # and =\nABDD_TIME_SCALE='2' # slow\n",
    );
    let empty_file = write("nothing.env", &format!("# sets nothing\n{others}"));
    let range_file = write("range.env", "ABDD_TIME_SCALE=1e-300\n");
    let missing_file = dir.join("missing.env").display().to_string();
    let from_file = |f: &str, line: usize| {
        format!("gherkin-cargo-test: patience x2 (default 1 x machine 2); ABDD_TIME_SCALE=2 ({f} line {line}); ABDD_MACHINE_ENV={f} (environment)")
    };

    // A blank value in the environment counts as unset: no header at all.
    for blank in ["", "   "] {
        let case = format!("process: a blank ABDD_ value is unset ({blank:?})");
        let out = run_child_env(
            token,
            "good",
            one_thread,
            &[("ABDD_TIME_SCALE", blank), ("ABDD_MACHINE_ENV", blank)],
        );
        let shown = format!("stdout {:?}, stderr {:?}", out.stdout, out.stderr);
        expect_shown(
            failures,
            &case,
            &shown,
            "an empty stderr, unchanged stdout, exit 0",
            out.stderr.is_empty() && normalize_timing(&out.stdout) == before && out.code == Some(0),
        );
    }

    let headers: Vec<(&str, &str, ChildEnv, String)> = vec![
        (
            "process: header from the project default",
            "good-default",
            vec![],
            "gherkin-cargo-test: patience x3 (default 3 x machine 1); ABDD_TIME_SCALE unset".into(),
        ),
        (
            "process: header from ABDD_TIME_SCALE",
            "good",
            vec![("ABDD_TIME_SCALE", "2")],
            "gherkin-cargo-test: patience x2 (default 1 x machine 2); ABDD_TIME_SCALE=2 (environment)".into(),
        ),
        (
            "process: header of 0.1 x 3, in ASCII to three places",
            "good-default-tenth",
            vec![("ABDD_TIME_SCALE", "3")],
            "gherkin-cargo-test: patience x0.3 (default 0.1 x machine 3); ABDD_TIME_SCALE=3 (environment)".into(),
        ),
        (
            "process: a padded environment value is trimmed",
            "good",
            vec![("ABDD_TIME_SCALE", " 2 ")],
            "gherkin-cargo-test: patience x2 (default 1 x machine 2); ABDD_TIME_SCALE=2 (environment)".into(),
        ),
        (
            "process: the scale at its lower end",
            "good",
            vec![("ABDD_TIME_SCALE", "0.01")],
            "gherkin-cargo-test: patience x0.01 (default 1 x machine 0.01); ABDD_TIME_SCALE=0.01 (environment)".into(),
        ),
        (
            "process: the scale at its upper end",
            "good",
            vec![("ABDD_TIME_SCALE", "1000")],
            "gherkin-cargo-test: patience x1000 (default 1 x machine 1000); ABDD_TIME_SCALE=1000 (environment)".into(),
        ),
        (
            "process: header from an ABDD_MACHINE_ENV file",
            "good",
            vec![("ABDD_MACHINE_ENV", good_file.as_str())],
            format!("gherkin-cargo-test: patience x2.5 (default 1 x machine 2.5); ABDD_TIME_SCALE=2.5 ({good_file} line 3); ABDD_MACHINE_ENV={good_file} (environment)"),
        ),
        (
            "process: the environment beats the file",
            "good",
            vec![("ABDD_MACHINE_ENV", good_file.as_str()), ("ABDD_TIME_SCALE", "2")],
            format!("gherkin-cargo-test: patience x2 (default 1 x machine 2); ABDD_TIME_SCALE=2 (environment); ABDD_MACHINE_ENV={good_file} (environment)"),
        ),
        (
            "process: an empty ABDD_TIME_SCALE falls through to the file",
            "good",
            vec![("ABDD_MACHINE_ENV", good_file.as_str()), ("ABDD_TIME_SCALE", "")],
            format!("gherkin-cargo-test: patience x2.5 (default 1 x machine 2.5); ABDD_TIME_SCALE=2.5 ({good_file} line 3); ABDD_MACHINE_ENV={good_file} (environment)"),
        ),
        (
            "process: export ABDD_TIME_SCALE=2",
            "good",
            vec![("ABDD_MACHINE_ENV", export_file.as_str())],
            from_file(&export_file, 1),
        ),
        (
            "process: a byte order mark and CRLF",
            "good",
            vec![("ABDD_MACHINE_ENV", bom_file.as_str())],
            from_file(&bom_file, 1),
        ),
        (
            "process: a quoted value holding # and =",
            "good",
            vec![("ABDD_MACHINE_ENV", hash_file.as_str())],
            from_file(&hash_file, 2),
        ),
        (
            "process: a file that sets nothing still prints the header",
            "good",
            vec![("ABDD_MACHINE_ENV", empty_file.as_str())],
            format!("gherkin-cargo-test: patience x1 (default 1 x machine 1); ABDD_TIME_SCALE unset; ABDD_MACHINE_ENV={empty_file} (environment)"),
        ),
    ];
    for (case, fixture, env, header) in &headers {
        let out = run_child_env(token, fixture, one_thread, env);
        let shown = format!("stdout {:?}, stderr {:?}", out.stdout, out.stderr);
        expect_shown(
            failures,
            case,
            &shown,
            "ASCII stderr with no settings-file text but ABDD_ lines",
            out.stderr.is_ascii() && !out.stderr.contains(MARKER) && !out.stdout.contains(MARKER),
        );
        expect_shown(
            failures,
            case,
            &shown,
            &format!("stderr is exactly the header {header:?}"),
            out.stderr == format!("{header}\n"),
        );
        expect_shown(
            failures,
            case,
            &shown,
            "stdout unchanged from before (timing normalized)",
            normalize_timing(&out.stdout) == before,
        );
        expect_shown(failures, case, &shown, "exit code 0", out.code == Some(0));
        let (code, text) = run_child_combined(token, fixture, env);
        expect_shown(
            failures,
            case,
            &format!("combined {text:?}"),
            "the header before the trials",
            code == Some(0)
                && text
                    .find(header.as_str())
                    .is_some_and(|h| text.find("running 3 tests").is_some_and(|r| h < r)),
        );
    }

    // Settings errors: stopped before any trial, exit 2, naming the
    // variable or the file and line.
    let errors: Vec<(&str, ChildEnv, String)> = vec![
        (
            "process: a bad ABDD_TIME_SCALE",
            vec![("ABDD_TIME_SCALE", "0")],
            "gherkin-cargo-test: settings error, no trial ran: ABDD_TIME_SCALE (environment) is \"0\"".into(),
        ),
        (
            "process: the scale just below its range",
            vec![("ABDD_TIME_SCALE", "0.0099")],
            "gherkin-cargo-test: settings error, no trial ran: ABDD_TIME_SCALE (environment) is \"0.0099\", which is not a time factor: it must be a number from 0.01 to 1000 (1.0 means no change)".into(),
        ),
        (
            "process: the scale just above its range",
            vec![("ABDD_TIME_SCALE", "1000.01")],
            "ABDD_TIME_SCALE (environment) is \"1000.01\", which is not a time factor: it must be a number from 0.01 to 1000".into(),
        ),
        (
            "process: a scale out of range in the file",
            vec![("ABDD_MACHINE_ENV", range_file.as_str())],
            format!("ABDD_TIME_SCALE ({range_file} line 1) is \"1e-300\", which is not a time factor: it must be a number from 0.01 to 1000"),
        ),
        (
            "process: a relative ABDD_MACHINE_ENV, even one naming a readable file",
            vec![("ABDD_MACHINE_ENV", "Cargo.toml")],
            "gherkin-cargo-test: settings error, no trial ran: ABDD_MACHINE_ENV is \"Cargo.toml\", a relative path; name the settings file by an absolute path".into(),
        ),
        (
            "process: a junk ABDD_TIME_SCALE",
            vec![("ABDD_TIME_SCALE", "fast")],
            "ABDD_TIME_SCALE (environment) is \"fast\"".into(),
        ),
        (
            "process: a malformed ABDD_MACHINE_ENV file",
            vec![("ABDD_MACHINE_ENV", bad_file.as_str())],
            format!("gherkin-cargo-test: settings error, no trial ran: {bad_file} line 7: ABDD_BROKEN: not a NAME=value line: \"ABDD_BROKEN is not a setting\"\n"),
        ),
        (
            "process: an unreadable ABDD_MACHINE_ENV file",
            vec![("ABDD_MACHINE_ENV", missing_file.as_str())],
            format!("ABDD_MACHINE_ENV names {missing_file}, which cannot be read"),
        ),
    ];
    for (case, env, says) in &errors {
        let out = run_child_env(token, "good", &[], env);
        let shown = format!("stdout {:?}, stderr {:?}", out.stdout, out.stderr);
        expect_shown(failures, case, &shown, "exit code 2", out.code == Some(2));
        expect_shown(
            failures,
            case,
            &shown,
            &format!("stderr contains {says:?}"),
            out.stderr.contains(says.as_str()),
        );
        expect_shown(
            failures,
            case,
            &shown,
            "no trial ran (empty stdout)",
            out.stdout.is_empty(),
        );
        expect_shown(
            failures,
            case,
            &shown,
            "no text of a non-ABDD_ line in stdout or stderr",
            !out.stderr.contains(MARKER) && !out.stdout.contains(MARKER),
        );
    }

    // The project default lengthens patience, never an absence window; the
    // machine scale lengthens both.
    for (env, want) in [
        (vec![], "patience 150ms absence_window 50ms\n"),
        (
            vec![("ABDD_TIME_SCALE", "2")],
            "patience 300ms absence_window 100ms\n",
        ),
        (
            vec![("ABDD_TIME_SCALE", "0.5")],
            "patience 75ms absence_window 50ms\n",
        ),
    ] {
        let case = format!("process: limits under a project default of 3, {env:?}");
        let out = run_child_env(token, "limits-default", &[], &env);
        let shown = format!("stdout {:?}, stderr {:?}", out.stdout, out.stderr);
        expect_shown(
            failures,
            &case,
            &shown,
            &format!("stdout {want:?}"),
            out.stdout == want && out.code == Some(0),
        );
    }

    // A real wait_for's limit, at the machine scale and doubled by it.
    for (scale, limit, source) in [
        (None, "50ms", "ABDD_TIME_SCALE unset"),
        (Some("2"), "100ms", "ABDD_TIME_SCALE=2 (environment)"),
    ] {
        let case = format!(
            "process: wait_for expiry at scale {}",
            scale.unwrap_or("unset")
        );
        let env: Vec<(&str, &str)> = scale.map(|v| ("ABDD_TIME_SCALE", v)).into_iter().collect();
        let out = run_child_env(token, "patience", &[], &env);
        let shown = format!("stdout {:?}, stderr {:?}", out.stdout, out.stderr);
        let says = format!(
            "limit {limit} = base 50ms x project default 1 x machine scale {} ({source})\n  role: When\n  cause: patience\n  in {}",
            scale.unwrap_or("1"),
            feature_path("tests/fixtures/patience", "patience.feature")
        );
        expect_shown(
            failures,
            &case,
            &shown,
            &format!("the expiry names {says:?}"),
            out.stdout.contains(&says),
        );
        expect_shown(
            failures,
            &case,
            &shown,
            "the first line names the step and what it waited for",
            out.stdout.contains(
                "step `When the light turns green` (feature line 4) failed: wait_for: gave up waiting for the light after ",
            ),
        );
        expect_shown(
            failures,
            &case,
            &shown,
            "the trial still reads FAILED and the run exits 101",
            out.code == Some(101)
                && out.stdout.lines().any(|l| {
                    l.starts_with("test Patience :: a light that never turns green")
                        && l.ends_with("... FAILED")
                }),
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

fn process_cases(failures: &mut Vec<String>) {
    let token = std::env::temp_dir().join(format!(
        "gct-guard-proof-child-{}.token",
        std::process::id()
    ));
    std::fs::write(&token, "guard-proof child token").expect("write the token file");

    // Each case: fixture, args, expected exit code, stderr must contain,
    // stdout must contain, whether refusal text must be absent from stderr.
    struct Case<'a> {
        name: &'a str,
        fixture: &'a str,
        args: &'a [&'a str],
        code: i32,
        stderr_has: &'a [&'a str],
        stdout_has: &'a [&'a str],
        stdout_lacks: &'a [&'a str],
        no_refusal: bool,
    }
    let cases = [
        Case {
            name: "mistyped filter",
            fixture: "good",
            args: &["Countr typo"],
            code: 101,
            stderr_has: &["the filter \"Countr typo\" ran no scenario in tests/fixtures/good;"],
            stdout_has: &[],
            stdout_lacks: &[],
            no_refusal: false,
        },
        Case {
            name: "checks only",
            fixture: "good",
            args: &["step definitions"],
            code: 101,
            stderr_has: &[
                "the filter \"step definitions\" ran no scenario in tests/fixtures/good;",
            ],
            stdout_has: &["test result: ok. 1 passed"],
            stdout_lacks: &[],
            no_refusal: false,
        },
        Case {
            name: "matching filter",
            fixture: "good",
            args: &["bound and passing"],
            code: 0,
            stderr_has: &[],
            stdout_has: &["test result: ok. 1 passed"],
            stdout_lacks: &[],
            no_refusal: true,
        },
        Case {
            name: "two filters",
            fixture: "good",
            args: &["beta", "alpha"],
            code: 2,
            stderr_has: &["2 name filters were given (\"beta\" and \"alpha\")"],
            stdout_has: &[],
            stdout_lacks: &["running", "test result", "test "],
            no_refusal: false,
        },
        Case {
            name: "two filters after --",
            fixture: "good",
            args: &["--", "--list", "zzz"],
            code: 2,
            stderr_has: &["2 name filters were given (\"--list\" and \"zzz\")"],
            stdout_has: &[],
            stdout_lacks: &["running", "test result", "test "],
            no_refusal: false,
        },
        Case {
            name: "--skip deselects every scenario",
            fixture: "good",
            args: &["--skip", " :: "],
            code: 101,
            stderr_has: &["the selection --skip \" :: \" ran no scenario in tests/fixtures/good;"],
            stdout_has: &["test result: ok. 1 passed"],
            stdout_lacks: &[],
            no_refusal: false,
        },
        Case {
            name: "--list lists nothing",
            fixture: "good",
            args: &["--list", "zzz"],
            code: 0,
            stderr_has: &[
                "gherkin-cargo-test: the filter \"zzz\" lists nothing in tests/fixtures/good",
            ],
            stdout_has: &[],
            stdout_lacks: &[],
            no_refusal: false,
        },
        Case {
            name: "--ignored zzz",
            fixture: "good",
            args: &["--ignored", "zzz"],
            code: 101,
            stderr_has: &["the filter \"zzz\" ran no scenario in tests/fixtures/good;"],
            stdout_has: &[],
            stdout_lacks: &[],
            no_refusal: false,
        },
        Case {
            name: "failing scenario",
            fixture: "failing",
            args: &["cleanup runs even"],
            code: 101,
            stderr_has: &[],
            stdout_has: &["1 failed"],
            stdout_lacks: &[],
            no_refusal: true,
        },
    ];
    for c in &cases {
        let case = format!("process: {}", c.name);
        let out = run_child(&token, c.fixture, c.args);
        let shown = format!("stdout {:?}, stderr {:?}", out.stdout, out.stderr);
        expect_shown(
            failures,
            &case,
            &shown,
            &format!("exit code {}, got {:?}", c.code, out.code),
            out.code == Some(c.code),
        );
        for s in c.stderr_has {
            expect_shown(
                failures,
                &case,
                &shown,
                &format!("stderr contains {s:?}"),
                out.stderr.contains(s),
            );
        }
        for s in c.stdout_has {
            expect_shown(
                failures,
                &case,
                &shown,
                &format!("stdout contains {s:?}"),
                out.stdout.contains(s),
            );
        }
        for s in c.stdout_lacks {
            expect_shown(
                failures,
                &case,
                &shown,
                &format!("stdout lacks {s:?}"),
                !out.stdout.contains(s),
            );
        }
        if c.no_refusal {
            expect_shown(
                failures,
                &case,
                &shown,
                "no refusal text on stderr",
                !out.stderr.contains("gherkin-cargo-test:"),
            );
        }
    }

    role_line_cases(failures, &token);
    waiting_cases(failures, &token);

    // A leaked child variable (its token file gone) does not enter child
    // mode quietly: the binary refuses, exit 3.
    let _ = std::fs::remove_file(&token);
    let out = run_child(&token, "good", &["bound and passing"]);
    expect(
        failures,
        "process: stale child variable",
        &format!(
            "exit 3 naming {CHILD_VAR}, got {:?} {:?}",
            out.code, out.stderr
        ),
        out.code == Some(3) && out.stderr.contains(CHILD_VAR),
    );
}

fn main() {
    child_mode();
    let mut failures: Vec<String> = Vec::new();

    // A fully bound feature: everything green, nothing ignored.
    check(
        &mut failures,
        "good",
        run(Features::new("tests/fixtures/good").feature("counter", value_steps)),
        3, // orphan guard + binding guard + 1 scenario
        0,
        0,
    );

    // The unused-definition guard (the wip ratchet's dual, node gh#5): one
    // definition no scenario consumes registers one failing trial naming it;
    // every other trial still passes — additive, the @only mechanism.
    check(
        &mut failures,
        "unused-definition",
        run(Features::new("tests/fixtures/good").feature(
            "counter",
            |reg: &mut StepRegistry<World>| {
                value_steps(reg);
                reg.define_exact("a definition nobody consumes", |_, _, _| {});
            },
        )),
        3, // orphan guard + binding guard + 1 scenario — all still pass
        1, // the consumer guard fails, naming the dead definition
        0,
    );

    // A wip-held scenario still consumes its bound steps — and the bound
    // consumer is deliberately the LAST step, behind an unbound one, so an
    // early-exit rewrite of the preflight could not have counted it
    // incidentally (the node sibling's mutation-verified lesson).
    check(
        &mut failures,
        "wip-consumer",
        run(Features::new("tests/fixtures/wipconsumer")
            .feature("gauges", |reg: &mut StepRegistry<World>| {
                reg.define(r"^the last gauge reads (\d+)$", |_, _, _| {});
            })
            .wip("gauges")),
        2, // orphan guard + (relaxed) binding guard — no consumer guard fires
        0,
        1, // the wip scenario itself stays parked (unbound first step)
    );

    // An unbound step: the binding guard FAILS (with a paste-ready snippet)
    // and the unbound scenario registers as ignored — never silently green.
    check(
        &mut failures,
        "unbound",
        run(Features::new("tests/fixtures/unbound").feature(
            "pipeline",
            |reg: &mut StepRegistry<World>| {
                reg.define_exact("a bound step", |_, _, _| {});
            },
        )),
        1, // orphan guard
        1, // binding guard fails
        1, // unbound scenario ignored
    );

    // The same feature marked .wip(): the ratchet is relaxed, guard passes.
    check(
        &mut failures,
        "unbound-wip",
        run(Features::new("tests/fixtures/unbound")
            .feature("pipeline", |reg: &mut StepRegistry<World>| {
                reg.define_exact("a bound step", |_, _, _| {});
            })
            .wip("pipeline")),
        2, // orphan guard + (relaxed) binding guard
        0,
        1, // unbound scenario still ignored
    );

    // --- Scenario-scoped wip + the wip register's own ratchet ----------------
    // .wip_scenarios(base, titles) holds open only the named scenarios (by
    // SOURCE title — an outline's title covers every expanded row) while the
    // rest of the feature keeps the full unbound-step ratchet. Both wip shapes
    // are ratcheted against rot: fully bound but still listed FAILS.

    let partial_defs = |reg: &mut StepRegistry<World>| {
        reg.define_exact("a bound step", |_, _, _| {});
    };

    // Covering exactly the pending constructs: green, the bound scenario stays
    // enforced, and the pending plain scenario + BOTH expanded rows are ignored.
    check(
        &mut failures,
        "scenario-wip",
        run(Features::new("tests/fixtures/partial")
            .feature("partial", partial_defs)
            .wip_scenarios("partial", ["pending thing", "pending sweep <k>"])),
        3, // orphan guard + binding guard + the enforced scenario
        0,
        3, // pending plain scenario + both expanded outline rows
    );

    // Covering only ONE pending construct: the uncovered outline's unbound
    // step fails the binding guard — the ratchet stays tight outside the entry.
    check(
        &mut failures,
        "scenario-wip-ratchet",
        run(Features::new("tests/fixtures/partial")
            .feature("partial", partial_defs)
            .wip_scenarios("partial", ["pending thing"])),
        2, // orphan guard + the enforced scenario
        1, // binding guard fails on the uncovered outline
        3,
    );

    // A title naming no Scenario/Scenario Outline: stranded, fails the guard.
    check(
        &mut failures,
        "scenario-wip-orphan-title",
        run(Features::new("tests/fixtures/partial")
            .feature("partial", partial_defs)
            .wip_scenarios(
                "partial",
                ["pending thing", "no such scenario", "pending sweep <k>"],
            )),
        2,
        1, // binding guard fails on the stranded title
        3,
    );

    // A fully bound scenario still listed: stale, fails the guard.
    check(
        &mut failures,
        "scenario-wip-stale",
        run(Features::new("tests/fixtures/partial")
            .feature("partial", partial_defs)
            .wip_scenarios("partial", ["ready", "pending thing", "pending sweep <k>"])),
        2,
        1, // binding guard fails: 'ready' is fully bound
        3,
    );

    // A fully bound FEATURE still marked .wip() whole: stale, fails the guard.
    check(
        &mut failures,
        "wip-stale",
        run(Features::new("tests/fixtures/good")
            .feature("counter", value_steps)
            .wip("counter")),
        2, // orphan guard + the scenario (which runs fine)
        1, // binding guard fails: nothing left unbound
        0,
    );

    // A wip basename naming no feature file: stranded, fails the orphan guard.
    check(
        &mut failures,
        "wip-orphan-base",
        run(Features::new("tests/fixtures/good")
            .feature("counter", value_steps)
            .wip("ghost")),
        2, // binding guard + the scenario
        1, // orphan guard fails
        0,
    );

    // `--include-ignored` executes the parked unbound placeholder: it must
    // FAIL with its reason, never pass vacuously (an Ok(()) placeholder body
    // would be a false green — the exact silence this crate exists to kill).
    check(
        &mut failures,
        "unbound-include-ignored",
        run_include_ignored(
            Features::new("tests/fixtures/unbound")
                .feature("pipeline", |reg: &mut StepRegistry<World>| {
                    reg.define_exact("a bound step", |_, _, _| {});
                })
                .wip("pipeline"),
        ),
        2, // orphan guard + (relaxed) binding guard
        1, // the unbound placeholder runs and fails loudly with its reason
        0,
    );

    // `--include-ignored` also force-runs @skip'd scenarios: their bodies are
    // the REAL steps (pinned here by the panicking step firing), so an
    // explicit override runs real code — skipped never means vacuous.
    check(
        &mut failures,
        "skip-include-ignored",
        run_include_ignored(Features::new("tests/fixtures/skip").feature(
            "skip",
            |reg: &mut StepRegistry<World>| {
                reg.define_exact("a step that panics", |_, _, _| {
                    panic!("@skip scenario executed — expected under --include-ignored");
                });
            },
        )),
        2, // orphan guard + binding guard
        1, // the force-run @skip scenario executes its real (panicking) step
        0,
    );

    // A step matching two definitions: the ambiguity guard fails. The scenario
    // itself refuses too (0.9.0): the execute_steps preflight fails the
    // scenario naming both bindings, before any step runs — so the guard AND
    // the scenario are red. Ambiguity is checked even for .wip() features.
    let ambiguous_defs = |reg: &mut StepRegistry<World>| {
        reg.define(r"^a doubly matched step$", |_, _, _| {});
        reg.define(r"doubly matched", |_, _, _| {});
    };
    check(
        &mut failures,
        "ambiguous",
        run(Features::new("tests/fixtures/ambiguous").feature("amb", ambiguous_defs)),
        1, // orphan guard
        2, // binding guard + the refusing scenario
        0,
    );
    check(
        &mut failures,
        "ambiguous-even-when-wip",
        run(Features::new("tests/fixtures/ambiguous")
            .feature("amb", ambiguous_defs)
            .wip("amb")),
        1,
        2,
        0,
    );

    // A definer whose feature file does not exist: the orphan guard fails.
    check(
        &mut failures,
        "orphan",
        run(Features::new("tests/fixtures/orphan")
            .feature("real", value_steps)
            .feature("ghost", |_reg: &mut StepRegistry<World>| {})),
        2, // real: binding guard + scenario
        1, // orphan guard fails
        0,
    );

    // A feature file with NO definer at all: same ratchet as unbound steps.
    check(
        &mut failures,
        "no-definer",
        run(Features::new("tests/fixtures/good")),
        1, // orphan guard
        1, // binding guard fails (everything unbound)
        1, // scenario ignored
    );

    // @skip: the scenario is ignored and its panicking step never runs
    // (a run would surface as a failure here).
    check(
        &mut failures,
        "skip",
        run(Features::new("tests/fixtures/skip").feature(
            "skip",
            |reg: &mut StepRegistry<World>| {
                reg.define_exact("a step that panics", |_, _, _| {
                    panic!("@skip scenario executed — skip is broken");
                });
            },
        )),
        2, // orphan guard + binding guard
        0,
        1, // skipped scenario
    );

    // @todo: the scenario runs and fails, but the failure does not gate the
    // suite; the sibling non-todo scenario still gates normally.
    check(
        &mut failures,
        "todo",
        run(Features::new("tests/fixtures/todo").feature(
            "todo",
            |reg: &mut StepRegistry<World>| {
                value_steps(reg);
                reg.define_exact("a step that panics", |_, _, _| {
                    panic!("intentional @todo failure");
                });
            },
        )),
        4, // orphan + binding + tolerated @todo scenario + passing scenario
        0,
        0,
    );

    // @todo + ambiguity (0.9.0): a binding defect outranks the tag — the
    // ambiguous scenario is a plain failing trial, never printed-and-
    // tolerated as declared debt. The non-@todo passing sibling still runs.
    check(
        &mut failures,
        "todo-ambiguous",
        run(Features::new("tests/fixtures/todo").feature(
            "todo",
            |reg: &mut StepRegistry<World>| {
                value_steps(reg);
                reg.define_exact("a step that panics", |_, _, _| {});
                reg.define("panics", |_, _, _| {});
            },
        )),
        2, // orphan + passing scenario
        2, // binding guard (ambiguity) + the refused @todo scenario
        0,
    );

    // The stale half of the @todo inversion (0.9.0): the same corpus with a
    // definer that no longer panics — the run that would first turn the
    // declared debt green goes red instead, naming the stale tag.
    check(
        &mut failures,
        "todo-stale",
        run(Features::new("tests/fixtures/todo").feature(
            "todo",
            |reg: &mut StepRegistry<World>| {
                value_steps(reg);
                reg.define_exact("a step that panics", |_, _, _| {});
            },
        )),
        3, // orphan + binding + passing scenario
        1, // the stale @todo
        0,
    );

    // @only: rejected loudly — there is no `--test-only` analog, so silence
    // would be the worst outcome. Rejection is ADDITIVE: the tagged scenario
    // AND its untagged sibling both still run — the rejection never narrows
    // the suite it polices (the same pin gherkin-node-test's onlytag fixture
    // asserts on node/bun/deno).
    check(
        &mut failures,
        "only",
        run(Features::new("tests/fixtures/only").feature("only", value_steps)),
        4, // orphan + binding + BOTH scenarios (tagged one included)
        1, // the @only rejection trial
        0,
    );

    // Duplicate titles: rejected the @only way — a failing trial, additive
    // (both copies still register and PASS). The rejection exists because a
    // duplicated title silently breaks name-filter focus: the filter matches
    // every copy (mirrors gherkin-node-test's duptitle fixture).
    check(
        &mut failures,
        "duptitle",
        run(Features::new("tests/fixtures/duptitle").feature("twins", value_steps)),
        4, // orphan + binding + BOTH twin scenarios
        1, // the duplicate-title rejection trial
        0,
    );

    // A Feature with no scenarios (header + narrative) is a parse error, so
    // it fails as the feature's "parses" trial — a file that registers
    // nothing must never read as passing.
    check(
        &mut failures,
        "noscenarios",
        run(Features::new("tests/fixtures/noscenarios")
            .feature("header-only", |_reg: &mut StepRegistry<World>| {})),
        1, // orphan guard
        1, // header-only :: parses
        0,
    );

    // A parse error fails as its own trial WITHOUT silencing sibling features.
    check(
        &mut failures,
        "parse-error",
        run(Features::new("tests/fixtures/parse-error")
            .feature("bad", |_reg: &mut StepRegistry<World>| {})
            .feature("ok", value_steps)),
        3, // orphan + ok binding + ok scenario
        1, // bad :: parses
        0,
    );

    // Deferred cleanup runs even though the scenario's last step fails, and
    // the reported failure is the step's (failure outranks cleanup).
    DEFER_RAN.store(false, Ordering::SeqCst);
    check(
        &mut failures,
        "defer-on-failure",
        run(Features::new("tests/fixtures/defer").feature(
            "defer",
            |reg: &mut StepRegistry<World>| {
                reg.define_exact("cleanup is registered", |ctx, _, _| {
                    ctx.defer(|_| DEFER_RAN.store(true, Ordering::SeqCst));
                });
                reg.define_exact("this step fails on purpose", |_, _, _| {
                    panic!("intentional failure");
                });
            },
        )),
        2, // orphan + binding
        1, // the failing scenario
        0,
    );
    if !DEFER_RAN.load(Ordering::SeqCst) {
        failures
            .push("defer-on-failure: deferred cleanup did not run after the step failure".into());
    }

    // A feature directory that doesn't exist: one loud failing trial, never a
    // silently empty (vacuously green) suite.
    check(
        &mut failures,
        "missing-dir",
        run(Features::new("tests/fixtures/does-not-exist")),
        0,
        1, // "feature directory is readable"
        0,
    );

    // An unreadable feature FILE: its "parses" trial fails; the orphan guard
    // still passes. (Permission bits — unix only.)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("gct-guards-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let file = dir.join("counter.feature");
        std::fs::copy("tests/fixtures/good/counter.feature", &file).expect("copy");
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o000)).expect("chmod");
        let c = run(Features::new(&dir).feature("counter", value_steps));
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644))
            .expect("chmod back");
        std::fs::remove_dir_all(&dir).expect("cleanup");
        check(&mut failures, "unreadable-file", c, 1, 1, 0); // orphan guard passes; "counter :: parses" fails
    }

    filter_guard_cases(&mut failures);
    process_cases(&mut failures);

    if failures.is_empty() {
        println!("guard-proof: all cases ok");
    } else {
        for f in &failures {
            eprintln!("guard-proof FAILURE: {f}");
        }
        exit(1);
    }
}
