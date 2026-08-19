import type { Variants } from 'framer-motion'

export const fadeUp: Variants = {
  hidden: { opacity: 0, y: 24 },
  visible: { opacity: 1, y: 0, transition: { duration: 0.65, ease: 'easeOut' } },
}

export function scrollToSection(href: string) {
  document.querySelector(href)?.scrollIntoView({ behavior: 'smooth' })
}

export function setPointerGlow(event: React.PointerEvent<HTMLElement>) {
  const rect = event.currentTarget.getBoundingClientRect()
  event.currentTarget.style.setProperty('--pointer-x', `${event.clientX - rect.left}px`)
  event.currentTarget.style.setProperty('--pointer-y', `${event.clientY - rect.top}px`)
}

export function clearPointerGlow(event: React.PointerEvent<HTMLElement>) {
  event.currentTarget.style.setProperty('--pointer-x', '-100px')
  event.currentTarget.style.setProperty('--pointer-y', '-100px')
}
