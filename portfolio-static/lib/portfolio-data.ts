export type Project = {
  title: string
  description: string
  image: string
  tags: string[]
  live: string
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
  role: 'Software Developer',
  tagline: 'I love building software that makes a difference.',
  bio: 'I\'m a software developer based in TN. I specialize in Rust, Python, and JavaScript, and I have a passion for creating innovative solutions that solve real-world problems. When I\'m not coding, you can find me exploring the outdoors.',
  email: 'temp@temp.com',
  location: 'Nashville, TN',
}

export const skills = ['Programming', 'Cyber Security', 'Rust', 'Python', 'Linux', 'Homelabbing']

// TODO: Add more projects and experience as needed
export const projects: Project[] = [
  {
    title: 'Wootili-View',
    description: 'Keyboard dynamic lighting RGB intergration for Wooting analog keyboards.',
    image: 'https://image.shutterstock.com/image-photo/closeup-analog-keyboard-keys-260nw-1921681870.jpg',
    tags: ['Hardware', 'Rust'],
    live: 'https://treymouledoux.github.io/Wootili-View/',
    github: 'https://github.com/treymouledoux/wootili-view',
    featured: true,
  },
  {
    title: 'Grid9',
    description: 'An esoteric programming language developed by me which gives users a limitation of 9 bits of memory.',
    image: 'https://github.com/treymouledoux/Grid9/blob/main/.github/assets/banner.png?raw=true',
    tags: ['Nim', 'Programming Language'],
    live: 'https://treymouledoux.github.io/Grid9/',
    github: 'https://github.com/treymouledoux/Grid9',
  },
  {
    title: 'Regy Bot',
    description: 'A lightweight and effienct regex moderation bot for discord built in rust which features advanced logging, smart compiled regex caches and registry, and a modular design with extension in mind.',
    image: 'https://github.com/treymouledoux/Regy-Bot/blob/master/.github/assets/regy_banner.png?raw=true',
    tags: ['Rust', 'Real deployment', 'Discord Bot', 'Regex'],
    live: 'https://treymouledoux.github.io/Regy-Bot/',
    github: 'https://github.com/treymouledoux/Regy-Bot',
  },
]

export const experience: Experience[] = [
  { period: '2022 — 2026', role: 'Code Sensei', company: 'Code Ninjas', description: 'Teaching and mentoring young developers in JavaScript, Lua, and AI. Worked a total of 4 years in this role throughout high school. Attended the program before working as a code sensei.' },
  { period: '2023 — 2025', role: 'CyberPatriot Competitor', company: 'National Youth Cyber Education Program (CyberPatriot)', description: 'Participated in 3 years of competitive cybersecurity events provided by the CyberPatriot program, teams are assigned to lock down windows, linux, and windows server instances and are graded accordingly.' },
  { period: '2026 — 2030', role: 'Undergraduate Student', company: 'Middle Tennessee State University', description: 'Learning and growing as a software developer.' },
]

export const socials = [
  { label: 'GitHub', href: 'https://github.com/treymouledoux' },
  { label: 'LinkedIn', href: 'https://www.linkedin.com/in/trey-mouledoux-77959a42b/' },
  { label: 'Instagram', href: 'https://www.instagram.com/treymouledoux/' },
]

export const navItems = [
  { label: 'About', href: '#about' },
  { label: 'Projects', href: '#projects' },
  { label: 'Experience', href: '#experience' },
  { label: 'Contact', href: '#contact' },
]
