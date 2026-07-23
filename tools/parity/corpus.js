// Differential parity corpus: node parser vs cargo parser. Each case is raw
// .feature text; the harness runs BOTH parsers and diffs the canonical dumps,
// so no case carries an expectation — disagreement itself is the finding.
// Sources: the README rejection matrix, test/harness.test.js, and crafted
// edge cases where two independent implementations plausibly diverge.

const A = {}; // expected-accept-ish (either way, both must agree)
const R = {}; // expected-reject-ish

// --- core grammar ---------------------------------------------------------------
A['basic'] = `Feature: Counter
  As a tester
  I want arithmetic
  So that sums exist

  Background:
    Given a counter at 0

  # a comment
  Scenario: increment once
    When I add 5
    Then the counter is 5
`;
A['outline-expansion'] = `Feature: F
  Scenario Outline: add <n> to get <total>
    When I add <n>
    Then the counter is <total>
    Examples:
      | n  | total |
      | 2  | 2     |
      | 10 | 10    |
`;
A['outline-placeholder-in-table'] = `Feature: F
  Scenario Outline: rows
    Given a table
      | k     | v   |
      | <key> | <v> |
    Then it holds
    Examples:
      | key | v |
      | a   | 1 |
      | b   | 2 |
`;
A['star-and-butt-keywords'] = `Feature: F
  Scenario: s
    * a precondition
    Given another
    And more
    But not this
    When poked
    Then it beeps
`;
A['step-table-basic'] = `Feature: F
  Scenario: s
    Given users
      | name  | role  |
      | ada   | admin |
      | linus | dev   |
    Then ok
`;
A['background-with-table'] = `Feature: F
  Background:
    Given config
      | k | v |
      | a | 1 |
  Scenario: s
    Then ok
`;

// --- escapes (adversarial-review hotspot: table-cell interpretation) -------------
A['cell-escapes'] = `Feature: F
  Scenario: s
    Given cells
      | a\\|b | c\\\\d | e\\nf | Cmd+\\ |
    Then ok
`;
A['cell-literal-backslash'] = `Feature: F
  Scenario: s
    Given paths
      | C:\\Temp | \\q | x\\ty |
    Then ok
`;
A['examples-escapes-and-subst'] = `Feature: F
  Scenario Outline: e
    Given cell <c>
    Then ok
    Examples:
      | c      |
      | a\\|b  |
      | e\\nf  |
`;
A['empty-cells'] = `Feature: F
  Scenario: s
    Given cells
      | a |  | b |
      |  |  |  |
    Then ok
`;

// --- tags -----------------------------------------------------------------------
A['feature-tags-inherit'] = `@smoke @AC1
Feature: F
  Scenario: plain
    Then ok
  @extra
  Scenario: tagged
    Then ok
`;
A['multi-tag-lines'] = `Feature: F
  @a
  @b @c
  Scenario: s
    Then ok
`;
A['same-tag-twice'] = `@skip
Feature: F
  @skip
  Scenario: s
    Given x
`;
A['only-tag-parses'] = `Feature: F
  @only
  Scenario: s
    Then ok
`;
A['glued-tags-token'] = `Feature: F
  @a@b
  Scenario: s
    Then ok
`;

// --- silent-narrative boundary ---------------------------------------------------
A['misspelled-keyword-is-narrative'] = `Feature: F
  Scenario: s
    Givenx not a step
    Given real
    Then ok
`;
A['non-english-keyword-is-narrative'] = `Feature: F
  Scenario: s
    Etantdonné un compteur
    Given real
    Then ok
`;
A['keyword-alone-is-narrative'] = `Feature: F
  Scenario: s
    Given
    Given real
    Then ok
`;

