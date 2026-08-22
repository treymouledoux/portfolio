'use client'

import { useState, type FormEvent } from 'react'
import { motion } from 'framer-motion'
import { ArrowUpRight, Check, Mail, MoveUpRight } from 'lucide-react'
import { experience, profile, projects, skills, socials } from '@/lib/portfolio-data'
import { submitContactForm, type ContactFormData } from '@/lib/contact'
import { fadeUp, scrollToSection } from '@/lib/portfolio-utils'

export function SectionHeading({ eyebrow, title }: { eyebrow: string; title: string }) { return <div className="section-heading"><div><p className="eyebrow">{eyebrow}</p><h2>{title}</h2></div></div> }

export function About() { return <section id="about" className="section-wrap section-block"><SectionHeading eyebrow="01 / About" title="Trey Mouledoux" /><div className="about-grid"><div className="about-lede"><p>{profile.bio}</p><a className="text-link" href={`https://www.linkedin.com/in/trey-mouledoux-77959a42b/`}>More about me <ArrowUpRight size={16} /></a></div><div className="skills-panel"><p className="eyebrow">Skills</p><div className="skill-grid">{skills.map((skill) => <span className="skill-chip" key={skill}>{skill}</span>)}</div></div></div></section> }

export function Projects() { return <section id="projects" className="section-wrap section-block"><SectionHeading eyebrow="02 / Selected work" title="A few things I've made." /><div className="project-grid">{projects.map((project, i) => <motion.article initial="hidden" whileInView="visible" viewport={{ once: true, margin: '-80px' }} variants={fadeUp} transition={{ delay: i * .08 }} className={`project-card ${project.featured ? 'featured' : ''}`} key={project.title}><div className="project-image"><img src={project.image} alt={`${project.title} project preview`} /><a href={project.live} aria-label={`View ${project.title}`} className="image-arrow"><ArrowUpRight size={19} /></a></div><div className="project-info"><div><p className="project-kicker">0{i + 1} — {project.tags.join(' / ')}</p><h3>{project.title}</h3><p>{project.description}</p></div><a href={project.github} className="project-link">View Repository <MoveUpRight size={15} /></a></div></motion.article>)}</div></section> }

export function Experience() { return <section id="experience" className="section-wrap section-block"><SectionHeading eyebrow="03 / Experience" title="The path so far." /><div className="timeline">{experience.map((item) => <div className="timeline-item" key={item.period}><div className="timeline-date">{item.period}</div><div className="timeline-dot" /><div className="timeline-content"><h3>{item.role}</h3><p className="company">{item.company}</p><p>{item.description}</p></div></div>)}</div></section> }

export function Contact({ submitted, setSubmitted }: { submitted: boolean; setSubmitted: (value: boolean) => void }) {
    const [submitting, setSubmitting] = useState(false);

    const handleFormSubmit = async (event: FormEvent<HTMLFormElement>): Promise<void> => {
        event.preventDefault();
        if (submitting || submitted) return;
        setSubmitting(true);

        const formData = new FormData(event.currentTarget);
        const data: ContactFormData = {
            name: String(formData.get('name') ?? ''),
            email: String(formData.get('email') ?? ''),
            message: String(formData.get('message') ?? ''),
        };

        try {
            const result = await submitContactForm(data);

            if (result.success) {
                console.log(result.reply_message);
                setSubmitted(true);
            }
        } catch (error) {
            console.error('Failed to send message:', error);
        } finally {
            setSubmitting(false);
        }
    };

    return <section id="contact" className="section-wrap contact-section section-block"><div className="contact-copy"><p className="eyebrow">04 / Contact</p><h2>Have a good idea?<br /><em>Let's make it real.</em></h2><p>I'm currently available for hire.</p><div className="social-list">{socials.map((social) => <a href={social.href} key={social.label}>{social.label} <ArrowUpRight size={15} /></a>)}</div></div>
    <form className={`contact-form ${submitted ? 'is-submitted' : ''}`} onSubmit={handleFormSubmit}>
        <fieldset disabled={submitted || submitting}>
        <label>Name<input required name="name" placeholder="Your name" /></label>
        <label>Email<input required type="email" name="email" placeholder="you@company.com" /></label>
        <label>Message<textarea required name="message" rows={4} placeholder="Tell me a little about your project..." /></label>
        <button className={`button button-dark ${submitted ? 'is-submitted' : ''}`} type="submit">
            {submitted ? <>Message sent <Check size={16} /></> : <>Send message <ArrowUpRight size={16} /></>}
        </button>
        </fieldset>
    </form>
</section> }

export function Footer() { return <footer className="footer section-wrap"><span>© 2026 {profile.name}</span><span>Designed & built with intention.</span><a href="#about">Back to top ↑</a></footer> }
