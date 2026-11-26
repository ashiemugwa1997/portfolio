export interface Link { label: string; href: string; text: string }
export interface Metric { value: string; label: string }
export interface Stage { step: string; detail: string }
export interface Group { name: string; items: string[] }
export interface Job { id: string; title: string; org: string; period: string; intro: string; groups: Group[] }
export interface Case { id: string; title: string; context: string; action: string; result: string }
export interface SkillArea { area: string; items: string[] }
export interface Project { name: string; detail: string; href?: string }
export interface Education { title: string; org: string; year: string }

export interface Profile {
  name: string
  role: string
  tagline: string
  location: string
  summary: string[]
  links: Link[]
  metrics: Metric[]
  pipeline: Stage[]
  experience: Job[]
  cases: Case[]
  skills: SkillArea[]
  projects: Project[]
  education: Education[]
  credentials: string[]
}

/** One line the command palette can find, and where it leads. */
export interface Hit { text: string; section: string; anchor: string }
