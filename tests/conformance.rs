// Conformance suite, ported from gherkin-node-test's test/harness.test.js:
// a rejection test for every loud-parser guard, parse-correctness checks,
// DataTable semantics, registry matching, snippet validity, execution and
// deferred-cleanup semantics, and the pure binding guard.

use std::sync::{Arc, Mutex};

use gherkin_cargo_test::{
    build_snippet, check_bindings, execute_steps, parse_feature, DataTable, GherkinSyntaxError,
    Step, StepRegistry,
};

fn err_of(text: &str) -> GherkinSyntaxError {
    parse_feature(text, "t.feature").expect_err("expected a GherkinSyntaxError")
}

fn assert_rejects(text: &str, line: usize, contains: &str) {
    let e = err_of(text);
    assert_eq!(e.line, line, "wrong line for {contains:?}: {e}");
    assert!(
        e.to_string().contains(contains),
        "missing {contains:?} in: {e}"
    );
    assert!(
        e.to_string().starts_with("t.feature:"),
        "missing file prefix in: {e}"
    );
}

// --- Rejections: every silence is a bug -----------------------------------------

#[test]
fn rejects_doc_strings() {
    assert_rejects(
        "Feature: f\n  Scenario: s\n    Given a\n    \"\"\"\n    body\n    \"\"\"\n",
        4,
        "doc strings",
    );
    assert_rejects(
        "Feature: f\n  Scenario: s\n    Given a\n    ```\n",
        4,
        "doc strings",
    );
}

#[test]
fn rejects_rule_keyword() {
    assert_rejects("Feature: f\nRule: grouping\n", 2, "Rule:");
}

#[test]
fn rejects_multiple_features() {
    assert_rejects("Feature: a\nFeature: b\n", 2, "multiple Feature:");
}

#[test]
fn rejects_multiple_backgrounds() {
    assert_rejects(
        "Feature: f\nBackground:\n  Given a\nBackground:\n",
        4,
        "multiple Background:",
    );
}

#[test]
fn rejects_background_after_scenario() {
    assert_rejects(
        "Feature: f\nScenario: s\n  Given a\nBackground:\n",
        4,
        "Background: must appear before any Scenario",
    );
}

#[test]
fn rejects_background_after_outline() {
    // flush-before-check: a preceding Outline counts as a Scenario.
    assert_rejects(
        "Feature: f\nScenario Outline: o\n  Given <x>\n  Examples:\n    | x |\n    | 1 |\nBackground:\n",
        7,
        "Background: must appear before any Scenario",
    );
}

#[test]
fn rejects_examples_outside_outline() {
    assert_rejects(
        "Feature: f\nScenario: s\n  Given a\nExamples:\n",
        4,
        "Examples: outside",
    );
}

#[test]
fn rejects_multiple_examples_blocks() {
    assert_rejects(
        "Feature: f\nScenario Outline: o\n  Given <x>\n  Examples:\n    | x |\n    | 1 |\n  Examples:\n",
        7,
        "multiple Examples:",
    );
}

#[test]
fn rejects_step_after_examples() {
    assert_rejects(
        "Feature: f\nScenario Outline: o\n  Given <x>\n  Examples:\n    | x |\n    | 1 |\n  Then late\n",
        7,
        "step after an Examples:",
    );
}

#[test]
fn rejects_outline_without_steps() {
    assert_rejects(
        "Feature: f\nScenario Outline: o\n  Examples:\n    | x |\n    | 1 |\n",
        2,
        "has no steps",
    );
}

#[test]
fn rejects_outline_without_examples() {
    assert_rejects(
        "Feature: f\nScenario Outline: o\n  Given a\n",
        2,
        "no Examples: block",
    );
}

#[test]
fn rejects_examples_without_header() {
    assert_rejects(
        "Feature: f\nScenario Outline: o\n  Given <x>\n  Examples:\n",
        2,
        "no header row",
    );
}

#[test]
fn rejects_examples_without_data_rows() {
    assert_rejects(
        "Feature: f\nScenario Outline: o\n  Given <x>\n  Examples:\n    | x |\n",
        2,
        "header but no data rows",
    );
}

