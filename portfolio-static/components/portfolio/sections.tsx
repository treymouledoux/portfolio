'use client'

import { useState, type FormEvent } from 'react'
import { ArrowDown, ArrowUpRight, Check, CodeXml } from 'lucide-react'
import { experience, profile, projects, skills, socials } from '@/lib/portfolio-data'
import { setContactCooldown, submitContactForm, validateContactForm, type ContactFormData } from '@/lib/contact'

function SectionHeading({ id, number, title, note }: { id: string; number: string; title: string; note?: string }) {
  return (
    <div className="section-heading">
      <div className="section-title"><span className="section-number">{number}</span><h2 id={id}>{title}</h2></div>
      {note && <p className="section-note">{note}</p>}
    </div>
  )
}

export function Hero() {
  return (
    <section id="home" className="hero section-wrap" aria-labelledby="hero-title">
      <div className="hero-copy">
        <p className="eyebrow"><span className="status-dot" aria-hidden="true" />{profile.location}</p>
        <h1 id="hero-title">Trey<br /><span>Mouledoux.</span></h1>
        <p className="hero-role">Software developer & computer science student.</p>
        <p className="hero-sub">{profile.tagline}</p>
        <div className="hero-actions">
          <a className="button button-dark" href="#projects">Explore my work <ArrowDown size={16} /></a>
          <a className="button button-quiet" href="https://github.com/treymouledoux"><CodeXml size={16} />GitHub <ArrowUpRight size={14} /></a>
        </div>
      </div>
      <aside className="hero-notes" aria-label="What I am doing now">
        <p className="eyebrow">These days</p>
        <div className="current-item"><span className="note-label">Working</span><p>Writing code.<br />Checking what AI gets wrong.</p><span className="note-detail">Software engineering · DataAnnotation</span></div>
        <div className="current-item"><span className="note-label">Studying</span><p>Computer science,<br />with a focus on cybersecurity.</p><span className="note-detail">Middle Tennessee State University</span></div>
        <a className="hero-contact" href="#contact">Say hello <ArrowUpRight size={16} /></a>
      </aside>
      <div className="hero-footnote"><a href="#projects">Selected projects below <ArrowDown size={13} /></a></div>
    </section>
  )
}

export function Projects() {
  return (
    <section id="projects" className="section-wrap section-block" aria-labelledby="projects-title">
      <SectionHeading id="projects-title" number="01" title="Selected work" note="Things I wanted to exist, so I built them." />
      <div className="project-grid">
        {projects.map((project, index) => (
          <article className={`project-card ${project.featured ? 'featured' : ''}`} key={project.title}>
            <a className="project-image" href={project.live ?? project.github} aria-label={`${project.title} ${project.live ? 'project site' : 'source code'}`}>
              <img src={project.image} alt={project.imageAlt} width={1600} height={900} loading="lazy" decoding="async" />
              <span className="image-arrow" aria-hidden="true"><ArrowUpRight size={20} /></span>
            </a>
            <div className="project-info">
              <p className="project-kicker"><span>{String(index + 1).padStart(2, '0')}</span>{project.tags.join(' / ')}</p>
              <h3>{project.title}</h3>
              <p className="project-description">{project.description}</p>
              <div className="project-links">
                {project.live && <a className="text-link" href={project.live}>Project site <ArrowUpRight size={14} /></a>}
                <a className={`text-link${project.live ? ' secondary-link' : ''}`} href={project.github}>Source code <CodeXml size={14} /></a>
                {project.listing && <a className="text-link secondary-link" href={project.listing.href}>{project.listing.label} <ArrowUpRight size={14} /></a>}
              </div>
            </div>
          </article>
        ))}
      </div>
    </section>
  )
}

export function About() {
  return (
    <section id="about" className="section-wrap section-block" aria-labelledby="about-title">
      <SectionHeading id="about-title" number="02" title="A little context" />
      <div className="about-grid">
        <div className="about-copy">
          <p className="about-lede">{profile.bio}</p>
          <p>I started building software before college. Along the way, I spent four years teaching kids to code and three years competing in CyberPatriot. Both taught me to slow down, understand the problem, and explain the solution.</p>
          <p>I also run a homelab and self-host services I use. Deploying something and keeping it running has been its own education.</p>
          <a className="text-link" href={profile.linkedin}>More on LinkedIn <ArrowUpRight size={15} /></a>
        </div>
        <div className="about-details">
          <div className="education-note"><span className="eyebrow">Currently at MTSU</span><h3>B.S. Computer Science</h3><p>Cybersecurity concentration<br />2026–2030, expected<br />ISA Cyber Club member</p></div>
          <dl className="skill-list">{skills.map((skill) => <div key={skill.label}><dt>{skill.label}</dt><dd>{skill.items}</dd></div>)}</dl>
        </div>
      </div>
    </section>
  )
}

