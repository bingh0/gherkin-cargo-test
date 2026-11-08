// gct-lint, run as a real process over temporary directories. Each finding
// line is checked against lint_feature itself, so the command can only add
// file handling around the library, never different findings.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

use gherkin_cargo_test::{lint_feature, lint_feature_strict, LintFinding};

const BIN: &str = env!("CARGO_BIN_EXE_gct-lint");

const CLEAN: &str = "Feature: Clean\n  Scenario: adds\n    When I add 2\n    Then the total is 2\n";
const NO_THEN: &str = "Feature: Hollow\n  Scenario: runs\n    When I poke it\n";
const DUP_TITLE: &str = concat!(
    "Feature: Twins\n",
    "  Scenario: same\n    When I add 1\n    Then the total is 1\n",
    "  Scenario: same\n    When I add 2\n    Then the total is 2\n",
);
const SKIPPED: &str =
    "Feature: Parked\n  @skip\n  Scenario: later\n    When I add 1\n    Then the total is 1\n";
const DIALECT: &str = "Feature: F\nRule: not supported\n";

/// A fresh, empty directory per test (tests run in parallel).
fn scratch(name: &str) -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("cli")
        .join(format!("{name}-{}", N.fetch_add(1, Ordering::Relaxed)));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn put(root: &Path, rel: &str, text: &str) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().expect("parent")).expect("parent dir");
    fs::write(path, text).expect("write fixture");
}

fn run(cwd: &Path, args: &[&str]) -> Output {
    Command::new(BIN)
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("run gct-lint")
}

