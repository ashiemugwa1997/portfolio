import type { Hit, Profile } from './types'

// only links people can safely click
export const safeHref = (h: string) => /^(https?:\/\/|mailto:)/i.test(h.trim())

// ── Corpus ──────────────────────────────────────────────────────────────────

// flatten the profile into lines, each pointing at its section
export function buildCorpus(p: Profile): Hit[] {
  const hits: Hit[] = []
  const add = (text: string, section: string, anchor: string) =>
    hits.push({ text: text.replace(/\s+/g, ' ').trim(), section, anchor })

  p.metrics.forEach((m) => add(`${m.value} ${m.label}`, 'At a glance', 'glance'))
  p.pipeline.forEach((s) => add(`${s.step}: ${s.detail}`, 'Release pipeline', 'pipeline'))
  p.experience.forEach((j) => {
    add(`${j.title}, ${j.org}`, 'Experience', `job-${j.id}`)
    j.groups.forEach((g) => g.items.forEach((i) => add(i, g.name || j.org, `job-${j.id}`)))
  })
  p.cases.forEach((c) => add(`${c.title}: ${c.result}`, 'Case studies', `case-${c.id}`))
  p.skills.forEach((s) => add(`${s.area}: ${s.items.join(', ')}`, 'Skills', 'skills'))
  p.projects.forEach((pr) => add(`${pr.name}: ${pr.detail}`, 'Projects', 'projects'))
  p.education.forEach((e) => add(`${e.title}, ${e.org}`, 'Education', 'education'))
  return hits
}

export interface Searcher {
  engine: 'rust-wasm' | 'js'
  search(query: string, max?: number): number[]
  /** matched character positions of line `index`, ascending, for highlighting */
  positions(index: number, query: string): number[]
}

// a word's characters as a whole run if present, else the first place they occur in order;
// positions count characters (code points), like the Rust engine
function inOrder(line: string, terms: string[]): number[] {
  const chars = Array.from(line.toLowerCase())
  const sameLength = chars.length === Array.from(line).length
  const at = new Set<number>()
  for (const term of terms) {
    const t = Array.from(term)
    let sub = -1
    if (sameLength) {
      for (let i = 0; i + t.length <= chars.length && sub < 0; i++) {
        if (t.every((c, k) => chars[i + k] === c)) sub = i
      }
    }
    if (sub >= 0) {
      t.forEach((_, k) => at.add(sub + k))
      continue
    }
    let i = 0
    for (const c of t) {
      while (i < chars.length && chars[i] !== c) i++
      if (i === chars.length) return []
      at.add(i++)
    }
  }
  return [...at].sort((a, b) => a - b)
}

/** Split `text` into plain and matched runs for rendering. */
export function runs(text: string, positions: number[]): { text: string; hit: boolean }[] {
  const hit = new Set(positions)
  const out: { text: string; hit: boolean }[] = []
  Array.from(text).forEach((ch, i) => {
    const on = hit.has(i)
    const last = out[out.length - 1]
    if (last && last.hit === on) last.text += ch
    else out.push({ text: ch, hit: on })
  })
  return out
}

interface EngineExports {
  memory: WebAssembly.Memory
  alloc(len: number): number
  dealloc(ptr: number, len: number): void
  load(ptr: number, len: number): number
  search(qPtr: number, qLen: number, out: number, max: number): number
  positions(line: number, qPtr: number, qLen: number, out: number, max: number): number
}

// ── Plain search (fallback) ─────────────────────────────────────────────────
export function jsSearcher(lines: string[]): Searcher {
  const lower = lines.map((l) => l.toLowerCase())
  return {
    engine: 'js',
    search(query, max = 8) {
      const terms = query.toLowerCase().split(/\s+/).filter(Boolean)
      if (!terms.length) return []
      const out: number[] = []
      lower.forEach((l, i) => {
        if (out.length < max && terms.every((t) => l.includes(t))) out.push(i)
      })
      return out
    },
    positions(index, query) {
      const terms = query.toLowerCase().split(/\s+/).filter(Boolean)
      return index < lines.length && terms.length ? inOrder(lines[index], terms) : []
    },
  }
}

// ── Rust engine (wasm) ──────────────────────────────────────────────────────
export async function wasmSearcher(lines: string[], url = '/engine.wasm'): Promise<Searcher> {
  const res = await fetch(url)
  if (!res.ok) throw new Error(`engine.wasm: ${res.status}`)
  const { instance } = await WebAssembly.instantiate(await res.arrayBuffer(), {})
  const ex = instance.exports as unknown as EngineExports
  const enc = new TextEncoder()

  const withBytes = <T>(bytes: Uint8Array, fn: (ptr: number) => T): T => {
    const ptr = ex.alloc(bytes.length)
    new Uint8Array(ex.memory.buffer, ptr, bytes.length).set(bytes)
    try {
      return fn(ptr)
    } finally {
      ex.dealloc(ptr, bytes.length)
    }
  }

  // buildCorpus strips newlines so one entry per line is safe
  const corpus = enc.encode(lines.join('\n'))
  withBytes(corpus, (ptr) => ex.load(ptr, corpus.length))

  return {
    engine: 'rust-wasm',
    search(query, max = 8) {
      const q = enc.encode(query)
      if (!q.length) return []
      const out = ex.alloc(max * 4)
      try {
        const n = withBytes(q, (qp) => ex.search(qp, q.length, out, max))
        // read after the call, memory may have grown
        return Array.from(new Uint32Array(ex.memory.buffer, out, n))
      } finally {
        ex.dealloc(out, max * 4)
      }
    },
    positions(index, query) {
      const q = enc.encode(query)
      if (!q.length || index < 0 || index >= lines.length) return []
      const max = 256
      const out = ex.alloc(max * 4)
      try {
        const n = withBytes(q, (qp) => ex.positions(index, qp, q.length, out, max))
        return Array.from(new Uint32Array(ex.memory.buffer, out, n))
      } finally {
        ex.dealloc(out, max * 4)
      }
    },
  }
}
