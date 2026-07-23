// Valid-biased differential fuzzer: structurally coherent features (matching
// placeholders, closed non-ragged tables, legal tag positions) so the ACCEPT
// path — outline expansion, substitution, escapes, tag inheritance — is
// exercised at scale, complementing fuzz.js's reject-heavy sweep.
// Usage: node fuzz-valid.js [count] [seed]
const { writeFileSync, mkdirSync } = require('node:fs');
const { execFileSync } = require('node:child_process');
const { join } = require('node:path');
const { parseFeature, lintFeature, GherkinSyntaxError } = require(process.env.GNT_PATH || '/home/biho/Documents/gherkin-node-test/index.js');

const COUNT = Number(process.argv[2] || 1000);
let seed = Number(process.argv[3] || 715);
const RUST_DUMP = require('node:path').join(__dirname, '../../target/debug/examples/dump');
const DIR = join(__dirname, 'fuzz-valid-out');
mkdirSync(DIR, { recursive: true });

const rnd = () => { seed ^= seed << 13; seed ^= seed >>> 17; seed ^= seed << 5; return ((seed >>> 0) / 0xffffffff); };
const pick = (a) => a[Math.floor(rnd() * a.length)];
const int = (n) => Math.floor(rnd() * n);
const maybe = (p) => rnd() < p;

const words = ['counter', 'is', 'at', 'poked', 'beeps', 'C:\\Temp', '"quoted"', '42', '3.14', 'café', 'sums to'];
const cells = ['a', '', 'a\\|b', 'c\\\\d', 'e\\nf', '\\q', '42', 'ada lovelace', 'café', '  padded  '];
const safeTags = ['@AC3', '@wip', '@smoke', '@a@b'];
const semantic = ['@skip', '@todo', '@only'];
const kw = ['Given', 'When', 'Then', 'And', 'But', '*'];
const text = (n) => Array.from({ length: 1 + int(n) }, () => pick(words)).join(' ');

function gen() {
  const L = [];
  // Titles seen this file: reused deliberately (maybe 0.12) so duplicate-title
  // — including its cross-construct and repeated-copy shapes — is generated on
  // purpose, not only by small-pool collisions.
  const titles = [];
  const title = () => {
    if (titles.length && maybe(0.12)) return pick(titles);
    const t = text(2);
    titles.push(t);
    return t;
  };
  const phs = ['n', 'x'].slice(0, 1 + int(2));
  const ph = () => `<${pick(phs)}>`;
  const cell = (allowPh) => (allowPh && maybe(0.3) ? ph() : pick(cells));
  const rowOf = (k, allowPh) => `      | ${Array.from({ length: k }, () => cell(allowPh)).join(' | ')} |`;
  const comment = () => { if (maybe(0.25)) L.push(`  # ${text(3)}`); };
  const blank = () => { if (maybe(0.25)) L.push(''); };
  const tagLine = () => {
    const t = [pick(safeTags)];
    if (maybe(0.3)) t.push(pick(safeTags));
    if (maybe(0.35)) t.push(pick(semantic)); // at most one semantic → no conflict
    L.push(`  ${t.join(' ')}`);
  };
  const steps = (allowPh, allowTable) => {
    for (let i = 0; i < 1 + int(3); i++) {
      L.push(`    ${pick(kw)} ${text(3)}${allowPh && maybe(0.5) ? ' ' + ph() : ''}`);
      // A near-miss line is narrative, so the file stays valid — this is the
      // accept-path exercise of near-miss-keyword.
      if (maybe(0.12)) L.push(`    ${pick(['given', 'WHEN', 'then', 'aNd', 'BUT'])} ${text(2)}`);
      if (maybe(0.06)) L.push(`  ${pick(['scenario: b', 'Scenario : b', 'examples :', 'rule: prose', 'scenarios: prose'])}`);
      comment();
      if (allowTable && maybe(0.3)) {
        const k = 1 + int(3);
        for (let r = 0; r < 1 + int(2); r++) L.push(rowOf(k, allowPh));
      }
    }
  };

  if (maybe(0.3)) L.push(`${pick(safeTags)}${maybe(0.3) ? ' ' + pick(semantic) : ''}`);
  L.push(`Feature: ${text(2)}`);
  if (maybe(0.4)) { L.push(`  As a ${text(1)}`); L.push(`  I want ${text(2)}`); }
  blank();
  if (maybe(0.35)) { L.push('  Background:'); steps(false, true); blank(); }
  // maybe(0.06): a Feature with ZERO scenarios — the no-scenarios dialect
  // error, whose message (near-miss hint included) is lint-parity-compared.
  const nScen = maybe(0.06) ? 0 : 1 + int(3);
  let lastOutlineTitle = null;
  if (nScen === 0 && maybe(0.5)) L.push(`  ${pick(['scenario: s', 'Scenario : s', 'SCENARIO OUTLINE: s', 'background :'])}`);
  for (let s = 0; s < nScen; s++) {
    comment();
    if (maybe(0.4)) {
      if (maybe(0.5)) tagLine();
      const oTitle = title();
      lastOutlineTitle = maybe(0.5) ? null : oTitle; // null when a placeholder follows
      L.push(`  Scenario Outline: ${oTitle}${lastOutlineTitle === null ? ' ' + ph() : ''}`);
      steps(true, true);
      L.push('    Examples:');
      comment();
      L.push(`      | ${phs.join(' | ')} |`);
      for (let r = 0; r < 1 + int(3); r++) L.push(rowOf(phs.length, false));
    } else {
      if (maybe(0.5)) tagLine();
      L.push(`  Scenario: ${title()}`);
      steps(maybe(0.2), true); // plain scenario: <n> is literal text, still legal
    }
    blank();
  }
  // maybe(0.35), when the last outline title had no placeholder: a plain
  // scenario whose title collides with an outline row's
  // expanded name — the duplicate-title post-expansion backstop.
  if (lastOutlineTitle && maybe(0.35)) {
    L.push(`  Scenario: ${lastOutlineTitle} [1]`);
    L.push(`    ${pick(kw)} ${text(3)}`);
    blank();
  }
  const eol = maybe(0.15) ? '\r\n' : '\n';
  return L.join(eol) + (maybe(0.85) ? eol : '');
}

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
    for (const o of p.outlines) {
      out.push(`OUTLINE\t${o.line}\t${o.rows}\t${o.headerLine}\t${esc(o.name)}`);
      out.push(`OHEADER\t${o.header.map(esc).join('\t')}`);
      out.push(`OPLACEHOLDERS\t${o.placeholders.map(esc).join('\t')}`);
    }
    for (const n of p.narrative) out.push(`NARRATIVE\t${n.line}\t${n.inBody}\t${esc(n.text)}`);
    return out.join('\n');
  } catch (e) {
    if (!(e instanceof GherkinSyntaxError)) return `NODE-CRASH\t${e.constructor.name}: ${e.message}`;
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
  const raw = gen();
  const file = join(DIR, `v${i}.feature`);
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
  } else if (diverged.length < 6) {
    diverged.push({ i, nd: (nd === rd ? 'lint>> ' + ndl : nd).slice(0, 500), rd: (nd === rd ? 'lint>> ' + rdl : rd).slice(0, 500) });
  }
}
console.log(`${identical}/${COUNT} identical (${accepts} accepted, ${rejects} rejected by both)`);
for (const d of diverged) console.log(`\n=== DIVERGENCE v${d.i}.feature\n--- node:\n${d.nd}\n--- rust:\n${d.rd}`);
process.exit(identical === COUNT ? 0 : 1);
