import { defineConfig } from 'vitest/config';
import { fileURLToPath } from 'node:url';

export default defineConfig({
  resolve: { alias: { '@': fileURLToPath(new URL('.', import.meta.url)) } },
  oxc: { jsx: { runtime: 'automatic' } },
  test: {
    environment: 'node',
    testTimeout: 30_000,
    include: ['test/**/*.test.{ts,tsx}'],
    // CI-003: JUnit output is opt-in through the environment.
    reporters: process.env.JUNIT_DIR ? ['default', 'junit'] : ['default'],
    outputFile: process.env.JUNIT_DIR ? { junit: `${process.env.JUNIT_DIR}/web.xml` } : undefined,
  },
});
