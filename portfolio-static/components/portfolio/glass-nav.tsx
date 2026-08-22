'use client'

import { useEffect, useState } from 'react'
import { AnimatePresence, motion, useScroll, useSpring } from 'framer-motion'
import { ArrowUpRight, Menu, Moon, Sun, X } from 'lucide-react'
import { navItems } from '@/lib/portfolio-data'
import { clearPointerGlow, scrollToSection, setPointerGlow } from '@/lib/portfolio-utils'

export function GlassNav({ dark, setDark }: { dark: boolean; setDark: (value: boolean) => void }) {
  const [active, setActive] = useState('about')
  const [open, setOpen] = useState(false)
  const { scrollYProgress } = useScroll()
  const scaleX = useSpring(scrollYProgress, { stiffness: 100, damping: 30 })

  useEffect(() => {
    const observer = new IntersectionObserver((entries) => entries.forEach((entry) => entry.isIntersecting && setActive(entry.target.id)), { rootMargin: '-35% 0px -55% 0px' })
    navItems.forEach(({ href }) => { const section = document.querySelector(href); if (section) observer.observe(section) })
    return () => observer.disconnect()
  }, [])

  const navigate = (href: string) => { setOpen(false); scrollToSection(href) }
  return <>
    <motion.div className="progress-bar" style={{ scaleX }} />
    <header className="fixed inset-x-0 top-4 z-50 px-4 sm:top-6">
      <nav className="glass-nav glass-interactive mx-auto flex max-w-5xl items-center justify-between p-2" onPointerMove={setPointerGlow} onPointerLeave={clearPointerGlow} aria-label="Main navigation">
        <button className="nav-mark" onClick={() => navigate('#home')} aria-label="Go to home">Trey Mouledoux<span>.</span></button>
        <div className="hidden items-center gap-1 md:flex">
          {navItems.map((item) => <button key={item.href} onClick={() => navigate(item.href)} className={`nav-link ${active === item.href.slice(1) ? 'active' : ''}`}>{active === item.href.slice(1) && <motion.span layoutId="active-nav-pill" transition={{ type: 'spring', stiffness: 340, damping: 18, mass: .65 }} className="active-nav-pill" />}{item.label}</button>)}
        </div>
        <div className="flex items-center gap-1">
          <button className="icon-button" onClick={() => setDark(!dark)} aria-label={`Switch to ${dark ? 'light' : 'dark'} mode`} title={`Switch to ${dark ? 'light' : 'dark'} mode`}>{dark ? <Moon size={17} /> : <Sun size={17} />}</button>
          <button className="icon-button menu-toggle" onClick={() => setOpen(!open)} aria-label={open ? 'Close menu' : 'Open menu'}>{open ? <X size={18} /> : <Menu size={18} />}</button>
          <button className="nav-cta hidden sm:flex" onClick={() => navigate('#contact')}>Let's talk <ArrowUpRight size={15} /></button>
        </div>
      </nav>
      <AnimatePresence>
        {open && <motion.div initial={{ opacity: 0, y: -10, scale: .97 }} animate={{ opacity: 1, y: 0, scale: 1 }} exit={{ opacity: 0, y: -10, scale: .97 }} transition={{ type: 'spring', stiffness: 420, damping: 30 }} className="glass-nav glass-interactive mobile-menu md:hidden" onPointerMove={setPointerGlow} onPointerLeave={clearPointerGlow}>{navItems.map((item) => <motion.button whileTap={{ scale: .97 }} key={item.href} onClick={() => navigate(item.href)} className="mobile-link">{item.label}<ArrowUpRight size={16} /></motion.button>)}</motion.div>}
      </AnimatePresence>
    </header>
  </>
}
