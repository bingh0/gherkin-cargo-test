//! gherkin-cargo-test
//! A tiny, honest Gherkin runner on top of `cargo test`, via libtest-mimic.
//!
//! This is the Rust sibling of gherkin-node-test (same author, same philosophy,
//! same grammar). It parses the practical core of Gherkin — Feature / Background /
//! Scenario / Scenario Outline + Examples, with Given·When·Then·And·But·* steps,
//! step-level data tables, and @skip/@todo tags — and turns each scenario into a
//! libtest-mimic Trial run under `cargo test`. Scenario Outlines are expanded
//! once per Examples row.
//!
//! The high-level entry point is the `Features` builder: it discovers every
//! *.feature in a directory, runs each against its OWN scoped registry with its
//! OWN typed World (step patterns and world state never leak between features),
//! and registers guard trials that fail on ambiguous steps, on unbound steps
//! (which would otherwise register as ignored — reported GREEN by the runner),
//! and on definer keys that match no feature file. A feature still being
//! bootstrapped opts out of the unbound-step ratchet by name via `.wip(base)`.
//!
//! SUPPORTED grammar (the practical core, guarded loudly):
//!   Feature:            one per file, required
//!   Background:         optional, at most one, before any Scenario
//!   Scenario:           free text title
//!   Scenario Outline:   + exactly one Examples: table; <placeholder> substitution
//!   Examples:           a leading header row then >=1 data row, pipe-delimited
//!   Steps:              Given | When | Then | And | But | *   followed by text
//!   Step data tables:   | rows after a step attach to it; the step closure
//!                       receives a cucumber-compatible DataTable as its last
//!                       argument (raw/rows/hashes/rows_hash/transpose). Cells
//!                       honor \| \\ \n escapes; other backslashes are literal.
//!   Tags:               @skip marks the trial ignored (steps must still BIND —
//!                       skip means "don't run", never "don't bind"); @todo runs
//!                       the scenario but a failure doesn't gate the suite; tags
//!                       on Feature: apply to all its scenarios; all other tags
//!                       (e.g. @AC3) are carried but have no effect. @only is
//!                       REJECTED loudly — use `cargo test <name filter>`.
//!                       Combining @skip/@todo/@only on one scenario is a loud
//!                       parse error — runners disagree on which would win.
//!   Comments (# ...) and the Feature narrative are ignored.
//!
//! DELIBERATELY NOT SUPPORTED. Structural misuse is REJECTED LOUDLY — each
//! returns a GherkinSyntaxError with file:line, so a feature file can't pass
//! *vacuously* by being silently mis-parsed:
//!   - doc strings (""" or ```)            - the Rule: keyword (Gherkin 6)
//!   - multiple Examples per Outline       - a step after its Examples table
//!   - a Scenario/Outline with no steps    - a table row with no preceding step
//!   - ragged table rows                   - a table row missing its closing |
//!   - tags anywhere but immediately before Feature:/Scenario:/Scenario Outline:
//!
//! Two non-features are NOT special-cased, by design (no dedicated error):
//!   - Cucumber Expressions ({int}, …): step text is matched by regex via
//!     StepRegistry — write a regex; there is no {int} expansion.
//!   - i18n: English keywords only. A non-English keyword line is treated as
//!     narrative and ignored; if that leaves a scenario empty the no-steps guard
//!     fires, so it still can't pass vacuously.
//!
//! If you need the real thing, reach for the `gherkin` crate or cucumber-rs.
//! See README.md for the full grammar and rationale.
//!
//! Two boring dependencies — `regex` (step matching) and `libtest-mimic`
//! (per-scenario Trials under `cargo test`). Zero-dependency is a non-goal here:
//! hand-rolling a regex engine or a test protocol would be its own foot-gun.

use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use libtest_mimic::{Arguments, Failed, Trial};
use regex::Regex;

// --- Error --------------------------------------------------------------------

/// Returned when a feature file uses syntax this parser does not support, or a
/// malformed construct it would otherwise mis-read. The message is prefixed
/// with `file:line:` and `.line` carries the 1-based line number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GherkinSyntaxError {
    pub line: usize,
    message: String,
}

impl GherkinSyntaxError {
    fn new(filename: &str, line: usize, msg: &str) -> Self {
        Self {
            line,
            message: format!("{filename}:{line}: {msg}"),
        }
    }
}

impl fmt::Display for GherkinSyntaxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for GherkinSyntaxError {}

// --- Parsed types ---------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub keyword: String,
    pub text: String,
    pub table: Option<Vec<Vec<String>>>,
    /// 1-based line in the .feature file. Steps expanded from a Scenario
    /// Outline keep their own source line (the SCENARIO carries the Outline's
    /// line) — verified against the node sibling by the differential parity
    /// harness (tools/parity).
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scenario {
    pub name: String,
    pub steps: Vec<Step>,
    pub line: usize,
    pub tags: Vec<String>,
}

/// One `Scenario Outline:` as written in the source, before expansion —
/// enough for lint rules that reason about the construct rather than its
/// expanded rows. Mirrors the node sibling's `ParsedFeature.outlines`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutlineMeta {
    pub name: String,
    pub line: usize,
    pub rows: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedFeature {
    pub feature: String,
    pub background: Vec<Step>,
    pub scenarios: Vec<Scenario>,
    pub outlines: Vec<OutlineMeta>,
}

// --- Data tables ----------------------------------------------------------------

/// A step's data table, API-compatible with cucumber's DataTable so step code
/// (and muscle memory) ports both ways.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataTable {
    raw: Vec<Vec<String>>,
}

