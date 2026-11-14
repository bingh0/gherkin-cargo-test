//! One way to wait.
//!
//! Scenarios that drive real software wait: for a process to start, a file to
//! appear, a message to arrive. Projects that each wrote their own waiting code
//! got it wrong in the same ways, so this module is the one place it lives.
//!
//! # Two kinds of wait
//!
//! - **A wait for something to happen** ([`wait_for`], [`wait_for_some`]) ends
//!   at the first poll that sees the thing has happened. Polls are up to
//!   100 ms apart, so it can end up to 100 ms after the event, and a condition
//!   that is true only between two polls is missed: the predicate should
//!   observe something that stays true once it happens. Its limit is
//!   [`patience`]: the base the author wrote, times the project default
//!   ([`Features::patience_default`]), times the machine's time factor. It
//!   may scale up or down: a longer limit costs nothing on a green run.
//! - **A check that something does NOT happen** ([`absent_for`]) always runs
//!   its whole window, so the window is the claim. Its length is
//!   [`absence_window`]: the base times `max(1, machine scale)`. It grows on a
//!   slow machine and never shrinks below what the author wrote: a shorter
//!   window would weaken the claim without any change to the scenario text.
//!   The project default does not apply, because it would lengthen every green
//!   run. Better than any window: [`absent_until`] waits for a later event that
//!   can only come after the forbidden one would have, then checks.
//!
//! A predicate is never interrupted: one that takes longer than the limit
//! makes the wait overrun by that much, and the expiry message reports the
//! real elapsed time. No wait is unbounded: a limit too large to be a deadline
//! fails at once as a harness error.
//!
//! # Settings
//!
//! The contract is two environment variables, resolved once per process. An
//! empty or whitespace-only value counts as unset (`VAR= cmd` is how a shell
//! unsets a variable for one command), and a value is trimmed before use.
//!
//! - `ABDD_TIME_SCALE`: the machine's time factor, a multiplier. Unset means
//!   1.0, and 1.0 means no change. Anything that is not a number from 0.01 to
//!   1000 is a loud error naming the variable, the value and the range.
//! - `ABDD_MACHINE_ENV`: optional absolute path of a settings file (a relative
//!   path is a loud error: under `cargo test` the current directory is the
//!   package root, which varies with how the run was started). Only lines that
//!   begin with `ABDD_` are looked at, after trimming, an optional `export `
//!   and, on the first line, a UTF-8 byte order mark. Every other line is
//!   skipped unread, so the file may be a general env file: no message ever
//!   quotes a line that is not an `ABDD_` line. On an `ABDD_` line the value
//!   is either quoted (`'…'` or `"…"`, which may hold `#` and `=`, followed by
//!   nothing but blanks or a `#` comment) or unquoted (up to the first `#`,
//!   trimmed). Anything else on an `ABDD_` line, a name given twice, or an
//!   unreadable file is a loud error naming the file, the line and the name.
//!   An `ABDD_MACHINE_ENV` line in the file is ignored: the file cannot
//!   redirect itself. A variable set in the environment wins over the file.
//!
//! Nothing is read from a home directory or any other place on the module's
//! own initiative: a settings file is read only when `ABDD_MACHINE_ENV` names
//! it. `ABDD_HEAVY_JOBS` and other names are neither read nor enforced.
//!
//! `Features::run()` resolves the settings for every project, whether or not
//! it uses this module. It prints one line to stderr naming the factors and
//! where each came from when a project default is set, `ABDD_TIME_SCALE` has a
//! value, or `ABDD_MACHINE_ENV` names a file (even one that sets nothing). A
//! settings error stops any project's run there, before any trial, with exit
//! status 2.
//!
//! # The cause line
//!
//! When a step fails because [`wait_for`] or [`wait_for_some`] ran out of
//! patience, because [`absent_until`]'s sentinel never came, or because the
//! step called [`harness_error`], the step's failure gets a line
//! `  cause: patience` or `  cause: harness` after its `  role:` line. It is
//! information for whoever reads a red, and changes no verdict.
//!
//! The mark is per thread and is matched to the failure's message text, so a
//! step that catches such a panic and then fails with a different message
//! gets no cause line, while one that panics again with the identical text
//! keeps the cause. **Call these functions on the step's own thread.** A wait
//! that expires on a helper thread the step spawned and joined loses the mark:
//! the step's failure then reads as an ordinary one.
//!
//! [`Features::patience_default`]: crate::Features::patience_default

use std::any::Any;
use std::cell::RefCell;
use std::ffi::OsString;
use std::fmt::{self, Display};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, Instant};

const TIME_SCALE: &str = "ABDD_TIME_SCALE";
const MACHINE_ENV: &str = "ABDD_MACHINE_ENV";

/// The sane range of a time factor (the machine scale and the project
/// default), both ends allowed. Below it every wait collapses to one check;
/// above it a wait can outlast any run.
pub(crate) const FACTOR_MIN: f64 = 0.01;
pub(crate) const FACTOR_MAX: f64 = 1000.0;

/// A factor as the header and messages print it: at most three decimal
/// places, trailing zeros trimmed (`0.30000000000000004` prints `0.3`, `3.0`
/// prints `3`).
pub(crate) fn factor(v: f64) -> String {
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    s.to_string()
}

// --- Settings ----------------------------------------------------------------

/// Where a resolved setting came from.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Source {
    Unset,
    Environment,
    File { path: String, line: usize },
}

impl Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Source::Unset => f.write_str("unset"),
            Source::Environment => f.write_str("environment"),
            Source::File { path, line } => write!(f, "{path} line {line}"),
        }
    }
}

/// The resolved settings of one process.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Settings {
    pub(crate) scale: f64,
    pub(crate) scale_source: Source,
    /// The settings file `ABDD_MACHINE_ENV` named, if any, for display.
    pub(crate) machine_env: Option<String>,
}

impl Settings {
    /// `ABDD_TIME_SCALE unset` / `ABDD_TIME_SCALE=2 (environment)` /
    /// `ABDD_TIME_SCALE=2 (/path line 3)`.
    fn describe_scale(&self) -> String {
        match self.scale_source {
            Source::Unset => format!("{TIME_SCALE} unset"),
            ref s => format!("{TIME_SCALE}={} ({s})", factor(self.scale)),
        }
    }
}

/// Parse a time factor: a number from [`FACTOR_MIN`] to [`FACTOR_MAX`].
/// `what` names the variable and its source for the error.
pub(crate) fn parse_scale(value: &str, what: &str) -> Result<f64, String> {
    match value.parse::<f64>() {
        Ok(v) if (FACTOR_MIN..=FACTOR_MAX).contains(&v) => Ok(v),
        _ => Err(format!(
            "{what} is {value:?}, which is not a time factor: it must be a number from \
             {FACTOR_MIN} to {FACTOR_MAX} (1.0 means no change)"
        )),
    }
}

/// One `ABDD_NAME=value` line of a settings file, kept with its line number.
#[derive(Debug, PartialEq)]
pub(crate) struct FileEntry {
    pub(crate) name: String,
    pub(crate) value: String,
    pub(crate) line: usize,
}

/// The text of a line after trimming, an optional `export ` and (on the
/// first line) a byte order mark: what decides whether the line counts.
fn line_body(raw: &str, first: bool) -> &str {
    let raw = if first {
        raw.strip_prefix('\u{feff}').unwrap_or(raw)
    } else {
        raw
    };
    let t = raw.trim();
    match t.strip_prefix("export") {
        Some(rest) if rest.starts_with([' ', '\t']) => rest.trim_start(),
        _ => t,
    }
}

