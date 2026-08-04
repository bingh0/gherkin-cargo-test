// Executed proof of the run manifest — the Rust analog of gherkin-node-test's
// test/manifest.test.js + its subprocess manifest tests, in the guards-proof
// style: each case builds a Features runner over a fixture dir, runs it
// through libtest-mimic IN-PROCESS, and asserts the manifest BYTES (the
// parity contract) plus the write-on-complete rule. The mixed fixture is a
// verbatim copy of the node sibling's features-manifest/mixed.feature, so the
// two suites prove the same title/status account from the same source.
//
// harness = false: main() prints one line per case and exits nonzero if any
// expectation does not hold.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::exit;

use gherkin_cargo_test::{Features, StepRegistry};
use libtest_mimic::{Arguments, Conclusion};

#[derive(Default)]
struct World;

fn mixed_steps(reg: &mut StepRegistry<World>) {
    reg.define_exact("a bound step", |_, _, _| {});
    // The @todo scenario's step: bound AND failing — declared debt that
    // still fails is the tag's green state under the xfail inversion.
    reg.define_exact("a bound step that still fails", |_, _, _| {
        panic!("still fails, as declared")
    });
}

fn red_steps(reg: &mut StepRegistry<World>) {
    reg.define_exact("a failing step", |_, _, _| panic!("red"));
    reg.define_exact("a passing step", |_, _, _| {});
}

fn run_args(f: Features, args: Arguments) -> Conclusion {
    libtest_mimic::run(&args, f.build_trials())
}

fn run(f: Features) -> Conclusion {
    run_args(
        f,
        Arguments {
            test_threads: Some(1),
            ..Default::default()
        },
    )
}

/// The out dir for manifest files; recreated per proof run.
fn out(name: &str) -> PathBuf {
    let dir = Path::new("target").join("manifest-proof");
    fs::create_dir_all(&dir).expect("create out dir");
    let p = dir.join(name);
    let _ = fs::remove_file(&p);
    p
}

/// Expected NDJSON for a fixture dir, mirroring the node test's row builder:
/// fixed key order, trailing newline. Expected rows are LITERAL relative
/// paths — `file` is recorded relative to the manifest's own directory
/// (target/manifest-proof here), and pinning the exact bytes is the point: an
/// absolute path leaking into a row is the defect this format rule exists to
/// prevent.
fn rows(dir: &str, feature: &str, rows: &[(&str, &str)]) -> String {
    let file = format!("../../tests/fixtures/{dir}/{feature}");
    let mut s = String::from("{\"run-manifest\":1}\n");
    s += &rows
        .iter()
        .map(|(title, status)| {
            format!("{{\"file\":\"{file}\",\"title\":\"{title}\",\"status\":\"{status}\"}}")
        })
        .collect::<Vec<_>>()
        .join("\n");
    s.push('\n');
    s
}

fn check(failures: &mut Vec<String>, name: &str, ok: bool, detail: String) {
    if ok {
        println!("manifest-proof: {name} ok");
    } else {
        failures.push(format!("{name}: {detail}"));
    }
}