impl DataTable {
    pub fn new(raw: Vec<Vec<String>>) -> Self {
        Self { raw }
    }

    /// A copy of every row.
    pub fn raw(&self) -> Vec<Vec<String>> {
        self.raw.clone()
    }

    /// All rows except the first (header) row.
    pub fn rows(&self) -> Vec<Vec<String>> {
        self.raw[1..].to_vec()
    }

    /// One map per non-header row, keyed by the header row.
    pub fn hashes(&self) -> Vec<HashMap<String, String>> {
        let header = &self.raw[0];
        self.raw[1..]
            .iter()
            .map(|r| header.iter().cloned().zip(r.iter().cloned()).collect())
            .collect()
    }

    /// A two-column table as a key → value map. Panics (fails the scenario)
    /// on any other shape — a silently mis-shaped lookup is a false green.
    pub fn rows_hash(&self) -> HashMap<String, String> {
        assert!(
            self.raw.iter().all(|r| r.len() == 2),
            "rows_hash() requires a table with exactly two columns"
        );
        self.raw
            .iter()
            .map(|r| (r[0].clone(), r[1].clone()))
            .collect()
    }

    /// Columns become rows.
    pub fn transpose(&self) -> DataTable {
        let cols = self.raw[0].len();
        DataTable::new(
            (0..cols)
                .map(|i| self.raw.iter().map(|r| r[i].clone()).collect())
                .collect(),
        )
    }
}

// --- Parser -----------------------------------------------------------------

fn placeholder_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"<([^>]+)>").expect("static regex"))
}

/// `Given I add 5` → `("Given", "I add 5")`; None for narrative/other lines.
/// `line` must already be trimmed.
fn step_keyword(line: &str) -> Option<(&str, &str)> {
    for kw in ["Given", "When", "Then", "And", "But", "*"] {
        if let Some(rest) = line.strip_prefix(kw) {
            if rest.starts_with(char::is_whitespace) {
                let text = rest.trim_start();
                if !text.is_empty() {
                    return Some((kw, text));
                }
            }
        }
    }
    None
}

struct OutlineAcc {
    name: String,
    steps: Vec<Step>,
    header: Option<Vec<String>>,
    rows: Vec<Vec<String>>,
    examples_seen: bool,
    line: usize,
    tags: Vec<String>,
}

/// Where step lines currently land (the JS version's `cur` pointer).
enum Cur {
    None,
    Background,
    LastScenario,
    Outline,
}

/// Substitute `<placeholder>`s from an Examples row map; unknown placeholder
/// is a loud error (almost always a typo — it would leak `<name>` into a step).
fn subst(
    s: &str,
    map: &HashMap<&str, &str>,
    filename: &str,
    line: usize,
) -> Result<String, GherkinSyntaxError> {
    let mut out = String::new();
    let mut last = 0;
    for c in placeholder_re().captures_iter(s) {
        let m = c.get(0).expect("group 0");
        let key = c.get(1).expect("group 1").as_str();
        match map.get(key) {
            Some(v) => {
                out.push_str(&s[last..m.start()]);
                out.push_str(v);
                last = m.end();
            }
            None => {
                return Err(GherkinSyntaxError::new(
                    filename,
                    line,
                    &format!("unknown placeholder <{key}> (no matching Examples column)"),
                ))
            }
        }
    }
    out.push_str(&s[last..]);
    Ok(out)
}

fn flush_outline(
    outline: &mut Option<OutlineAcc>,
    scenarios: &mut Vec<Scenario>,
    outlines: &mut Vec<OutlineMeta>,
    filename: &str,
) -> Result<(), GherkinSyntaxError> {
    let Some(o) = outline.take() else {
        return Ok(());
    };
    if o.steps.is_empty() {
        return Err(GherkinSyntaxError::new(
            filename,
            o.line,
            &format!("Scenario Outline \"{}\" has no steps", o.name),
        ));
    }
    if !o.examples_seen {
        return Err(GherkinSyntaxError::new(
            filename,
            o.line,
            "Scenario Outline has no Examples: block",
        ));
    }
    let Some(header) = &o.header else {
        return Err(GherkinSyntaxError::new(
            filename,
            o.line,
            "Scenario Outline Examples: has no header row",
        ));
    };
    if o.rows.is_empty() {
        return Err(GherkinSyntaxError::new(
            filename,
            o.line,
            "Scenario Outline Examples: has a header but no data rows",
        ));
    }
    outlines.push(OutlineMeta {
        name: o.name.clone(),
        line: o.line,
        rows: o.rows.len(),
    });
    for (i, row) in o.rows.iter().enumerate() {
        let map: HashMap<&str, &str> = header
            .iter()
            .map(String::as_str)
            .zip(row.iter().map(String::as_str))
            .collect();
        let mut steps = Vec::with_capacity(o.steps.len());
        for st in &o.steps {
            let table = match &st.table {
                None => None,
                Some(t) => {
                    let mut rows = Vec::with_capacity(t.len());
                    for r in t {
                        let mut cells = Vec::with_capacity(r.len());
                        for cell in r {
                            cells.push(subst(cell, &map, filename, o.line)?);
                        }
                        rows.push(cells);
                    }
                    Some(rows)
                }
            };
            steps.push(Step {
                keyword: st.keyword.clone(),
                text: subst(&st.text, &map, filename, o.line)?,
                table,
                line: st.line,
            });
        }
        scenarios.push(Scenario {
            name: format!("{} [{}]", subst(&o.name, &map, filename, o.line)?, i + 1),
            steps,
            line: o.line,
            tags: o.tags.clone(),
        });
    }
    Ok(())
}

