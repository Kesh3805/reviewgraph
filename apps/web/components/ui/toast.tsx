'use client';

import { X } from 'lucide-react';
import { createContext, useCallback, useContext, useMemo, useState, type ReactNode } from 'react';
import { cn } from '@/lib/utils';

export interface Toast {
  id: number;
  message: string;
  variant: 'default' | 'error';
}

interface ToastApi {
  show: (message: string, variant?: Toast['variant']) => void;
}

const ToastContext = createContext<ToastApi>({ show: () => {} });

const TOAST_MS = 6000;

/** Minimal toast stack (bottom right), announced politely to assistive technology. */
export function ToastProvider({ children }: { children: ReactNode }) {
  const [toasts, setToasts] = useState<Toast[]>([]);
  const dismiss = useCallback((id: number) => setToasts((t) => t.filter((x) => x.id !== id)), []);
  const show = useCallback(
    (message: string, variant: Toast['variant'] = 'default') => {
      const id = Date.now() + Math.random();
      setToasts((t) => [...t, { id, message, variant }]);
      setTimeout(() => dismiss(id), TOAST_MS);
    },
    [dismiss],
  );
  const api = useMemo(() => ({ show }), [show]);

  return (
    <ToastContext.Provider value={api}>
      {children}
      <div
        aria-live="polite"
        className="fixed right-4 bottom-4 z-50 flex w-80 flex-col gap-2"
        data-testid="toasts"
      >
        {toasts.map((t) => (
          <div
            key={t.id}
            role={t.variant === 'error' ? 'alert' : 'status'}
            className={cn(
              'flex items-start justify-between gap-2 rounded-md border bg-popover p-3 text-sm shadow-md',
              t.variant === 'error' && 'border-destructive/40 text-destructive',
            )}
          >
            <span>{t.message}</span>
            <button type="button" aria-label="Dismiss" onClick={() => dismiss(t.id)}>
              <X className="size-4" aria-hidden />
            </button>
          </div>
        ))}
      </div>
    </ToastContext.Provider>
  );
}

export function useToast(): ToastApi {
  return useContext(ToastContext);
}
