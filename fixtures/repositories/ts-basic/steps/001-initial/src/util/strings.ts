export const MAX_LENGTH = 255;
export let counter = 0;
var legacy = "x";
const API_TOKEN = "super-secret-token-value";
const { alpha, beta: [gamma] } = { alpha: 1, beta: [2] };

export const shout = (s: string): string => s.toUpperCase();
export const whisper = function (s: string) {
  return s.toLowerCase();
};

export function pad(s: string, n: number): string;
export function pad(s: string, n: number, fill: string): string;
export function pad(s: string, n: number, fill = " "): string {
  return s.padEnd(n, fill);
}