fn stdout(o: &Output) -> Vec<String> {
    String::from_utf8_lossy(&o.stdout)
        .lines()
        .map(str::to_string)
        .collect()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

/// What the CLI must print for one file: the library's findings, formatted.
fn expected(name: &str, findings: Vec<LintFinding>) -> Vec<String> {
    findings
        .iter()
        .map(|f| {
            format!(
                "{name}:{}: {}[{}]: {}",
                f.line,
                f.severity.as_str(),
                f.rule.as_str(),
                f.message
            )
        })
        .collect()
}

#[test]
fn a_clean_corpus_exits_zero_and_prints_no_findings() {
    let dir = scratch("clean");
    put(&dir, "features/a.feature", CLEAN);
    let o = run(&dir, &["features"]);
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    assert!(stdout(&o).is_empty(), "{:?}", stdout(&o));
    assert_eq!(stderr(&o), "gct-lint: 1 file, 0 errors, 0 warnings\n");
}

#[test]
fn finding_lines_are_the_library_findings_verbatim() {
    let dir = scratch("verbatim");
    put(&dir, "features/twins.feature", DUP_TITLE);
    put(&dir, "features/hollow.feature", NO_THEN);
    let o = run(&dir, &["features"]);
    let mut want = expected(
        "features/hollow.feature",
        lint_feature(NO_THEN, "features/hollow.feature"),
    );
    want.extend(expected(
        "features/twins.feature",
        lint_feature(DUP_TITLE, "features/twins.feature"),
    ));
    assert!(!want.is_empty());
    assert_eq!(stdout(&o), want);
    assert_eq!(o.status.code(), Some(1), "duplicate-title is an error");
    assert_eq!(stderr(&o), "gct-lint: 2 files, 1 error, 1 warning\n");
}

#[test]
fn a_dialect_error_is_a_finding_and_fails() {
    let dir = scratch("dialect");
    put(&dir, "features/bad.feature", DIALECT);
    let o = run(&dir, &["features"]);
    assert_eq!(
        stdout(&o),
        expected(
            "features/bad.feature",
            lint_feature(DIALECT, "features/bad.feature")
        )
    );
    assert!(
        stdout(&o)[0].starts_with("features/bad.feature:2: error[dialect]: "),
        "{:?}",
        stdout(&o)
    );
    assert_eq!(o.status.code(), Some(1));
}

#[test]
fn warnings_alone_pass_and_strict_makes_them_fail() {
    let dir = scratch("strict");
    put(&dir, "features/hollow.feature", NO_THEN);
    let o = run(&dir, &["features"]);
    assert_eq!(o.status.code(), Some(0), "a warning alone must not fail");
    assert_eq!(stdout(&o).len(), 1);
    assert!(
        stdout(&o)[0].contains(": warn[no-then]: "),
        "{:?}",
        stdout(&o)
    );

    let o = run(&dir, &["--strict", "features"]);
    assert_eq!(o.status.code(), Some(1));
    assert_eq!(
        stdout(&o),
        expected(
            "features/hollow.feature",
            lint_feature_strict(NO_THEN, "features/hollow.feature")
        )
    );
    assert!(
        stdout(&o)[0].contains(": error[no-then]: "),
        "{:?}",
        stdout(&o)
    );
    assert_eq!(
        stderr(&o),
        "gct-lint: 1 file, 1 error, 0 warnings (strict)\n"
    );
}

#[test]
fn strict_adds_the_strict_tag_rule() {
    let dir = scratch("strict-tag");
    put(&dir, "features/parked.feature", SKIPPED);
    assert!(
        stdout(&run(&dir, &["features"])).is_empty(),
        "@skip is clean outside strict"
    );
    let o = run(&dir, &["--strict", "features"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(
        stdout(&o)
            .iter()
            .any(|l| l.contains(": error[strict-tag]: ")),
        "{:?}",
        stdout(&o)
    );
}

#[test]
fn directories_are_walked_recursively_in_sorted_order_skipping_build_and_hidden_dirs() {
    let dir = scratch("walk");
    for rel in [
        "features/z.feature",
        "features/a/y.feature",
        "features/b.feature",
        "features/.hidden/x.feature",
        "features/target/t.feature",
        "features/node_modules/n.feature",
    ] {
        put(&dir, rel, NO_THEN);
    }
    put(&dir, "features/notes.txt", NO_THEN);
    let o = run(&dir, &["features"]);
    let files: Vec<String> = stdout(&o)
        .iter()
        .map(|l| l.split(':').next().unwrap_or_default().to_string())
        .collect();
    assert_eq!(
        files,
        [
            "features/a/y.feature",
            "features/b.feature",
            "features/z.feature"
        ],
        "sorted depth-first; hidden, target/, node_modules/ and non-.feature files skipped"
    );
    assert_eq!(stderr(&o), "gct-lint: 3 files, 0 errors, 3 warnings\n");
}

#[test]
fn a_path_named_explicitly_is_always_linted() {
    let dir = scratch("explicit");
    put(&dir, "features/.hidden/x.feature", NO_THEN);
    put(&dir, "spec.txt", NO_THEN);
    let o = run(&dir, &["features/.hidden", "spec.txt"]);
    let files: Vec<String> = stdout(&o)
        .iter()
        .map(|l| l.split(':').next().unwrap_or_default().to_string())
        .collect();
    assert_eq!(files, ["features/.hidden/x.feature", "spec.txt"]);
}

#[test]
fn with_no_path_it_lints_the_features_directory() {
    let dir = scratch("default");
    put(&dir, "features/hollow.feature", NO_THEN);
    put(&dir, "elsewhere/ignored.feature", DUP_TITLE);
    let o = run(&dir, &[]);
    assert_eq!(o.status.code(), Some(0));
    assert_eq!(stdout(&o).len(), 1);
    assert!(
        stdout(&o)[0].starts_with("features/hollow.feature:"),
        "{:?}",
        stdout(&o)
    );
}

#[test]
fn nothing_to_lint_is_a_failure_not_a_pass() {
    let dir = scratch("empty");
    fs::create_dir_all(dir.join("features")).expect("empty features dir");
    let o = run(&dir, &["features"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(
        stderr(&o).starts_with("gct-lint: no .feature files under features\n"),
        "{}",
        stderr(&o)
    );

    // An unreadable path is a different message from an empty directory.
    let o = run(&dir, &["no-such-dir"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(
        stderr(&o).starts_with("gct-lint: no-such-dir: "),
        "{}",
        stderr(&o)
    );
    assert!(!stderr(&o).contains("no .feature files"), "{}", stderr(&o));
}

#[test]
fn each_named_directory_must_hold_a_feature_file() {
    let dir = scratch("each-dir");
    put(&dir, "features/a.feature", CLEAN);
    fs::create_dir_all(dir.join("empty")).expect("empty dir");
    let o = run(&dir, &["features", "empty"]);
    assert_eq!(
        o.status.code(),
        Some(1),
        "an empty directory fails even beside a full one"
    );
    assert!(
        stderr(&o).contains("gct-lint: no .feature files under empty\n"),
        "{}",
        stderr(&o)
    );
}

#[test]
fn walked_paths_are_the_argument_as_typed_then_slash_then_the_relative_path() {
    let dir = scratch("spelling");
    put(&dir, "features/sub/a.feature", NO_THEN);
    let first = |args: &[&str]| {
        let o = run(&dir, args);
        stdout(&o)[0]
            .split(':')
            .next()
            .unwrap_or_default()
            .to_string()
    };
    // No normalization: `./` survives, a trailing slash is not doubled.
    assert_eq!(first(&["./features"]), "./features/sub/a.feature");
    assert_eq!(first(&["features/"]), "features/sub/a.feature");
    assert_eq!(first(&["features//sub"]), "features//sub/a.feature");
    // A file named directly is printed exactly as typed.
    assert_eq!(
        first(&["./features/sub/../sub/a.feature"]),
        "./features/sub/../sub/a.feature"
    );
}

#[test]
fn a_dialect_fault_yields_one_line_per_file() {
    let dir = scratch("one-dialect");
    put(
        &dir,
        "features/bad.feature",
        "Feature: F\nRule: one\nRule: two\n",
    );
    let o = run(&dir, &["features"]);
    assert_eq!(
        stdout(&o).len(),
        1,
        "the parser stops at the first fault: {:?}",
        stdout(&o)
    );
    assert!(
        stderr(&o).ends_with("gct-lint: 1 file, 1 error, 0 warnings\n"),
        "{}",
        stderr(&o)
    );
}

#[test]
fn usage_errors_exit_two_and_help_and_version_exit_zero() {
    let dir = scratch("usage");
    for bad in ["--bogus", "-", "-x"] {
        let o = run(&dir, &[bad]);
        assert_eq!(o.status.code(), Some(2), "{bad}");
        assert!(
            stderr(&o).contains("unknown option"),
            "{bad}: {}",
            stderr(&o)
        );
    }
    let o = run(&dir, &["--help"]);
    assert_eq!(o.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&o.stdout).starts_with("usage: gct-lint"));
    let o = run(&dir, &["-V"]);
    assert_eq!(o.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&o.stdout),
        format!("gct-lint {}\n", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn a_path_after_double_dash_is_a_path_even_if_it_looks_like_an_option() {
    let dir = scratch("dashdash");
    put(&dir, "--strict", NO_THEN);
    let o = run(&dir, &["--", "--strict"]);
    assert_eq!(
        o.status.code(),
        Some(0),
        "linted as a file, not read as the flag"
    );
    assert!(stdout(&o)[0].starts_with("--strict:"), "{:?}", stdout(&o));
}
