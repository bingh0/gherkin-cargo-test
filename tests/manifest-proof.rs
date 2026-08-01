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
/// fixed key order, `/`-normalized path, trailing newline.
fn rows(dir: &str, feature: &str, rows: &[(&str, &str)]) -> String {
    let file = Path::new("tests/fixtures")
        .join(dir)
        .join(feature)
        .display()
        .to_string()
        .replace(std::path::MAIN_SEPARATOR, "/");
    let mut s = rows
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

    // A directory that registers zero scenarios writes a ZERO-BYTE account —
    // visibly empty, never a lone newline, never absent.
    let empty_dir = Path::new("target").join("manifest-proof").join("empty");
    fs::create_dir_all(&empty_dir).expect("create empty dir");
    let m5 = out("empty.ndjson");
    let _ = run(Features::new(&empty_dir).manifest(&m5));
    check(
        &mut failures,
        "empty dir writes zero bytes",
        fs::read(&m5).map(|b| b.is_empty()).unwrap_or(false),
        format!("expected empty file at {}", m5.display()),
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
