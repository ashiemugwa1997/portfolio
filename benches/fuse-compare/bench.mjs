// node bench.mjs [corpus.txt]
// Top-10 search with fuzzrank-wasm and Fuse.js over the same list, each used as its docs suggest.
import { readdirSync, readFileSync, statSync } from 'node:fs'
import { homedir } from 'node:os'
import { join } from 'node:path'
import Fuse from 'fuse.js'
import { createIndex } from '../../npm/fuzzrank-wasm/index.js'

const QUERIES = ['lib', 'srcmod', 'tcpstream', 'jsonvalue', 'rtime', 'hyperbody', 'zzqx', 'd']
const RUNS = 10

function corpus() {
  if (process.argv[2]) return readFileSync(process.argv[2], 'utf8').split('\n').filter(Boolean)
  const root = join(process.env.CARGO_HOME ?? join(homedir(), '.cargo'), 'registry', 'src')
  const out = []
  const walk = (d) => {
    for (const name of readdirSync(d)) {
      const p = join(d, name)
      if (statSync(p).isDirectory()) walk(p)
      else if (name.endsWith('.rs')) out.push(p.slice(root.length + 1).replace(/\\/g, '/').split('/').slice(1).join('/'))
    }
  }
  walk(root)
  return out.sort()
}

function time(fn) {
  fn()
  const t = performance.now()
  for (let i = 0; i < RUNS; i++) fn()
  return (performance.now() - t) / RUNS
}

const lines = corpus()
console.log(`corpus: ${lines.length} file paths\n`)

let t = performance.now()
const fz = await createIndex(lines)
const fzBuild = performance.now() - t
t = performance.now()
const fuse = new Fuse(lines, { includeScore: true, ignoreLocation: true, threshold: 0.4 })
const fuseBuild = performance.now() - t
console.log(`build: fuzzrank-wasm ${fzBuild.toFixed(1)} ms (incl. loading wasm), Fuse.js ${fuseBuild.toFixed(1)} ms\n`)

const pad = (s, n) => String(s).padStart(n)
console.log(`${'query'.padEnd(11)} ${pad('fuzzrank-wasm', 14)} ${pad('Fuse.js', 12)}   speed-up`)
let a = 0
let b = 0
for (const q of QUERIES) {
  const tf = time(() => fz.search(q, 10))
  const tu = time(() => fuse.search(q, { limit: 10 }))
  a += tf
  b += tu
  console.log(`${q.padEnd(11)} ${pad(tf.toFixed(2) + ' ms', 14)} ${pad(tu.toFixed(2) + ' ms', 12)}   ${(tu / tf).toFixed(0)}x`)
}
console.log(`${'total'.padEnd(11)} ${pad(a.toFixed(2) + ' ms', 14)} ${pad(b.toFixed(2) + ' ms', 12)}   ${(b / a).toFixed(0)}x`)
console.log('\nsample results for "tcpstream":')
console.log('  fuzzrank:', fz.search('tcpstream', 3).map((i) => lines[i]))
console.log('  Fuse.js: ', fuse.search('tcpstream', { limit: 3 }).map((r) => r.item))
