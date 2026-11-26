// fuzzrank (Rust) compiled to WebAssembly: fuzzy search for command palettes and filters.

const enc = new TextEncoder()

// ── Loading ─────────────────────────────────────────────────────────────────

async function instantiate(source) {
  if (source instanceof WebAssembly.Module) return WebAssembly.instantiate(source, {})
  if (source instanceof ArrayBuffer || ArrayBuffer.isView(source)) {
    return (await WebAssembly.instantiate(source, {})).instance
  }
  const url = source ?? new URL('./fuzzrank.wasm', import.meta.url)
  if (typeof process !== 'undefined' && process.versions?.node && String(url).startsWith('file:')) {
    const { readFile } = await import('node:fs/promises')
    return (await WebAssembly.instantiate(await readFile(url), {})).instance
  }
  const res = await fetch(url)
  if (!res.ok) throw new Error(`fuzzrank.wasm: HTTP ${res.status}`)
  return (await WebAssembly.instantiate(await res.arrayBuffer(), {})).instance
}

// ── Index ───────────────────────────────────────────────────────────────────

/**
 * Build a search index over `lines`.
 * @param {string[]} lines
 * @param {{ wasm?: string | URL | ArrayBuffer | ArrayBufferView | WebAssembly.Module }} [options]
 */
export async function createIndex(lines, options = {}) {
  const instance = await instantiate(options.wasm)
  const ex = instance.exports

  const withBytes = (bytes, fn) => {
    const ptr = ex.alloc(bytes.length)
    new Uint8Array(ex.memory.buffer, ptr, bytes.length).set(bytes)
    try {
      return fn(ptr)
    } finally {
      ex.dealloc(ptr, bytes.length)
    }
  }

  const withOut = (max, fn) => {
    const out = ex.alloc(max * 4)
    try {
      const n = fn(out)
      // read after the call: memory may have grown
      return Array.from(new Uint32Array(ex.memory.buffer, out, n))
    } finally {
      ex.dealloc(out, max * 4)
    }
  }

  const corpus = enc.encode(lines.map((l) => String(l).replace(/[\r\n]+/g, ' ')).join('\n'))
  withBytes(corpus, (ptr) => ex.load(ptr, corpus.length))
  const size = lines.length

  return {
    size,
    /** Indexes of the best `limit` lines, best first. */
    search(query, limit = 10) {
      const q = enc.encode(query)
      if (!q.length || limit <= 0) return []
      return withOut(limit, (out) => withBytes(q, (qp) => ex.search(qp, q.length, out, limit)))
    },
    /** Matched character positions of line `index` (UTF-16 safe for the BMP), ascending. */
    positions(index, query) {
      const q = enc.encode(query)
      if (!q.length || index < 0 || index >= size) return []
      return withOut(256, (out) => withBytes(q, (qp) => ex.positions(index, qp, q.length, out, 256)))
    },
  }
}

/**
 * Wrap the matched characters of `text` in <mark>, escaping everything else.
 * @param {string} text
 * @param {number[]} positions character positions from `positions()`
 */
export function highlight(text, positions) {
  const hit = new Set(positions)
  let html = ''
  let open = false
  let i = 0
  for (const ch of text) {
    const on = hit.has(i++)
    if (on !== open) {
      html += on ? '<mark>' : '</mark>'
      open = on
    }
    html += ch.replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`)
  }
  return open ? html + '</mark>' : html
}
