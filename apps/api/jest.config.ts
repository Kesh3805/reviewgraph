import type { Config } from 'jest';

const config: Config = {
  rootDir: '.',
  testEnvironment: 'node',
  testMatch: ['<rootDir>/test/**/*.spec.ts'],
  transform: { '^.+\.ts$': ['ts-jest', { tsconfig: '<rootDir>/tsconfig.json' }] },
  moduleFileExtensions: ['ts', 'js', 'json'],
  // The first suite pays the cold ts-jest compile of the whole AppModule.
  testTimeout: 30_000,
  // CI-003: JUnit and advisory coverage are opt-in through the environment so local runs stay fast.
  reporters: process.env.JUNIT_DIR
    ? [
        'default',
        [
          '<rootDir>/../../scripts/ci/jest-junit-reporter.cjs',
          { output: `${process.env.JUNIT_DIR}/api.xml` },
        ],
      ]
    : ['default'],
  collectCoverage: process.env.COVERAGE === '1',
  coverageReporters: ['json-summary', 'text-summary'],
  coverageDirectory: process.env.COVERAGE_DIR ?? '<rootDir>/coverage',
};

export default config;
