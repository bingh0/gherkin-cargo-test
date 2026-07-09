// The crate's own acceptance layer: features/*.feature run through the real
// runner. `cargo test --test features` (or plain `cargo test`) executes it;
// name filters work per scenario: `cargo test --test features -- 'Counter'`.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use gherkin_cargo_test::{Features, StepRegistry};

// --- counter.feature ---------------------------------------------------------

#[derive(Default)]
struct Counter {
    count: i64,
}

fn counter_steps(reg: &mut StepRegistry<Counter>) {
    reg.define(r"^a counter at (-?\d+)$", |ctx, args, _| {
        ctx.world.count = args[0].parse().expect("integer");
    });
    reg.define(r"^I add (-?\d+)$", |ctx, args, _| {
        ctx.world.count += args[0].parse::<i64>().expect("integer");
    });
    reg.define_exact("I add these amounts", |ctx, _, table| {
        for row in table.expect("step has a data table").hashes() {
            ctx.world.count += row["amount"].parse::<i64>().expect("integer");
        }
    });
    reg.define(r"^the counter is (-?\d+)$", |ctx, args, _| {
        assert_eq!(ctx.world.count, args[0].parse::<i64>().expect("integer"));
    });
    // Bound but never run: its scenario is @skip. Skip means "don't run",
    // never "don't bind" — the binding guard still ratchets this step.
    reg.define_exact("I detonate", |_, _, _| {
        panic!("@skip scenario executed — skip is broken");
    });
}

// --- tables.feature ----------------------------------------------------------

#[derive(Default)]
struct Tables {
    users: Vec<HashMap<String, String>>,
    config: HashMap<String, String>,
}

fn tables_steps(reg: &mut StepRegistry<Tables>) {
    reg.define_exact("these users", |ctx, _, table| {
        ctx.world.users = table.expect("step has a data table").hashes();
    });
    reg.define(r#"^user "([^"]*)" has role "([^"]*)"$"#, |ctx, args, _| {
        let user = ctx
            .world
            .users
            .iter()
            .find(|u| u["name"] == args[0])
            .unwrap_or_else(|| panic!("no user named {:?}", args[0]));
        assert_eq!(user["role"], args[1]);
    });
    reg.define(r"^there are (\d+) users$", |ctx, args, _| {
        assert_eq!(
            ctx.world.users.len(),
            args[0].parse::<usize>().expect("integer")
        );
    });
    reg.define_exact("this config", |ctx, _, table| {
        ctx.world.config = table.expect("step has a data table").rows_hash();
    });
    reg.define(r#"^config "([^"]*)" is "([^"]*)"$"#, |ctx, args, _| {
        assert_eq!(ctx.world.config[&args[0]], args[1]);
    });
}

// --- cleanup.feature ---------------------------------------------------------

#[derive(Default)]
struct Cleanup {
    dir: Option<PathBuf>,
}

static SEQ: AtomicUsize = AtomicUsize::new(0);

fn cleanup_steps(reg: &mut StepRegistry<Cleanup>) {
    reg.define(
        r#"^a scratch dir with a file named "([^"]*)"$"#,
        |ctx, args, _| {
            let dir = std::env::temp_dir().join(format!(
                "gct-demo-{}-{}",
                std::process::id(),
                SEQ.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&dir).expect("create scratch dir");
            fs::write(dir.join(&args[0]), "probe").expect("write scratch file");
            ctx.world.dir = Some(dir.clone());
            ctx.defer(move |_| {
                let _ = fs::remove_dir_all(&dir);
            });
        },
    );
    reg.define(r#"^the scratch file "([^"]*)" exists$"#, |ctx, args, _| {
        let dir = ctx.world.dir.as_ref().expect("scratch dir was created");
        assert!(dir.join(&args[0]).is_file(), "scratch file missing");
    });
}

fn main() {
    Features::new("features")
        .feature("counter", counter_steps)
        .feature("tables", tables_steps)
        .feature("cleanup", cleanup_steps)
        .run()
}
