'use client';

import { ThemeProvider } from 'next-themes';
import type { ReactNode } from 'react';
import { QueryProvider } from '@/lib/query-client';

/** Dark mode (class strategy, follows the OS by default) and TanStack Query. */
export function Providers({ children, nonce }: { children: ReactNode; nonce?: string }) {
  return (
    <ThemeProvider attribute="class" defaultTheme="system" enableSystem nonce={nonce}>
      <QueryProvider>{children}</QueryProvider>
    </ThemeProvider>
  );
}
