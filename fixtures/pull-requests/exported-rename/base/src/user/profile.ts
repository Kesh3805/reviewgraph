import { formatName } from '../util/names';

export function title(first: string, last: string): string {
  return formatName(first, last);
}
