// Executed proof of every runner guard — the Rust analog of gherkin-node-test's
// subprocess tests. Each case builds a Features runner over a fixture dir,
// runs it through libtest-mimic IN-PROCESS, and asserts the Conclusion counts:
// guards must actually fire (not merely exist), @skip must not run, @todo must
// not gate, a parse error must fail loudly without silencing sibling features,
// and deferred cleanup must run even when a step fails.
//
// harness = false: main() prints one line per case and exits nonzero if any
// expectation does not hold.

use std::process::exit;
use std::sync::atomic::{AtomicBool, Ordering};

use gherkin_cargo_test::{Features, StepRegistry};
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

fn main() {
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
    // itself still runs (first match wins), so it passes — the guard is what
    // makes the suite red. Ambiguity is checked even for .wip() features.
    let ambiguous_defs = |reg: &mut StepRegistry<World>| {
        reg.define(r"^a doubly matched step$", |_, _, _| {});
        reg.define(r"doubly matched", |_, _, _| {});
    };
    check(
        &mut failures,
        "ambiguous",
        run(Features::new("tests/fixtures/ambiguous").feature("amb", ambiguous_defs)),
        2, // orphan guard + scenario
        1, // binding guard fails
        0,
    );
    check(
        &mut failures,
        "ambiguous-even-when-wip",
        run(Features::new("tests/fixtures/ambiguous")
            .feature("amb", ambiguous_defs)
            .wip("amb")),
        2,
        1,
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

    if failures.is_empty() {
        println!("guard-proof: all cases ok");
    } else {
        for f in &failures {
            eprintln!("guard-proof FAILURE: {f}");
        }
        exit(1);
    }
}
