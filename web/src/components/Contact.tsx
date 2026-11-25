import { useState } from 'react'

type Status = { kind: 'idle' | 'sending' | 'sent' } | { kind: 'error'; message: string }

export default function Contact() {
  const [status, setStatus] = useState<Status>({ kind: 'idle' })

  const submit = async (e: React.FormEvent<HTMLFormElement>) => {
    e.preventDefault()
    const form = e.currentTarget
    const data = Object.fromEntries(new FormData(form)) as Record<string, string>
    setStatus({ kind: 'sending' })
    try {
      const res = await fetch('/api/contact', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ name: data.name, email: data.email, message: data.message, website: data.website }),
      })
      if (res.ok) {
        form.reset()
        setStatus({ kind: 'sent' })
        return
      }
      const body = await res.json().catch(() => ({}))
      setStatus({ kind: 'error', message: body.error ?? 'Something went wrong. Please email me instead.' })
    } catch {
      setStatus({ kind: 'error', message: 'Could not reach the server. Please email me instead.' })
    }
  }

  if (status.kind === 'sent') {
    return (
      <p className="sent" role="status">
        Thanks, your message is in. I'll reply by email.
      </p>
    )
  }

  return (
    <form className="contact" onSubmit={submit} noValidate={false}>
      <label>
        Name
        <input name="name" required maxLength={100} autoComplete="name" />
      </label>
      <label>
        Email
        <input name="email" type="email" required maxLength={200} autoComplete="email" />
      </label>
      <label className="wide">
        Message
        <textarea name="message" required maxLength={4000} rows={5} />
      </label>
      {/* honeypot, bots fill it */}
      <label className="trap" aria-hidden="true">
        Website
        <input name="website" tabIndex={-1} autoComplete="off" />
      </label>
      <div className="wide actions">
        <button type="submit" disabled={status.kind === 'sending'}>
          {status.kind === 'sending' ? 'Sending…' : 'Send message'}
        </button>
        {status.kind === 'error' && (
          <p className="error" role="alert">
            {status.message}
          </p>
        )}
      </div>
    </form>
  )
}
