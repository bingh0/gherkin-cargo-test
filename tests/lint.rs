// lint_feature self-tests, ported 1:1 from the node sibling's
// test/lint.test.js. Every rule gets a positive (fires) and a negative
// (stays quiet) case — a linter that can't stay quiet on a good spec is
// noise, and noise gets allowlisted into silence. Finding TEXT parity with
// the node sibling is verified separately (differentially) by tools/parity.

use gherkin_cargo_test::{lint_feature, LintFinding, LintRule, LintSeverity};

fn feat(body: &str) -> String {
    format!("Feature: Lint demo\n{body}")
}

fn rules(findings: &[LintFinding]) -> Vec<&'static str> {
    findings.iter().map(|f| f.rule.as_str()).collect()
}

// --- quiet on a good spec -------------------------------------------------------

#[test]
fn no_findings_for_a_well_formed_feature() {
    let findings = lint_feature(
        &feat(concat!(
            "Background:\n  Given a counter at 0\n",
            "Scenario: increment once\n  When I add 5\n  Then the counter is 5\n",
            "Scenario Outline: add amounts\n  When I add <n>\n  Then the counter is <total>\n",
            "  Examples:\n    | n | total |\n    | 2 | 2 |\n    | 10 | 10 |\n",
        )),
        "<feature>",
    );
    assert!(findings.is_empty(), "expected none, got {findings:?}");
}

// --- dialect gate ----------------------------------------------------------------

#[test]
fn dialect_is_a_single_error_finding_not_an_err() {
    let findings = lint_feature("Feature: F\nRule: not supported\n", "x.feature");
    assert_eq!(findings.len(), 1);
    let f = &findings[0];
    assert_eq!(f.rule, LintRule::Dialect);
    assert_eq!(f.severity, LintSeverity::Error);
    assert_eq!(f.line, 2);
    assert!(f.message.contains("Rule: keyword is not supported"));
    // The message must not re-embed the file:line prefix the finding structures.
    assert!(
        !f.message.contains("x.feature:2:"),
        "prefix not stripped: {}",
        f.message
    );
}

#[test]
fn dialect_suppresses_all_other_lints() {
    let findings = lint_feature(
        &feat("Scenario: no then, but unparseable later\n  When I poke it\n\"\"\"\ndoc\n\"\"\"\n"),
        "<feature>",
    );
    assert_eq!(rules(&findings), ["dialect"]);
}

// --- no-then ---------------------------------------------------------------------

#[test]
fn no_then_flags_a_given_when_only_scenario_at_its_line() {
    let findings = lint_feature(
        &feat("Scenario: poke\n  Given a thing\n  When I poke it\n"),
        "<feature>",
    );
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].rule, LintRule::NoThen);
    assert_eq!(findings[0].severity, LintSeverity::Warn);
    assert_eq!(findings[0].line, 2);
    assert!(findings[0].message.contains("\"poke\""));
    assert!(findings[0].message.contains("asserts nothing"));
}

#[test]
fn and_after_then_inherits_then_and_satisfies() {
    let findings = lint_feature(
        &feat("Scenario: ok\n  When I poke it\n  Then it beeps\n  And the light is green\n"),
        "<feature>",
    );
    assert!(findings.is_empty(), "{findings:?}");
}

#[test]
fn and_after_when_stays_when() {
    let findings = lint_feature(
        &feat("Scenario: tail\n  When I poke it\n  And I poke it again\n"),
        "<feature>",
    );
    assert_eq!(rules(&findings), ["no-then"]);
}

#[test]
fn background_then_resolves_across_the_boundary() {
    // Odd spec style, but keyword resolution must cross the Background
    // boundary the same way execution order does.
    let findings = lint_feature(
        &feat(concat!(
            "Background:\n  Given a counter at 0\n  Then the counter exists\n",
            "Scenario: continues\n  And the counter is 0\n",
        )),
        "<feature>",
    );
    assert!(findings.is_empty(), "{findings:?}");
}

#[test]
fn outline_without_then_is_flagged_once_not_per_row() {
    let findings = lint_feature(
        &feat(concat!(
            "Scenario Outline: poke <n> times\n  When I poke it <n> times\n",
            "  Examples:\n    | n |\n    | 1 |\n    | 2 |\n    | 3 |\n",
        )),
        "<feature>",
    );
    assert_eq!(rules(&findings), ["no-then"]);
    assert!(findings[0]
        .message
        .contains("Scenario Outline \"poke <n> times\""));
}

