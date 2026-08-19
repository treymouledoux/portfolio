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

export const skills = ['Programming', 'Cyber Security', 'Rust', 'Python', 'Linux', 'Homelabing']

// TODO: Add more projects and experience as needed
export const projects: Project[] = [
  {
    title: 'Arc / Financial clarity',
    description: 'A calmer way to understand your money, designed for the next generation of builders.',
    image: 'https://images.unsplash.com/photo-1551288049-bebda4e38f71?auto=format&fit=crop&w=1200&q=85',
    tags: ['Product', 'Brand', 'Web app'],
    live: '#',
    github: '#',
    featured: true,
  },
  {
    title: 'Morrow Objects',
    description: 'An editorial commerce experience for objects made to last.',
    image: 'https://images.unsplash.com/photo-1494438639946-1ebd1d20bf85?auto=format&fit=crop&w=1200&q=85',
    tags: ['E-commerce', 'Art direction'],
    live: '#',
    github: '#',
  },
  {
    title: 'Field Notes',
    description: 'A lightweight research tool that helps teams make better decisions together.',
    image: 'https://images.unsplash.com/photo-1456324504439-367cee3b3c32?auto=format&fit=crop&w=1200&q=85',
    tags: ['SaaS', 'UX strategy'],
    live: '#',
    github: '#',
  },
]

export const experience: Experience[] = [
  { period: '2022 — 2026', role: 'Code Sensei', company: 'Code Ninjas', description: 'Teaching and mentoring aspiring developers in JavaScript, Lua, and AI.' },
  { period: '2026 — 2030', role: 'Student', company: 'MTSU', description: 'Learning and growing as a software developer.' },
]

export const socials = [
  { label: 'GitHub', href: 'https://github.com/treymouledoux' },
  { label: 'LinkedIn', href: 'https://www.linkedin.com/in/trey-mouledoux-77959a42b/' },
  { label: 'Instagram', href: 'https://www.instagram.com/treymouledoux/' },
]

export const navItems = [
  { label: 'Home', href: '#home' },
  { label: 'About', href: '#about' },
  { label: 'Projects', href: '#projects' },
  { label: 'Experience', href: '#experience' },
  { label: 'Contact', href: '#contact' },
]