#[test]
fn rejects_ragged_examples_row() {
    assert_rejects(
        "Feature: f\nScenario Outline: o\n  Given <x>\n  Examples:\n    | x | y |\n    | 1 |\n",
        6,
        "Examples row has 1 cell(s); header has 2",
    );
}

#[test]
fn rejects_ragged_step_table_row() {
    assert_rejects(
        "Feature: f\nScenario: s\n  Given t\n    | a | b |\n    | c |\n",
        5,
        "table row has 1 cell(s); this step's table has 2",
    );
}

#[test]
fn rejects_row_missing_closing_pipe() {
    assert_rejects(
        "Feature: f\nScenario: s\n  Given t\n    | a | b\n",
        4,
        "closing |",
    );
}

#[test]
fn rejects_empty_table_row() {
    assert_rejects(
        "Feature: f\nScenario: s\n  Given t\n    |\n",
        4,
        "empty table row",
    );
}

#[test]
fn rejects_table_row_without_step() {
    assert_rejects(
        "Feature: f\nScenario: s\n  | a |\n",
        3,
        "table row without a preceding step",
    );
}

#[test]
fn rejects_table_row_before_scenario() {
    assert_rejects(
        "Feature: f\n| a |\n",
        2,
        "table row before any Scenario or Background",
    );
}

#[test]
fn rejects_step_before_scenario() {
    assert_rejects(
        "Feature: f\nGiven early\n",
        2,
        "step before any Scenario or Background",
    );
}

#[test]
fn rejects_unknown_placeholder() {
    assert_rejects(
        "Feature: f\nScenario Outline: o\n  Given <typo>\n  Examples:\n    | x |\n    | 1 |\n",
        2,
        "unknown placeholder <typo>",
    );
}

#[test]
fn rejects_scenario_without_steps() {
    assert_rejects(
        "Feature: f\nScenario: empty\nScenario: s\n  Given a\n",
        2,
        "has no steps",
    );
}

#[test]
fn rejects_non_english_keyword_as_empty_scenario() {
    // "Angenommen" reads as narrative; the scenario is left empty → loud.
    assert_rejects(
        "Feature: f\nScenario: s\n  Angenommen ein Wert\n",
        2,
        "has no steps",
    );
}

#[test]
fn rejects_missing_feature_line() {
    assert_rejects("Scenario: s\n  Given a\n", 3, "no Feature: line");
}

#[test]
fn rejects_misplaced_tags() {
    for (src, line) in [
        ("Feature: f\n@skip\nBackground:\n  Given a\n", 3),
        ("Feature: f\nScenario Outline: o\n  Given <x>\n@skip\n  Examples:\n    | x |\n    | 1 |\n", 5),
        ("Feature: f\nScenario: s\n@skip\n  Given a\n", 4),
        ("Feature: f\nScenario: s\n  Given t\n@skip\n    | a |\n", 5),
    ] {
        assert_rejects(src, line, "must immediately precede");
    }
}

#[test]
fn rejects_dangling_tags_at_eof() {
    // EOF errors report the position after the last line (same as the JS
    // implementation: a trailing \n yields a final empty line 5).
    assert_rejects(
        "Feature: f\nScenario: s\n  Given a\n@skip\n",
        5,
        "dangling tags",
    );
}

#[test]
fn rejects_near_miss_semantic_tags() {
    for tag in ["@Skip", "@SKIP", "@Todo", "@ONLY", "@Only"] {
        let src = format!("Feature: f\n{tag}\nScenario: s\n  Given a\n");
        assert_rejects(&src, 2, "near-miss");
    }
}

// Combined semantic tags: this runner resolves @skip before @todo and rejects
// @only, the JS sibling's runtimes each do something else — a combination
// can't mean the same thing everywhere, so it must not mean anything silently.
// (Dialect parity with gherkin-node-test, which rejects these identically.)
#[test]
fn rejects_conflicting_semantic_tags_on_a_scenario() {
    assert_rejects(
        "Feature: f\n@skip @only\nScenario: s\n  Given x\n",
        3,
        "conflicting tags (@skip @only)",
    );
}