/// Parse a settings file's text. Only lines whose body (see [`line_body`])
/// begins with `ABDD_` are looked at; every other line is skipped without
/// being parsed or quoted, because the file may be a general env file holding
/// secrets. On an `ABDD_` line, anything but `NAME=value` with a value that
/// is quoted (then only blanks or a `#` comment may follow) or unquoted (up
/// to the first `#`, trimmed) is an error naming `path`, the line and the
/// name, quoting that line. A name given twice is an error too: one of the
/// two would otherwise be silently ignored. An `ABDD_MACHINE_ENV` line is
/// skipped: the file cannot redirect itself.
pub(crate) fn parse_file(path: &str, text: &str) -> Result<Vec<FileEntry>, String> {
    let mut out: Vec<FileEntry> = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let line = i + 1;
        let body = line_body(raw, i == 0);
        if !body.starts_with("ABDD_") {
            continue;
        }
        let name_len = body
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(body.len());
        let name = &body[..name_len];
        let bad = |why: &str| format!("{path} line {line}: {name}: {why}: {body:?}");
        let Some(after_eq) = body[name_len..].trim_start().strip_prefix('=') else {
            return Err(bad("not a NAME=value line"));
        };
        if name == MACHINE_ENV {
            continue;
        }
        let rest = after_eq.trim_start();
        let value = match rest.chars().next() {
            Some(q @ ('"' | '\'')) => {
                let inner = &rest[1..];
                let Some(end) = inner.find(q) else {
                    return Err(bad("the value's quote is never closed"));
                };
                let tail = inner[end + 1..].trim_start();
                if !(tail.is_empty() || tail.starts_with('#')) {
                    return Err(bad(
                        "text after the closing quote (only a # comment may follow)",
                    ));
                }
                &inner[..end]
            }
            _ => {
                let v = rest.split('#').next().unwrap_or("").trim();
                if v.contains(['"', '\'']) {
                    return Err(bad(
                        "a quote inside an unquoted value (quote the whole value)",
                    ));
                }
                v
            }
        };
        if let Some(prev) = out.iter().find(|e| e.name == name) {
            return Err(format!(
                "{path} line {line}: {name} is set again (first on line {}); keep one",
                prev.line
            ));
        }
        out.push(FileEntry {
            name: name.to_string(),
            value: value.to_string(),
            line,
        });
    }
    Ok(out)
}

/// An environment value with blanks trimmed; None when unset, empty or only
/// blanks (`VAR= cmd` is how a shell unsets a variable for one command).
fn env_value(v: Option<OsString>) -> Option<OsString> {
    let v = v?;
    match v.to_str() {
        Some(s) if s.trim().is_empty() => None,
        Some(s) => Some(OsString::from(s.trim())),
        None => Some(v),
    }
}

/// Resolve the settings from an environment lookup and a file reader. Pure:
/// the real process environment and file system are passed in by
/// [`settings`], and unit tests pass their own. `read_file` is called only
/// when `ABDD_MACHINE_ENV` names a file by an absolute path, and gets that
/// path exactly as the environment holds it.
pub(crate) fn resolve(
    env: &dyn Fn(&str) -> Option<OsString>,
    read_file: &dyn Fn(&Path) -> Result<String, String>,
) -> Result<Settings, String> {
    let machine_env = env_value(env(MACHINE_ENV)).map(PathBuf::from);
    let shown = machine_env
        .as_ref()
        .map(|p| p.to_string_lossy().into_owned());
    let entries = match (&machine_env, &shown) {
        (Some(p), Some(shown)) => {
            if !p.is_absolute() {
                return Err(format!(
                    "{MACHINE_ENV} is {shown:?}, a relative path; name the settings file by an \
                     absolute path (under cargo test the current directory is the package root, \
                     which varies with how the run was started)"
                ));
            }
            let text = read_file(p)
                .map_err(|e| format!("{MACHINE_ENV} names {shown}, which cannot be read: {e}"))?;
            parse_file(shown, &text)?
        }
        _ => Vec::new(),
    };
    let env_scale = env_value(env(TIME_SCALE)).map(|v| v.to_string_lossy().into_owned());
    let (scale, scale_source) = if let Some(v) = env_scale {
        (
            parse_scale(&v, &format!("{TIME_SCALE} (environment)"))?,
            Source::Environment,
        )
    } else if let Some(e) = entries.iter().find(|e| e.name == TIME_SCALE) {
        let path = shown.clone().unwrap_or_default();
        let source = Source::File { path, line: e.line };
        (
            parse_scale(e.value.trim(), &format!("{TIME_SCALE} ({source})"))?,
            source,
        )
    } else {
        (1.0, Source::Unset)
    };
    Ok(Settings {
        scale,
        scale_source,
        machine_env: shown,
    })
}

/// This process's settings, resolved once from the real environment.
pub(crate) fn settings() -> &'static Result<Settings, String> {
    static SETTINGS: OnceLock<Result<Settings, String>> = OnceLock::new();
    SETTINGS.get_or_init(|| {
        resolve(&|name| std::env::var_os(name), &|path| {
            std::fs::read_to_string(path).map_err(|e| e.to_string())
        })
    })
}

// --- The project default -----------------------------------------------------

static PROJECT_DEFAULT: OnceLock<f64> = OnceLock::new();

/// Set the project default once per process (see
/// `Features::patience_default`).
pub(crate) fn set_project_default(factor: f64) {
    if let Err(e) = check_project_default(factor) {
        panic!("{e}");
    }
    let set = *PROJECT_DEFAULT.get_or_init(|| factor);
    assert!(
        set == factor,
        "patience_default({factor}): the project default is already {set} in this process; \
         set it once"
    );
}

/// The range check of a project default, apart from setting it.
pub(crate) fn check_project_default(factor: f64) -> Result<(), String> {
    if (FACTOR_MIN..=FACTOR_MAX).contains(&factor) {
        Ok(())
    } else {
        Err(format!(
            "patience_default({factor}): the project default must be a number from {FACTOR_MIN} \
             to {FACTOR_MAX} (1.0 means no change)"
        ))
    }
}

fn project_default() -> f64 {
    PROJECT_DEFAULT.get().copied().unwrap_or(1.0)
}

/// The run header for `Features::run()`: `Ok(None)` when there is nothing to
/// say, `Err` on a settings error.
pub(crate) fn run_header() -> Result<Option<String>, String> {
    let s = settings().as_ref().map_err(Clone::clone)?;
    let default_set = PROJECT_DEFAULT.get().is_some();
    if !default_set && s.scale_source == Source::Unset && s.machine_env.is_none() {
        return Ok(None);
    }
    Ok(Some(header_line(project_default(), s)))
}

pub(crate) fn header_line(default: f64, s: &Settings) -> String {
    let mut line = format!(
        "gherkin-cargo-test: patience x{} (default {} x machine {}); {}",
        factor(default * s.scale),
        factor(default),
        factor(s.scale),
        s.describe_scale()
    );
    if let Some(p) = &s.machine_env {
        line.push_str(&format!("; {MACHINE_ENV}={p} (environment)"));
    }
    line
}

// --- The cause mark ----------------------------------------------------------

/// Why a step failed, when the harness knows. Information only.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Cause {
    Patience,
    Harness,
}

impl Cause {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Cause::Patience => "patience",
            Cause::Harness => "harness",
        }
    }
}

thread_local! {
    /// The cause and the exact message of the last marked panic on this
    /// thread.
    static MARK: RefCell<Option<(Cause, String)>> = const { RefCell::new(None) };
}

