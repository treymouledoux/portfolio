export type Project = {
  title: string
  description: string
  image: string
  imageAlt: string
  tags: string[]
  live?: string
  listing?: { label: string; href: string }
  github: string
  featured?: boolean
}

export type Experience = {
  period: string
  role: string
  company: string
  description: string
}

export const profile = {
  name: 'Trey Mouledoux',
  role: 'Software developer',
  tagline: 'Systems, small experiments, and software I actually use.',
  bio: 'I work mostly in Rust and Python, with projects that range from keyboard lighting to programming languages. I like understanding how something works, taking it apart, and making it do something useful.',
  email: 'trey.mouledoux@icloud.com',
  location: 'Nashville, Tennessee',
  linkedin: 'https://www.linkedin.com/in/trey-mouledoux/',
}

export const skills = [
  { label: 'Languages', items: 'Rust, Python, TypeScript, C++, Go, Nim, Lua' },
  { label: 'Tools & systems', items: 'Linux, Git, CLI tools, self-hosted services' },
  { label: 'Interests', items: 'Systems programming, security, automation, language design' },
]

export const projects: Project[] = [
  {
    title: 'Wootili-View',
    description: 'Screen-reactive lighting for Wooting analog keyboards. Written in Rust, using the Wooting SDK to turn what is on screen into lighting across the keys.',
    image: '/projects/wootili-view.jpeg',
    imageAlt: 'Wooting keyboard running Wootili-View screen-reactive RGB lighting',
    tags: ['Rust', 'Hardware', 'Wooting SDK'],
    listing: { label: 'Featured in Awesome Wooting', href: 'https://github.com/WootingKb/awesome-wooting#software' },
    github: 'https://github.com/treymouledoux/wootili-view',
    featured: true,
  },
  {
    title: 'Grid9',
    description: 'An esoteric programming language built around a 3×3 grid of bits. Rebuilt in Rust, with a command-line interpreter and support for compound conditions.',
    image: '/projects/grid9.png',
    imageAlt: 'Grid9 programming language project banner',
    tags: ['Rust', 'Interpreter', 'Language design'],
    live: 'https://treymouledoux.github.io/Grid9/',
    github: 'https://github.com/treymouledoux/Grid9',
  },
  {
    title: 'Regy Bot',
    description: 'A Rust moderation bot for Discord. Compiled regular-expression caching, logging, and a modular design keep the moderation rules manageable as they grow.',
    image: '/projects/regy-bot.png',
    imageAlt: 'Regy Bot Discord moderation project banner',
    tags: ['Rust', 'Discord', 'Regex'],
    github: 'https://github.com/treymouledoux/Regy-Bot',
  },
]

export const experience: Experience[] = [
  {
    period: '2026 — Present',
    role: 'Software Engineer, AI Training',
    company: 'DataAnnotation · Contract, remote',
    description: 'Writing and debugging code, evaluating agentic coding sessions, and explaining where generated solutions fail. The work calls for checking what the code actually does, not just whether the explanation sounds right.',
  },
  {
    period: '2022 — 2026',
    role: 'Code Sensei',
    company: 'Code Ninjas · Franklin, TN',
    description: 'Taught JavaScript, Lua, and introductory AI concepts to young students. I went through the program myself before returning as an instructor, and spent four years learning how to explain technical ideas clearly.',
  },
  {
    period: '2023 — 2025',
    role: 'CyberPatriot Competitor',
    company: 'National Youth Cyber Education Program',
    description: 'Secured Windows, Linux, and Windows Server systems in timed team competitions. Three years of finding vulnerabilities, coordinating fixes, and learning to troubleshoot under pressure.',
  },
]

export const socials = [
  { label: 'GitHub', href: 'https://github.com/treymouledoux' },
  { label: 'LinkedIn', href: profile.linkedin },
  { label: 'Instagram', href: 'https://www.instagram.com/treymouledoux/' },
]

export const navItems = [
  { label: 'Work', href: '#projects' },
  { label: 'About', href: '#about' },
  { label: 'Experience', href: '#experience' },
  { label: 'Contact', href: '#contact' },
]