#[test]
fn rejects_conflicting_semantic_tags_on_an_outline() {
    assert_rejects(
        "Feature: f\n@todo @skip\nScenario Outline: o\n  Given <a>\nExamples:\n  | a |\n  | 1 |\n",
        3,
        "conflicting tags (@todo @skip)",
    );
}

#[test]
fn rejects_conflicting_tags_on_the_feature_line() {
    assert_rejects(
        "@skip @todo\nFeature: f\n",
        2,
        "conflicting tags (@skip @todo)",
    );
}

#[test]
fn rejects_feature_tag_conflicting_with_scenario_tag() {
    assert_rejects(
        "@skip\nFeature: f\n@only\nScenario: s\n  Given x\n",
        4,
        "conflicting tags (@skip @only)",
    );
}

#[test]
fn duplicate_semantic_tag_is_not_a_conflict() {
    let p = parse_feature(
        "@skip\nFeature: f\n@skip\nScenario: s\n  Given x\n",
        "t.feature",
    )
    .expect("same tag twice is redundant, not ambiguous");
    assert_eq!(p.scenarios[0].tags, vec!["@skip", "@skip"]);
}

// --- Parse correctness ----------------------------------------------------------

#[test]
fn parses_the_practical_core() {
    let src = "\
# comment
@smoke
Feature: Core
  As a user
  I want things
  So that stuff

  Background:
    Given a base

  @AC1
  Scenario: plain
    When I act
    Then it worked

  Scenario Outline: outline <name>
    When I use <name>
    Then I see <result>
      | echo | <result> |
    Examples:
      | name | result |
      | a    | 1      |
      | b    | 2      |
";
    let p = parse_feature(src, "core.feature").expect("parses");
    assert_eq!(p.feature, "Core");
    assert_eq!(p.background.len(), 1);
    assert_eq!(p.background[0].text, "a base");
    assert_eq!(p.scenarios.len(), 3); // 1 plain + 2 expanded outline rows

    let plain = &p.scenarios[0];
    assert_eq!(plain.name, "plain");
    assert_eq!(plain.tags, vec!["@smoke", "@AC1"]); // feature tags apply to all
    assert_eq!(plain.steps.len(), 2);
    assert_eq!(plain.steps[0].keyword, "When");

    let o1 = &p.scenarios[1];
    assert_eq!(o1.name, "outline a [1]"); // placeholder in the NAME too
    assert_eq!(o1.tags, vec!["@smoke"]);
    assert_eq!(o1.steps[0].text, "I use a");
    // placeholder substitution reaches step data tables:
    assert_eq!(
        o1.steps[1].table.as_ref().expect("table")[0],
        vec!["echo", "1"]
    );
    assert_eq!(p.scenarios[2].name, "outline b [2]");
    assert_eq!(p.scenarios[2].steps[0].text, "I use b");
}

#[test]
fn parses_crlf_and_star_keyword() {
    let p = parse_feature("Feature: f\r\nScenario: s\r\n  * anything\r\n", "f.feature")
        .expect("parses");
    assert_eq!(p.scenarios[0].steps[0].keyword, "*");
    assert_eq!(p.scenarios[0].steps[0].text, "anything");
    assert_eq!(p.scenarios[0].steps[0].line, 3);
}

#[test]
fn parses_cell_escapes() {
    let p = parse_feature(
        "Feature: f\nScenario: s\n  Given t\n    | pipe\\|r | back\\\\slash | new\\nline | c:\\temp |\n",
        "f.feature",
    )
    .expect("parses");
    let t = p.scenarios[0].steps[0].table.as_ref().expect("table");
    assert_eq!(t[0], vec!["pipe|r", "back\\slash", "new\nline", "c:\\temp"]);
}

#[test]
fn keyword_prefix_is_not_a_step() {
    // "Butter…" must not parse as a But step; as the scenario's only line it
    // reads as narrative and the no-steps guard fires.
    assert_rejects(
        "Feature: f\nScenario: s\n  Butter the bread\n",
        2,
        "has no steps",
    );
}