/// Split one `| a | b |` row into trimmed cells. Honors Gherkin cell escapes
/// (\| → |, \\ → \, \n → newline); a backslash before any other character is
/// literal. A row that does not end with a closing | is a loud error — the
/// naive split would silently drop the trailing cell.
fn split_row(
    line: &str,
    filename: &str,
    line_no: usize,
) -> Result<Vec<String>, GherkinSyntaxError> {
    let mut cells = Vec::new();
    let mut buf = String::new();
    let mut chars = line.chars().skip(1).peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.peek() {
                Some(&n @ ('|' | '\\')) => {
                    buf.push(n);
                    chars.next();
                    continue;
                }
                Some(&'n') => {
                    buf.push('\n');
                    chars.next();
                    continue;
                }
                _ => {}
            }
        }
        if c == '|' {
            cells.push(buf.trim().to_string());
            buf.clear();
            continue;
        }
        buf.push(c);
    }
    if !buf.trim().is_empty() {
        return Err(GherkinSyntaxError::new(
            filename,
            line_no,
            "table row must end with a closing |",
        ));
    }
    if cells.is_empty() {
        return Err(GherkinSyntaxError::new(
            filename,
            line_no,
            "empty table row",
        ));
    }
    Ok(cells)
}

/// Parse raw .feature file contents. `filename` is used only to prefix errors.
pub fn parse_feature(text: &str, filename: &str) -> Result<ParsedFeature, GherkinSyntaxError> {
    let mut feature = String::new();
    let mut feature_seen = false;
    let mut background_seen = false;
    let mut background: Vec<Step> = Vec::new();
    let mut scenarios: Vec<Scenario> = Vec::new();
    let mut outlines: Vec<OutlineMeta> = Vec::new();
    let mut cur = Cur::None;
    let mut outline: Option<OutlineAcc> = None;
    let mut in_examples = false;
    let mut feature_tags: Vec<String> = Vec::new();
    let mut pending_tags: Vec<String> = Vec::new();

    let fail = |line: usize, msg: &str| -> GherkinSyntaxError {
        GherkinSyntaxError::new(filename, line, msg)
    };
    // Reject pending tags on a line that must not carry them.
    let no_tags = |pending: &[String], line_no: usize| -> Result<(), GherkinSyntaxError> {
        if pending.is_empty() {
            Ok(())
        } else {
            Err(GherkinSyntaxError::new(
                filename,
                line_no,
                &format!(
                    "tags ({}) must immediately precede Feature:, Scenario:, or Scenario Outline:",
                    pending.join(" ")
                ),
            ))
        }
    };
    // Reject a combination of semantic tags. This runner resolves @skip before
    // @todo and rejects @only; the JS sibling's runtimes each do something
    // else again — a combination cannot mean the same thing everywhere, so it
    // must not mean anything silently. (Mirrors gherkin-node-test.)
    let no_tag_conflict = |tags: &[String], line_no: usize| -> Result<(), GherkinSyntaxError> {
        let mut semantic: Vec<&str> = Vec::new();
        for t in tags {
            if matches!(t.as_str(), "@skip" | "@todo" | "@only") && !semantic.contains(&t.as_str())
            {
                semantic.push(t);
            }
        }
        if semantic.len() > 1 {
            return Err(GherkinSyntaxError::new(
                filename,
                line_no,
                &format!(
                    "conflicting tags ({}) — @skip/@todo/@only are mutually exclusive; keep exactly one",
                    semantic.join(" ")
                ),
            ));
        }
        Ok(())
    };

    let mut line_no = 0;
    for raw in text.split('\n') {
        line_no += 1;
        let line = raw.trim_end_matches('\r').trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('@') {
            for t in line.split_whitespace() {
                // A near-miss of a semantic tag (@Skip, @SKIP, @Todo…) would be
                // silently inert. Reject it loudly.
                let lower = t.to_lowercase();
                if ["@skip", "@todo", "@only"].contains(&lower.as_str()) && t != lower {
                    return Err(fail(
                        line_no,
                        &format!("tag {t} looks like {lower} but isn't exact — a near-miss tag is silently inert; use lowercase"),
                    ));
                }
                pending_tags.push(t.to_string());
            }
            continue;
        }

        // Reject constructs that would otherwise be silently mis-parsed.
        if line.starts_with("\"\"\"") || line.starts_with("```") {
            return Err(fail(
                line_no,
                "doc strings (\"\"\" / ```) are not supported",
            ));
        }
        if line.starts_with("Rule:") {
            return Err(fail(line_no, "the Rule: keyword is not supported"));
        }

        if let Some(rest) = line.strip_prefix("Feature:") {
            if feature_seen {
                return Err(fail(line_no, "multiple Feature: blocks in one file"));
            }
            flush_outline(&mut outline, &mut scenarios, &mut outlines, filename)?;
            feature = rest.trim().to_string();
            feature_seen = true;
            feature_tags = std::mem::take(&mut pending_tags);
            no_tag_conflict(&feature_tags, line_no)?;
            cur = Cur::None;
            in_examples = false;
            continue;
        }
        if line.starts_with("Background:") {
            no_tags(&pending_tags, line_no)?;
            if background_seen {
                return Err(fail(line_no, "multiple Background: blocks"));
            }
            // Expand any pending outline first, so the check below sees it.
            flush_outline(&mut outline, &mut scenarios, &mut outlines, filename)?;
            if !scenarios.is_empty() {
                return Err(fail(line_no, "Background: must appear before any Scenario"));
            }
            cur = Cur::Background;
            background_seen = true;
            in_examples = false;
            continue;
        }
        if let Some(rest) = line.strip_prefix("Scenario Outline:") {
            flush_outline(&mut outline, &mut scenarios, &mut outlines, filename)?;
            let mut tags = feature_tags.clone();
            tags.extend(std::mem::take(&mut pending_tags));
            no_tag_conflict(&tags, line_no)?;
            outline = Some(OutlineAcc {
                name: rest.trim().to_string(),
                steps: Vec::new(),
                header: None,
                rows: Vec::new(),
                examples_seen: false,
                line: line_no,
                tags,
            });
            cur = Cur::Outline;
            in_examples = false;
            continue;
        }
        if let Some(rest) = line.strip_prefix("Scenario:") {
            flush_outline(&mut outline, &mut scenarios, &mut outlines, filename)?;
            let mut tags = feature_tags.clone();
            tags.extend(std::mem::take(&mut pending_tags));
            no_tag_conflict(&tags, line_no)?;
            scenarios.push(Scenario {
                name: rest.trim().to_string(),
                steps: Vec::new(),
                line: line_no,
                tags,
            });
            cur = Cur::LastScenario;
            in_examples = false;
            continue;
        }
        if line.starts_with("Examples:") {
            no_tags(&pending_tags, line_no)?;
            let Some(o) = outline.as_mut() else {
                return Err(fail(line_no, "Examples: outside a Scenario Outline"));
            };
            if o.examples_seen {
                return Err(fail(
                    line_no,
                    "multiple Examples: blocks per Scenario Outline are not supported",
                ));
            }
            o.examples_seen = true;
            in_examples = true;
            continue;
        }
        if let Some((keyword, step_text)) = step_keyword(line) {
            no_tags(&pending_tags, line_no)?;
            if in_examples {
                return Err(fail(
                    line_no,
                    "step after an Examples: table (steps must precede Examples)",
                ));
            }
            let step = Step {
                keyword: keyword.to_string(),
                text: step_text.to_string(),
                table: None,
                line: line_no,
            };
            match cur {
                Cur::None => return Err(fail(line_no, "step before any Scenario or Background")),
                Cur::Background => background.push(step),
                Cur::LastScenario => scenarios
                    .last_mut()
                    .expect("scenario exists")
                    .steps
                    .push(step),
                Cur::Outline => outline.as_mut().expect("outline exists").steps.push(step),
            }
            continue;
        }
        if line.starts_with('|') {
            no_tags(&pending_tags, line_no)?;
            let cells = split_row(line, filename, line_no)?;
            if in_examples {
                let o = outline.as_mut().expect("in_examples implies outline");
                match &o.header {
                    None => o.header = Some(cells),
                    Some(h) if cells.len() != h.len() => {
                        return Err(fail(
                            line_no,
                            &format!(
                                "Examples row has {} cell(s); header has {}",
                                cells.len(),
                                h.len()
                            ),
                        ))
                    }
                    Some(_) => o.rows.push(cells),
                }
                continue;
            }
            let steps = match cur {
                Cur::None => {
                    return Err(fail(line_no, "table row before any Scenario or Background"))
                }
                Cur::Background => &mut background,
                Cur::LastScenario => &mut scenarios.last_mut().expect("scenario exists").steps,
                Cur::Outline => &mut outline.as_mut().expect("outline exists").steps,
            };
            // A table row after a step is that step's data table.
            let Some(last) = steps.last_mut() else {
                return Err(fail(line_no, "table row without a preceding step"));
            };
            match &mut last.table {
                None => last.table = Some(vec![cells]),
                Some(t) if cells.len() != t[0].len() => {
                    return Err(fail(
                        line_no,
                        &format!(
                            "table row has {} cell(s); this step's table has {}",
                            cells.len(),
                            t[0].len()
                        ),
                    ))
                }
                Some(t) => t.push(cells),
            }
            continue;
        }
        // Anything else (Feature narrative: "As a…/I want…/So that…") is ignored.
    }
    flush_outline(&mut outline, &mut scenarios, &mut outlines, filename)?;
    if !pending_tags.is_empty() {
        return Err(fail(
            line_no,
            &format!("dangling tags ({}) at end of file", pending_tags.join(" ")),
        ));
    }
    if !feature_seen {
        return Err(fail(line_no, "no Feature: line found"));
    }
    // A scenario with no steps would run zero assertions and pass vacuously. This
    // also catches step lines silently dropped as narrative (e.g. a misspelled or
    // non-English keyword) when they were a scenario's only steps.
    for sc in &scenarios {
        if sc.steps.is_empty() {
            return Err(fail(
                sc.line,
                &format!("Scenario \"{}\" has no steps", sc.name),
            ));
        }
    }
    Ok(ParsedFeature {
        feature,
        background,
        scenarios,
        outlines,
    })
}

