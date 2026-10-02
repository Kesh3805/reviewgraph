/* eslint-disable */
// Runs the telemetry scenario in a plain Node process. Jest sandboxes `require`, which bypasses
// the module hooks auto-instrumentation relies on, so these tests spawn this script instead.
// A tiny TypeScript require hook lets it load the sources without a build step.
const fs = require('node:fs');
const ts = require('typescript');

require.extensions['.ts'] = (module, filename) => {
  const { outputText } = ts.transpileModule(fs.readFileSync(filename, 'utf8'), {
    fileName: filename,
    compilerOptions: {
      module: ts.ModuleKind.CommonJS,
      target: ts.ScriptTarget.ES2023,
      experimentalDecorators: true,
      emitDecoratorMetadata: true,
      esModuleInterop: true,
      isolatedModules: true,
    },
  });
  module._compile(outputText, filename);
};

Object.assign(process.env, {
  NODE_ENV: 'test',
  PORT: '8080',
  DATABASE_URL: 'postgres://u:p@127.0.0.1:1/db',
  REDIS_URL: 'redis://127.0.0.1:1',
  ENGINE_INTERNAL_URL: 'http://127.0.0.1:1',
  SERVICE_JWT_SECRET: 's'.repeat(32),
  SESSION_JWT_SECRET: 't'.repeat(32),
  TOKEN_CACHE_KEY: Buffer.alloc(32, 1).toString('base64'),
  WEB_ORIGIN: 'http://localhost:3000',
  GITHUB_ENABLED: 'false',
});

require('reflect-metadata');
const { InMemorySpanExporter, SimpleSpanProcessor } = require('@opentelemetry/sdk-trace-base');
const { startTelemetry } = require('../../src/telemetry/sdk');

const exporter = new InMemorySpanExporter();
// Telemetry must start before http, pg or ioredis are required by the scenario.
startTelemetry(
  {
    enabled: true,
    endpoint: 'http://127.0.0.1:1',
    headers: {},
    serviceName: 'reviewgraph-api-test',
    serviceVersion: '0.0.0',
    environment: 'test',
  },
  {
    spanProcessors: [new SimpleSpanProcessor(exporter)],
    logRecordProcessors: [],
    metricReaders: [],
  },
);

const { runScenario } = require('./fixtures.ts');
runScenario(exporter).then(
  (result) => {
    process.stdout.write(`RESULT:${JSON.stringify(result)}\n`);
    process.exit(0);
  },
  (err) => {
    console.error(err);
    process.exit(1);
  },
);
