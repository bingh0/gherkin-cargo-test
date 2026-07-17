// Mirror of gherkin-cargo-test's examples/dump.rs — identical canonical
// format so the two dumps diff byte-for-byte. See dump.rs for the format.
const { readFileSync } = require('node:fs');
const { parseFeature, GherkinSyntaxError } = require(process.env.GNT_PATH || '/home/biho/Documents/gherkin-node-test/index.js');

const esc = (s) => s.replace(/\\/g, '\\\\').replace(/\t/g, '\\t').replace(/\n/g, '\\n');

const out = [];
const dumpStep = (prefix, st) => {
  out.push(`${prefix}\t${st.line}\t${esc(st.keyword)}\t${esc(st.text)}`);
  if (st.table) for (const row of st.table) out.push(`ROW\t${row.map(esc).join('\t')}`);
};

const path = process.argv[2];
if (!path) { console.error('usage: dump-node.js <file.feature>'); process.exit(2); }
const text = readFileSync(path, 'utf8');
try {
  const p = parseFeature(text, path);
  out.push(`FEATURE\t${esc(p.feature)}`);
  for (const st of p.background) dumpStep('BSTEP', st);
  for (const sc of p.scenarios) {
    out.push(`SCENARIO\t${sc.line}\t${esc(sc.name)}`);
    if (sc.tags.length) out.push(`TAGS\t${sc.tags.map(esc).join('\t')}`);
    for (const st of sc.steps) dumpStep('STEP', st);
  }
} catch (e) {
  if (!(e instanceof GherkinSyntaxError)) throw e;
  out.length = 0;
  out.push(`REJECT\t${e.line}`);
}
console.log(out.join('\n'));