// --- DataTable -------------------------------------------------------------------

fn tbl(rows: &[&[&str]]) -> DataTable {
    DataTable::new(
        rows.iter()
            .map(|r| r.iter().map(|c| c.to_string()).collect())
            .collect(),
    )
}

#[test]
fn datatable_surface() {
    let t = tbl(&[&["name", "role"], &["ada", "admin"], &["bob", "dev"]]);
    assert_eq!(t.raw().len(), 3);
    assert_eq!(t.rows(), vec![vec!["ada", "admin"], vec!["bob", "dev"]]);
    let h = t.hashes();
    assert_eq!(h[0]["name"], "ada");
    assert_eq!(h[1]["role"], "dev");
    let tr = t.transpose();
    assert_eq!(tr.raw()[0], vec!["name", "ada", "bob"]);

    let cfg = tbl(&[&["retries", "3"], &["mode", "fast"]]);
    let m = cfg.rows_hash();
    assert_eq!(m["retries"], "3");
    assert_eq!(m["mode"], "fast");
}

#[test]
#[should_panic(expected = "exactly two columns")]
fn rows_hash_rejects_non_two_column_tables() {
    tbl(&[&["a", "b", "c"]]).rows_hash();
}

// --- StepRegistry ------------------------------------------------------------------

#[test]
fn registry_matches_and_captures() {
    let mut reg: StepRegistry<()> = StepRegistry::new();
    reg.define(r"^I add (\d+) and (\d+)$", |_, _, _| {});
    let (idx, args) = reg.find("I add 2 and 40").expect("matches");
    assert_eq!(idx, 0);
    assert_eq!(args, vec!["2", "40"]);
    assert!(reg.find("I subtract 2").is_none());
}

#[test]
fn define_exact_escapes_metacharacters() {
    let mut reg: StepRegistry<()> = StepRegistry::new();
    reg.define_exact("costs $5 (net)", |_, _, _| {});
    assert!(reg.find("costs $5 (net)").is_some());
    assert!(reg.find("costs $5 Xnet)").is_none());
}

#[test]
fn first_match_wins_and_ambiguity_is_detectable() {
    let mut reg: StepRegistry<()> = StepRegistry::new();
    reg.define(r"^a step$", |_, _, _| {});
    reg.define(r"step", |_, _, _| {});
    assert_eq!(reg.find("a step").expect("matches").0, 0);
    assert_eq!(reg.matching("a step").len(), 2);
}

// --- Snippets ----------------------------------------------------------------------

/// Pull the regex source back out of a generated snippet and compile it —
/// proving the paste-ready definition actually matches its own step.
fn snippet_regex(snippet: &str) -> regex::Regex {
    let start = snippet.find("r#\"").expect("raw string open") + 3;
    let end = snippet.find("\"#").expect("raw string close");
    regex::Regex::new(&snippet[start..end]).expect("snippet regex compiles")
}

#[test]
fn snippet_matches_its_own_step() {
    for (text, captures) in [
        ("I add 5", 1),
        ("I wait 2.5 seconds then 3 more", 2),
        ("the file \"a.txt\" has 10 rows", 2),
        ("plain text with (parens) and $cost", 0),
    ] {
        let s = build_snippet(text);
        assert!(s.contains("panic!"), "snippet body must throw, got: {s}");
        let re = snippet_regex(&s);
        let c = re
            .captures(text)
            .unwrap_or_else(|| panic!("snippet regex must match {text:?}"));
        assert_eq!(c.len() - 1, captures, "capture count for {text:?}");
    }
}

#[test]
fn snippet_generalizes_numbers_and_strings() {
    let re = snippet_regex(&build_snippet("I add 5"));
    assert!(re.is_match("I add 99"));
    let re = snippet_regex(&build_snippet("the file \"a.txt\" exists"));
    assert!(re.is_match("the file \"other.bin\" exists"));
}