// --- Linter -------------------------------------------------------------------

/// A lint finding's rule. The string forms (`as_str`) match the node sibling
/// exactly, so differential parity can compare finding streams byte-for-byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LintRule {
    Dialect,
    NoThen,
    VagueThen,
    SingleRowOutline,
}

impl LintRule {
    pub fn as_str(self) -> &'static str {
        match self {
            LintRule::Dialect => "dialect",
            LintRule::NoThen => "no-then",
            LintRule::VagueThen => "vague-then",
            LintRule::SingleRowOutline => "single-row-outline",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LintSeverity {
    Error,
    Warn,
}

impl LintSeverity {
    pub fn as_str(self) -> &'static str {
        match self {
            LintSeverity::Error => "error",
            LintSeverity::Warn => "warn",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LintFinding {
    pub rule: LintRule,
    pub severity: LintSeverity,
    pub line: usize,
    pub message: String,
}

/// Words that make a Then assert nothing checkable. Deliberately short —
/// corpus evidence (see the node sibling's 0.4.0 commit) shows morphological
/// broadening false-positives real specs while catching zero real vagueness.
/// `(?-u:…)` pins the whole match to ASCII semantics — word boundaries AND
/// case folding — matching JS's non-unicode `/i` exactly. With Unicode
/// folding on, `(?i)` would match the Kelvin sign `K` (U+212A) as `k`, which
/// JS's `/i` refuses to fold; the adversarial review caught this divergence
/// live (a `worKs` Then flagged here, silent in node).
fn vague_then_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)(?-u:\b(works|correctly|properly|as expected|handles|appropriate)\b)")
            .expect("static regex")
    })
}

