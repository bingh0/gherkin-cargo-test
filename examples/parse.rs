// Corpus checker: parse every .feature path given on the command line and
// report accept/reject per file. Useful for checking an existing feature
// corpus against this crate's micro-grammar before porting a project:
//
//   cargo run --example parse -- path/to/features/*.feature

use std::process::exit;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("usage: parse <file.feature>...");
        exit(2);
    }
    let mut rejected = 0usize;
    let mut scenarios = 0usize;
    for path in &args {
        match std::fs::read_to_string(path) {
            Err(e) => {
                rejected += 1;
                eprintln!("READ ERROR {path}: {e}");
            }
            Ok(text) => match gherkin_cargo_test::parse_feature(&text, path) {
                Ok(p) => {
                    scenarios += p.scenarios.len();
                    println!("ok      {path} ({} scenarios)", p.scenarios.len());
                }
                Err(e) => {
                    rejected += 1;
                    println!("REJECT  {e}");
                }
            },
        }
    }
    println!(
        "\n{} file(s), {} rejected, {} scenarios total",
        args.len(),
        rejected,
        scenarios
    );
    exit(if rejected > 0 { 1 } else { 0 });
}
