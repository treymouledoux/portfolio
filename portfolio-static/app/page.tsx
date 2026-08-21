'use client'

import { useEffect, useState } from 'react'
import { GlassNav } from '@/components/portfolio/glass-nav'
import { About, Contact, Experience, Footer, Projects } from '@/components/portfolio/sections'

export default function Page() {
  const [dark, setDark] = useState(false)
  useEffect(() => {
    const preference = window.matchMedia('(prefers-color-scheme: dark)')
    const syncPreference = () => setDark(preference.matches)
    syncPreference()
    preference.addEventListener('change', syncPreference)
    return () => preference.removeEventListener('change', syncPreference)
  }, [])
  useEffect(() => { document.documentElement.classList.toggle('dark', dark) }, [dark])
  return <div className="portfolio-shell"><GlassNav dark={dark} setDark={setDark} /><main><About /><Projects /><Experience /><Contact /></main><Footer /></div>
}
