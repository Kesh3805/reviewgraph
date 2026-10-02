/**
 * Telemetry entry point. Must run before anything imports `http` or `pg`, so it is loaded with
 * `node --import ./dist/telemetry/instrumentation.js dist/main.js` rather than from `main.ts`.
 * Importing it starts the SDK (once per process) from the environment; with no OTLP endpoint
 * or `RG_OTEL_ENABLED=false` it does nothing.
 */
import { ZodError } from 'zod';
import { loadDotenv } from '../config/config.module';
import { parseTelemetryConfig } from './config';
import { startTelemetry } from './sdk';

const EX_CONFIG = 78;

loadDotenv();
try {
  startTelemetry(parseTelemetryConfig(process.env));
} catch (err) {
  if (err instanceof ZodError) {
    // Names the variable and rule only; values are never printed.
    console.error(
      `Invalid telemetry configuration:\n${err.issues
        .map((i) => `  - ${i.path.join('.')}: ${i.message}`)
        .join('\n')}`,
    );
    process.exit(EX_CONFIG);
  }
  throw err;
}
