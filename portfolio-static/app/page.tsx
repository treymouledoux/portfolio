'use client'

import { useEffect, useState } from 'react'
import { motion, useScroll, useSpring } from 'framer-motion'
import { ArrowUpRight, Check, Github, Linkedin, Mail, Menu, Moon, MoveUpRight, Sun, X } from 'lucide-react'
import { experience, navItems, profile, projects, skills, socials } from '@/lib/portfolio-data'

const fadeUp = { hidden: { opacity: 0, y: 24 }, visible: { opacity: 1, y: 0, transition: { duration: 0.65, ease: 'easeOut' } } }

function GlassNav({ dark, setDark }: { dark: boolean; setDark: (value: boolean) => void }) {
  const [active, setActive] = useState('home')
  const [open, setOpen] = useState(false)
  const { scrollYProgress } = useScroll()
  const scaleX = useSpring(scrollYProgress, { stiffness: 100, damping: 30 })

  useEffect(() => {
    const observer = new IntersectionObserver((entries) => entries.forEach((entry) => entry.isIntersecting && setActive(entry.target.id)), { rootMargin: '-35% 0px -55% 0px' })
    navItems.forEach(({ href }) => { const section = document.querySelector(href); if (section) observer.observe(section) })
    return () => observer.disconnect()
  }, [])

  const navigate = (href: string) => { setOpen(false); document.querySelector(href)?.scrollIntoView({ behavior: 'smooth' }) }
  return <>
    <motion.div className="progress-bar" style={{ scaleX }} />
    <header className="fixed inset-x-0 top-4 z-50 px-4 sm:top-6">
      <nav className="glass-nav mx-auto flex max-w-5xl items-center justify-between p-2" aria-label="Main navigation">
        <button className="nav-mark" onClick={() => navigate('#home')} aria-label="Go to home">AM<span>.</span></button>
        <div className="hidden items-center gap-1 md:flex">
          {navItems.map((item) => <button key={item.href} onClick={() => navigate(item.href)} className={`nav-link ${active === item.href.slice(1) ? 'active' : ''}`}>{item.label}</button>)}
        </div>
        <div className="flex items-center gap-1">
          <button className="icon-button" onClick={() => setDark(!dark)} aria-label={`Switch to ${dark ? 'light' : 'dark'} mode`}>{dark ? <Sun size={17} /> : <Moon size={17} />}</button>
          <button className="icon-button md:hidden" onClick={() => setOpen(!open)} aria-label={open ? 'Close menu' : 'Open menu'}>{open ? <X size={18} /> : <Menu size={18} />}</button>
          <button className="nav-cta hidden sm:flex" onClick={() => navigate('#contact')}>Let&apos;s talk <ArrowUpRight size={15} /></button>
        </div>
      </nav>
      {open && <div className="glass-nav mobile-menu md:hidden">{navItems.map((item) => <button key={item.href} onClick={() => navigate(item.href)} className="mobile-link">{item.label}<ArrowUpRight size={16} /></button>)}</div>}
    </header>
  </>
}

function SectionHeading({ eyebrow, title, children }: { eyebrow: string; title: string; children?: React.ReactNode }) {
  return <div className="section-heading"><div><p className="eyebrow">{eyebrow}</p><h2>{title}</h2></div>{children}</div>
}