// --- Execution and deferred cleanup ---------------------------------------------

fn steps_of(src: &str) -> Vec<Step> {
    let p = parse_feature(src, "x.feature").expect("parses");
    let sc = &p.scenarios[0];
    p.background
        .iter()
        .chain(sc.steps.iter())
        .cloned()
        .collect()
}

#[test]
fn executes_steps_against_a_typed_world() {
    #[derive(Default)]
    struct W {
        n: i64,
    }
    let mut reg: StepRegistry<W> = StepRegistry::new();
    reg.define(r"^n is (\d+)$", |ctx, args, _| {
        ctx.world.n = args[0].parse().expect("int")
    });
    reg.define(r"^n doubles$", |ctx, _, _| ctx.world.n *= 2);
    let steps = steps_of("Feature: f\nScenario: s\n  Given n is 21\n  When n doubles\n");
    let w = execute_steps(&steps, &reg, W::default()).expect("passes");
    assert_eq!(w.n, 42);
}

#[test]
fn undefined_step_fails_with_a_snippet() {
    let reg: StepRegistry<()> = StepRegistry::new();
    let steps = steps_of("Feature: f\nScenario: s\n  Given nothing binds 42\n");
    let e = execute_steps(&steps, &reg, ()).expect_err("must fail");
    assert!(e.contains("Undefined step: nothing binds 42"), "got: {e}");
    assert!(e.contains("reg.define(r#\""), "snippet missing: {e}");
}

#[test]
fn failure_message_names_the_step_and_feature_line() {
    let mut reg: StepRegistry<()> = StepRegistry::new();
    reg.define_exact("it breaks", |_, _, _| panic!("boom"));
    let steps = steps_of("Feature: f\nScenario: s\n  Given it breaks\n");
    let e = execute_steps(&steps, &reg, ()).expect_err("must fail");
    assert!(e.contains("Given it breaks"), "got: {e}");
    assert!(e.contains("feature line 3"), "got: {e}");
    assert!(e.contains("boom"), "got: {e}");
}

#[test]
fn table_arrives_as_the_last_argument() {
    #[derive(Default)]
    struct W {
        sum: i64,
    }
    let mut reg: StepRegistry<W> = StepRegistry::new();
    reg.define_exact("amounts", |ctx, _, table| {
        for row in table.expect("table").rows() {
            ctx.world.sum += row[0].parse::<i64>().expect("int");
        }
    });
    let steps = steps_of(
        "Feature: f\nScenario: s\n  Given amounts\n    | amount |\n    | 3 |\n    | 4 |\n",
    );
    let w = execute_steps(&steps, &reg, W::default()).expect("passes");
    assert_eq!(w.sum, 7);
}

#[test]
fn background_steps_carry_data_tables() {
    // The table-row arm for Cur::Background — a Background step's data table
    // must attach to it and reach the step function at execution time.
    let src = "Feature: f\nBackground:\n  Given base config\n    | retries | 3 |\nScenario: s\n  Then retries is 3\n";
    let p = parse_feature(src, "f.feature").expect("parses");
    assert_eq!(
        p.background[0].table.as_ref().expect("background table")[0],
        vec!["retries", "3"]
    );

    #[derive(Default)]
    struct W {
        retries: String,
    }
    let mut reg: StepRegistry<W> = StepRegistry::new();
    reg.define_exact("base config", |ctx, _, table| {
        ctx.world.retries = table.expect("table").rows_hash()["retries"].clone();
    });
    reg.define(r"^retries is (\d+)$", |ctx, args, _| {
        assert_eq!(ctx.world.retries, args[0]);
    });
    let steps: Vec<Step> = p
        .background
        .iter()
        .chain(p.scenarios[0].steps.iter())
        .cloned()
        .collect();
    execute_steps(&steps, &reg, W::default()).expect("passes");
}

