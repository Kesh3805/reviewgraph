import type { Severity } from './dashboard';

/** Shared severity colour tokens (Tailwind classes) used by every screen. */
export const SEVERITY_TOKENS: Record<Severity, { label: string; dot: string; text: string }> = {
  critical: { label: 'Critical', dot: 'bg-red-600', text: 'text-red-700 dark:text-red-400' },
  high: { label: 'High', dot: 'bg-orange-500', text: 'text-orange-700 dark:text-orange-400' },
  medium: { label: 'Medium', dot: 'bg-yellow-500', text: 'text-yellow-700 dark:text-yellow-400' },
  low: { label: 'Low', dot: 'bg-blue-500', text: 'text-blue-700 dark:text-blue-400' },
  info: { label: 'Info', dot: 'bg-slate-400', text: 'text-slate-600 dark:text-slate-400' },
};

export const SEVERITY_ORDER: Severity[] = ['critical', 'high', 'medium', 'low', 'info'];
