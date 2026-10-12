import { MAX_ATTEMPTS } from './limits';

export function canRetry(attempts: number): boolean {
  return attempts < MAX_ATTEMPTS;
}
