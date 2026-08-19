import { Analytics } from '@vercel/analytics/next'
import type { Metadata, Viewport } from 'next'
import './globals.css'

export const metadata: Metadata = {
  title: 'Alex Morgan — Product designer & creative developer',
  description: 'The portfolio of Alex Morgan, a product designer and creative developer turning complex ideas into clear digital experiences.',
  generator: 'v0.app',
  openGraph: {
    title: 'Alex Morgan — Product designer & creative developer',
    description: 'Designing digital experiences with clarity.',
    type: 'website',
  },
  icons: {
    icon: [{ url: '/icon-light-32x32.png', media: '(prefers-color-scheme: light)' }, { url: '/icon-dark-32x32.png', media: '(prefers-color-scheme: dark)' }, { url: '/icon.svg', type: 'image/svg+xml' }],
    apple: '/apple-icon.png',
  },
}

export const viewport: Viewport = {
  colorScheme: 'dark light',
  themeColor: [{ media: '(prefers-color-scheme: light)', color: '#f3f3ef' }, { media: '(prefers-color-scheme: dark)', color: '#151614' }],
}

export default function RootLayout({ children }: Readonly<{ children: React.ReactNode }>) {
  return <html lang="en" className="dark"><body className="antialiased">{children}{process.env.NODE_ENV === 'production' && <Analytics />}</body></html>
}
