'use client';

import { ThemeProvider } from 'next-themes';
import type { ReactNode } from 'react';
import { ToastProvider } from '@/components/ui/toast';
import { QueryProvider } from '@/lib/query-client';

/** Dark mode (class strategy, follows the OS by default), TanStack Query and toasts. */
export function Providers({ children, nonce }: { children: ReactNode; nonce?: string }) {
  return (
    <ThemeProvider attribute="class" defaultTheme="system" enableSystem nonce={nonce}>
      <QueryProvider>
        <ToastProvider>{children}</ToastProvider>
      </QueryProvider>
    </ThemeProvider>
  );
}
