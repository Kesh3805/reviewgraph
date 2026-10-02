import { Controller, Get, Module, Param } from '@nestjs/common';
import { NestFactory } from '@nestjs/core';
import type { NestExpressApplication } from '@nestjs/platform-express';
import type { InMemorySpanExporter } from '@opentelemetry/sdk-trace-base';
import { Redis } from 'ioredis';
import { createServer, type Server, type Socket } from 'node:net';
import { Client } from 'pg';
import { AppModule } from '../../src/app.module';
import { configureApp } from '../../src/app.setup';
import { TracerService } from '../../src/telemetry/tracer.service';
import { PARENT_SPAN_ID, TRACE_ID } from './constants';

/** Minimal Postgres wire-protocol server: accepts any startup and answers every query. */
function fakePostgres(): Promise<Server> {
  const msg = (type: string, body = Buffer.alloc(0)) => {
    const out = Buffer.alloc(5 + body.length);
    out.write(type, 0, 'latin1');
    out.writeInt32BE(4 + body.length, 1);
    body.copy(out, 5);
    return out;
  };
  const ready = msg('Z', Buffer.from('I'));
  const handle = (socket: Socket) => {
    let started = false;
    let buffer = Buffer.alloc(0);
    socket.on('error', () => undefined);
    socket.on('data', (chunk) => {
      buffer = Buffer.concat([buffer, chunk]);
      for (;;) {
        if (!started) {
          if (buffer.length < 8) return;
          const len = buffer.readInt32BE(0);
          if (buffer.length < len) return;
          const code = buffer.readInt32BE(4);
          buffer = buffer.subarray(len);
          if (code === 80877103) {
            socket.write('N'); // no SSL
            continue;
          }
          started = true;
          const auth = Buffer.alloc(4);
          socket.write(Buffer.concat([msg('R', auth), ready]));
          continue;
        }
        if (buffer.length < 5) return;
        const len = buffer.readInt32BE(1);
        if (buffer.length < len + 1) return;
        const type = String.fromCharCode(buffer[0] as number);
        buffer = buffer.subarray(len + 1);
        if (type === 'P') socket.write(msg('1'));
        else if (type === 'B') socket.write(msg('2'));
        else if (type === 'D') socket.write(msg('n'));
        else if (type === 'E') socket.write(msg('C', Buffer.from('SELECT 0\0')));
        else if (type === 'S') socket.write(ready);
        else if (type === 'Q')
          socket.write(Buffer.concat([msg('C', Buffer.from('SELECT 0\0')), ready]));
        else if (type === 'X') socket.end();
      }
    });
  };
  return new Promise((resolve) => {
    const server = createServer(handle).listen(0, '127.0.0.1', () => resolve(server));
  });
}

/** Replies +OK to every RESP command it receives. */
function fakeRedis(): Promise<Server> {
  return new Promise((resolve) => {
    const server = createServer((socket) => {
      socket.on('error', () => undefined);
      socket.on('data', (data) => {
        const commands = data.toString().match(/^\*\d+\r\n/gm)?.length ?? 0;
        socket.write('+OK\r\n'.repeat(commands));
      });
    }).listen(0, '127.0.0.1', () => resolve(server));
  });
}

const ports = { pg: 0, redis: 0 };

@Controller('things')
class ThingsController {
  constructor(private readonly tracer: TracerService) {}

  @Get(':id')
  async get(@Param('id') id: string): Promise<{ id: string }> {
    const pg = new Client({
      host: '127.0.0.1',
      port: ports.pg,
      user: 'u',
      database: 'db',
      ssl: false,
    });
    await pg.connect();
    await pg.query('SELECT $1::text AS v', ['super-secret-param']);
    await pg.end();

    const redis = new Redis({ host: '127.0.0.1', port: ports.redis, enableReadyCheck: false });
    redis.on('error', () => undefined);
    await redis.set('some-key', 'super-secret-value');
    redis.disconnect();

    await this.tracer.withSpan('diff_analysis', { request_id: `r-${id}` }, () => 'ok');
    return { id };
  }
}

@Module({ imports: [AppModule], controllers: [ThingsController] })
class ScenarioModule {}

export async function runScenario(exporter: InMemorySpanExporter) {
  const [pgServer, redisServer] = await Promise.all([fakePostgres(), fakeRedis()]);
  ports.pg = (pgServer.address() as { port: number }).port;
  ports.redis = (redisServer.address() as { port: number }).port;

  const app = await NestFactory.create<NestExpressApplication>(ScenarioModule, {
    bodyParser: false,
    logger: false,
  });
  configureApp(app, 'http://localhost:3000');
  await app.listen(0, '127.0.0.1');
  const base = await app.getUrl();

  // fetch is not instrumented, so the traceparent below is the one the server sees.
  const traced = await fetch(`${base}/api/v1/things/42?token=hunter2`, {
    headers: {
      traceparent: `00-${TRACE_ID}-${PARENT_SPAN_ID}-01`,
      authorization: 'Bearer top-secret-token',
    },
  });
  const health = await fetch(`${base}/api/v1/health/live`);
  await new Promise((resolve) => setTimeout(resolve, 200));

  const spans = exporter.getFinishedSpans().map((s) => ({
    name: s.name,
    kind: s.kind,
    traceId: s.spanContext().traceId,
    spanId: s.spanContext().spanId,
    parentSpanId: s.parentSpanContext?.spanId,
    attributes: s.attributes,
    instrumentationScope: s.instrumentationScope.name,
  }));
  await app.close();
  pgServer.close();
  redisServer.close();
  return { tracedStatus: traced.status, healthStatus: health.status, spans };
}
