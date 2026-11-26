import { afterEach, describe, expect, it, vi } from 'vitest'
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import App from './App'
import { buildCorpus, jsSearcher, safeHref } from './search'
import type { Profile } from './types'
import profileJson from '../../server/content/profile.json'

const profile = profileJson as Profile

function mockFetch(handler: (url: string, init?: RequestInit) => Response | Promise<Response>) {
  const fn = vi.fn((input: RequestInfo | URL, init?: RequestInit) => Promise.resolve(handler(String(input), init)))
  vi.stubGlobal('fetch', fn)
  return fn
}

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } })

afterEach(() => {
  cleanup()
  vi.unstubAllGlobals()
})

describe('search corpus', () => {
  it('covers every section and points each line at a real anchor', () => {
    const hits = buildCorpus(profile)
    const anchors = new Set(hits.map((h) => h.anchor))
    for (const a of ['glance', 'pipeline', 'skills', 'projects', 'education']) expect(anchors).toContain(a)
    profile.experience.forEach((j) => expect(anchors).toContain(`job-${j.id}`))
    expect(hits.every((h) => !h.text.includes('\n'))).toBe(true)
  })

  it('only safe links are clickable', () => {
    expect(['https://x.co', 'mailto:a@b.co', ' HTTP://x.co'].every(safeHref)).toBe(true)
    expect(['javascript:alert(1)', 'data:text/html,x', '//evil.co', ' JavaScript:x'].some(safeHref)).toBe(false)
  })

  it('plain search needs every word', () => {
    const s = jsSearcher(['Deployment pipeline with rollback', 'SAML single sign-on'])
    expect(s.search('pipeline ROLLBACK')).toEqual([0])
    expect(s.search('saml rollback')).toEqual([])
    expect(s.search('  ')).toEqual([])
  })
})

describe('App', () => {
  it('renders the profile from the API', async () => {
    mockFetch((url) => (url === '/api/profile' ? json(profile) : new Response('', { status: 404 })))
    render(<App />)
    expect(await screen.findByRole('heading', { level: 1 })).toHaveProperty('textContent', 'AshleyMugwambi')
    expect(screen.getByText('22 → 8 min')).toBeTruthy()
    expect(screen.getByRole('heading', { name: 'Safer, faster releases' })).toBeTruthy()
  })

  it('says so when the profile cannot be loaded', async () => {
    mockFetch(() => new Response('', { status: 500 }))
    render(<App />)
    expect(await screen.findByText(/could not be loaded/)).toBeTruthy()
  })

  it('switches experience tabs', async () => {
    mockFetch((url) => (url === '/api/profile' ? json(profile) : new Response('', { status: 404 })))
    render(<App />)
    const tab = await screen.findByRole('tab', { name: 'Security and performance' })
    fireEvent.click(tab)
    expect(tab.getAttribute('aria-selected')).toBe('true')
    expect(screen.getByText(/EY penetration test, closing/)).toBeTruthy()
  })

  it('opens search with Ctrl+K and finds a line (plain search when wasm is absent)', async () => {
    mockFetch((url) => (url === '/api/profile' ? json(profile) : new Response('', { status: 404 })))
    render(<App />)
    await screen.findByRole('heading', { level: 1 })
    fireEvent.keyDown(window, { key: 'k', ctrlKey: true })
    const box = await screen.findByRole('textbox', { name: 'Search' })
    fireEvent.change(box, { target: { value: 'saml' } })
    const options = screen.getAllByRole('option')
    expect(options[0].textContent).toMatch(/SAML single sign-on/)
    fireEvent.keyDown(box, { key: 'Escape' })
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull())
  })

  it('sends the contact form and shows the server error when there is one', async () => {
    let calls = 0
    const fetchFn = mockFetch((url) => {
      if (url === '/api/profile') return json(profile)
      if (url === '/api/contact') {
        calls++
        return calls === 1 ? json({ error: 'Please check your email address.' }, 422) : json({ status: 'received' }, 202)
      }
      return new Response('', { status: 404 })
    })
    render(<App />)
    await screen.findByRole('heading', { level: 1 })
    fireEvent.change(screen.getByLabelText('Name'), { target: { value: 'Rudo' } })
    fireEvent.change(screen.getByLabelText('Email'), { target: { value: 'rudo@example.com' } })
    fireEvent.change(screen.getByLabelText('Message'), { target: { value: 'Hello' } })
    fireEvent.click(screen.getByRole('button', { name: 'Send message' }))
    expect((await screen.findByRole('alert')).textContent).toBe('Please check your email address.')

    fireEvent.click(screen.getByRole('button', { name: 'Send message' }))
    expect(await screen.findByRole('status')).toBeTruthy()
    const body = JSON.parse(String(fetchFn.mock.calls.at(-1)![1]!.body))
    expect(body).toMatchObject({ name: 'Rudo', email: 'rudo@example.com', message: 'Hello', website: '' })
  })
})
