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

static DEFER_RAN: AtomicBool = AtomicBool::new(false);

fn main() {
    let mut failures: Vec<String> = Vec::new();
    let mut check = |name: &str, c: Conclusion, passed: u64, failed: u64, ignored: u64| {
        let got = (c.num_passed, c.num_failed, c.num_ignored);
        if got == (passed, failed, ignored) {
            println!("guard-proof: {name} ok {got:?}");
        } else {
            failures.push(format!(
                "{name}: expected (passed, failed, ignored) = ({passed}, {failed}, {ignored}), got {got:?}"
            ));
        }
    };

    // A fully bound feature: everything green, nothing ignored.
    check(
        "good",
        run(Features::new("tests/fixtures/good").feature("counter", value_steps)),
        3, // orphan guard + binding guard + 1 scenario
        0,
        0,
    );

    // An unbound step: the binding guard FAILS (with a paste-ready snippet)
    // and the unbound scenario registers as ignored — never silently green.
    check(
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

    // A step matching two definitions: the ambiguity guard fails. The scenario
    // itself still runs (first match wins), so it passes — the guard is what
    // makes the suite red. Ambiguity is checked even for .wip() features.
    let ambiguous_defs = |reg: &mut StepRegistry<World>| {
        reg.define(r"^a doubly matched step$", |_, _, _| {});
        reg.define(r"doubly matched", |_, _, _| {});
    };
    check(
        "ambiguous",
        run(Features::new("tests/fixtures/ambiguous").feature("amb", ambiguous_defs)),
        2, // orphan guard + scenario
        1, // binding guard fails
        0,
    );
    check(
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
        "no-definer",
        run(Features::new("tests/fixtures/good")),
        1, // orphan guard
        1, // binding guard fails (everything unbound)
        1, // scenario ignored
    );

    // @skip: the scenario is ignored and its panicking step never runs
    // (a run would surface as a failure here).
    check(
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
    // would be the worst outcome. The scenario itself still runs.
    check(
        "only",
        run(Features::new("tests/fixtures/only").feature("only", value_steps)),
        3, // orphan + binding + scenario
        1, // the @only rejection trial
        0,
    );

    // A parse error fails as its own trial WITHOUT silencing sibling features.
    check(
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

    if failures.is_empty() {
        println!("guard-proof: all cases ok");
    } else {
        for f in &failures {
            eprintln!("guard-proof FAILURE: {f}");
        }
        exit(1);
    }
}
