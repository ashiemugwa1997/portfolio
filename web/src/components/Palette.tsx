import { useEffect, useMemo, useRef, useState } from 'react'
import type { Hit } from '../types'
import type { Searcher } from '../search'

interface Props {
  hits: Hit[]
  searcher: Searcher
  onClose: () => void
}

// ctrl+k search, ranked by the rust engine
export default function Palette({ hits, searcher, onClose }: Props) {
  const [query, setQuery] = useState('')
  const [cursor, setCursor] = useState(0)
  const input = useRef<HTMLInputElement>(null)

  const results = useMemo(() => searcher.search(query, 8).map((i) => hits[i]).filter(Boolean), [query, hits, searcher])

  useEffect(() => input.current?.focus(), [])
  useEffect(() => setCursor(0), [query])

  // ── Helpers ────────────────────────────────────────────────────────────

  const go = (hit: Hit) => {
    onClose()
    const el = document.getElementById(hit.anchor)
    el?.scrollIntoView({ behavior: 'smooth', block: 'start' })
    el?.classList.add('flash')
    window.setTimeout(() => el?.classList.remove('flash'), 1400)
  }

  const onKey = (e: React.KeyboardEvent) => {
    if (e.key === 'Escape') onClose()
    else if (e.key === 'ArrowDown') {
      e.preventDefault()
      setCursor((c) => Math.min(c + 1, results.length - 1))
    } else if (e.key === 'ArrowUp') {
      e.preventDefault()
      setCursor((c) => Math.max(c - 1, 0))
    } else if (e.key === 'Enter' && results[cursor]) go(results[cursor])
  }

  return (
    <div className="scrim" onMouseDown={onClose}>
      <div
        className="palette"
        role="dialog"
        aria-modal="true"
        aria-label="Search the portfolio"
        onMouseDown={(e) => e.stopPropagation()}
        onKeyDown={onKey}
      >
        <input
          ref={input}
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="Try “rollback”, “sso” or “react native”"
          aria-label="Search"
          aria-controls="palette-results"
          aria-activedescendant={results[cursor] ? `hit-${cursor}` : undefined}
        />
        <ul id="palette-results" role="listbox">
          {results.map((h, i) => (
            <li
              key={`${h.anchor}-${i}`}
              id={`hit-${i}`}
              role="option"
              aria-selected={i === cursor}
              onMouseEnter={() => setCursor(i)}
              onClick={() => go(h)}
            >
              <span className="mono where">{h.section}</span>
              <span>{h.text}</span>
            </li>
          ))}
          {query.trim() && results.length === 0 && <li className="empty">Nothing matches “{query}”.</li>}
        </ul>
        <p className="mono engine">
          ↑↓ to move · Enter to go · Esc to close
          <span>{searcher.engine === 'rust-wasm' ? 'ranked by Rust · wasm' : 'plain search'}</span>
        </p>
      </div>
    </div>
  )
}
