import { useEffect, useMemo, useState } from 'react'
import type { Profile } from './types'
import { buildCorpus, jsSearcher, safeHref, wasmSearcher, type Searcher } from './search'
import Palette from './components/Palette'
import Contact from './components/Contact'
import Experience from './components/Experience'

const NAV = [
  ['glance', 'Impact'],
  ['experience', 'Experience'],
  ['cases', 'Case studies'],
  ['skills', 'Skills'],
  ['contact', 'Contact'],
] as const

export default function App() {
  const [profile, setProfile] = useState<Profile | null>(null)
  const [failed, setFailed] = useState(false)
  const [paletteOpen, setPaletteOpen] = useState(false)

  // ── Load profile ──────────────────────────────────────────────────────

  useEffect(() => {
    const ctrl = new AbortController()
    fetch('/api/profile', { signal: ctrl.signal })
      .then((r) => (r.ok ? r.json() : Promise.reject(r.status)))
      .then(setProfile)
      .catch((e) => e?.name !== 'AbortError' && setFailed(true))
    return () => ctrl.abort()
  }, [])

  // ── Search engine ─────────────────────────────────────────────────────

  const corpus = useMemo(() => (profile ? buildCorpus(profile) : []), [profile])
  const [searcher, setSearcher] = useState<Searcher | null>(null)
  useEffect(() => {
    if (!corpus.length) return
    const lines = corpus.map((h) => h.text)
    setSearcher(jsSearcher(lines))
    let live = true
    wasmSearcher(lines)
      .then((s) => live && setSearcher(s))
      .catch(() => {}) // keep plain search
    return () => {
      live = false
    }
  }, [corpus])

  // ── Ctrl+K ────────────────────────────────────────────────────────────

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'k') {
        e.preventDefault()
        setPaletteOpen((o) => !o)
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])

  if (failed) {
    return (
      <main className="state">
        <p>The profile could not be loaded. Please refresh in a moment.</p>
      </main>
    )
  }
  if (!profile) {
    return (
      <main className="state" aria-busy="true">
        <p className="mono">loading profile…</p>
      </main>
    )
  }

  const [first, ...rest] = profile.name.split(' ')

  return (
    <>
      <header className="bar">
        <a className="mark" href="#top" aria-label="Back to top">
          AM<span className="dot" />
        </a>
        <nav aria-label="Sections">
          {NAV.map(([id, label]) => (
            <a key={id} href={`#${id}`}>
              {label}
            </a>
          ))}
        </nav>
        <button className="search-btn" onClick={() => setPaletteOpen(true)}>
          <span>Search</span>
          <kbd>Ctrl K</kbd>
        </button>
      </header>

      <main id="top">
        <section className="hero">
          <div className="hero-copy">
            <p className="eyebrow mono">
              {profile.role} · {profile.location}
            </p>
            <h1>
              {first}
              <br />
              <span className="accent">{rest.join(' ')}</span>
            </h1>
            <p className="tagline">{profile.tagline}</p>
            {profile.summary.map((s) => (
              <p key={s.slice(0, 24)} className="summary">
                {s}
              </p>
            ))}
            <ul className="links">
              {profile.links.filter((l) => safeHref(l.href)).map((l) => (
                <li key={l.label}>
                  <a href={l.href} target={l.href.startsWith('http') ? '_blank' : undefined} rel="noopener noreferrer">
                    <span className="mono">{l.label}</span> {l.text}
                  </a>
                </li>
              ))}
            </ul>
          </div>
          <Grid />
        </section>

        <section id="glance" className="section">
          <SectionHead index="01" title="At a glance" />
          <div className="metrics">
            {profile.metrics.map((m) => (
              <div key={m.value} className="metric">
                <strong className="mono">{m.value}</strong>
                <span>{m.label}</span>
              </div>
            ))}
          </div>
        </section>

        <section id="pipeline" className="section">
          <SectionHead index="02" title="How I ship" note="The release pipeline I rebuilt for a platform 8,000+ people rely on." />
          <ol className="pipeline">
            {profile.pipeline.map((s, i) => (
              <li key={s.step} style={{ ['--i' as string]: i }}>
                <span className="node" aria-hidden="true" />
                <strong>{s.step}</strong>
                <span>{s.detail}</span>
              </li>
            ))}
          </ol>
        </section>

        <section id="experience" className="section">
          <SectionHead index="03" title="Experience" />
          <Experience jobs={profile.experience} />
        </section>

        <section id="cases" className="section">
          <SectionHead index="04" title="Case studies" />
          <div className="cases">
            {profile.cases.map((c) => (
              <article key={c.id} id={`case-${c.id}`} className="case">
                <h3>{c.title}</h3>
                <dl>
                  <dt>Context</dt>
                  <dd>{c.context}</dd>
                  <dt>Action</dt>
                  <dd>{c.action}</dd>
                  <dt>Result</dt>
                  <dd className="result">{c.result}</dd>
                </dl>
              </article>
            ))}
          </div>
        </section>

        <section id="skills" className="section">
          <SectionHead index="05" title="Skills" />
          <div className="skills">
            {profile.skills.map((s) => (
              <div key={s.area} className="skill-row">
                <h3 className="mono">{s.area}</h3>
                <ul>
                  {s.items.map((i) => (
                    <li key={i}>{i}</li>
                  ))}
                </ul>
              </div>
            ))}
          </div>
        </section>

        <section id="projects" className="section">
          <SectionHead index="06" title="Projects" />
          <ul className="projects">
            {profile.projects.map((p) => (
              <li key={p.name}>
                <strong>{p.name}</strong>
                <span>{p.detail}</span>
              </li>
            ))}
          </ul>
        </section>

        <section id="education" className="section two-col">
          <div>
            <SectionHead index="07" title="Education" />
            <ul className="plain">
              {profile.education.map((e) => (
                <li key={e.title}>
                  <strong>{e.title}</strong>
                  <span>
                    {e.org}
                    {e.year && ` · ${e.year}`}
                  </span>
                </li>
              ))}
            </ul>
          </div>
          <div>
            <SectionHead index="08" title="Credentials" />
            <ul className="plain">
              {profile.credentials.map((c) => (
                <li key={c}>
                  <strong>{c}</strong>
                </li>
              ))}
            </ul>
          </div>
        </section>

        <section id="contact" className="section">
          <SectionHead index="09" title="Get in touch" note="References available on request." />
          <Contact />
        </section>
      </main>

      <footer className="foot mono">
        <span>© {new Date().getFullYear()} {profile.name}</span>
        <span>
          React front end · Go API · Rust search engine in WebAssembly
          {searcher && <em> (search running on {searcher.engine === 'rust-wasm' ? 'Rust/wasm' : 'plain JS'})</em>}
        </span>
      </footer>

      {paletteOpen && searcher && <Palette hits={corpus} searcher={searcher} onClose={() => setPaletteOpen(false)} />}
    </>
  )
}

