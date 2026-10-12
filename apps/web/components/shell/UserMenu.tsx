'use client';

import { LogOut, Moon, Sun } from 'lucide-react';
import { useTheme } from 'next-themes';
import { useState } from 'react';
import { Button } from '@/components/ui/button';
import { api } from '@/lib/api-client';
import type { SessionUser } from '@/lib/session';

export function UserMenu({ user }: { user: SessionUser }) {
  const [open, setOpen] = useState(false);
  const { resolvedTheme, setTheme } = useTheme();
  const display = user.display_name ?? user.login;

  async function signOut() {
    try {
      await api.POST('/api/v1/auth/logout');
    } finally {
      window.location.assign('/login');
    }
  }

  return (
    <div className="relative">
      <Button
        variant="ghost"
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
      >
        <span
          aria-hidden
          className="flex size-6 items-center justify-center rounded-full bg-secondary text-xs font-semibold uppercase"
        >
          {display.slice(0, 1)}
        </span>
        <span className="max-w-32 truncate">{display}</span>
      </Button>
      {open && (
        <div
          role="menu"
          className="absolute right-0 z-10 mt-1 w-48 rounded-md border bg-popover p-1 text-popover-foreground shadow-md"
        >
          <p className="truncate px-2 py-1.5 text-xs text-muted-foreground">@{user.login}</p>
          <button
            role="menuitem"
            className="flex w-full items-center gap-2 rounded-sm px-2 py-1.5 text-sm hover:bg-accent"
            onClick={() => setTheme(resolvedTheme === 'dark' ? 'light' : 'dark')}
          >
            {resolvedTheme === 'dark' ? (
              <Sun className="size-4" aria-hidden />
            ) : (
              <Moon className="size-4" aria-hidden />
            )}
            Toggle theme
          </button>
          <button
            role="menuitem"
            className="flex w-full items-center gap-2 rounded-sm px-2 py-1.5 text-sm hover:bg-accent"
            onClick={signOut}
          >
            <LogOut className="size-4" aria-hidden />
            Sign out
          </button>
        </div>
      )}
    </div>
  );
}