/// Forget any mark on this thread. `execute_steps` calls it before each step.
pub(crate) fn clear_cause() {
    MARK.with(|m| *m.borrow_mut() = None);
}

/// Take this thread's mark, and return its cause only if `payload` is the
/// very failure that was marked (same message). A step that caught a marked
/// panic and then failed for another reason gets None.
pub(crate) fn take_cause(payload: &(dyn Any + Send)) -> Option<Cause> {
    let (cause, marked) = MARK.with(|m| m.borrow_mut().take())?;
    let msg = payload
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| payload.downcast_ref::<&str>().copied())?;
    (msg == marked).then_some(cause)
}

#[track_caller]
fn fail(cause: Cause, msg: String) -> ! {
    MARK.with(|m| *m.borrow_mut() = Some((cause, msg.clone())));
    std::panic::panic_any(msg)
}

// --- Arithmetic and polling --------------------------------------------------

fn scaled(base: Duration, factor: f64) -> Duration {
    Duration::try_from_secs_f64(base.as_secs_f64() * factor).unwrap_or(Duration::MAX)
}

pub(crate) fn patience_with(base: Duration, default: f64, scale: f64) -> Duration {
    scaled(base, default * scale)
}

pub(crate) fn absence_with(base: Duration, scale: f64) -> Duration {
    scaled(base, scale.max(1.0))
}

pub(crate) const FIRST_POLL: Duration = Duration::from_millis(5);
pub(crate) const MAX_POLL: Duration = Duration::from_millis(100);

/// The interval after `interval`: doubled, capped at [`MAX_POLL`].
pub(crate) fn next_poll(interval: Duration) -> Duration {
    (interval * 2).min(MAX_POLL)
}

/// How long to sleep now: the interval, but never past the deadline.
pub(crate) fn nap(interval: Duration, now: Instant, deadline: Instant) -> Duration {
    interval.min(deadline.saturating_duration_since(now))
}

/// Call `check` until it returns Some or `limit` has passed. Checks at least
/// once, and once more at the deadline. Ok carries the value and when it was
/// found; Err carries the elapsed time at giving up. A `limit` that cannot be
/// a deadline (the instant arithmetic overflows) fails at once, before any
/// check, as a harness error naming `what`: no wait is unbounded.
#[track_caller]
fn poll<T>(
    what: &str,
    limit: Duration,
    mut check: impl FnMut() -> Option<T>,
) -> Result<(T, Duration), Duration> {
    let start = Instant::now();
    let Some(deadline) = start.checked_add(limit) else {
        fail(
            Cause::Harness,
            format!(
                "gherkin-cargo-test: the limit {limit:?} for {what} is too large: no deadline can \
                 be set that far ahead; check the base, the project default and {TIME_SCALE}"
            ),
        )
    };
    let mut interval = FIRST_POLL;
    loop {
        if let Some(v) = check() {
            return Ok((v, start.elapsed()));
        }
        let now = Instant::now();
        if now >= deadline {
            return Err(now - start);
        }
        thread::sleep(nap(interval, now, deadline));
        interval = next_poll(interval);
    }
}

/// The factors a patience limit was computed from, for the message.
pub(crate) struct Factors {
    pub(crate) default: f64,
    pub(crate) scale: f64,
    pub(crate) source: String,
}

#[track_caller]
fn current_factors() -> Factors {
    match settings() {
        Ok(s) => Factors {
            default: project_default(),
            scale: s.scale,
            source: s.describe_scale(),
        },
        Err(e) => fail(Cause::Harness, format!("gherkin-cargo-test: {e}")),
    }
}

impl Factors {
    fn limit(&self, base: Duration) -> Duration {
        patience_with(base, self.default, self.scale)
    }

    fn explain(&self, base: Duration) -> String {
        format!(
            "limit {:?} = base {base:?} x project default {} x machine scale {} ({})",
            self.limit(base),
            factor(self.default),
            factor(self.scale),
            self.source
        )
    }
}

#[track_caller]
pub(crate) fn wait_core<T>(
    what: &str,
    base: Duration,
    f: &Factors,
    probe: impl FnMut() -> Option<T>,
) -> T {
    match poll(what, f.limit(base), probe) {
        Ok((v, _)) => v,
        Err(elapsed) => fail(
            Cause::Patience,
            format!(
                "wait_for: gave up waiting for {what} after {elapsed:?}; {}",
                f.explain(base)
            ),
        ),
    }
}

#[track_caller]
pub(crate) fn absent_for_core(
    what: &str,
    base: Duration,
    scale: f64,
    mut forbidden: impl FnMut() -> bool,
) {
    let window = absence_with(base, scale);
    if let Ok(((), at)) = poll(what, window, || forbidden().then_some(())) {
        panic!(
            "absent_for: {what} was seen {at:?} into its {window:?} window \
             (base {base:?} x max(1, machine scale {}))",
            factor(scale)
        );
    }
}

#[track_caller]
pub(crate) fn absent_until_core(
    what: &str,
    mut forbidden: impl FnMut() -> bool,
    sentinel_what: &str,
    base: Duration,
    f: &Factors,
    mut sentinel: impl FnMut() -> bool,
) {
    enum Seen {
        Before,
        AtSentinel,
        Clean,
    }
    let outcome = poll(sentinel_what, f.limit(base), || {
        if forbidden() {
            Some(Seen::Before)
        } else if sentinel() {
            Some(if forbidden() {
                Seen::AtSentinel
            } else {
                Seen::Clean
            })
        } else {
            None
        }
    });
    match outcome {
        Ok((Seen::Clean, _)) => {}
        Ok((Seen::Before, at)) => {
            panic!("absent_until: {what} was seen {at:?} into the wait for {sentinel_what}")
        }
        Ok((Seen::AtSentinel, at)) => panic!(
            "absent_until: {what} was seen when {sentinel_what} arrived, {at:?} into the wait"
        ),
        Err(elapsed) => fail(
            Cause::Patience,
            format!(
                "absent_until: gave up waiting for {sentinel_what} after {elapsed:?}, so nothing \
                 can be concluded about {what}; {}",
                f.explain(base)
            ),
        ),
    }
}

// --- Public API ----------------------------------------------------------------

/// The limit for a wait that ends as soon as something happens:
/// `base × project default × machine scale` (`ABDD_TIME_SCALE`, 1.0 when
/// unset). The project default is 1.0 unless `Features::patience_default`
/// set it. Panics (cause `harness`) on a settings error.
#[track_caller]
pub fn patience(base: Duration) -> Duration {
    current_factors().limit(base)
}

/// The length of a window in which something must NOT happen:
/// `base × max(1.0, machine scale)`. It grows on a slow machine and never
/// shrinks below `base`, because a shorter window weakens the claim without
/// any change to the scenario text. The project default does not apply: an
/// absence window always runs its full length, so the default would lengthen
/// every green run. Panics (cause `harness`) on a settings error.
#[track_caller]
pub fn absence_window(base: Duration) -> Duration {
    absence_with(base, current_factors().scale)
}

