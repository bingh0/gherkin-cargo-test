// gct-lint: the feature-file linter as a command. A thin shell over the pure
// `lint_feature` / `lint_feature_strict`: this file owns only what the
// library deliberately refuses to do, which is reading the filesystem,
// walking directories, printing and choosing an exit code. Every finding's
// rule, severity, line and message come from the library unchanged, so the
// CLI says exactly what a guard test calling `lint_feature` would say.
//
// The contract (walk, path spelling, output, exit codes) is shared with the
// node sibling's gnt-lint, so both print the same stdout for the same tree.
//
//   gct-lint                      # lints ./features, recursively
//   gct-lint features/ extra.feature
//   gct-lint --strict features/   # warnings become errors, + strict-tag

use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::Path;
use std::process::exit;

use gherkin_cargo_test::{lint_feature, lint_feature_strict, LintSeverity};

const USAGE: &str = "\
usage: gct-lint [--strict] [PATH]...

Lint .feature files against the gherkin-cargo-test dialect and spec lints.
Each PATH is a .feature file or a directory, walked recursively in sorted
order (hidden directories, target/ and node_modules/ are skipped). With no
PATH, lints ./features. One line per finding on stdout:

  path:line: severity[rule]: message

  --strict       promote every warning to an error and add the strict-tag
                 rule (lint_feature_strict)
  -h, --help     print this help
  -V, --version  print the version

Exit status: 0 when no finding is an error (warnings alone pass; --strict
makes them fail), 1 on any error finding, an unreadable path or a directory
with no .feature files, 2 on a usage error.

There is no allow list here: keep known debt in a guard test's ALLOW
register (README: \"A guard test to paste\").";

/// One file to lint: where it is, and how it is printed.
struct Target {
    path: std::path::PathBuf,
    shown: String,
}

// Directories a walk never enters: identical in both siblings' CLIs. A path
// named on the command line is always honored, skipped name or not.
fn skipped_dir(name: &str) -> bool {
    name.starts_with('.') || name == "target" || name == "node_modules"
}

/// Printed spelling of a walked entry: the parent's spelling, '/', the name.
/// Plain concatenation, never a path join: joins normalize differently per
/// language (`./features` survives in Rust and not in node), and both
/// siblings must print the same bytes.
fn join_shown(base: &str, name: &str) -> String {
    let sep_at_end = base.ends_with('/') || (cfg!(windows) && base.ends_with('\\'));
    if sep_at_end {
        format!("{base}{name}")
    } else {
        format!("{base}/{name}")
    }
}

/// Walk `dir` depth-first in sorted order, pushing every .feature file.
fn walk(dir: &Path, shown: &str, out: &mut Vec<Target>) -> io::Result<()> {
    let mut entries: Vec<fs::DirEntry> = fs::read_dir(dir)?.collect::<Result<_, _>>()?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = entry.path();
        let child = join_shown(shown, &name);
        let ft = entry.file_type()?;
        // Symlinked directories are not followed (no cycles); a symlink to a
        // file is linted like the file.
        if ft.is_dir() {
            if !skipped_dir(&name) {
                walk(&path, &child, out)?;
            }
        } else if (ft.is_file() || (ft.is_symlink() && path.is_file()))
            && path.extension().is_some_and(|e| e == "feature")
        {
            out.push(Target { path, shown: child });
        }
    }
    Ok(())
}

fn main() {
    let mut strict = false;
    let mut args: Vec<OsString> = Vec::new();
    let mut options_done = false;
    for arg in std::env::args_os().skip(1) {
        let text = arg.to_string_lossy();
        if !options_done && text.starts_with('-') {
            match text.as_ref() {
                "--" => options_done = true,
                "--strict" => strict = true,
                "-h" | "--help" => {
                    println!("{USAGE}");
                    exit(0);
                }
                "-V" | "--version" => {
                    println!("gct-lint {}", env!("CARGO_PKG_VERSION"));
                    exit(0);
                }
                _ => {
                    eprintln!("gct-lint: unknown option {text}\n\n{USAGE}");
                    exit(2);
                }
            }
        } else {
            args.push(arg);
        }
    }
    if args.is_empty() {
        args.push("features".into());
    }

    let mut failed = false;
    let mut files: Vec<Target> = Vec::new();
    for arg in &args {
        // A path named on the command line is printed exactly as typed.
        let shown = arg.to_string_lossy().into_owned();
        let path = Path::new(arg);
        let meta = match fs::metadata(path) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("gct-lint: {shown}: {e}");
                failed = true;
                continue;
            }
        };
        if meta.is_dir() {
            let before = files.len();
            if let Err(e) = walk(path, &shown, &mut files) {
                eprintln!("gct-lint: {shown}: {e}");
                failed = true;
            } else if files.len() == before {
                // Nothing to lint must not read as a pass: a typo'd or
                // emptied directory would otherwise go green forever.
                eprintln!("gct-lint: no .feature files under {shown}");
                failed = true;
            }
        } else {
            // A file named explicitly is linted whatever its extension.
            files.push(Target {
                path: path.to_path_buf(),
                shown,
            });
        }
    }

    let (mut errors, mut warnings) = (0usize, 0usize);
    for file in &files {
        let text = match fs::read_to_string(&file.path) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("gct-lint: {}: {e}", file.shown);
                failed = true;
                continue;
            }
        };
        let findings = if strict {
            lint_feature_strict(&text, &file.shown)
        } else {
            lint_feature(&text, &file.shown)
        };
        for f in findings {
            match f.severity {
                LintSeverity::Error => errors += 1,
                LintSeverity::Warn => warnings += 1,
            }
            println!(
                "{}:{}: {}[{}]: {}",
                file.shown,
                f.line,
                f.severity.as_str(),
                f.rule.as_str(),
                f.message
            );
        }
    }

    let plural = |n: usize| if n == 1 { "" } else { "s" };
    eprintln!(
        "gct-lint: {} file{}, {} error{}, {} warning{}{}",
        files.len(),
        plural(files.len()),
        errors,
        plural(errors),
        warnings,
        plural(warnings),
        if strict { " (strict)" } else { "" }
    );
    exit(if failed || errors > 0 { 1 } else { 0 });
}