fn main() {
    let mut failures: Vec<String> = Vec::new();

    let mixed_expected = rows(
        "manifest",
        "mixed.feature",
        &[
            ("passes", "passed"),
            ("pending thing", "unbound"),
            ("skipped one", "skipped"),
            ("sweep 1 [1]", "passed"),
            ("sweep 2 [2]", "passed"),
            ("todo one", "todo"),
        ],
    );

    // A green full run writes the account, sorted, byte-identical to the
    // node sibling's for the same fixture (modulo the path prefix).
    let m = out("mixed.ndjson");
    let c = run(Features::new("tests/fixtures/manifest")
        .feature("mixed", mixed_steps)
        .wip_scenarios("mixed", ["pending thing"])
        .manifest(&m));
    let got = fs::read_to_string(&m).unwrap_or_else(|e| format!("<unreadable: {e}>"));
    check(
        &mut failures,
        "green run writes sorted rows",
        c.num_failed == 0 && got == mixed_expected,
        format!(
            "failed={} got:\n{got}\nexpected:\n{mixed_expected}",
            c.num_failed
        ),
    );

    // --include-ignored executes the parked @skip body and the unbound
    // placeholder (which FAILS by design) — the manifest bytes must not
    // move: statuses come from registration, never from run mode.
    let m2 = out("mixed-include-ignored.ndjson");
    let c = run_args(
        Features::new("tests/fixtures/manifest")
            .feature("mixed", mixed_steps)
            .wip_scenarios("mixed", ["pending thing"])
            .manifest(&m2),
        Arguments {
            test_threads: Some(1),
            include_ignored: true,
            ..Default::default()
        },
    );
    let got = fs::read_to_string(&m2).unwrap_or_else(|e| format!("<unreadable: {e}>"));
    check(
        &mut failures,
        "--include-ignored writes identical bytes",
        got == mixed_expected && c.num_failed > 0,
        format!(
            "failed={} (unbound placeholder must fail when executed) got:\n{got}",
            c.num_failed
        ),
    );

    // The defect that drove 0.8.1: an ABSOLUTE feature-dir argument (the
    // robust idiom in a real adopter's suite) must write the exact same
    // bytes — machine paths must not leak into committed rows.
    let m_abs = out("mixed-absdir.ndjson");
    let abs_dir = std::env::current_dir()
        .expect("cwd")
        .join("tests/fixtures/manifest");
    let c = run(Features::new(&abs_dir)
        .feature("mixed", mixed_steps)
        .wip_scenarios("mixed", ["pending thing"])
        .manifest(&m_abs));
    let got = fs::read_to_string(&m_abs).unwrap_or_else(|e| format!("<unreadable: {e}>"));
    check(
        &mut failures,
        "absolute dir argument writes identical relative bytes",
        c.num_failed == 0 && got == mixed_expected,
        format!(
            "failed={} got:\n{got}\nexpected:\n{mixed_expected}",
            c.num_failed
        ),
    );

    // A feature dir INSIDE the manifest's directory relativizes to a bare
    // subpath — no ../ prefix (the node sibling pins the same shape in its
    // code-point-sort test).
    let inside_dir = Path::new("target")
        .join("manifest-proof")
        .join("features-inside");
    fs::create_dir_all(&inside_dir).expect("create inside dir");
    fs::write(
        inside_dir.join("inside.feature"),
        "Feature: inside\n  Scenario: sits beside the manifest\n    Given a bound step\n",
    )
    .expect("write inside.feature");
    let m_in = out("inside.ndjson");
    let c = run(Features::new(&inside_dir)
        .feature("inside", mixed_steps)
        .manifest(&m_in));
    let inside_expected = concat!(
        "{\"run-manifest\":1}\n",
        "{\"file\":\"features-inside/inside.feature\",",
        "\"title\":\"sits beside the manifest\",\"status\":\"passed\"}\n"
    );
    let got = fs::read_to_string(&m_in).unwrap_or_else(|e| format!("<unreadable: {e}>"));
    check(
        &mut failures,
        "feature dir inside the manifest dir yields a bare subpath",
        c.num_failed == 0 && got == inside_expected,
        format!(
            "failed={} got:\n{got}\nexpected:\n{inside_expected}",
            c.num_failed
        ),
    );

    // A name-filtered run observes only part of the suite: never writes.
    let m3 = out("mixed-filtered.ndjson");
    let _ = run_args(
        Features::new("tests/fixtures/manifest")
            .feature("mixed", mixed_steps)
            .wip_scenarios("mixed", ["pending thing"])
            .manifest(&m3),
        Arguments {
            test_threads: Some(1),
            filter: Some("passes".into()),
            ..Default::default()
        },
    );
    check(
        &mut failures,
        "filtered run never writes",
        !m3.exists(),
        "a partial account must not be written".into(),
    );

    // A red run still writes — the run completed, the account is honest, and
    // the failure still fails the suite.
    let m4 = out("red.ndjson");
    let c = run(Features::new("tests/fixtures/manifestfail")
        .feature("red", red_steps)
        .manifest(&m4));
    let expected = rows(
        "manifestfail",
        "red.feature",
        &[("fails", "failed"), ("passes", "passed")],
    );
    let got = fs::read_to_string(&m4).unwrap_or_else(|e| format!("<unreadable: {e}>"));
    check(
        &mut failures,
        "red run writes failed row",
        c.num_failed == 1 && got == expected,
        format!("failed={} got:\n{got}\nexpected:\n{expected}", c.num_failed),
    );

    // A stale @todo (its step now passes) is recorded as the failure it is:
    // the run is red and the account explains the red — 'failed', never a
    // green-looking 'todo' declaration (ratified 2026-08-03, owner queue).
    let m_stale = out("mixed-stale-todo.ndjson");
    let c = run(Features::new("tests/fixtures/manifest")
        .feature("mixed", |reg: &mut StepRegistry<World>| {
            reg.define_exact("a bound step", |_, _, _| {});
            reg.define_exact("a bound step that still fails", |_, _, _| {});
        })
        .wip_scenarios("mixed", ["pending thing"])
        .manifest(&m_stale));
    let got = fs::read_to_string(&m_stale).unwrap_or_else(|e| format!("<unreadable: {e}>"));
    let stale_expected = rows(
        "manifest",
        "mixed.feature",
        &[
            ("passes", "passed"),
            ("pending thing", "unbound"),
            ("skipped one", "skipped"),
            ("sweep 1 [1]", "passed"),
            ("sweep 2 [2]", "passed"),
            ("todo one", "failed"),
        ],
    );
    check(
        &mut failures,
        "stale @todo records failed, run red",
        c.num_failed == 1 && got == stale_expected,
        format!("failed={} got:
{got}
expected:
{stale_expected}", c.num_failed),
    );

    // 0.9.0: an empty feature directory is REFUSED — one failing trial, no
    // account written or claimed. The zero-byte-account rationale lost to
    // the honest-run contract's refusal: nothing-was-enforced must be loud,
    // not a green file.
    let empty_dir = Path::new("target").join("manifest-proof").join("empty");
    fs::create_dir_all(&empty_dir).expect("create empty dir");
    let m5 = out("empty.ndjson");
    let c = run(Features::new(&empty_dir).manifest(&m5));
    check(
        &mut failures,
        "empty dir is refused, no account",
        c.num_failed == 1 && !m5.exists(),
        format!("failed={}, manifest exists={}", c.num_failed, m5.exists()),
    );

    // A feature that fails to parse disables the writer: the account cannot
    // be complete, and a manifest omitting the broken feature's rows would
    // be absence-shaped. (The node sibling reaches the same end by refusing
    // at load; here the parse failure is a failing trial and siblings run.)
    let m6 = out("parse-error.ndjson");
    let c = run(Features::new("tests/fixtures/parse-error").manifest(&m6));
    check(
        &mut failures,
        "parse failure never writes",
        !m6.exists() && c.num_failed > 0,
        format!("failed={}, manifest exists={}", c.num_failed, m6.exists()),
    );

    // Under cargo-nextest (detected via its documented env vars) the manifest
    // is refused the @only way: ONE registered failing trial naming the fix,
    // additive — scenarios still run, and nothing is written.
    std::env::set_var("NEXTEST", "1");
    let m7 = out("nextest.ndjson");
    let c = run(Features::new("tests/fixtures/manifest")
        .feature("mixed", mixed_steps)
        .wip_scenarios("mixed", ["pending thing"])
        .manifest(&m7));
    std::env::remove_var("NEXTEST");
    check(
        &mut failures,
        "nextest refusal: one red trial, scenarios still run, nothing written",
        c.num_failed == 1 && c.num_passed >= 5 && !m7.exists(),
        format!(
            "(passed, failed)=({}, {}), manifest exists={}",
            c.num_passed,
            c.num_failed,
            m7.exists()
        ),
    );

    if failures.is_empty() {
        println!("manifest-proof: all ok");
        exit(0);
    }
    for f in &failures {
        eprintln!("manifest-proof FAILURE: {f}");
    }
    exit(1);
}
