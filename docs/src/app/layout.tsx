import { type Metadata } from 'next'
import { IBM_Plex_Sans, Bricolage_Grotesque } from 'next/font/google'
import clsx from 'clsx'

import { Providers } from '@/app/providers'
import { Layout } from '@/components/Layout'

import '@/styles/tailwind.css'

const ibmPlexSans = IBM_Plex_Sans({
  subsets: ['latin'],
  display: 'swap',
  variable: '--font-ibm-plex-sans',
  weight: ['300', '400', '500', '600', '700'],
})

const bricolageGrotesque = Bricolage_Grotesque({
  subsets: ['latin'],
  display: 'swap',
  variable: '--font-bricolage-grotesque',
})

export const metadata: Metadata = {
  title: {
    template: '%s - MailLaser Docs',
    default: 'MailLaser - Email-to-Webhook Forwarder',
  },
  description:
    'MailLaser receives emails via SMTP and instantly forwards them as JSON webhook payloads. Lightweight, resilient, and simple to deploy.',
}

export default function RootLayout({
  children,
}: {
  children: React.ReactNode
}) {
  return (
    <html
      lang="en"
      className={clsx(
        'h-full antialiased',
        ibmPlexSans.variable,
        bricolageGrotesque.variable,
      )}
      suppressHydrationWarning
    >
      <body className="flex min-h-full bg-stone-50 dark:bg-stone-950">
        <Providers>
          <Layout>{children}</Layout>
        </Providers>
      </body>
    </html>
  )
}
