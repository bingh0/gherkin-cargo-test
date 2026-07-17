// Generative differential fuzzer: build pseudo-random .feature files from a
// weighted pool of grammar-ish and hostile line templates, run both parsers,
// compare canonical dumps. Deterministic PRNG so any divergence is
// reproducible by seed. Usage: node fuzz.js [count] [seed]
const { writeFileSync, mkdirSync } = require('node:fs');
const { execFileSync } = require('node:child_process');
const { join } = require('node:path');
const { parseFeature, lintFeature, GherkinSyntaxError } = require(process.env.GNT_PATH || '/home/biho/Documents/gherkin-node-test/index.js');

const COUNT = Number(process.argv[2] || 1000);
let seed = Number(process.argv[3] || 20260716);
const RUST_DUMP = require('node:path').join(__dirname, '../../target/debug/examples/dump');
const DIR = join(__dirname, 'fuzz-out');
mkdirSync(DIR, { recursive: true });

const rnd = () => {
  seed ^= seed << 13; seed ^= seed >>> 17; seed ^= seed << 5;
  return ((seed >>> 0) / 0xffffffff);
};
const pick = (a) => a[Math.floor(rnd() * a.length)];
const int = (n) => Math.floor(rnd() * n);

const words = ['counter', 'is', 'at', 'the', 'a', 'poked', 'beeps', 'C:\\Temp', 'Cmd+\\', '"quoted"', '42', '3.14', '<n>', '<x>', '<typo>', 'café', 'naïve', 'works'];
const cellPool = ['a', '', ' ', 'a\\|b', 'c\\\\d', 'e\\nf', '\\q', 'x\\', '<n>', '<x>', '42', 'ada lovelace', '|nope', 'café'];
const tagPool = ['@skip', '@todo', '@only', '@AC3', '@Skip', '@SKIP', '@Todo', '@a@b', '@wip'];
const kw = ['Given', 'When', 'Then', 'And', 'But', '*'];

const text = (n) => Array.from({ length: 1 + int(n) }, () => pick(words)).join(' ');
const row = () => `      | ${Array.from({ length: 1 + int(4) }, () => pick(cellPool)).join(' | ')} |`;

const lineGens = [
  // weighted toward coherent structure so deep paths get exercised
  () => `Feature: ${text(3)}`,
  () => `  Scenario: ${text(3)}`,
  () => `  Scenario: ${text(3)}`,
  () => `  Scenario Outline: ${text(3)}`,
  () => `  Background:`,
  () => `    Examples:`,
  () => `    ${pick(kw)} ${text(4)}`,
  () => `    ${pick(kw)} ${text(4)}`,
  () => `    ${pick(kw)} ${text(4)}`,
  () => row(),
  () => row(),
  () => `  ${pick(tagPool)}${rnd() < 0.4 ? ' ' + pick(tagPool) : ''}`,
  () => `  # ${text(4)}`,
  () => '',
  () => `  ${text(4)}`,             // narrative junk
  () => `    """`,
  () => '    ```',
  () => `  Rule: ${text(2)}`,
  () => `      | ${pick(cellPool)} | ${pick(cellPool)}`, // missing closing pipe
  () => '      |',
  () => `${pick(kw)}${text(2)}`,    // glued keyword (narrative)
];

const esc = (s) => s.replace(/\\/g, '\\\\').replace(/\t/g, '\\t').replace(/\n/g, '\\n');
const nodeDump = (raw, name) => {
  try {
    const p = parseFeature(raw, name);
    const out = [`FEATURE\t${esc(p.feature)}`];
    const step = (pre, st) => {
      out.push(`${pre}\t${st.line}\t${esc(st.keyword)}\t${esc(st.text)}`);
      if (st.table) for (const r of st.table) out.push(`ROW\t${r.map(esc).join('\t')}`);
    };
    for (const st of p.background) step('BSTEP', st);
    for (const sc of p.scenarios) {
      out.push(`SCENARIO\t${sc.line}\t${esc(sc.name)}`);
      if (sc.tags.length) out.push(`TAGS\t${sc.tags.map(esc).join('\t')}`);
      for (const st of sc.steps) step('STEP', st);
    }
    return out.join('\n');
  } catch (e) {
    if (!(e instanceof GherkinSyntaxError)) return `NODE-CRASH\t${e.constructor.name}`;
    return `REJECT\t${e.line}`;
  }
};

const nodeLint = (raw, name) => {
  try {
    return lintFeature(raw, name)
      .map((f) => `FINDING\t${f.rule}\t${f.severity}\t${f.line}\t${esc(f.message)}`)
      .join('\n');
  } catch (e) { return `NODE-CRASH\t${e.constructor.name}`; }
};

let identical = 0, accepts = 0, rejects = 0;
const diverged = [];
for (let i = 0; i < COUNT; i++) {
  // 70%: start structurally sane (Feature first); 30%: fully random
  const n = 2 + int(18);
  const lines = Array.from({ length: n }, () => pick(lineGens)());
  if (rnd() < 0.7) lines.unshift(`Feature: ${text(2)}`);
  const raw = lines.join('\n') + (rnd() < 0.8 ? '\n' : '');
  const file = join(DIR, `f${i}.feature`);
  writeFileSync(file, raw);
  const nd = nodeDump(raw, file);
  let rd;
  try { rd = execFileSync(RUST_DUMP, [file], { encoding: 'utf8' }).replace(/\r?\n$/, ''); }
  catch (e) { rd = `RUST-CRASH\t${(e.stderr || e.message).toString().split('\n')[0]}`; }
  const ndl = nodeLint(raw, file);
  let rdl;
  try { rdl = execFileSync(RUST_DUMP, ['--lint', file], { encoding: 'utf8' }).replace(/\r?\n$/, ''); }
  catch (e) { rdl = `RUST-CRASH\t${(e.stderr || e.message).toString().split('\n')[0]}`; }
  if (nd === rd && ndl === rdl) {
    identical += 1;
    if (nd.startsWith('REJECT')) rejects += 1; else accepts += 1;
  } else if (diverged.length < 8) {
    diverged.push({ i, nd: (nd === rd ? 'lint>> ' + ndl : nd).slice(0, 400), rd: (nd === rd ? 'lint>> ' + rdl : rd).slice(0, 400) });
  }
}
console.log(`${identical}/${COUNT} identical (${accepts} accepted, ${rejects} rejected by both)`);
for (const d of diverged) console.log(`\n=== DIVERGENCE f${d.i}.feature\n--- node:\n${d.nd}\n--- rust:\n${d.rd}`);
process.exit(identical === COUNT ? 0 : 1);