/// Poll `predicate` until it returns true or [`patience`]`(base)` has passed.
/// Returns at the first poll that sees it true. Checks at least once, and
/// once more at the deadline before giving up. The poll interval starts at
/// 5 ms and doubles to 100 ms, never sleeping past the deadline, so the
/// return can lag the event by up to 100 ms, and a condition that is true
/// only between two polls is missed: `predicate` should observe something
/// that stays true once it happens (a record, a file, a counter).
///
/// `predicate` is never interrupted: one that takes longer than the limit
/// makes the wait overrun by that much, and the expiry message reports the
/// real elapsed time. A limit too large to be a deadline fails at once
/// (cause `harness`) instead of waiting forever.
///
/// On expiry, panics with a message naming `what`, the limit and how it was
/// computed, and marks the cause `patience` (the step's failure gets a
/// `  cause: patience` line). Call it on the step's own thread: the mark is
/// per thread, and an expiry on a helper thread loses it.
#[track_caller]
pub fn wait_for(what: &str, base: Duration, mut predicate: impl FnMut() -> bool) {
    let f = current_factors();
    wait_core(what, base, &f, || predicate().then_some(()))
}

/// [`wait_for`], for a probe that returns the value it was waiting for.
/// Everything said there holds here: it returns at the first poll whose
/// `probe` returns Some, so `probe` should observe something that stays
/// available once it happens; a probe is never interrupted.
#[track_caller]
pub fn wait_for_some<T>(what: &str, base: Duration, probe: impl FnMut() -> Option<T>) -> T {
    let f = current_factors();
    wait_core(what, base, &f, probe)
}

/// Watch for the whole [`absence_window`]`(base)`, polling `forbidden`. If it
/// returns true at any poll, or at the final check when the window closes,
/// panics as an ordinary assertion failure naming `what` and how far into the
/// window it was seen. It never returns early: the window is the claim.
///
/// No cause mark: the forbidden thing happening is a real failure, not a
/// timeout. Prefer [`absent_until`] when a later event can end the wait.
/// `forbidden` must observe something that stays observable (a record, a
/// file, a counter): a poll can miss a transient.
#[track_caller]
pub fn absent_for(what: &str, base: Duration, forbidden: impl FnMut() -> bool) {
    let scale = current_factors().scale;
    absent_for_core(what, base, scale, forbidden)
}

/// The preferred absence check: wait for a later event, the sentinel, that
/// can only come after the forbidden one would have, then check that the
/// forbidden one was not seen.
///
/// While waiting it polls `forbidden`; seeing it is an ordinary assertion
/// failure. When `sentinel` returns true, it checks `forbidden` once more and
/// returns. If the sentinel does not arrive within [`patience`]`(base)`,
/// nothing can be concluded: that is a patience expiry naming the sentinel
/// (cause `patience`).
///
/// `forbidden` must observe something that STAYS observable (a record, a
/// file, a counter): a poll can miss a transient, and a missed transient
/// passes. Call it on the step's own thread (see [`wait_for`]).
#[track_caller]
pub fn absent_until(
    what: &str,
    forbidden: impl FnMut() -> bool,
    sentinel_what: &str,
    base: Duration,
    sentinel: impl FnMut() -> bool,
) {
    let f = current_factors();
    absent_until_core(what, forbidden, sentinel_what, base, &f, sentinel)
}

