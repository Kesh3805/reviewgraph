import type { Severity } from './types';

const EMOJI: Record<Severity, string> = {
  critical: '\u{1F534}',
  high: '\u{1F534}',
  medium: '\u{1F7E0}',
  low: '\u{1F7E1}',
  info: '⚪',
};

export const severityEmoji = (s: Severity): string => EMOJI[s];

export const severityLabel = (s: Severity): string => s.charAt(0).toUpperCase() + s.slice(1);

/** Highest first, for grouping and ordering. */
export const SEVERITY_ORDER: readonly Severity[] = ['critical', 'high', 'medium', 'low', 'info'];
