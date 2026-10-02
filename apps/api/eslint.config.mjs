import base from '@reviewgraph/config/eslint';

export default [...base, { ignores: ['jest.config.ts', 'test/telemetry/run.cjs'] }];