#[test]
fn failure_message_carries_string_panic_payloads() {
    // assert!/assert_eq! failures panic with a String payload, not &str.
    let mut reg: StepRegistry<()> = StepRegistry::new();
    reg.define_exact("it breaks formatted", |_, _, _| panic!("code {}", 7));
    let steps = steps_of("Feature: f\nScenario: s\n  Given it breaks formatted\n");
    let e = execute_steps(&steps, &reg, ()).expect_err("must fail");
    assert!(e.contains("code 7"), "got: {e}");
}

#[test]
fn non_string_panic_payloads_are_still_reported() {
    let mut reg: StepRegistry<()> = StepRegistry::new();
    reg.define_exact("it panics oddly", |_, _, _| std::panic::panic_any(42_i32));
    let steps = steps_of("Feature: f\nScenario: s\n  Given it panics oddly\n");
    let e = execute_steps(&steps, &reg, ()).expect_err("must fail");
    assert!(e.contains("non-string panic payload"), "got: {e}");
}

#[test]
fn defer_runs_lifo_after_passing_steps() {
    #[derive(Default)]
    struct W {
        order: Vec<u8>,
    }
    let mut reg: StepRegistry<W> = StepRegistry::new();
    reg.define_exact("two cleanups", |ctx, _, _| {
        ctx.defer(|w| w.order.push(1));
        ctx.defer(|w| w.order.push(2));
    });
    let steps = steps_of("Feature: f\nScenario: s\n  Given two cleanups\n");
    let w = execute_steps(&steps, &reg, W::default()).expect("passes");
    assert_eq!(w.order, vec![2, 1], "deferred cleanup must run LIFO");
}

#[test]
fn defer_runs_even_when_a_step_fails_and_the_step_failure_wins() {
    let ran = Arc::new(Mutex::new(Vec::<&str>::new()));
    let mut reg: StepRegistry<()> = StepRegistry::new();
    let r = Arc::clone(&ran);
    reg.define_exact("cleanup then boom", move |ctx, _, _| {
        let r = Arc::clone(&r);
        ctx.defer(move |_| {
            r.lock().expect("lock").push("cleaned");
            panic!("cleanup error should be outranked");
        });
        panic!("step failure");
    });
    let steps = steps_of("Feature: f\nScenario: s\n  Given cleanup then boom\n");
    let e = execute_steps(&steps, &reg, ()).expect_err("must fail");
    assert!(
        e.contains("step failure"),
        "step failure must outrank cleanup: {e}"
    );
    assert!(
        !e.contains("cleanup error"),
        "cleanup error must be outranked: {e}"
    );
    assert_eq!(
        *ran.lock().expect("lock"),
        vec!["cleaned"],
        "cleanup must run on failure"
    );
}

#[test]
fn cleanup_error_alone_fails_the_scenario() {
    let mut reg: StepRegistry<()> = StepRegistry::new();
    reg.define_exact("passing with bad cleanup", |ctx, _, _| {
        ctx.defer(|_| panic!("leaky cleanup"));
    });
    let steps = steps_of("Feature: f\nScenario: s\n  Given passing with bad cleanup\n");
    let e = execute_steps(&steps, &reg, ()).expect_err("must fail");
    assert!(e.contains("cleanup failed"), "got: {e}");
    assert!(e.contains("leaky cleanup"), "got: {e}");
}

// --- The binding guard (pure form) -----------------------------------------------

#[test]
fn binding_guard_reports_ambiguity() {
    let mut reg: StepRegistry<()> = StepRegistry::new();
    reg.define(r"^a step$", |_, _, _| {});
    reg.define(r"step", |_, _, _| {});
    let p =
        parse_feature("Feature: f\nScenario: s\n  Given a step\n", "f.feature").expect("parses");
    let e = check_bindings(&p, &reg, "f", false).expect_err("must fail");
    assert!(e.contains("matching >1 definition"), "got: {e}");
    assert!(e.contains("\"a step\""), "got: {e}");
    // Ambiguity is a hard error even for wip features.
    check_bindings(&p, &reg, "f", true).expect_err("ambiguity must fail even when wip");
}