export function Experience() {
  return (
    <section id="experience" className="section-wrap section-block" aria-labelledby="experience-title">
      <SectionHeading id="experience-title" number="03" title="Experience" />
      <div className="experience-list">{experience.map((item) => (
        <article className="experience-item" key={item.company}>
          <p className="experience-date">{item.period}</p>
          <div><h3>{item.role}</h3><p className="company">{item.company}</p><p className="experience-description">{item.description}</p></div>
        </article>
      ))}</div>
    </section>
  )
}

export function Contact({ submitted, setSubmitted }: { submitted: boolean; setSubmitted: (value: boolean) => void }) {
  const [submitting, setSubmitting] = useState(false)
  const [replyMessage, setReplyMessage] = useState('')
  const [errorMessage, setErrorMessage] = useState('')

  const handleFormSubmit = async (event: FormEvent<HTMLFormElement>): Promise<void> => {
    event.preventDefault()
    if (submitting || submitted) return
    setReplyMessage('')
    setErrorMessage('')
    const formData = new FormData(event.currentTarget)
    const data: ContactFormData = {
      name: String(formData.get('name') ?? ''),
      email: String(formData.get('email') ?? ''),
      message: String(formData.get('message') ?? ''),
    }
    const validationError = validateContactForm(data)
    if (validationError) {
      setErrorMessage(validationError)
      return
    }
    setSubmitting(true)
    try {
      const result = await submitContactForm(data)
      if (result.success) {
        setReplyMessage(result.reply_message ?? 'Thanks, your message has been sent.')
        setContactCooldown()
        setSubmitted(true)
      } else {
        setErrorMessage(result.reply_message ?? 'Your message could not be sent. Please check your details and try again.')
      }
    } catch (error) {
      setErrorMessage(error instanceof Error ? error.message : 'Unable to send message. Please try again.')
    } finally {
      setSubmitting(false)
    }
  }

  return (
    <section id="contact" className="section-wrap section-block" aria-labelledby="contact-title">
      <SectionHeading id="contact-title" number="04" title="Get in touch" />
      <div className="contact-grid">
        <div className="contact-copy">
          <h3>Have something<br />in mind?</h3>
          <p>For work, a project, or a question about something I built. Send a note using the form below.</p>
          <div className="social-list">{socials.map((social) => <a href={social.href} key={social.label}>{social.label}<ArrowUpRight size={14} /></a>)}</div>
        </div>
        <form className="contact-form" onSubmit={handleFormSubmit} aria-label="Send Trey a message">
          <fieldset disabled={submitted || submitting}>
            <div className="form-row">
              <label htmlFor="contact-name">Name<input id="contact-name" required name="name" autoComplete="name" maxLength={50} placeholder="Your name" /></label>
              <label htmlFor="contact-email">Email<input id="contact-email" required type="email" name="email" autoComplete="email" maxLength={254} placeholder="you@example.com" /></label>
            </div>
            <label htmlFor="contact-message">Message<textarea id="contact-message" required name="message" rows={5} minLength={100} maxLength={5000} aria-describedby="message-hint" placeholder="What are you working on?" /></label>
            <p id="message-hint" className="form-hint">A little detail helps: 100–5,000 characters.</p>
            <button className={`button button-dark ${submitted ? 'is-submitted' : ''}`} type="submit">{submitted ? <>Message sent<Check size={16} /></> : submitting ? 'Sending…' : <>Send message<ArrowUpRight size={16} /></>}</button>
          </fieldset>
          {submitted && !replyMessage && <p className="contact-status" role="status">Thanks for reaching out. You can send another message one hour after your previous submission.</p>}
          {replyMessage && <p className="contact-status contact-status-success" role="status">{replyMessage}</p>}
          {errorMessage && <p className="contact-status contact-status-error" role="alert">{errorMessage}</p>}
        </form>
      </div>
    </section>
  )
}

export function Footer() {
  return <footer className="footer section-wrap"><a className="footer-name" href="#home">Trey Mouledoux<span>.</span></a><span>© {new Date().getFullYear()} · Built with Rust & Next.js</span><a className="back-to-top" href="#home">Back to top<ArrowUpRight size={14} /></a></footer>
}
