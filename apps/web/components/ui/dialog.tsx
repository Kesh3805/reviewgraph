'use client';

import { X } from 'lucide-react';
import { useEffect, useId, type ReactNode } from 'react';
import { cn } from '@/lib/utils';

/** A minimal modal dialog: rendered only while open, closed with Escape or the close button. */
export function Dialog({
  open,
  onClose,
  title,
  description,
  children,
  className,
}: {
  open: boolean;
  onClose: () => void;
  title: string;
  description?: string;
  children: ReactNode;
  className?: string;
}) {
  const titleId = useId();
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [open, onClose]);

  if (!open) return null;
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4">
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        className={cn(
          'max-h-[90vh] w-full max-w-lg overflow-y-auto rounded-xl border bg-background p-6 shadow-lg',
          className,
        )}
      >
        <div className="mb-4 flex items-start justify-between gap-4">
          <div className="space-y-1">
            <h2 id={titleId} className="text-lg font-semibold">
              {title}
            </h2>
            {description && <p className="text-sm text-muted-foreground">{description}</p>}
          </div>
          <button
            type="button"
            aria-label="Close"
            className="rounded-sm p-1 text-muted-foreground hover:bg-accent"
            onClick={onClose}
          >
            <X className="size-4" aria-hidden />
          </button>
        </div>
        {children}
      </div>
    </div>
  );
}