export default function Page() {
  const [dark, setDark] = useState(true)
  const [sent, setSent] = useState(false)
  useEffect(() => { document.documentElement.classList.toggle('dark', dark) }, [dark])
  return <div className="portfolio-shell"><GlassNav dark={dark} setDark={setDark} />
    <main>
      <section id="home" className="hero section-wrap">
        <motion.div initial="hidden" animate="visible" variants={fadeUp} className="hero-copy">
          <p className="eyebrow"><span className="status-dot" /> Available for select projects</p>
          <h1>Designing digital <em>experiences</em> with clarity.</h1>
          <p className="hero-sub">{profile.tagline}</p>
          <div className="hero-actions"><button className="button button-dark" onClick={() => document.querySelector('#projects')?.scrollIntoView({ behavior: 'smooth' })}>View projects <ArrowUpRight size={17} /></button><button className="button button-glass" onClick={() => document.querySelector('#contact')?.scrollIntoView({ behavior: 'smooth' })}>Get in touch <Mail size={16} /></button></div>
        </motion.div>
        <motion.div initial={{ opacity: 0, scale: .94 }} animate={{ opacity: 1, scale: 1 }} transition={{ duration: .8, delay: .15 }} className="hero-art"><div className="portrait-frame"><img src="https://images.unsplash.com/photo-1500648767791-00dcc994a43e?auto=format&fit=crop&w=900&q=85" alt="Portrait of Alex Morgan" /></div><div className="art-caption"><span>AM / 2026</span><span>NYC — 40.7128° N</span></div></motion.div>
      </section>

      <section id="about" className="section-wrap section-block"><SectionHeading eyebrow="01 / About" title="A thoughtful mix of logic and instinct." /><div className="about-grid"><div className="about-lede"><p>{profile.bio}</p><a className="text-link" href={`mailto:${profile.email}`}>More about me <ArrowUpRight size={16} /></a></div><div className="skills-panel"><p className="eyebrow">Capabilities</p><div className="skill-grid">{skills.map((skill) => <span className="skill-chip" key={skill}>{skill}</span>)}</div></div></div></section>

      <section id="projects" className="section-wrap section-block"><SectionHeading eyebrow="02 / Selected work" title="A few things I&apos;ve made." /><div className="project-grid">{projects.map((project, i) => <motion.article initial="hidden" whileInView="visible" viewport={{ once: true, margin: '-80px' }} variants={fadeUp} transition={{ delay: i * .08 }} className={`project-card ${project.featured ? 'featured' : ''}`} key={project.title}><div className="project-image"><img src={project.image} alt={`${project.title} project preview`} /><a href={project.live} aria-label={`View ${project.title}`} className="image-arrow"><ArrowUpRight size={19} /></a></div><div className="project-info"><div><p className="project-kicker">0{i + 1} — {project.tags.join(' / ')}</p><h3>{project.title}</h3><p>{project.description}</p></div><a href={project.github} className="project-link">View case study <MoveUpRight size={15} /></a></div></motion.article>)}</div></section>

      <section id="experience" className="section-wrap section-block"><SectionHeading eyebrow="03 / Experience" title="The path so far." /><div className="timeline">{experience.map((item) => <div className="timeline-item" key={item.period}><div className="timeline-date">{item.period}</div><div className="timeline-dot" /><div className="timeline-content"><h3>{item.role}</h3><p className="company">{item.company}</p><p>{item.description}</p></div></div>)}</div></section>

      <section id="contact" className="section-wrap contact-section section-block"><div className="contact-copy"><p className="eyebrow">04 / Contact</p><h2>Have a good idea?<br /><em>Let&apos;s make it real.</em></h2><p>I&apos;m currently available for select freelance collaborations and product design roles.</p><div className="social-list">{socials.map((social) => <a href={social.href} key={social.label}>{social.label} <ArrowUpRight size={15} /></a>)}</div></div><form className="contact-form" onSubmit={(event) => { event.preventDefault(); setSent(true) }}><label>Name<input required name="name" placeholder="Your name" /></label><label>Email<input required type="email" name="email" placeholder="you@company.com" /></label><label>Message<textarea required name="message" rows={4} placeholder="Tell me a little about your project..." /></label><button className="button button-dark" type="submit">{sent ? <>Message sent <Check size={16} /></> : <>Send message <ArrowUpRight size={16} /></>}</button></form></section>
    </main>
    <footer className="footer section-wrap"><span>© 2026 {profile.name}</span><span>Designed & built with intention.</span><a href="#home">Back to top ↑</a></footer>
  </div>
}
