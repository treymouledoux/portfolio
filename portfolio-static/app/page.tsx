'use client'

import { useEffect, useState } from 'react'
import { GlassNav } from '@/components/portfolio/glass-nav'
import { About, Contact, Experience, Footer, Hero, Projects } from '@/components/portfolio/sections'
import { getContactCooldownRemaining } from '@/lib/contact'

export default function Page() {
  const [dark, setDark] = useState(false)
  const [themeReady, setThemeReady] = useState(false)
  const [submitted, setSubmitted] = useState(false)

  useEffect(() => {
    const preference = window.matchMedia('(prefers-color-scheme: dark)')
    let saved: string | null = null
    try { saved = localStorage.getItem('portfolio-theme') } catch { /* Storage can be unavailable in private browsers. */ }
    setDark(saved ? saved === 'dark' : preference.matches)
    setThemeReady(true)
    const syncPreference = () => {
      try { if (localStorage.getItem('portfolio-theme')) return } catch { /* Follow the system preference. */ }
      setDark(preference.matches)
    }
    preference.addEventListener('change', syncPreference)
    return () => preference.removeEventListener('change', syncPreference)
  }, [])

  useEffect(() => {
    if (themeReady) document.documentElement.classList.toggle('dark', dark)
  }, [dark, themeReady])

  useEffect(() => {
    const remaining = getContactCooldownRemaining()
    if (!remaining) return
    setSubmitted(true)
    const timeout = window.setTimeout(() => setSubmitted(false), remaining)
    return () => window.clearTimeout(timeout)
  }, [submitted])

  const changeTheme = (value: boolean) => {
    setDark(value)
    try { localStorage.setItem('portfolio-theme', value ? 'dark' : 'light') } catch { /* The toggle still works without persistence. */ }
  }

  return (
    <>
      <a className="skip-link" href="#main-content">Skip to content</a>
      <GlassNav dark={dark} setDark={changeTheme} />
      <div className="portfolio-shell">
        <main id="main-content">
          <Hero />
          <Projects />
          <About />
          <Experience />
          <Contact submitted={submitted} setSubmitted={setSubmitted} />
        </main>
        <Footer />
      </div>
    </>
  )
}
