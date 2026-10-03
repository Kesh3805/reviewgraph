import base from '@reviewgraph/config/eslint';

const OCTOKIT = {
  group: ['@octokit/*'],
  message: 'Octokit may only be imported under src/providers/github/** (use the provider ports).',
};

export default [
  ...base,
  {
    ignores: [
      'jest.config.ts',
      'jest.integration.config.ts',
      'test/telemetry/run.cjs',
      'src/db/generated.ts',
    ],
  },
  // Parameterized queries only: `sql.raw` is allowed solely inside src/db/ (API-002).
  {
    files: ['src/**/*.ts', 'test/**/*.ts'],
    ignores: ['src/db/**'],
    rules: {
      'no-restricted-syntax': [
        'error',
        {
          selector: "CallExpression[callee.object.name='sql'][callee.property.name='raw']",
          message: 'sql.raw is only allowed under src/db/. Use parameterized queries.',
        },
      ],
    },
  },
  // Provider SDKs stay inside the GitHub provider module.
  {
    files: ['src/**/*.ts'],
    ignores: ['src/providers/github/**'],
    rules: { 'no-restricted-imports': ['error', { patterns: [OCTOKIT] }] },
  },
  // Domain modules may import only providers/ports (invariant 4, API-006).
  {
    files: ['src/{reviews,publisher,findings,repositories}/**/*.ts'],
    rules: {
      'no-restricted-imports': [
        'error',
        {
          patterns: [
            OCTOKIT,
            {
              regex: 'providers/(?!ports(/|$))',
              message:
                'Domain modules may import only providers/ports, never a provider implementation.',
            },
          ],
        },
      ],
    },
  },
];