fn is_primary(kw: &str) -> bool {
    matches!(kw, "Given" | "When" | "Then")
}

/// Lint one feature file's text: the dialect gate plus deterministic spec
/// lints. Pure text-in/findings-out — no filesystem, no environment, no test
/// registration — for holding `.feature` files to this dialect and quality
/// floor in a repo whose executor is something else (cucumber-rs, or the
/// node sibling's runners). The node sibling ships the same function with
/// IDENTICAL finding text, verified differentially by tools/parity.
///
/// Rules:
///  - `dialect` (error): the text is outside the supported subset — the exact
///    GherkinSyntaxError as a finding. The parser stops at the first
///    violation, so a dialect finding is always alone.
///  - `no-then` (warn): a scenario whose own steps never resolve to Then — it
///    runs code but asserts nothing. And/But/* inherit the preceding primary
///    keyword, resolved across the Background boundary.
///  - `vague-then` (warn): a Then-resolved step containing a word from the
///    banned-vagueness list above.
///  - `single-row-outline` (warn): a Scenario Outline with one Examples row —
///    a scenario with extra ceremony, and usually a missing case.
///
/// Findings from a Scenario Outline land once per source construct — except a
/// vagueness introduced BY a placeholder substitution, which lands for exactly
/// the rows that produce it. Severity is descriptive, not policy: the
/// wip-style debt register (filtering by rule) belongs to the consumer.
pub fn lint_feature(text: &str, filename: &str) -> Vec<LintFinding> {
    type Seen = HashSet<(LintRule, usize, String)>;
    // Identical (rule, line, message) triples collapse: expanded outline rows
    // share their source lines, so a row-independent finding lands once while
    // a substitution-dependent one (different message text) lands per row.
    fn warn(
        findings: &mut Vec<LintFinding>,
        seen: &mut Seen,
        rule: LintRule,
        line: usize,
        message: String,
    ) {
        if seen.insert((rule, line, message.clone())) {
            findings.push(LintFinding {
                rule,
                severity: LintSeverity::Warn,
                line,
                message,
            });
        }
    }
    fn check_vague(findings: &mut Vec<LintFinding>, seen: &mut Seen, st: &Step) {
        if let Some(m) = vague_then_re().find(&st.text) {
            warn(
                findings,
                seen,
                LintRule::VagueThen,
                st.line,
                format!(
                    "vague Then \"{}\" — \"{}\" is not a checkable outcome; name the observable result",
                    st.text,
                    m.as_str()
                ),
            );
        }
    }

    let parsed = match parse_feature(text, filename) {
        Ok(p) => p,
        Err(e) => {
            // The structured finding already carries .line; strip the parser's
            // file:line prefix so consumers composing "file:line: message"
            // from the finding don't print it twice.
            let msg = e.to_string();
            let prefix = format!("{filename}:{}: ", e.line);
            let message = msg.strip_prefix(&prefix).unwrap_or(&msg).to_string();
            return vec![LintFinding {
                rule: LintRule::Dialect,
                severity: LintSeverity::Error,
                line: e.line,
                message,
            }];
        }
    };

    let mut findings: Vec<LintFinding> = Vec::new();
    let mut seen: Seen = HashSet::new();

    // Background steps are shared by every scenario: resolve and lint them once.
    let mut bg_last: Option<&str> = None;
    for st in &parsed.background {
        if is_primary(&st.keyword) {
            bg_last = Some(st.keyword.as_str());
        }
        if bg_last == Some("Then") {
            check_vague(&mut findings, &mut seen, st);
        }
    }

    let outline_by_line: HashMap<usize, &OutlineMeta> =
        parsed.outlines.iter().map(|o| (o.line, o)).collect();
    for sc in &parsed.scenarios {
        let outline = outline_by_line.get(&sc.line).copied();
        let mut last = bg_last;
        let mut has_then = false;
        for st in &sc.steps {
            if is_primary(&st.keyword) {
                last = Some(st.keyword.as_str());
            }
            if last == Some("Then") {
                has_then = true;
                check_vague(&mut findings, &mut seen, st);
            }
        }
        if !has_then {
            let label = match outline {
                Some(o) => format!("Scenario Outline \"{}\"", o.name),
                None => format!("Scenario \"{}\"", sc.name),
            };
            warn(
                &mut findings,
                &mut seen,
                LintRule::NoThen,
                sc.line,
                format!("{label} has no Then step — it runs code but asserts nothing"),
            );
        }
    }

    for o in &parsed.outlines {
        if o.rows == 1 {
            warn(
                &mut findings,
                &mut seen,
                LintRule::SingleRowOutline,
                o.line,
                format!(
                    "Scenario Outline \"{}\" has one Examples row — a scenario with extra ceremony, and usually a missing case",
                    o.name
                ),
            );
        }
    }

    // Stable sort: line ties keep push order, matching the node sibling.
    findings.sort_by_key(|f| f.line);
    findings
}