// --- whitespace / line-ending edges ----------------------------------------------
A['crlf'] = 'Feature: F\r\nScenario: s\r\n  Given x\r\n  Then ok\r\n';
A['trailing-blank-lines'] = `Feature: F
  Scenario: s
    Then ok



`;
A['comment-inside-table'] = `Feature: F
  Scenario: s
    Given cells
      | a | b |
      # interleaved comment
      | c | d |
    Then ok
`;
A['comment-inside-examples'] = `Feature: F
  Scenario Outline: e
    Given <x>
    Then ok
    Examples:
      | x |
      # interleaved comment
      | 1 |
      | 2 |
`;
A['empty-scenario-name'] = `Feature: F
  Scenario:
    Then ok
`;
A['empty-feature-name'] = `Feature:
  Scenario: s
    Then ok
`;
A['no-eol-at-eof'] = 'Feature: F\nScenario: s\n  Then ok';

// --- lint parity: banned-word matrix + boundary/folding hostiles -------------------
// The unit tests for lintFeature/lint_feature are hand-ported between the two
// repos — exactly the drift class this harness exists to distrust — so every
// banned word, casing, and boundary case is ALSO held differentially here.
// The kelvin case is a survivor's trophy: rust's (?i) Unicode folding matched
// K (U+212A) as k while JS's non-unicode /i refused, until the rust pattern
// pinned ASCII folding with (?-u:…).
A['vague-each-banned-word'] = `Feature: F
  Scenario: a
    When poked
    Then it works
  Scenario: b
    When poked
    Then it renders correctly
  Scenario: c
    When poked
    Then it is handled properly
  Scenario: d
    When poked
    Then output is as expected
  Scenario: e
    When poked
    Then it handles errors
  Scenario: f
    When poked
    Then an appropriate response is sent
`;
A['vague-case-variants'] = `Feature: F
  Scenario: upper
    When poked
    Then it WORKS
  Scenario: title
    When poked
    Then it Works
  Scenario: mixed
    When poked
    Then output is As ExPeCtEd
`;
A['vague-non-matches'] = `Feature: F
  Scenario: containment
    When poked
    Then the workshop opens and reworks nothing
  Scenario: adverb-dodges
    When poked
    Then it responds appropriately
  Scenario: participle-dodges
    When poked
    Then the error is handled
  Scenario: underscore-glue
    When poked
    Then works_ is a symbol name
`;
A['vague-unicode-adjacency'] = `Feature: F
  Scenario: kelvin
    When poked
    Then it worKs
  Scenario: long-s
    When poked
    Then it workſ
  Scenario: acute-suffix
    When poked
    Then it worksé
  Scenario: acute-prefix
    When poked
    Then éworks here
`;
A['no-then-and-single-row-compose'] = `Feature: F
  Scenario: no assertion
    When I poke it
  Scenario Outline: lonely and vague
    When I add <n>
    Then it works
    Examples:
      | n |
      | 1 |
`;

