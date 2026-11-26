export interface FuzzIndex {
  /** Number of lines indexed. */
  readonly size: number
  /** Indexes of the best `limit` lines for `query`, best first. Every word must match. */
  search(query: string, limit?: number): number[]
  /** Matched character positions of line `index`, ascending; empty when it does not match. */
  positions(index: number, query: string): number[]
}

export interface Options {
  /** Where to load fuzzrank.wasm from; defaults to the file next to this module. */
  wasm?: string | URL | ArrayBuffer | ArrayBufferView | WebAssembly.Module
}

/** Build a search index over `lines`. */
export function createIndex(lines: readonly string[], options?: Options): Promise<FuzzIndex>

/** Wrap the matched characters of `text` in <mark>, HTML-escaping everything else. */
export function highlight(text: string, positions: readonly number[]): string
