// Mirror of gherkin-cargo-test's examples/dump.rs — identical canonical
// format so the two dumps diff byte-for-byte. See dump.rs for the format,
// including the --lint mode (FINDING lines; finding text is part of the
// parity contract).
const { readFileSync } = require('node:fs');
const { parseFeature, lintFeature, GherkinSyntaxError } = require(process.env.GNT_PATH || '/home/biho/Documents/gherkin-node-test/index.js');

const esc = (s) => s.replace(/\\/g, '\\\\').replace(/\t/g, '\\t').replace(/\n/g, '\\n');

const out = [];
const dumpStep = (prefix, st) => {
  out.push(`${prefix}\t${st.line}\t${esc(st.keyword)}\t${esc(st.text)}`);
  if (st.table) for (const row of st.table) out.push(`ROW\t${row.map(esc).join('\t')}`);
};

const args = process.argv.slice(2);
const lintMode = args[0] === '--lint';
const path = lintMode ? args[1] : args[0];
if (!path) { console.error('usage: dump-node.js [--lint] <file.feature>'); process.exit(2); }
const text = readFileSync(path, 'utf8');
if (lintMode) {
  for (const f of lintFeature(text, path)) {
    out.push(`FINDING\t${f.rule}\t${f.severity}\t${f.line}\t${esc(f.message)}`);
  }
  console.log(out.join('\n'));
  process.exit(0);
}
try {
  const p = parseFeature(text, path);
  out.push(`FEATURE\t${esc(p.feature)}`);
  for (const st of p.background) dumpStep('BSTEP', st);
  for (const sc of p.scenarios) {
    out.push(`SCENARIO\t${sc.line}\t${esc(sc.name)}`);
    if (sc.tags.length) out.push(`TAGS\t${sc.tags.map(esc).join('\t')}`);
    for (const st of sc.steps) dumpStep('STEP', st);
  }
  for (const n of p.narrative) out.push(`NARRATIVE\t${n.line}\t${n.inBody}\t${esc(n.text)}`);
} catch (e) {
  if (!(e instanceof GherkinSyntaxError)) throw e;
  out.length = 0;
  out.push(`REJECT\t${e.line}`);
}
console.log(out.join('\n'));
