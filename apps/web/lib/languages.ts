const EXTENSION_LANG: Record<string, string> = {
  ts: 'typescript',
  tsx: 'tsx',
  js: 'javascript',
  jsx: 'jsx',
  mjs: 'javascript',
  cjs: 'javascript',
  json: 'json',
  yaml: 'yaml',
  yml: 'yaml',
  rs: 'rust',
  py: 'python',
  go: 'go',
  java: 'java',
  kt: 'kotlin',
  sql: 'sql',
  md: 'markdown',
  css: 'css',
  html: 'html',
  sh: 'bash',
  toml: 'toml',
};

/** Shiki language id for a file path, or null to render plain text. */
export function languageForPath(path: string): string | null {
  const ext = path.split('.').pop()?.toLowerCase() ?? '';
  return EXTENSION_LANG[ext] ?? null;
}
