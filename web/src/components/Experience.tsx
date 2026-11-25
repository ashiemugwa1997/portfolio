import { useState } from 'react'
import type { Job } from '../types'

export default function Experience({ jobs }: { jobs: Job[] }) {
  return (
    <ol className="timeline">
      {jobs.map((j, i) => (
        <JobItem key={j.id} job={j} open={i === 0} />
      ))}
    </ol>
  )
}

function JobItem({ job, open }: { job: Job; open: boolean }) {
  const named = job.groups.filter((g) => g.name)
  const [active, setActive] = useState(0)
  const loose = job.groups.filter((g) => !g.name).flatMap((g) => g.items)

  return (
    <li id={`job-${job.id}`} className={open ? 'job current' : 'job'}>
      <div className="job-head">
        <h3>
          {job.title} <span className="org">· {job.org}</span>
        </h3>
        <span className="mono period">{job.period}</span>
      </div>
      <p className="job-intro">{job.intro}</p>
      {loose.length > 0 && (
        <ul className="bullets">
          {loose.map((t) => (
            <li key={t}>{t}</li>
          ))}
        </ul>
      )}
      {named.length > 0 && (
        <div className="tabs">
          <div role="tablist" aria-label={`${job.org} work areas`}>
            {named.map((g, i) => (
              <button
                key={g.name}
                role="tab"
                id={`tab-${job.id}-${i}`}
                aria-selected={i === active}
                aria-controls={`panel-${job.id}`}
                onClick={() => setActive(i)}
              >
                {g.name}
              </button>
            ))}
          </div>
          <ul className="bullets" role="tabpanel" id={`panel-${job.id}`} aria-labelledby={`tab-${job.id}-${active}`}>
            {named[active].items.map((t) => (
              <li key={t}>{t}</li>
            ))}
          </ul>
        </div>
      )}
    </li>
  )
}
