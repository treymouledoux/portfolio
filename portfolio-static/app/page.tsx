'use client'

import { useEffect, useState } from 'react'
import { GlassNav } from '@/components/portfolio/glass-nav'
import { About, Contact, Experience, Footer, Projects } from '@/components/portfolio/sections'
import { getContactCooldownRemaining } from '@/lib/contact'

export default function Page() {
  const [dark, setDark] = useState(false)
  const [submitted, setSubmitted] = useState(false)
  useEffect(() => {
    const preference = window.matchMedia('(prefers-color-scheme: dark)')
    const syncPreference = () => setDark(preference.matches)
    syncPreference()
    preference.addEventListener('change', syncPreference)
    return () => preference.removeEventListener('change', syncPreference)
  }, [])
  useEffect(() => { document.documentElement.classList.toggle('dark', dark) }, [dark])
  useEffect(() => {
    const remaining = getContactCooldownRemaining()
    if (!remaining) return

    setSubmitted(true)
    const timeout = window.setTimeout(() => setSubmitted(false), remaining)
    return () => window.clearTimeout(timeout)
  }, [submitted])
  return <div className="portfolio-shell"><GlassNav dark={dark} setDark={setDark} submitted={submitted} /><main><About /><Projects /><Experience /><Contact submitted={submitted} setSubmitted={setSubmitted} /></main><Footer /></div>
}