// ── Helpers ──────────────────────────────────────────────────────────────

function SectionHead({ index, title, note }: { index: string; title: string; note?: string }) {
  return (
    <div className="section-head">
      <span className="mono index">{index}</span>
      <h2>{title}</h2>
      {note && <p>{note}</p>}
    </div>
  )
}

// grid diagram, pulse running through the network
function Grid() {
  const nodes = [
    [40, 60], [150, 30], [260, 80], [90, 170], [210, 190], [310, 160], [140, 280], [270, 290],
  ]
  const edges = [[0, 1], [1, 2], [0, 3], [1, 4], [2, 5], [3, 4], [4, 5], [3, 6], [4, 7], [6, 7], [5, 7]]
  return (
    <svg className="grid" viewBox="0 0 350 330" role="img" aria-label="Network diagram">
      {edges.map(([a, b], i) => (
        <line key={i} x1={nodes[a][0]} y1={nodes[a][1]} x2={nodes[b][0]} y2={nodes[b][1]} className="edge" />
      ))}
      <path className="pulse" d={`M${nodes[0]} L${nodes[1]} L${nodes[4]} L${nodes[7]}`} />
      {nodes.map(([x, y], i) => (
        <circle key={i} cx={x} cy={y} r={i === 4 ? 9 : 5.5} className={i === 4 ? 'hub' : 'node'} />
      ))}
    </svg>
  )
}