// --- World context ------------------------------------------------------------

/// Per-scenario execution context: the feature's typed World plus `defer`.
///
/// `ctx.defer(f)` registers scenario-scoped cleanup: deferred closures run in
/// reverse (LIFO) order after the steps, INCLUDING when a step panicked — so a
/// failing assertion can't leak temp dirs/processes. The step failure, if any,
/// outranks cleanup errors; with no step failure the first cleanup error fails
/// the scenario.
type DeferFn<W> = Box<dyn FnOnce(&mut W)>;

pub struct Ctx<W> {
    pub world: W,
    deferred: Vec<DeferFn<W>>,
}

impl<W> Ctx<W> {
    pub fn new(world: W) -> Self {
        Self {
            world,
            deferred: Vec::new(),
        }
    }

    pub fn defer(&mut self, f: impl FnOnce(&mut W) + 'static) {
        self.deferred.push(Box::new(f));
    }
}

// --- Step registry ----------------------------------------------------------

type StepFn<W> = Box<dyn Fn(&mut Ctx<W>, &[String], Option<&DataTable>) + Send + Sync>;

/// One feature's step definitions, matched against a typed World `W`.
/// There is no global registry: each feature gets its own, so one feature's
/// patterns can never match another feature's steps.
pub struct StepRegistry<W> {
    defs: Vec<(Regex, StepFn<W>)>,
}

impl<W> Default for StepRegistry<W> {
    fn default() -> Self {
        Self::new()
    }
}

impl<W> StepRegistry<W> {
    pub fn new() -> Self {
        Self { defs: Vec::new() }
    }

    /// Register a step by regex source. Capture groups become the step's
    /// `args` (as strings — parse them in the step). An invalid pattern is a
    /// configuration error and panics loudly.
    pub fn define(
        &mut self,
        pattern: &str,
        f: impl Fn(&mut Ctx<W>, &[String], Option<&DataTable>) + Send + Sync + 'static,
    ) -> &mut Self {
        let re =
            Regex::new(pattern).unwrap_or_else(|e| panic!("invalid step pattern {pattern:?}: {e}"));
        self.defs.push((re, Box::new(f)));
        self
    }

    /// Register a step matched as an exact literal (escaped and anchored).
    pub fn define_exact(
        &mut self,
        text: &str,
        f: impl Fn(&mut Ctx<W>, &[String], Option<&DataTable>) + Send + Sync + 'static,
    ) -> &mut Self {
        let pattern = format!("^{}$", regex::escape(text));
        self.define(&pattern, f)
    }

    /// First definition matching `text` → (definition index, capture args).
    pub fn find(&self, text: &str) -> Option<(usize, Vec<String>)> {
        for (i, (re, _)) in self.defs.iter().enumerate() {
            if let Some(c) = re.captures(text) {
                let args = (1..c.len())
                    .map(|g| c.get(g).map(|m| m.as_str().to_string()).unwrap_or_default())
                    .collect();
                return Some((i, args));
            }
        }
        None
    }

    /// Indices of ALL definitions matching `text` (the ambiguity guard).
    pub fn matching(&self, text: &str) -> Vec<usize> {
        self.defs
            .iter()
            .enumerate()
            .filter(|(_, (re, _))| re.is_match(text))
            .map(|(i, _)| i)
            .collect()
    }
}

// --- Snippets ----------------------------------------------------------------