// --- vague-then ------------------------------------------------------------------

#[test]
fn each_banned_word_fires_at_the_step_line() {
    for bad in [
        "it works",
        "it renders correctly",
        "it is handled properly",
        "output is as expected",
        "it handles errors",
        "an appropriate response is sent",
    ] {
        let findings = lint_feature(
            &feat(&format!("Scenario: s\n  When I poke it\n  Then {bad}\n")),
            "<feature>",
        );
        assert_eq!(
            findings.len(),
            1,
            "expected one finding for {bad:?}: {findings:?}"
        );
        assert_eq!(findings[0].rule, LintRule::VagueThen);
        assert_eq!(findings[0].line, 4);
        assert!(findings[0].message.contains("name the observable result"));
    }
}

#[test]
fn only_then_resolved_steps_are_checked() {
    // "works" in a Given/When describes state, not an unchecked assertion.
    let findings = lint_feature(
        &feat("Scenario: s\n  Given the pump works\n  When I poke it\n  Then the gauge reads 5\n"),
        "<feature>",
    );
    assert!(findings.is_empty(), "{findings:?}");
}

#[test]
fn an_and_inheriting_then_is_checked_too() {
    let findings = lint_feature(
        &feat("Scenario: s\n  When I poke it\n  Then the gauge reads 5\n  And everything works\n"),
        "<feature>",
    );
    assert_eq!(rules(&findings), ["vague-then"]);
    assert_eq!(findings[0].line, 5);
}

#[test]
fn row_independent_vague_step_in_an_outline_fires_once() {
    let findings = lint_feature(
        &feat(concat!(
            "Scenario Outline: add <n>\n  When I add <n>\n  Then it works\n",
            "  Examples:\n    | n |\n    | 1 |\n    | 2 |\n",
        )),
        "<feature>",
    );
    assert_eq!(rules(&findings), ["vague-then"]);
}

#[test]
fn substitution_introduced_vagueness_fires_for_exactly_those_rows() {
    let findings = lint_feature(
        &feat(concat!(
            "Scenario Outline: add <n>\n  When I add <n>\n  Then the counter <outcome>\n",
            "  Examples:\n    | n | outcome |\n    | 1 | is 1 |\n    | 2 | works |\n    | 3 | works |\n",
        )),
        "<feature>",
    );
    // Rows 2 and 3 both substitute to "the counter works" — identical text on
    // the same source line collapses to one finding; row 1 is clean.
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert_eq!(findings[0].rule, LintRule::VagueThen);
    assert!(findings[0].message.contains("the counter works"));
}

// --- single-row-outline ------------------------------------------------------------

#[test]
fn one_data_row_is_flagged_two_are_not() {
    let one = lint_feature(
        &feat(concat!(
            "Scenario Outline: add <n>\n  When I add <n>\n  Then the counter is <n>\n",
            "  Examples:\n    | n |\n    | 1 |\n",
        )),
        "<feature>",
    );
    assert_eq!(rules(&one), ["single-row-outline"]);
    assert_eq!(one[0].line, 2);

    let two = lint_feature(
        &feat(concat!(
            "Scenario Outline: add <n>\n  When I add <n>\n  Then the counter is <n>\n",
            "  Examples:\n    | n |\n    | 1 |\n    | 2 |\n",
        )),
        "<feature>",
    );
    assert!(two.is_empty(), "{two:?}");
}

// --- ordering & composition -------------------------------------------------------

#[test]
fn findings_are_sorted_by_line_across_rules() {
    let findings = lint_feature(
        &feat(concat!(
            "Scenario: no assertion\n  When I poke it\n",
            "Scenario Outline: lonely row\n  When I add <n>\n  Then it works\n",
            "  Examples:\n    | n |\n    | 1 |\n",
        )),
        "<feature>",
    );
    let got: Vec<(&str, usize)> = findings.iter().map(|f| (f.rule.as_str(), f.line)).collect();
    assert_eq!(
        got,
        [("no-then", 2), ("single-row-outline", 4), ("vague-then", 6)]
    );
}

// --- near-miss-keyword -------------------------------------------------------------

