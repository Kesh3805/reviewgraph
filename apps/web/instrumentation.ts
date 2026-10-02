/**
 * Server-side OpenTelemetry for Next (OBS-002 configuration). Exports spans to the same OTLP
 * endpoint as the API and propagates W3C `traceparent` on calls to the API. With no endpoint
 * configured this is a no-op.
 */
export async function register(): Promise<void> {
  if (!process.env.OTEL_EXPORTER_OTLP_ENDPOINT || process.env.RG_OTEL_ENABLED === 'false') return;
  const { registerOTel } = await import('@vercel/otel');
  registerOTel({
    serviceName: process.env.OTEL_SERVICE_NAME ?? 'reviewgraph-web',
    instrumentationConfig: {
      fetch: {
        propagateContextUrls: [process.env.API_INTERNAL_URL ?? 'http://127.0.0.1:8080'],
      },
    },
  });
}
