import type { Metadata, Viewport } from 'next'
import './globals.css'

const description = 'Trey Mouledoux is a software developer and computer science student in Tennessee. Rust, Python, systems programming, and open-source projects.'

export const metadata: Metadata = {
  metadataBase: new URL('https://treymouledoux.me'),
  title: 'Trey Mouledoux — Software Developer',
  description,
  openGraph: {
    title: 'Trey Mouledoux — Software Developer',
    description,
    type: 'website',
    url: 'https://treymouledoux.me',
    locale: 'en_US',
  },
  icons: {
    icon: [
      { url: '/icon-light-32x32.png?v=2', media: '(prefers-color-scheme: light)' },
      { url: '/icon-dark-32x32.png?v=2', media: '(prefers-color-scheme: dark)' },
      { url: '/icon.svg?v=2', type: 'image/svg+xml' },
    ],
    apple: '/apple-icon.png?v=2',
  },
}

export const viewport: Viewport = {
  colorScheme: 'light dark',
  themeColor: [
    { media: '(prefers-color-scheme: light)', color: '#f6f5f0' },
    { media: '(prefers-color-scheme: dark)', color: '#171b19' },
  ],
}

export default function RootLayout({ children }: Readonly<{ children: React.ReactNode }>) {
  return (
    <html lang="en" suppressHydrationWarning>
      <head><script dangerouslySetInnerHTML={{ __html: "try{var t=localStorage.getItem('portfolio-theme');document.documentElement.classList.toggle('dark',t?t==='dark':matchMedia('(prefers-color-scheme: dark)').matches)}catch(e){document.documentElement.classList.toggle('dark',matchMedia('(prefers-color-scheme: dark)').matches)}" }} /></head>
      <body>{children}</body>
    </html>
  )
}
