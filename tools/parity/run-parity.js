// Differential parity harness: write the corpus to disk, dump each case with
// both parsers, diff byte-for-byte. Exit 1 on any divergence.
const { writeFileSync, mkdirSync } = require('node:fs');
const { execFileSync } = require('node:child_process');
const { join } = require('node:path');
const { cases } = require('./corpus');

const DIR = join(__dirname, 'corpus-out');
const RUST_DUMP = require('node:path').join(__dirname, '../../target/debug/examples/dump');
const NODE_DUMP = join(__dirname, 'dump-node.js');
mkdirSync(DIR, { recursive: true });

const run = (cmd, args) => {
  try {
    return execFileSync(cmd, args, { encoding: 'utf8' }).trimEnd();
  } catch (e) {
    return `HARNESS-ERROR\t${(e.stderr || e.message).toString().split('\n')[0]}`;
  }
};

let same = 0;
const diverged = [];
for (const [name, text] of Object.entries(cases)) {
  const file = join(DIR, `${name}.feature`);
  writeFileSync(file, text);
  const node = run('node', [NODE_DUMP, file]);
  const rust = run(RUST_DUMP, [file]);
  if (node === rust) { same += 1; continue; }
  diverged.push({ name, node, rust });
}

console.log(`${same}/${Object.keys(cases).length} cases identical`);
for (const d of diverged) {
  console.log(`\n=== DIVERGENCE: ${d.name}`);
  const n = d.node.split('\n'); const r = d.rust.split('\n');
  for (let i = 0; i < Math.max(n.length, r.length); i++) {
    if (n[i] !== r[i]) {
      console.log(`  node: ${n[i] ?? '<absent>'}`);
      console.log(`  rust: ${r[i] ?? '<absent>'}`);
    }
  }
}
process.exit(diverged.length ? 1 : 0);
