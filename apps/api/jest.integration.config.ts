import type { Config } from 'jest';

/** Integration tests run against the dev Postgres and Redis (see integration/env.ts). */
const config: Config = {
  rootDir: '.',
  testEnvironment: 'node',
  testMatch: ['<rootDir>/integration/**/*.int.spec.ts'],
  transform: { '^.+\.ts$': ['ts-jest', { tsconfig: '<rootDir>/tsconfig.json' }] },
  moduleFileExtensions: ['ts', 'js', 'json'],
  setupFiles: ['<rootDir>/integration/env.ts'],
  // One shared database: files run serially, and each test creates its own organizations.
  maxWorkers: 1,
  testTimeout: 30_000,
};

export default config;