// --- rejection matrix, one case per row -------------------------------------------
R['docstring-triple-quote'] = `Feature: F
  Scenario: s
    Given a
    """
    body
    """
`;
R['docstring-backticks'] = `Feature: F
  Scenario: s
    Given a
    \`\`\`
`;
R['rule-keyword'] = `Feature: F
Rule: grouping
`;
R['multiple-features'] = `Feature: a
Feature: b
`;
R['multiple-backgrounds'] = `Feature: F
Background:
  Given a
Background:
`;
R['background-after-scenario'] = `Feature: F
Scenario: s
  Given a
Background:
`;
R['multiple-examples'] = `Feature: F
Scenario Outline: o
  Given <x>
  Examples:
    | x |
    | 1 |
  Examples:
    | x |
    | 2 |
`;
R['examples-header-only'] = `Feature: F
Scenario Outline: o
  Given <x>
  Examples:
    | x |
`;
R['examples-no-header'] = `Feature: F
Scenario Outline: o
  Given <x>
  Examples:
`;
R['examples-outside-outline'] = `Feature: F
Scenario: s
  Given a
  Examples:
    | x |
    | 1 |
`;
R['outline-no-examples'] = `Feature: F
Scenario Outline: o
  Given <x>
`;
R['outline-no-steps'] = `Feature: F
Scenario Outline: o
  Examples:
    | x |
    | 1 |
`;
R['ragged-examples-row'] = `Feature: F
Scenario Outline: o
  Given <x>
  Examples:
    | x | y |
    | 1 |
`;
R['ragged-step-table'] = `Feature: F
Scenario: s
  Given cells
    | a | b |
    | c |
  Then ok
`;
R['row-missing-closing-pipe'] = `Feature: F
Scenario: s
  Given cells
    | a | b
  Then ok
`;
R['lone-pipe-row'] = `Feature: F
Scenario: s
  Given cells
    |
  Then ok
`;
R['table-before-step'] = `Feature: F
Scenario: s
  | a |
`;
R['table-before-scenario'] = `Feature: F
  | a |
`;
R['step-before-scenario'] = `Feature: F
Given orphan
`;
R['step-after-examples'] = `Feature: F
Scenario Outline: o
  Given <x>
  Examples:
    | x |
    | 1 |
  Then late step
`;
R['unknown-placeholder'] = `Feature: F
Scenario Outline: o
  Given <x> and <typo>
  Examples:
    | x |
    | 1 |
`;
R['scenario-no-steps'] = `Feature: F
Scenario: empty
Scenario: real
  Then ok
`;
R['no-feature-line'] = `Scenario: s
  Given x
`;
R['empty-file'] = '';
R['whitespace-only'] = '   \n\n  \n';
R['dangling-tags-eof'] = `Feature: F
Scenario: s
  Then ok
@dangling
`;
R['tag-before-background'] = `Feature: F
@nope
Background:
  Given a
`;
R['tag-before-step'] = `Feature: F
Scenario: s
  @nope
  Then ok
`;
R['tag-before-examples'] = `Feature: F
Scenario Outline: o
  Given <x>
  @nope
  Examples:
    | x |
    | 1 |
`;
R['near-miss-skip'] = `Feature: F
@Skip
Scenario: s
  Then ok
`;
R['near-miss-upper-only'] = `Feature: F
@ONLY
Scenario: s
  Then ok
`;
R['conflicting-tags-same-line'] = `Feature: F
@skip @todo
Scenario: s
  Then ok
`;
R['conflicting-feature-scenario-tags'] = `@skip
Feature: F
@only
Scenario: s
  Then ok
`;
R['second-feature-after-content'] = `Feature: F
Scenario: s
  Then ok
Feature: G
`;

// --- 0.6.0: duplicate-title, unused-column, no-scenarios --------------------------
A['duplicate-title-plain'] = `Feature: F
  Scenario: twin
    Given a
    Then b
  Scenario: twin
    Given a
    Then b
`;
A['duplicate-title-outline-pair'] = `Feature: F
  Scenario Outline: adds <a>
    When I add <a>
    Then I see <a>
    Examples:
      | a |
      | 1 |
      | 2 |
  Scenario Outline: adds <a>
    When I add <a>
    Then I see <a>
    Examples:
      | a |
      | 1 |
      | 2 |
`;
A['duplicate-title-backstop'] = `Feature: F
  Scenario Outline: adds <a>
    When I add <a>
    Then I see <a>
    Examples:
      | a |
      | 1 |
      | 2 |
  Scenario: adds 1 [1]
    Given a
    Then b
`;
A['unused-column-label'] = `Feature: F
  Scenario Outline: adds <a>
    When I add <a>
    Then I see <a>
    Examples:
      | case  | a | expected |
      | small | 1 | 1        |
      | big   | 9 | 9        |
`;
A['unused-column-table-cell-ref'] = `Feature: F
  Scenario Outline: rows
    Given a table
      | v      |
      | <cell> |
    Then ok <x>
    Examples:
      | cell | x |
      | a    | 1 |
      | b    | 2 |
`;
R['no-scenarios-header-narrative'] = `Feature: Overdraft alerts
  The ledger flags any account below zero.
  Alerts go out before the close of day.
`;
R['no-scenarios-near-miss-hint'] = `Feature: F
scenario: s
  given a
  then ok
`;
R['no-scenarios-background-only'] = `Feature: F
  Background:
    Given a
`;

module.exports = { cases: { ...A, ...R } };