/// Build a paste-ready step definition for an unbound step: numbers become
/// (\d+) / ([\d.]+) captures, "quoted strings" become "([^"]*)", everything
/// else is regex-escaped. The generated body PANICS — an empty body would turn
/// the pasted definition into an instant vacuous pass, the exact failure mode
/// this harness exists to prevent.
pub fn build_snippet(text: &str) -> String {
    static TOKEN: OnceLock<Regex> = OnceLock::new();
    let token = TOKEN.get_or_init(|| Regex::new(r#""[^"]*"|\d+(?:\.\d+)?"#).expect("static regex"));
    let mut src = String::new();
    let mut last = 0;
    for m in token.find_iter(text) {
        src.push_str(&regex::escape(&text[last..m.start()]));
        src.push_str(if m.as_str().starts_with('"') {
            "\"([^\"]*)\""
        } else if m.as_str().contains('.') {
            r"([\d.]+)"
        } else {
            r"(\d+)"
        });
        last = m.end();
    }
    src.push_str(&regex::escape(&text[last..]));
    format!(
        "reg.define(r#\"^{src}$\"#, |ctx, args, table| {{\n    panic!(\"pending: implement this step\");\n}});"
    )
}

// --- Execution --------------------------------------------------------------

fn panic_msg(e: Box<dyn Any + Send>) -> String {
    if let Some(s) = e.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = e.downcast_ref::<String>() {
        s.clone()
    } else {
        "non-string panic payload".to_string()
    }
}

/// Run a flat list of steps against a fresh `world`. Fails on an undefined
/// step or a panicking assertion (the failure message carries the step text
/// and its .feature line). Deferred cleanup (`ctx.defer`) runs LIFO even when
/// a step failed; the step failure outranks cleanup errors. Exposed so tests
/// and other harnesses can drive it without going through `cargo test`.
pub fn execute_steps<W>(steps: &[Step], reg: &StepRegistry<W>, world: W) -> Result<W, String> {
    let mut ctx = Ctx::new(world);
    let mut failure: Option<String> = None;
    for step in steps {
        match reg.find(&step.text) {
            None => {
                failure = Some(format!(
                    "Undefined step: {}\nDefine it with:\n{}",
                    step.text,
                    build_snippet(&step.text)
                ));
                break;
            }
            Some((idx, args)) => {
                let table = step.table.as_ref().map(|t| DataTable::new(t.clone()));
                let f = &reg.defs[idx].1;
                if let Err(e) =
                    catch_unwind(AssertUnwindSafe(|| f(&mut ctx, &args, table.as_ref())))
                {
                    failure = Some(format!(
                        "step `{} {}` (feature line {}) failed: {}",
                        step.keyword,
                        step.text,
                        step.line,
                        panic_msg(e)
                    ));
                    break;
                }
            }
        }
    }
    let mut deferred = std::mem::take(&mut ctx.deferred);
    while let Some(f) = deferred.pop() {
        if let Err(e) = catch_unwind(AssertUnwindSafe(|| f(&mut ctx.world))) {
            failure.get_or_insert_with(|| format!("cleanup failed: {}", panic_msg(e)));
        }
    }
    match failure {
        Some(f) => Err(f),
        None => Ok(ctx.world),
    }
}

// --- Guards (pure, so tests can drive them without a runner) -------------------

/// The per-feature binding guard: every step must match exactly one definition
/// — no ambiguity, and (unless `wip`) no unbound steps, because unbound
/// scenarios register as ignored, which `cargo test` reports as GREEN. The
/// failure message includes a paste-ready snippet per missing step. @skip'd
/// scenarios are ratcheted too: skip means "don't run", never "don't bind".
pub fn check_bindings<W>(
    parsed: &ParsedFeature,
    reg: &StepRegistry<W>,
    base: &str,
    wip: bool,
) -> Result<(), String> {
    let steps: Vec<&Step> = parsed
        .background
        .iter()
        .chain(parsed.scenarios.iter().flat_map(|s| s.steps.iter()))
        .collect();
    let ambiguous: Vec<String> = steps
        .iter()
        .filter(|s| reg.matching(&s.text).len() > 1)
        .map(|s| format!("\"{}\"", s.text))
        .collect();
    if !ambiguous.is_empty() {
        return Err(format!(
            "steps matching >1 definition: {}",
            ambiguous.join("; ")
        ));
    }
    if !wip {
        let mut seen = HashSet::new();
        let unresolved: Vec<&str> = steps
            .iter()
            .filter(|s| reg.find(&s.text).is_none())
            .map(|s| s.text.as_str())
            .filter(|t| seen.insert(*t))
            .collect();
        if !unresolved.is_empty() {
            return Err(format!(
                "unbound steps would register as ignored (reported green); bind them or mark '{base}' as .wip():\n\n{}",
                unresolved
                    .iter()
                    .map(|t| format!("// {t}\n{}", build_snippet(t)))
                    .collect::<Vec<_>>()
                    .join("\n\n")
            ));
        }
    }
    Ok(())
}

// --- High-level runner --------------------------------------------------------

fn feature_trials<W: Default + 'static>(
    path: &Path,
    reg: StepRegistry<W>,
    base: &str,
    wip: bool,
) -> Vec<Trial> {
    let file = path.display().to_string();
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            let msg = format!("cannot read {file}: {e}");
            return vec![Trial::test(format!("{base} :: parses"), move || {
                Err(Failed::from(msg))
            })];
        }
    };
    let parsed = match parse_feature(&text, &file) {
        Ok(p) => p,
        Err(e) => {
            let msg = e.to_string();
            return vec![Trial::test(format!("{base} :: parses"), move || {
                Err(Failed::from(msg))
            })];
        }
    };
    let reg = Arc::new(reg);
    let parsed = Arc::new(parsed);
    let mut trials = Vec::new();

    // Binding guard (ambiguity always; unbound-step ratchet unless wip).
    {
        let parsed = Arc::clone(&parsed);
        let reg = Arc::clone(&reg);
        let base = base.to_string();
        trials.push(Trial::test(
            format!(
                "{base} :: step definitions are {}",
                if wip {
                    "unambiguous"
                } else {
                    "complete and unambiguous"
                }
            ),
            move || check_bindings(&parsed, &reg, &base, wip).map_err(Failed::from),
        ));
    }

    // @only would silently deselect everything else in cucumber-land; here it
    // has no `cargo test` analog, so it is rejected loudly instead of ignored.
    if parsed
        .scenarios
        .iter()
        .any(|s| s.tags.iter().any(|t| t == "@only"))
    {
        let msg = format!(
            "{file}: @only is not supported; run one scenario with `cargo test -- '<name substring>'`"
        );
        trials.push(Trial::test(
            format!("{base} :: @only is not supported"),
            move || Err(Failed::from(msg)),
        ));
    }

    for sc in &parsed.scenarios {
        let title = format!("{} :: {}", parsed.feature, sc.name);
        let steps: Vec<Step> = parsed
            .background
            .iter()
            .chain(sc.steps.iter())
            .cloned()
            .collect();
        // Unbound scenario: registered but ignored (never silently green — the
        // binding guard above fails the suite unless this feature is wip). The
        // placeholder body FAILS with its reason: an Ok(()) body would pass
        // vacuously under `cargo test -- --include-ignored`, the exact
        // false green this crate exists to prevent.
        let missing: Vec<&str> = steps
            .iter()
            .filter(|s| reg.find(&s.text).is_none())
            .map(|s| s.text.as_str())
            .collect();
        if !missing.is_empty() {
            let reason = format!(
                "{} undefined step(s); first: \"{}\"",
                missing.len(),
                missing[0]
            );
            trials.push(
                Trial::test(title, move || Err(Failed::from(reason)))
                    .with_kind("unbound")
                    .with_ignored_flag(true),
            );
            continue;
        }
        if sc.tags.iter().any(|t| t == "@skip") {
            let reg = Arc::clone(&reg);
            trials.push(
                Trial::test(title, move || {
                    execute_steps(&steps, &reg, W::default())
                        .map(|_| ())
                        .map_err(Failed::from)
                })
                .with_kind("skip")
                .with_ignored_flag(true),
            );
            continue;
        }
        let todo = sc.tags.iter().any(|t| t == "@todo");
        let reg = Arc::clone(&reg);
        let file = file.clone();
        let trial_title = title.clone();
        let mut trial = Trial::test(title, move || {
            match execute_steps(&steps, &reg, W::default()) {
                Ok(_) => Ok(()),
                Err(e) if todo => {
                    eprintln!("(@todo, failure tolerated) {trial_title}: {e}");
                    Ok(())
                }
                Err(e) => Err(Failed::from(format!("{e}\n  in {file}"))),
            }
        });
        if todo {
            trial = trial.with_kind("todo");
        }
        trials.push(trial);
    }
    trials
}

