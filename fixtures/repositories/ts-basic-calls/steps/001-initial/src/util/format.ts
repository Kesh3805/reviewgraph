export default function formatNumber(value: number): string {
  return value.toFixed(2);
}

export const formatLabel = (label: string, value: number): string => `${label}: ${value}`;