#[test]
fn a_wrong_case_keyword_inside_a_scenario_is_flagged() {
    // The scenario keeps a Given and a Then, so neither the no-steps guard nor
    // no-then fires. Without this rule the "when" line vanishes in silence.
    let findings = lint_feature(
        &feat("Scenario: increment once\n  Given a counter at 0\n  when I add 5\n  Then the counter is 5\n"),
        "<feature>",
    );
    assert_eq!(rules(&findings), ["near-miss-keyword"]);
    assert_eq!(findings[0].line, 4);
    assert_eq!(findings[0].severity, LintSeverity::Warn);
    assert!(findings[0]
        .message
        .contains("\"when\" is not the step keyword \"When\""));
}

#[test]
fn any_casing_that_is_not_the_exact_spelling_is_flagged() {
    for (bad, good) in [
        ("GIVEN", "Given"),
        ("gIvEn", "Given"),
        ("THEN", "Then"),
        ("and", "And"),
        ("BUT", "But"),
    ] {
        let findings = lint_feature(
            &feat(&format!(
                "Scenario: S\n  Given a counter at 0\n  {bad} something happens\n  Then the counter is 5\n"
            )),
            "<feature>",
        );
        assert_eq!(
            rules(&findings),
            ["near-miss-keyword"],
            "{bad} should be flagged"
        );
        assert!(
            findings[0]
                .message
                .contains(&format!("is not the step keyword \"{good}\"")),
            "{bad}: {}",
            findings[0].message
        );
    }
}

#[test]
fn fires_alongside_no_then_when_the_lost_step_was_the_only_then() {
    let findings = lint_feature(
        &feat("Scenario: S\n  Given a counter at 0\n  then the counter is 0\n"),
        "<feature>",
    );
    assert_eq!(rules(&findings), ["no-then", "near-miss-keyword"]);
}

#[test]
fn a_lost_step_that_empties_the_scenario_stays_a_dialect_error() {
    // The no-steps guard fails the parse first, and a dialect finding is always alone.
    let findings = lint_feature(
        &feat("Scenario: S\n  given a counter at 0\n  then it is 0\n"),
        "<feature>",
    );
    assert_eq!(rules(&findings), ["dialect"]);
}

#[test]
fn the_feature_narrative_is_prose_and_is_never_flagged() {
    // This is why the step half is scoped to scenario bodies. A CORRECTLY
    // cased step out here is already the dialect error "step before any Scenario".
    let findings = lint_feature(
        concat!(
            "Feature: F\n  As a user\n  when the store is seeded I want a counter\n",
            "  and I want it to start at zero\n  So that life is good\n\n",
            "Scenario: S\n  Given a counter at 0\n  Then the counter is 0\n",
        ),
        "<feature>",
    );
    assert!(findings.is_empty(), "expected none, got {findings:?}");
}

#[test]
fn background_bodies_are_checked_too() {
    let findings = lint_feature(
        &feat(concat!(
            "Background:\n  and a seeded store\n  Given a counter at 0\n",
            "Scenario: S\n  Then the counter is 0\n",
        )),
        "<feature>",
    );
    assert_eq!(rules(&findings), ["near-miss-keyword"]);
    assert_eq!(findings[0].line, 3);
}

#[test]
fn stays_quiet_on_words_that_merely_begin_with_a_keyword() {
    let findings = lint_feature(
        &feat("Scenario: S\n  Given a counter at 0\n  Givens are not keywords\n  Then the counter is 0\n"),
        "<feature>",
    );
    assert!(findings.is_empty(), "expected none, got {findings:?}");
}

#[test]
fn a_bare_keyword_with_no_step_text_is_ordinary_narrative() {
    // "given" alone could not have been a step at any casing, so flagging it
    // would be a false positive rather than a rescued requirement.
    let findings = lint_feature(
        &feat("Scenario: S\n  Given a counter at 0\n  given\n  Then the counter is 0\n"),
        "<feature>",
    );
    assert!(findings.is_empty(), "expected none, got {findings:?}");
}

#[test]
fn a_tab_between_keyword_and_text_is_a_real_step_not_a_near_miss() {
    let findings = lint_feature(
        &feat("Scenario: S\n  Given\ta counter at 0\n  When I add 5\n  Then the counter is 5\n"),
        "<feature>",
    );
    assert!(findings.is_empty(), "expected none, got {findings:?}");
}

