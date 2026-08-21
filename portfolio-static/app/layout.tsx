import { GlassFilters } from "@/components/GlassFilters";
import type { Metadata, Viewport } from 'next'
import './globals.css'

export const metadata: Metadata = {
  title: 'Trey Mouledoux: Software Developer',
  description: 'The portfolio of Trey Mouledoux',
  openGraph: {
    title: 'Trey Mouledoux — Software Developer',
    description: 'Creating innovative software solutions.',

    type: 'website',
  },
  icons: {
    icon: [{ url: '/icon-light-32x32.png?v=2', media: '(prefers-color-scheme: light)' }, { url: '/icon-dark-32x32.png?v=2', media: '(prefers-color-scheme: dark)' }, { url: '/icon.svg?v=2', type: 'image/svg+xml' }],
    apple: '/apple-icon.png?v=2',
  },
}

export const viewport: Viewport = {
  colorScheme: 'dark light',
  themeColor: [{ media: '(prefers-color-scheme: light)', color: '#f3f3ef' }, { media: '(prefers-color-scheme: dark)', color: '#151614' }],
}

export default function RootLayout({ children }: Readonly<{ children: React.ReactNode }>) {
  return (
    <html lang="en">
      <body className="antialiased">
        <GlassFilters />
        {children}
      </body>
    </html>
  );
}
