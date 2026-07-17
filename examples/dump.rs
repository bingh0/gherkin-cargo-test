// Canonical AST dump for differential parity testing against the node
// sibling: run the SAME .feature corpus through both parsers and compare the
// dumps byte-for-byte. gherkin-node-test ships the mirror dumper; neither
// side hand-writes expectations, so the corpus can't drift in tandem with
// either implementation.
//
//   cargo run --example dump -- path/to/file.feature
//
// One record per line, tab-separated; every free-text field is escaped
// (\ → \\, tab → \t, newline → \n) so cells containing separators stay
// unambiguous:
//   REJECT <line>                     the parser rejected the file
//   FEATURE <name>
//   BSTEP <line> <keyword> <text>     background step
//   ROW <c1> <c2> ...                 data-table row of the preceding step
//   SCENARIO <line> <name>
//   TAGS <t1> <t2> ...                only when non-empty
//   STEP <line> <keyword> <text>

use gherkin_cargo_test::{parse_feature, Step};

fn esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\t', "\\t").replace('\n', "\\n")
}

fn dump_step(prefix: &str, st: &Step) {
    println!("{prefix}\t{}\t{}\t{}", st.line, esc(&st.keyword), esc(&st.text));
    if let Some(table) = &st.table {
        for row in table {
            let cells: Vec<String> = row.iter().map(|c| esc(c)).collect();
            println!("ROW\t{}", cells.join("\t"));
        }
    }
}

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: dump <file.feature>");
        std::process::exit(2);
    });
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        eprintln!("READ ERROR {path}: {e}");
        std::process::exit(2);
    });
    match parse_feature(&text, &path) {
        Err(e) => println!("REJECT\t{}", e.line),
        Ok(p) => {
            println!("FEATURE\t{}", esc(&p.feature));
            for st in &p.background {
                dump_step("BSTEP", st);
            }
            for sc in &p.scenarios {
                println!("SCENARIO\t{}\t{}", sc.line, esc(&sc.name));
                if !sc.tags.is_empty() {
                    let tags: Vec<String> = sc.tags.iter().map(|t| esc(t)).collect();
                    println!("TAGS\t{}", tags.join("\t"));
                }
                for st in &sc.steps {
                    dump_step("STEP", st);
                }
            }
        }
    }
}
