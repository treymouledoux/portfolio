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
  name: 'Alex Morgan',
  role: 'Product designer & creative developer',
  tagline: 'I turn complex ideas into clear, considered digital experiences.',
  bio: 'I’m a multidisciplinary designer and developer based in New York. I work at the intersection of brand, product, and technology to make digital products feel more human.',
  email: 'hello@alexmorgan.design',
  location: 'New York, NY',
}

export const skills = ['Product design', 'Design systems', 'Art direction', 'React', 'Next.js', 'Prototyping', 'Webflow', 'Framer Motion']

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
  { period: '2022 — Now', role: 'Independent designer & developer', company: 'Self-employed', description: 'Partnering with early-stage teams and ambitious founders to shape products from first sketch to launch.' },
  { period: '2020 — 2022', role: 'Senior product designer', company: 'Northstar Studio', description: 'Led product design across fintech and healthcare, building systems that scaled from MVP to millions of users.' },
  { period: '2017 — 2020', role: 'Designer / developer', company: 'Good Company', description: 'Created identities and interactive experiences for culture, technology, and consumer brands.' },
]

export const socials = [
  { label: 'GitHub', href: '#' },
  { label: 'LinkedIn', href: '#' },
  { label: 'X / Twitter', href: '#' },
]

export const navItems = [
  { label: 'Home', href: '#home' },
  { label: 'About', href: '#about' },
  { label: 'Projects', href: '#projects' },
  { label: 'Experience', href: '#experience' },
  { label: 'Contact', href: '#contact' },
]