#[test]
fn comment_tag_and_table_lines_are_not_mistaken_for_steps() {
    let findings = lint_feature(
        &feat(concat!(
            "Scenario Outline: add <n>\n  # when I add things\n  Given a counter at 0\n",
            "  When I add <n>\n  Then the counter is <n>\n",
            "  Examples:\n    | n |\n    | 1 |\n    | 2 |\n",
        )),
        "<feature>",
    );
    assert!(findings.is_empty(), "expected none, got {findings:?}");
}

#[test]
fn a_lowercase_scenario_silently_merges_into_the_previous_scenario() {
    // Without the construct half of this rule the file is finding-free:
    // scenario "b" never exists, so the no-steps guard cannot fire, and its
    // Then merges into "a", so no-then cannot fire either. The scenario does
    // not weaken — it vanishes.
    let findings = lint_feature(
        &feat("Scenario: a\n  Given x\n  Then y\nscenario: b\n  Given p\n  Then q\n"),
        "<feature>",
    );
    assert_eq!(rules(&findings), ["near-miss-keyword"]);
    assert_eq!(findings[0].line, 5);
    assert!(
        findings[0]
            .message
            .contains("\"scenario:\" is not the construct keyword \"Scenario:\""),
        "{}",
        findings[0].message
    );
}

#[test]
fn construct_headers_are_exact_form_not_merely_exact_case() {
    // Wrong spacing is dropped exactly as silently as wrong case.
    for (bad, shown) in [
        ("Scenario : b", "Scenario :"),
        ("SCENARIO: b", "SCENARIO:"),
        ("ScenarioOutline: b", "ScenarioOutline:"),
        ("scenario:b", "scenario:"),
    ] {
        let findings = lint_feature(
            &feat(&format!(
                "Scenario: a\n  Given x\n  Then y\n{bad}\n  Given p\n  Then q\n"
            )),
            "<feature>",
        );
        assert_eq!(
            rules(&findings),
            ["near-miss-keyword"],
            "{bad} should be flagged"
        );
        assert!(
            findings[0]
                .message
                .starts_with(&format!("\"{shown}\" is not the construct keyword")),
            "{bad}: {}",
            findings[0].message
        );
    }
}

#[test]
fn an_outline_typo_and_its_lowercase_examples_are_both_flagged() {
    // "Scenario outline:" is narrative, so its steps merge into scenario "a";
    // "examples:" is narrative too, so the table under it glues itself to the
    // last merged step as a data table. The file parses — two findings.
    let findings = lint_feature(
        &feat(concat!(
            "Scenario: a\n  Given x\n  Then y\n",
            "Scenario outline: add <n>\n  Given a counter\n  When I add <n>\n  Then I get <n>\n",
            "  examples:\n    | n |\n    | 1 |\n",
        )),
        "<feature>",
    );
    assert_eq!(rules(&findings), ["near-miss-keyword", "near-miss-keyword"]);
    let lines: Vec<usize> = findings.iter().map(|f| f.line).collect();
    assert_eq!(lines, [5, 9]);
    assert!(findings[0]
        .message
        .contains("\"Scenario outline:\" is not the construct keyword \"Scenario Outline:\""));
    assert!(findings[1]
        .message
        .contains("\"examples:\" is not the construct keyword \"Examples:\""));
}

#[test]
fn construct_near_misses_are_flagged_outside_bodies_too() {
    // The step check is body-scoped, but constructs are recognized anywhere,
    // so their near misses matter anywhere — including the Feature narrative.
    let findings = lint_feature(
        concat!(
            "Feature: F\n  As a user\n  background: a seeded store\n\n",
            "Scenario: S\n  Given a counter at 0\n  Then the counter is 0\n",
        ),
        "<feature>",
    );
    assert_eq!(rules(&findings), ["near-miss-keyword"]);
    assert_eq!(findings[0].line, 3);
}

#[test]
fn construct_like_prose_without_the_colon_shape_stays_quiet() {
    // Plural or unlisted words do not form a construct header; `rule:` is
    // exempt because the exact `Rule:` is itself a dialect error, so a near
    // miss is not a rescue — and "rule: …" is plausible prose.
    let findings = lint_feature(
        concat!(
            "Feature: F\n  scenarios: covered in the payments epic\n",
            "  features: split per team\n  example: the happy path\n\n",
            "Scenario: S\n  Given a counter at 0\n  rule: refunds beat store credit\n",
            "  Then the counter is 0\n",
        ),
        "<feature>",
    );
    assert!(findings.is_empty(), "expected none, got {findings:?}");
}
