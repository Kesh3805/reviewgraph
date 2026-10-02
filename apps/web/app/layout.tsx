import type { Metadata } from 'next';
import { headers } from 'next/headers';
import type { ReactNode } from 'react';
import { Providers } from '@/components/providers';
import './globals.css';

export const metadata: Metadata = {
  title: { default: 'ReviewGraph', template: '%s | ReviewGraph' },
  description: 'Graph-native pull request review.',
};

export default async function RootLayout({ children }: { children: ReactNode }) {
  // Reading the per-request nonce also opts the app into dynamic rendering, which the
  // nonce-based CSP requires.
  const nonce = (await headers()).get('x-nonce') ?? undefined;
  return (
    <html lang="en" suppressHydrationWarning>
      <body className="min-h-screen font-sans">
        <Providers nonce={nonce}>{children}</Providers>
      </body>
    </html>
  );
}