#[test]
fn binding_guard_ratchets_unbound_steps_with_snippets() {
    let reg: StepRegistry<()> = StepRegistry::new();
    let p = parse_feature(
        "Feature: f\nScenario: a\n  Given lonely 1\nScenario: b\n  Given lonely 1\n  And lonely 2\n",
        "f.feature",
    )
    .expect("parses");
    let e = check_bindings(&p, &reg, "f", false).expect_err("must fail");
    assert!(e.contains("unbound steps"), "got: {e}");
    assert!(e.contains(".wip()"), "got: {e}");
    assert!(e.contains("// lonely 1"), "got: {e}");
    assert!(e.contains("// lonely 2"), "got: {e}");
    // deduped: "lonely 1" appears in two scenarios but once in the message
    assert_eq!(
        e.matches("// lonely 1").count(),
        1,
        "unbound steps must dedupe: {e}"
    );
    // wip relaxes the ratchet (and only the ratchet)
    check_bindings(&p, &reg, "f", true).expect("wip allows unbound");
}

#[test]
fn binding_guard_checks_background_and_skip_scenarios_too() {
    let reg: StepRegistry<()> = StepRegistry::new();
    let p = parse_feature(
        "Feature: f\nBackground:\n  Given base\n@skip\nScenario: s\n  Given skipped-but-must-bind\n",
        "f.feature",
    )
    .expect("parses");
    let e = check_bindings(&p, &reg, "f", false).expect_err("must fail");
    assert!(e.contains("// base"), "background steps are ratcheted: {e}");
    assert!(
        e.contains("// skipped-but-must-bind"),
        "skip means don't run, never don't bind: {e}"
    );
}

// --- 0.6.0: Feature with no scenarios --------------------------------------------

#[test]
fn a_feature_with_no_scenarios_is_rejected_at_the_feature_line() {
    // A header + narrative registers nothing: zero trials, zero assertions,
    // and a green run. Same hazard as a scenario with no steps, one level up.
    assert_rejects(
        "Feature: Charge voting\n  Ties break toward the lower charge.\n",
        1,
        "Feature \"Charge voting\" has no scenarios",
    );
}

#[test]
fn the_no_scenarios_error_names_a_construct_near_miss_when_one_emptied_the_file() {
    // lint_feature returns early on a dialect error, so its near-miss scan
    // never runs for this file — the hint keeps the diagnostic from being
    // masked.
    let e = err_of("Feature: F\nscenario: s\n  given a\n  then ok\n");
    assert!(
        e.to_string()
            .contains("(line 2 \"scenario:\" is not the exact construct keyword \"Scenario:\")"),
        "missing hint: {e}"
    );
}

#[test]
fn a_background_alone_does_not_count_as_a_scenario() {
    assert_rejects(
        "Feature: F\nBackground:\n  Given a\n",
        1,
        "has no scenarios",
    );
}

#[test]
fn a_skip_only_scenario_still_counts_skip_is_not_absence() {
    let p = parse_feature(
        "Feature: F\n@skip\nScenario: s\n  Given a\n  Then b\n",
        "t.feature",
    )
    .expect("parses");
    assert_eq!(p.scenarios.len(), 1);
}

// --- 0.6.0: OutlineMeta records header, header_line, placeholders -----------------

#[test]
fn outline_meta_records_header_line_and_referenced_placeholders() {
    let p = parse_feature(
        concat!(
            "Feature: F\nScenario Outline: t <title>\n  When I add:\n    | v |\n    | <cell> |\n",
            "  Then ok <x>\n  Examples:\n    | title | cell | x | spare |\n    | a | b | c | d |\n    | e | f | g | h |\n",
        ),
        "t.feature",
    )
    .expect("parses");
    assert_eq!(p.outlines.len(), 1);
    let o = &p.outlines[0];
    assert_eq!(o.name, "t <title>");
    assert_eq!(o.line, 2);
    assert_eq!(o.rows, 2);
    assert_eq!(o.header, vec!["title", "cell", "x", "spare"]);
    assert_eq!(o.header_line, 8);
    // First-appearance order: title, then step text/tables in step order.
    assert_eq!(o.placeholders, vec!["title", "cell", "x"]);
}