type EntryBuilder = Box<dyn FnOnce(&Path, bool) -> Vec<Trial>>;

/// Discover and run every *.feature in a directory, each against its OWN
/// scoped registry and typed World. Guards registered alongside the scenarios:
///
///  - every `.feature(base, definer)` must name an existing feature file (a
///    renamed feature can't silently strand its steps);
///  - within each feature, every step must match exactly one definition — no
///    ambiguity, and (unless the feature is marked `.wip(base)`) no unbound
///    steps. The failure message includes a paste-ready snippet per missing
///    step.
///
/// ```no_run
/// // tests/features.rs   (Cargo.toml: [[test]] name = "features", harness = false)
/// use gherkin_cargo_test::{Features, StepRegistry};
///
/// #[derive(Default)]
/// struct Counter { count: i64 }
///
/// fn counter_steps(reg: &mut StepRegistry<Counter>) {
///     reg.define(r"^a counter at (\d+)$", |ctx, args, _| {
///         ctx.world.count = args[0].parse().unwrap();
///     });
/// }
///
/// fn main() {
///     Features::new("features")
///         .feature("counter", counter_steps)
///         .run()
/// }
/// ```
pub struct Features {
    dir: PathBuf,
    entries: Vec<(String, EntryBuilder)>,
    wip: HashSet<String>,
}

impl Features {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            entries: Vec::new(),
            wip: HashSet::new(),
        }
    }

    /// Bind a feature basename to its step definer. The definer runs against a
    /// fresh registry typed to this feature's own World.
    pub fn feature<W: Default + 'static>(
        mut self,
        base: &str,
        definer: impl FnOnce(&mut StepRegistry<W>) + 'static,
    ) -> Self {
        assert!(
            !self.entries.iter().any(|(b, _)| b == base),
            "duplicate definer for feature '{base}'"
        );
        let base_owned = base.to_string();
        self.entries.push((
            base.to_string(),
            Box::new(move |path, wip| {
                let mut reg = StepRegistry::new();
                definer(&mut reg);
                feature_trials(path, reg, &base_owned, wip)
            }),
        ));
        self
    }

    /// Mark a feature as work-in-progress: unbound steps are allowed (its
    /// scenarios register as ignored instead of failing the binding guard).
    pub fn wip(mut self, base: &str) -> Self {
        self.wip.insert(base.to_string());
        self
    }

    /// Build every trial (guards + scenarios) without running them. Public so
    /// the guard behavior itself is testable; normal callers use `run()`.
    pub fn build_trials(self) -> Vec<Trial> {
        let mut trials = Vec::new();
        let mut files: Vec<PathBuf> = match fs::read_dir(&self.dir) {
            Ok(rd) => rd
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|x| x == "feature"))
                .collect(),
            Err(e) => {
                let msg = format!("cannot read feature dir {}: {e}", self.dir.display());
                return vec![Trial::test("feature directory is readable", move || {
                    Err(Failed::from(msg))
                })];
            }
        };
        files.sort();
        let bases: Vec<String> = files
            .iter()
            .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
            .collect();

        // Orphan guard: a definer whose feature file no longer exists.
        let orphaned: Vec<String> = self
            .entries
            .iter()
            .map(|(b, _)| b.clone())
            .filter(|b| !bases.contains(b))
            .collect();
        let dir = self.dir.display().to_string();
        trials.push(Trial::test(
            "step definers map only to existing feature files",
            move || {
                if orphaned.is_empty() {
                    Ok(())
                } else {
                    Err(Failed::from(format!(
                        "definers with no matching .feature in {dir}: {}",
                        orphaned.join(", ")
                    )))
                }
            },
        ));

        let mut entry_map: HashMap<String, EntryBuilder> = self.entries.into_iter().collect();
        for (path, base) in files.iter().zip(bases.iter()) {
            let wip = self.wip.contains(base);
            match entry_map.remove(base) {
                Some(builder) => trials.extend(builder(path, wip)),
                // No definer: run against an empty registry — every scenario is
                // unbound (ignored) and the binding guard fails unless wip.
                None => trials.extend(feature_trials::<()>(path, StepRegistry::new(), base, wip)),
            }
        }
        trials
    }

    /// Run under `cargo test` (in a `harness = false` test target) and exit
    /// with the suite's status.
    pub fn run(self) -> ! {
        let args = Arguments::from_args();
        libtest_mimic::run(&args, self.build_trials()).exit()
    }
}
