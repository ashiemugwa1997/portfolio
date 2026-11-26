// Brotli + gzip copies of every text asset in dist/, so the server sends them as they are
// instead of compressing the same bytes on every request.
import { readdirSync, readFileSync, statSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { brotliCompressSync, constants, gzipSync } from 'node:zlib'

const dir = fileURLToPath(new URL('../dist/', import.meta.url))
const text = /\.(html|js|css|svg|json|wasm|txt)$/

function* files(d) {
  for (const name of readdirSync(d)) {
    const p = join(d, name)
    if (statSync(p).isDirectory()) yield* files(p)
    else if (text.test(name)) yield p
  }
}

let raw = 0
let br = 0
for (const f of files(dir)) {
  const body = readFileSync(f)
  if (body.length < 512) continue
  const b = brotliCompressSync(body, { params: { [constants.BROTLI_PARAM_QUALITY]: 11, [constants.BROTLI_PARAM_SIZE_HINT]: body.length } })
  writeFileSync(`${f}.br`, b)
  writeFileSync(`${f}.gz`, gzipSync(body, { level: 9 }))
  raw += body.length
  br += b.length
}
console.log(`precompressed: ${(raw / 1024).toFixed(1)} KB -> ${(br / 1024).toFixed(1)} KB brotli`)
