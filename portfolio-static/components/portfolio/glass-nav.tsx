'use client'

import { useEffect, useRef, useState } from 'react'
import { ArrowUpRight, Menu, Moon, Sun, X } from 'lucide-react'
import { navItems } from '@/lib/portfolio-data'

type GlassNavProps = {
  dark: boolean
  setDark: (value: boolean) => void
}

export function GlassNav({ dark, setDark }: GlassNavProps) {
  const [active, setActive] = useState('home')
  const [open, setOpen] = useState(false)
  const headerRef = useRef<HTMLElement>(null)
  const menuButtonRef = useRef<HTMLButtonElement>(null)

  useEffect(() => {
    const sections = ['home', ...navItems.map(({ href }) => href.slice(1))]
      .map((id) => document.getElementById(id))
      .filter((section): section is HTMLElement => Boolean(section))
    let frame = 0
    const updateActive = () => {
      frame = 0
      const marker = window.innerHeight * 0.32
      const current = [...sections].reverse().find((section) => section.getBoundingClientRect().top <= marker)
      setActive(current?.id ?? 'home')
    }
    const onScroll = () => {
      if (!frame) frame = window.requestAnimationFrame(updateActive)
    }
    updateActive()
    window.addEventListener('scroll', onScroll, { passive: true })
    window.addEventListener('resize', onScroll)
    return () => {
      window.cancelAnimationFrame(frame)
      window.removeEventListener('scroll', onScroll)
      window.removeEventListener('resize', onScroll)
    }
  }, [])

  useEffect(() => {
    if (!open) return
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        setOpen(false)
        menuButtonRef.current?.focus()
      }
    }
    const onPointerDown = (event: PointerEvent) => {
      if (!headerRef.current?.contains(event.target as Node)) setOpen(false)
    }
    const desktop = window.matchMedia('(min-width: 768px)')
    const onResize = () => { if (desktop.matches) setOpen(false) }
    document.addEventListener('keydown', onKeyDown)
    document.addEventListener('pointerdown', onPointerDown)
    desktop.addEventListener('change', onResize)
    return () => {
      document.removeEventListener('keydown', onKeyDown)
      document.removeEventListener('pointerdown', onPointerDown)
      desktop.removeEventListener('change', onResize)
    }
  }, [open])

  return (
    <header className="site-header" ref={headerRef}>
      <nav className="glass-nav" aria-label="Main navigation">
        <a className="nav-mark" href="#home" onClick={() => setOpen(false)} aria-label="Trey Mouledoux, back to top">
          Trey Mouledoux<span aria-hidden="true">.</span>
        </a>
        <div className="desktop-links">
          {navItems.map((item) => (
            <a key={item.href} href={item.href} className={`nav-link ${active === item.href.slice(1) ? 'active' : ''}`} aria-current={active === item.href.slice(1) ? 'location' : undefined}>
              {item.label}
            </a>
          ))}
        </div>
        <div className="nav-controls">
          <button className="icon-button" onClick={() => setDark(!dark)} aria-label={`Switch to ${dark ? 'light' : 'dark'} mode`} aria-pressed={dark}>
            {dark ? <Sun size={17} /> : <Moon size={17} />}
          </button>
          <a className="nav-cta" href="#contact" onClick={() => setOpen(false)}>Get in touch <ArrowUpRight size={14} /></a>
          <button ref={menuButtonRef} className="icon-button menu-toggle" onClick={() => setOpen(!open)} aria-label={open ? 'Close menu' : 'Open menu'} aria-expanded={open} aria-controls="mobile-navigation">
            {open ? <X size={20} /> : <Menu size={20} />}
          </button>
        </div>
      </nav>
      <div className="mobile-menu-reveal" data-open={open} inert={!open} aria-hidden={!open}>
        <div className="mobile-menu-clip">
          <nav id="mobile-navigation" className="glass-nav mobile-menu" aria-label="Mobile navigation">
            {navItems.map((item) => (
              <a key={item.href} href={item.href} className="mobile-link" onClick={() => setOpen(false)} aria-current={active === item.href.slice(1) ? 'location' : undefined}>
                {item.label}<ArrowUpRight size={16} />
              </a>
            ))}
          </nav>
        </div>
      </div>
    </header>
  )
}