/// Fail the step as the harness's fault, not the product's: a fixture that
/// would not start, a port it could not bind. Panics with `message` and marks
/// the cause `harness` (the step's failure gets a `  cause: harness` line).
/// Call it on the step's own thread.
#[track_caller]
pub fn harness_error(message: impl Display) -> ! {
    fail(Cause::Harness, message.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::panic::{catch_unwind, AssertUnwindSafe};

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    // --- Scale parsing -------------------------------------------------------

    #[test]
    fn scale_accepts_ordinary_float_spellings_in_range() {
        for (v, want) in [
            ("1.0", 1.0),
            ("1", 1.0),
            ("2.0", 2.0),
            ("0.5", 0.5),
            ("3", 3.0),
            ("2.", 2.0),
            (".5", 0.5),
            ("+2", 2.0),
            ("1e1", 10.0),
            ("0.01", 0.01),
            ("1000", 1000.0),
            ("1e3", 1000.0),
        ] {
            assert_eq!(parse_scale(v, "X"), Ok(want), "{v}");
        }
    }

    #[test]
    fn scale_rejects_everything_else_naming_variable_value_and_range() {
        for v in [
            "0",
            "0.0",
            "-1",
            "-0.5",
            "NaN",
            "nan",
            "inf",
            "-inf",
            "infinity",
            "",
            " ",
            "abc",
            "1x",
            "1,5",
            " 2",
            "0.0099",
            "0.00999999",
            "1000.001",
            "1000.1",
            "1e-300",
            "1.7976931348623157e308",
        ] {
            let e = parse_scale(v, "ABDD_TIME_SCALE (environment)").expect_err(v);
            assert!(e.starts_with("ABDD_TIME_SCALE (environment) is "), "{e}");
            assert!(e.contains(&format!("{v:?}")), "{e}");
            assert!(e.contains("a number from 0.01 to 1000"), "{e}");
        }
    }

    // --- Resolution ----------------------------------------------------------

    fn resolve_in(env: &[(&str, &str)], files: &[(&str, &str)]) -> Result<Settings, String> {
        let env: HashMap<String, OsString> = env
            .iter()
            .map(|(k, v)| (k.to_string(), OsString::from(v)))
            .collect();
        let files: HashMap<PathBuf, String> = files
            .iter()
            .map(|(k, v)| (PathBuf::from(k), v.to_string()))
            .collect();
        resolve(&|n| env.get(n).cloned(), &|p| {
            files
                .get(p)
                .cloned()
                .ok_or_else(|| "No such file or directory".to_string())
        })
    }

    #[test]
    fn unset_means_one_and_no_file_is_read() {
        let s = resolve(&|_| None, &|_| panic!("no file may be read")).unwrap();
        assert_eq!(s.scale, 1.0);
        assert_eq!(s.scale_source, Source::Unset);
        assert_eq!(s.machine_env, None);
    }

    #[test]
    fn environment_scale_is_used_trimmed_and_bad_one_is_loud() {
        let s = resolve_in(&[("ABDD_TIME_SCALE", "2")], &[]).unwrap();
        assert_eq!((s.scale, s.scale_source), (2.0, Source::Environment));
        let s = resolve_in(&[("ABDD_TIME_SCALE", " \t2.5 ")], &[]).unwrap();
        assert_eq!((s.scale, s.scale_source), (2.5, Source::Environment));
        let e = resolve_in(&[("ABDD_TIME_SCALE", "0")], &[]).unwrap_err();
        assert!(e.contains("ABDD_TIME_SCALE (environment) is \"0\""), "{e}");
    }

    #[test]
    fn the_scale_range_holds_at_both_ends_from_the_environment_and_the_file() {
        for (v, ok) in [
            ("0.01", true),
            ("0.0099", false),
            ("1000", true),
            ("1000.01", false),
        ] {
            let r = resolve_in(&[("ABDD_TIME_SCALE", v)], &[]);
            assert_eq!(r.is_ok(), ok, "{v}: {r:?}");
            let text = format!("ABDD_TIME_SCALE={v}\n");
            let r = resolve_in(&[("ABDD_MACHINE_ENV", "/m.env")], &[("/m.env", &text)]);
            assert_eq!(r.is_ok(), ok, "file {v}: {r:?}");
            if let Err(e) = r {
                assert!(
                    e.starts_with(&format!("ABDD_TIME_SCALE (/m.env line 1) is \"{v}\"")),
                    "{e}"
                );
                assert!(e.contains("a number from 0.01 to 1000"), "{e}");
            }
        }
    }

    #[test]
    fn file_supplies_the_scale_with_its_path_and_line() {
        let s = resolve_in(
            &[("ABDD_MACHINE_ENV", "/m.env")],
            &[(
                "/m.env",
                "# comment\nABDD_HEAVY_JOBS=4\nABDD_TIME_SCALE=3\n",
            )],
        )
        .unwrap();
        assert_eq!(s.scale, 3.0);
        assert_eq!(
            s.scale_source,
            Source::File {
                path: "/m.env".into(),
                line: 3
            }
        );
        assert_eq!(s.machine_env.as_deref(), Some("/m.env"));
    }

    #[test]
    fn environment_wins_over_the_file() {
        let s = resolve_in(
            &[("ABDD_MACHINE_ENV", "/m.env"), ("ABDD_TIME_SCALE", "2")],
            &[("/m.env", "ABDD_TIME_SCALE=3\n")],
        )
        .unwrap();
        assert_eq!((s.scale, s.scale_source), (2.0, Source::Environment));
    }

    #[test]
    fn an_empty_or_blank_environment_value_counts_as_unset() {
        // The scale falls through to the file.
        for blank in ["", " ", "\t \t"] {
            let s = resolve_in(
                &[("ABDD_MACHINE_ENV", "/m.env"), ("ABDD_TIME_SCALE", blank)],
                &[("/m.env", "ABDD_TIME_SCALE=3\n")],
            )
            .unwrap();
            assert_eq!(
                (s.scale, s.scale_source),
                (
                    3.0,
                    Source::File {
                        path: "/m.env".into(),
                        line: 1
                    }
                ),
                "{blank:?}"
            );
            // And with no file, to the default.
            let s = resolve_in(&[("ABDD_TIME_SCALE", blank)], &[]).unwrap();
            assert_eq!((s.scale, s.scale_source), (1.0, Source::Unset));
            // A blank ABDD_MACHINE_ENV names no file: nothing is read.
            let s = resolve(
                &|n| (n == "ABDD_MACHINE_ENV").then(|| OsString::from(blank)),
                &|_| panic!("no file may be read"),
            )
            .unwrap();
            assert_eq!(s.machine_env, None, "{blank:?}");
        }
    }

    #[test]
    fn a_bad_scale_in_the_file_names_file_and_line() {
        let e = resolve_in(
            &[("ABDD_MACHINE_ENV", "/m.env")],
            &[("/m.env", "\nABDD_TIME_SCALE=-1\n")],
        )
        .unwrap_err();
        assert!(
            e.contains("ABDD_TIME_SCALE (/m.env line 2) is \"-1\""),
            "{e}"
        );
    }

    #[test]
    fn an_unreadable_named_file_is_loud() {
        let e = resolve_in(&[("ABDD_MACHINE_ENV", "/missing.env")], &[]).unwrap_err();
        assert!(
            e.contains("ABDD_MACHINE_ENV names /missing.env, which cannot be read"),
            "{e}"
        );
    }

    #[test]
    fn a_relative_settings_path_is_refused_and_never_read() {
        for rel in ["m.env", "./m.env", "../abdd/m.env", "conf/m.env"] {
            let e = resolve(
                &|n| (n == "ABDD_MACHINE_ENV").then(|| OsString::from(rel)),
                &|_| panic!("a relative path must not be read"),
            )
            .unwrap_err();
            assert!(
                e.starts_with(&format!("ABDD_MACHINE_ENV is {rel:?}, a relative path")),
                "{e}"
            );
            assert!(e.contains("absolute path"), "{e}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_non_utf8_settings_path_is_opened_as_given() {
        use std::os::unix::ffi::OsStringExt;
        let raw = b"/tmp/m\xff.env".to_vec();
        let want = PathBuf::from(OsString::from_vec(raw.clone()));
        let s = resolve(
            &|n| (n == "ABDD_MACHINE_ENV").then(|| OsString::from_vec(raw.clone())),
            &|p| {
                assert_eq!(
                    p,
                    want.as_path(),
                    "the path must reach the reader unchanged"
                );
                Ok("ABDD_TIME_SCALE=2\n".into())
            },
        )
        .unwrap();
        assert_eq!(s.scale, 2.0);
        assert_eq!(s.machine_env.as_deref(), Some("/tmp/m\u{fffd}.env"));
    }

    #[test]
    fn a_malformed_abdd_line_is_loud_even_when_the_environment_sets_the_scale() {
        let e = resolve_in(
            &[("ABDD_MACHINE_ENV", "/m.env"), ("ABDD_TIME_SCALE", "2")],
            &[("/m.env", "ABDD_TIME_SCALE=3\nABDD_JUNK\n")],
        )
        .unwrap_err();
        assert!(e.starts_with("/m.env line 2: ABDD_JUNK:"), "{e}");
    }

    // --- File format ---------------------------------------------------------

    fn one(text: &str) -> (String, String) {
        let v = parse_file("f", text).unwrap();
        assert_eq!(v.len(), 1, "{text:?} -> {v:?}");
        (v[0].name.clone(), v[0].value.clone())
    }

    #[test]
    fn every_accepted_form() {
        let want = ("ABDD_X".to_string(), "1.5".to_string());
        for text in [
            "ABDD_X=1.5",
            "  ABDD_X = 1.5  ",
            "ABDD_X=1.5 # trailing comment",
            "ABDD_X=1.5# no space",
            "ABDD_X=\"1.5\"",
            "ABDD_X='1.5'",
            "ABDD_X = '1.5' # quoted and commented",
            "ABDD_X=\"1.5\"# comment right after",
            "\n# full-line comment\n   \nABDD_X=1.5\n\n",
            "\tABDD_X=1.5\t",
            "ABDD_X=1.5\r\n",
            "export ABDD_X=1.5",
            "  export\tABDD_X=1.5",
            "\u{feff}ABDD_X=1.5",
            "\u{feff}export ABDD_X=1.5\r\n",
        ] {
            assert_eq!(one(text), want, "{text:?}");
        }
        assert_eq!(one("ABDD_X="), ("ABDD_X".into(), String::new()));
        assert_eq!(one("ABDD_X=\"\""), ("ABDD_X".into(), String::new()));
        assert_eq!(one("ABDD_X=a b"), ("ABDD_X".into(), "a b".into()));
        let v = parse_file("f", "ABDD_A=1\n\nABDD_B=2\n").unwrap();
        assert_eq!((v[0].line, v[1].line), (1, 3));
        let v = parse_file("f", "ABDD_A=1\r\nOTHER=x\r\nABDD_B=2\r\n").unwrap();
        assert_eq!((v[0].value.as_str(), v[1].value.as_str()), ("1", "2"));
    }

    #[test]
    fn a_quoted_value_may_hold_hash_and_equals() {
        assert_eq!(one("ABDD_X=\"a#b\""), ("ABDD_X".into(), "a#b".into()));
        assert_eq!(
            one("ABDD_X='a=b#c' # note"),
            ("ABDD_X".into(), "a=b#c".into())
        );
        assert_eq!(one("ABDD_X=\"it's\""), ("ABDD_X".into(), "it's".into()));
        assert_eq!(one("ABDD_X=' 2 '"), ("ABDD_X".into(), " 2 ".into()));
        // An unquoted value ends at the first #.
        assert_eq!(one("ABDD_X=a#b"), ("ABDD_X".into(), "a".into()));
        // A quoted scale with blanks inside is trimmed before parsing.
        let s = resolve_in(
            &[("ABDD_MACHINE_ENV", "/m.env")],
            &[("/m.env", "ABDD_TIME_SCALE=\" 2 \" # quoted\n")],
        )
        .unwrap();
        assert_eq!(s.scale, 2.0);
    }

    #[test]
    fn a_bom_is_skipped_only_on_the_first_line() {
        let v = parse_file("f", "\u{feff}ABDD_A=1\n\u{feff}ABDD_B=2\n").unwrap();
        assert_eq!(v.len(), 1, "{v:?}");
        assert_eq!(v[0].name, "ABDD_A");
    }

    #[test]
    fn only_abdd_lines_are_looked_at() {
        // Malformed, unbalanced, odd and secret-bearing lines that do not
        // begin with ABDD_ are skipped unread.
        let text = "HOME=/x\nPATH='a'\nabdd_x=1\nOTHER=\"unclosed\nmy-var=1\njunk junk\n\
                    SECRET_TOKEN=\"abc#def\" trailing\n=1\nexportABDD_Z=1\n1ABDD=1\n\
                    ABDD-X=1\nexport\nABDD_Y=2\n";
        let v = parse_file("f", text).unwrap();
        assert_eq!(v.len(), 1, "{v:?}");
        assert_eq!((v[0].name.as_str(), v[0].line), ("ABDD_Y", 13));
    }

    #[test]
    fn an_abdd_machine_env_line_in_the_file_is_ignored() {
        let v = parse_file(
            "f",
            "ABDD_MACHINE_ENV=/etc/passwd\nABDD_MACHINE_ENV=again\nABDD_TIME_SCALE=2\n",
        )
        .unwrap();
        assert_eq!(v.len(), 1, "{v:?}");
        assert_eq!(v[0].name, "ABDD_TIME_SCALE");
        let s = resolve_in(
            &[("ABDD_MACHINE_ENV", "/m.env")],
            &[("/m.env", "ABDD_MACHINE_ENV=/other.env\nABDD_TIME_SCALE=2\n")],
        )
        .unwrap();
        assert_eq!(s.machine_env.as_deref(), Some("/m.env"));
        assert_eq!(s.scale, 2.0);
    }

    #[test]
    fn every_rejected_form_names_file_line_and_name_and_quotes_that_line() {
        for (text, line, body) in [
            ("ABDD_X", 1, "ABDD_X"),
            (
                "ABDD_X=1\nABDD_Y no equals sign",
                2,
                "ABDD_Y no equals sign",
            ),
            ("ABDD_X junk=1", 1, "ABDD_X junk=1"),
            ("ABDD_X-Y=1", 1, "ABDD_X-Y=1"),
            ("ABDD_X=\"1", 1, "ABDD_X=\"1"),
            ("ABDD_X=1'", 1, "ABDD_X=1'"),
            ("ABDD_X=\"1'", 1, "ABDD_X=\"1'"),
            ("ABDD_X='a'b'", 1, "ABDD_X='a'b'"),
            ("ABDD_X=\"", 1, "ABDD_X=\""),
            ("ABDD_X='2'\"", 1, "ABDD_X='2'\""),
            ("ABDD_X=\"2\" 3", 1, "ABDD_X=\"2\" 3"),
            ("export ABDD_X=\"1", 1, "ABDD_X=\"1"),
        ] {
            let e = parse_file("/etc/m.env", text).expect_err(text);
            let name: String = body
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            assert!(
                e.starts_with(&format!("/etc/m.env line {line}: {name}: ")),
                "{text:?}: {e}"
            );
            assert!(e.ends_with(&format!(": {body:?}")), "{text:?}: {e}");
        }
        let e = parse_file("/etc/m.env", "ABDD_X=1\nABDD_X=2").unwrap_err();
        assert_eq!(
            e,
            "/etc/m.env line 2: ABDD_X is set again (first on line 1); keep one"
        );
    }

    /// No message from the settings path may hold text from a line that is
    /// not an ABDD_ line, whatever the error and wherever the bad line is.
    #[test]
    fn no_message_quotes_a_non_abdd_line() {
        const SECRET: &str = "hunter2-SECRET-MARKER";
        let others = [
            format!("API_KEY={SECRET}"),
            format!("PASSWORD=\"{SECRET}"),
            format!("{SECRET} not even a NAME=value line"),
            format!("TOKEN='{SECRET}'\"#"),
            format!("export X={SECRET}"),
        ];
        let bad_abdd = [
            "ABDD_TIME_SCALE=0",
            "ABDD_TIME_SCALE=\"2",
            "ABDD_JUNK",
            "ABDD_TIME_SCALE=2\nABDD_TIME_SCALE=3",
        ];
        for other in &others {
            for (before, after) in [(true, false), (false, true), (true, true)] {
                for bad in bad_abdd {
                    let mut text = String::new();
                    if before {
                        text.push_str(other);
                        text.push('\n');
                    }
                    text.push_str(bad);
                    text.push('\n');
                    if after {
                        text.push_str(other);
                        text.push('\n');
                    }
                    let e = resolve_in(&[("ABDD_MACHINE_ENV", "/m.env")], &[("/m.env", &text)])
                        .unwrap_err();
                    assert!(!e.contains(SECRET), "{text:?} leaked: {e}");
                }
                // And a good file around the same line: no error, nothing quoted.
                let text = format!("{other}\nABDD_TIME_SCALE=2\n{other}\n");
                let s =
                    resolve_in(&[("ABDD_MACHINE_ENV", "/m.env")], &[("/m.env", &text)]).unwrap();
                let header = header_line(1.0, &s);
                assert!(!header.contains(SECRET), "{header}");
            }
        }
    }

    // --- Arithmetic ----------------------------------------------------------

    #[test]
    fn patience_multiplies_base_default_and_scale() {
        assert_eq!(patience_with(ms(100), 1.0, 1.0), ms(100));
        assert_eq!(patience_with(ms(100), 3.0, 1.0), ms(300));
        assert_eq!(patience_with(ms(100), 1.0, 2.0), ms(200));
        assert_eq!(patience_with(ms(100), 3.0, 2.0), ms(600));
        assert_eq!(patience_with(ms(100), 1.0, 0.5), ms(50));
        assert_eq!(patience_with(ms(100), 3.0, 0.5), ms(150));
        assert_eq!(patience_with(Duration::MAX, 2.0, 2.0), Duration::MAX);
    }

    #[test]
    fn absence_window_never_shrinks_and_ignores_the_default() {
        assert_eq!(absence_with(ms(100), 0.5), ms(100));
        assert_eq!(absence_with(ms(100), 1.0), ms(100));
        assert_eq!(absence_with(ms(100), 2.0), ms(200));
        // The signature has no project default: it cannot apply. And the
        // public function reads only the scale, never project_default().
        let header = header_line(
            3.0,
            &Settings {
                scale: 0.5,
                scale_source: Source::Environment,
                machine_env: None,
            },
        );
        assert!(
            header.contains("patience x1.5 (default 3 x machine 0.5)"),
            "{header}"
        );
    }

    #[test]
    fn header_names_each_source() {
        let s = |scale, scale_source, machine_env: Option<&str>| Settings {
            scale,
            scale_source,
            machine_env: machine_env.map(String::from),
        };
        assert_eq!(
            header_line(3.0, &s(1.0, Source::Unset, None)),
            "gherkin-cargo-test: patience x3 (default 3 x machine 1); ABDD_TIME_SCALE unset"
        );
        assert_eq!(
            header_line(1.0, &s(2.0, Source::Environment, None)),
            "gherkin-cargo-test: patience x2 (default 1 x machine 2); ABDD_TIME_SCALE=2 (environment)"
        );
        let file = Source::File {
            path: "/m.env".into(),
            line: 4,
        };
        assert_eq!(
            header_line(1.0, &s(2.5, file, Some("/m.env"))),
            "gherkin-cargo-test: patience x2.5 (default 1 x machine 2.5); ABDD_TIME_SCALE=2.5 (/m.env line 4); ABDD_MACHINE_ENV=/m.env (environment)"
        );
    }

    #[test]
    fn the_header_is_ascii_and_prints_factors_to_three_places() {
        let s = |scale, scale_source| Settings {
            scale,
            scale_source,
            machine_env: None,
        };
        // 0.1 x 3 is 0.30000000000000004 in f64.
        assert_eq!(0.1_f64 * 3.0, 0.30000000000000004);
        let h = header_line(0.1, &s(3.0, Source::Environment));
        assert_eq!(
            h,
            "gherkin-cargo-test: patience x0.3 (default 0.1 x machine 3); ABDD_TIME_SCALE=3 (environment)"
        );
        assert!(h.is_ascii(), "{h}");
        let h = header_line(1.0 / 3.0, &s(2.0 / 3.0, Source::Environment));
        assert_eq!(
            h,
            "gherkin-cargo-test: patience x0.222 (default 0.333 x machine 0.667); ABDD_TIME_SCALE=0.667 (environment)"
        );
        for (v, want) in [
            (3.0, "3"),
            (0.30000000000000004, "0.3"),
            (2.5, "2.5"),
            (0.01, "0.01"),
            (1000.0, "1000"),
            (0.1005, "0.101"),
            (10.0, "10"),
        ] {
            assert_eq!(factor(v), want, "{v}");
        }
    }

    #[test]
    fn the_poll_schedule_backs_off_and_never_sleeps_past_a_deadline() {
        let mut i = FIRST_POLL;
        let mut seen = vec![i];
        for _ in 0..10 {
            i = next_poll(i);
            seen.push(i);
        }
        assert_eq!(
            &seen[..6],
            &[ms(5), ms(10), ms(20), ms(40), ms(80), ms(100)]
        );
        assert!(seen[5..].iter().all(|d| *d == MAX_POLL));
        let now = Instant::now();
        for interval in &seen {
            for left in [0, 1, 3, 7, 50, 99, 100, 1000] {
                let deadline = now + ms(left);
                let n = nap(*interval, now, deadline);
                assert!(
                    n <= ms(left),
                    "{interval:?} with {left} ms left slept {n:?}"
                );
                assert!(n <= *interval);
            }
        }
        // Past the deadline: no sleep at all.
        assert_eq!(nap(ms(100), now + ms(10), now), Duration::ZERO);
    }

    // --- Timing (lower bounds strict, upper bounds generous) ------------------

    fn unit() -> Factors {
        Factors {
            default: 1.0,
            scale: 1.0,
            source: "ABDD_TIME_SCALE unset".into(),
        }
    }

    /// Run `f`, return its panic message and this thread's cause for it.
    fn caught(f: impl FnOnce()) -> Option<(String, Option<Cause>)> {
        clear_cause();
        let e = catch_unwind(AssertUnwindSafe(f)).err()?;
        let cause = take_cause(&*e);
        let msg = e
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
            .unwrap();
        Some((msg, cause))
    }

    #[test]
    fn wait_for_returns_promptly_when_the_predicate_turns_true() {
        let start = Instant::now();
        let mut calls = 0;
        wait_core("the flag", ms(5000), &unit(), || {
            calls += 1;
            (start.elapsed() >= ms(30)).then_some(())
        });
        let took = start.elapsed();
        assert!(took >= ms(30), "{took:?}");
        assert!(took < ms(2000), "returned late: {took:?}");
        assert!(calls >= 2);
    }

    #[test]
    fn wait_for_some_returns_the_value() {
        let mut n = 0;
        let v = wait_core("three", ms(5000), &unit(), || {
            n += 1;
            (n == 3).then_some(n * 10)
        });
        assert_eq!(v, 30);
    }

    #[test]
    fn wait_for_expires_no_earlier_than_its_limit_and_is_marked_patience() {
        let start = Instant::now();
        let (msg, cause) =
            caught(|| wait_core::<()>("the light", ms(60), &unit(), || None)).expect("expiry");
        let took = start.elapsed();
        assert!(took >= ms(60), "gave up early: {took:?}");
        assert!(took < ms(3000), "{took:?}");
        assert_eq!(cause, Some(Cause::Patience));
        assert!(
            msg.starts_with("wait_for: gave up waiting for the light after "),
            "{msg}"
        );
        assert!(
            msg.ends_with("limit 60ms = base 60ms x project default 1 x machine scale 1 (ABDD_TIME_SCALE unset)"),
            "{msg}"
        );
    }

    #[test]
    fn wait_for_checks_at_least_once_with_a_zero_base_and_at_the_deadline() {
        let mut calls = 0;
        wait_core("now", Duration::ZERO, &unit(), || {
            calls += 1;
            Some(())
        });
        assert_eq!(calls, 1);
        let mut calls = 0;
        assert!(
            caught(|| wait_core::<()>("never", Duration::ZERO, &unit(), || {
                calls += 1;
                None
            }))
            .is_some()
        );
        assert_eq!(calls, 1);
        // True only once the deadline has passed: the final check sees it.
        let start = Instant::now();
        wait_core("late", ms(40), &unit(), || {
            (start.elapsed() >= ms(40)).then_some(())
        });
    }

    #[test]
    fn absent_for_runs_its_whole_window() {
        let start = Instant::now();
        let mut calls = 0;
        absent_for_core("noise", ms(60), 1.0, || {
            calls += 1;
            false
        });
        let took = start.elapsed();
        assert!(took >= ms(60), "returned early: {took:?}");
        assert!(calls >= 2);
        // At scale 0.5 the window does not shrink.
        let start = Instant::now();
        absent_for_core("noise", ms(60), 0.5, || false);
        assert!(start.elapsed() >= ms(60));
    }

    #[test]
    fn absent_for_fails_unmarked_when_the_forbidden_thing_appears_midway() {
        let start = Instant::now();
        let (msg, cause) =
            caught(|| absent_for_core("the alarm", ms(2000), 1.0, || start.elapsed() >= ms(30)))
                .expect("failure");
        assert!(start.elapsed() < ms(1500), "did not fail when seen");
        assert_eq!(cause, None);
        assert!(msg.starts_with("absent_for: the alarm was seen "), "{msg}");
        assert!(
            msg.contains("into its 2s window (base 2s x max(1, machine scale 1))"),
            "{msg}"
        );
    }

    #[test]
    fn absent_for_fails_when_the_forbidden_thing_appears_only_at_the_end() {
        let start = Instant::now();
        let (msg, cause) =
            caught(|| absent_for_core("the alarm", ms(50), 1.0, || start.elapsed() >= ms(50)))
                .expect("the final check must see it");
        assert_eq!(cause, None);
        assert!(msg.contains("the alarm was seen"), "{msg}");
    }

    #[test]
    fn absent_until_returns_when_the_sentinel_arrives() {
        let start = Instant::now();
        absent_until_core(
            "a duplicate",
            || false,
            "the receipt",
            ms(5000),
            &unit(),
            || start.elapsed() >= ms(30),
        );
        let took = start.elapsed();
        assert!(took >= ms(30) && took < ms(2000), "{took:?}");
    }

    #[test]
    fn absent_until_fails_unmarked_when_the_forbidden_thing_is_seen_first() {
        let start = Instant::now();
        let (msg, cause) = caught(|| {
            absent_until_core(
                "a duplicate",
                || start.elapsed() >= ms(20),
                "the receipt",
                ms(5000),
                &unit(),
                || false,
            )
        })
        .expect("failure");
        assert_eq!(cause, None);
        assert!(
            msg.starts_with("absent_until: a duplicate was seen "),
            "{msg}"
        );
        assert!(msg.contains("into the wait for the receipt"), "{msg}");
        // Seen only at the sentinel's arrival: the re-check catches it.
        let mut n = 0;
        let (msg, cause) = caught(|| {
            absent_until_core(
                "a duplicate",
                || {
                    n += 1;
                    n > 1
                },
                "the receipt",
                ms(5000),
                &unit(),
                || true,
            )
        })
        .expect("failure");
        assert_eq!(cause, None);
        assert!(msg.contains("was seen when the receipt arrived"), "{msg}");
    }

    #[test]
    fn absent_until_expires_as_patience_naming_the_sentinel() {
        let start = Instant::now();
        let (msg, cause) = caught(|| {
            absent_until_core(
                "a duplicate",
                || false,
                "the receipt",
                ms(50),
                &unit(),
                || false,
            )
        })
        .expect("expiry");
        assert!(start.elapsed() >= ms(50));
        assert_eq!(cause, Some(Cause::Patience));
        assert!(
            msg.starts_with("absent_until: gave up waiting for the receipt after "),
            "{msg}"
        );
        assert!(
            msg.contains("nothing can be concluded about a duplicate; limit 50ms"),
            "{msg}"
        );
    }

    #[test]
    fn absent_until_waits_for_patience_not_the_absence_window() {
        // With a project default of 0.1 and scale 0.5, patience(200ms) is
        // 10ms while absence_window(200ms) is 200ms: the expiry must come
        // well before 200ms and name the patience limit.
        let f = Factors {
            default: 0.1,
            scale: 0.5,
            source: "ABDD_TIME_SCALE=0.5 (environment)".into(),
        };
        let start = Instant::now();
        let (msg, cause) = caught(|| {
            absent_until_core(
                "a duplicate",
                || false,
                "the receipt",
                ms(200),
                &f,
                || false,
            )
        })
        .expect("expiry");
        let took = start.elapsed();
        assert!(took >= ms(10), "gave up early: {took:?}");
        assert!(
            took < ms(150),
            "waited the absence window, not patience: {took:?}"
        );
        assert_eq!(cause, Some(Cause::Patience));
        assert!(
            msg.ends_with(
                "limit 10ms = base 200ms x project default 0.1 x machine scale 0.5 \
                 (ABDD_TIME_SCALE=0.5 (environment))"
            ),
            "{msg}"
        );
        // And upward: a default of 3 at scale 1 gives patience 150ms against
        // an absence window of 50ms; a sentinel at 100ms must be waited for.
        let f = Factors {
            default: 3.0,
            scale: 1.0,
            source: "ABDD_TIME_SCALE unset".into(),
        };
        let start = Instant::now();
        absent_until_core(
            "a duplicate",
            || false,
            "the receipt",
            ms(50),
            &f,
            || start.elapsed() >= ms(100),
        );
        assert!(start.elapsed() >= ms(100));
    }

    #[test]
    fn a_slow_predicate_overruns_and_the_message_reports_the_real_elapsed_time() {
        let (msg, _) = caught(|| {
            wait_core::<()>("the slow probe", ms(10), &unit(), || {
                thread::sleep(ms(120));
                None
            })
        })
        .expect("expiry");
        // "after 120.xms" or more: the real elapsed time, not the limit.
        let after = msg
            .split("after ")
            .nth(1)
            .and_then(|r| r.split(';').next())
            .unwrap_or_default();
        assert!(
            after.ends_with("ms")
                && after.trim_end_matches("ms").parse::<f64>().unwrap_or(0.0) >= 120.0,
            "{msg}"
        );
        assert!(msg.contains("limit 10ms"), "{msg}");
    }

    #[test]
    fn harness_error_is_marked_harness_and_a_different_failure_is_not() {
        let (msg, cause) = caught(|| harness_error("port 8080 is taken")).unwrap();
        assert_eq!(
            (msg.as_str(), cause),
            ("port 8080 is taken", Some(Cause::Harness))
        );
        // Caught, then a different failure: the mark does not match it.
        let (_, cause) = caught(|| {
            let _ = catch_unwind(|| harness_error("first"));
            panic!("second");
        })
        .unwrap();
        assert_eq!(cause, None);
    }

    #[test]
    fn project_default_set_twice_must_agree() {
        // 1.0 is the unset value, so setting it here changes no other test.
        set_project_default(1.0);
        set_project_default(1.0);
        let e = catch_unwind(|| set_project_default(2.0)).expect_err("must panic");
        let msg = e.downcast_ref::<String>().unwrap();
        assert!(
            msg.contains("already 1 in this process; set it once"),
            "{msg}"
        );
    }

    #[test]
    fn project_default_must_lie_in_range() {
        for bad in [
            0.0,
            -1.0,
            f64::NAN,
            f64::INFINITY,
            f64::MAX,
            1e-300,
            0.0099,
            1000.001,
        ] {
            let e = catch_unwind(|| set_project_default(bad)).expect_err("must panic");
            let msg = e.downcast_ref::<String>().unwrap();
            assert!(
                msg.starts_with(&format!("patience_default({bad}): ")),
                "{msg}"
            );
            assert!(msg.contains("a number from 0.01 to 1000"), "{msg}");
        }
        // Both ends are allowed (checked without setting the process-wide
        // default, which other tests share).
        for (v, ok) in [
            (0.01, true),
            (0.0099, false),
            (1000.0, true),
            (1000.01, false),
        ] {
            assert_eq!(check_project_default(v).is_ok(), ok, "{v}");
        }
    }

    #[test]
    fn an_unrepresentable_deadline_fails_fast_as_a_harness_error() {
        // Each case runs on its own thread with a hard bound, so a
        // regression that loops forever fails here instead of hanging.
        fn bounded(
            name: &str,
            f: impl FnOnce() -> Option<(String, Option<Cause>)> + Send + 'static,
        ) {
            let (tx, rx) = std::sync::mpsc::channel();
            thread::spawn(move || {
                let _ = tx.send(f());
            });
            let got = rx
                .recv_timeout(Duration::from_secs(10))
                .unwrap_or_else(|_| {
                    panic!("{name}: the wait did not fail fast (hung or no result)")
                });
            let (msg, cause) =
                got.unwrap_or_else(|| panic!("{name}: the wait returned instead of failing"));
            assert_eq!(cause, Some(Cause::Harness), "{name}: {msg}");
            assert!(
                msg.starts_with("gherkin-cargo-test: the limit ") && msg.contains(" is too large"),
                "{name}: {msg}"
            );
        }
        let start = Instant::now();
        let mut calls = 0;
        bounded("wait_for", move || {
            caught(|| {
                wait_core::<()>("the moon", Duration::MAX, &unit(), || {
                    calls += 1;
                    None
                })
            })
        });
        bounded("wait_for, scaled", || {
            let f = Factors {
                default: 1000.0,
                scale: 1000.0,
                source: "x".into(),
            };
            caught(|| wait_core::<()>("the moon", Duration::from_secs(u64::MAX / 4), &f, || None))
        });
        bounded("absent_for", || {
            caught(|| absent_for_core("noise", Duration::MAX, 1.0, || false))
        });
        bounded("absent_until", || {
            caught(|| {
                absent_until_core(
                    "noise",
                    || false,
                    "the end",
                    Duration::MAX,
                    &unit(),
                    || false,
                )
            })
        });
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "{:?}",
            start.elapsed()
        );
    }
}
