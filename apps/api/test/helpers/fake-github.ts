import { createServer, type IncomingMessage, type Server, type ServerResponse } from 'node:http';
import type { AddressInfo } from 'node:net';
import { generateKeyPairSync, type KeyObject } from 'node:crypto';
import { jwtVerify } from 'jose';

export interface FakeRequest {
  method: string;
  path: string;
  headers: IncomingMessage['headers'];
  body: unknown;
}

export interface FakeRoute {
  (req: FakeRequest, res: ServerResponse): void | Promise<void>;
}

export interface TestAppKey {
  privateKey: KeyObject;
  publicKey: KeyObject;
  pem: string;
}

export function generateAppKey(): TestAppKey {
  const { privateKey, publicKey } = generateKeyPairSync('rsa', { modulusLength: 2048 });
  return {
    privateKey,
    publicKey,
    pem: privateKey.export({ type: 'pkcs1', format: 'pem' }).toString(),
  };
}

/**
 * Minimal fake GitHub API (DEV-005 stand-in for unit tests): validates the App JWT on token
 * minting, issues `ghs_<n>` tokens and lets a test add routes and revoke tokens.
 */
export class FakeGithub {
  readonly requests: FakeRequest[] = [];
  readonly revoked = new Set<string>();
  readonly routes = new Map<string, FakeRoute>();
  mintCount = 0;
  mintDelayMs = 0;
  /** Seconds until minted tokens expire. */
  tokenLifetimeSeconds = 3600;
  /** When set, minting answers with this status instead of a token. */
  mintFailure?: { status: number; headers?: Record<string, string>; body?: unknown };
  appPermissions: Record<string, string> = {
    contents: 'read',
    pull_requests: 'write',
    checks: 'write',
    metadata: 'read',
    issues: 'read',
  };
  private server?: Server;

  constructor(private readonly publicKey: KeyObject) {}

  get url(): string {
    const { port } = this.server!.address() as AddressInfo;
    return `http://127.0.0.1:${port}`;
  }

  async start(): Promise<void> {
    this.server = createServer((req, res) => {
      void this.handle(req, res);
    });
    await new Promise<void>((resolve) => this.server!.listen(0, '127.0.0.1', resolve));
  }

  async stop(): Promise<void> {
    this.server?.closeAllConnections();
    await new Promise<void>((resolve) => this.server?.close(() => resolve()));
  }

  route(method: string, path: string, handler: FakeRoute): void {
    this.routes.set(`${method} ${path}`, handler);
  }

  private async handle(req: IncomingMessage, res: ServerResponse): Promise<void> {
    const chunks: Buffer[] = [];
    for await (const chunk of req) chunks.push(chunk as Buffer);
    const raw = Buffer.concat(chunks).toString('utf8');
    const path = (req.url ?? '/').split('?')[0] ?? '/';
    const fake: FakeRequest = {
      method: req.method ?? 'GET',
      path,
      headers: req.headers,
      body: raw ? (JSON.parse(raw) as unknown) : undefined,
    };
    this.requests.push(fake);
    const json = (status: number, body: unknown, headers: Record<string, string> = {}) => {
      res.writeHead(status, { 'content-type': 'application/json', ...headers });
      res.end(JSON.stringify(body));
    };

    const mint = /^\/app\/installations\/(\d+)\/access_tokens$/.exec(path);
    if (mint && fake.method === 'POST') {
      if (!(await this.validAppJwt(req.headers.authorization))) {
        return json(401, { message: 'bad jwt' });
      }
      if (this.mintDelayMs) await new Promise((r) => setTimeout(r, this.mintDelayMs));
      if (this.mintFailure) {
        return json(
          this.mintFailure.status,
          this.mintFailure.body ?? { message: 'failure' },
          this.mintFailure.headers,
        );
      }
      this.mintCount += 1;
      return json(201, {
        token: `ghs_fake${this.mintCount}_${'x'.repeat(24)}`,
        expires_at: new Date(Date.now() + this.tokenLifetimeSeconds * 1000).toISOString(),
        permissions: (fake.body as { permissions?: unknown } | undefined)?.permissions ?? {},
      });
    }
    if (path === '/app' && fake.method === 'GET') {
      if (!(await this.validAppJwt(req.headers.authorization))) {
        return json(401, { message: 'bad jwt' });
      }
      return json(200, { id: 1, slug: 'reviewgraph', permissions: this.appPermissions });
    }
    const custom = this.routes.get(`${fake.method} ${path}`);
    if (custom) {
      const token = /^token (\S+)$/.exec(req.headers.authorization ?? '')?.[1];
      if (!token || this.revoked.has(token)) return json(401, { message: 'Bad credentials' });
      return void (await custom(fake, res));
    }
    json(404, { message: 'Not Found' });
  }

  private async validAppJwt(header: string | undefined): Promise<boolean> {
    const token = /^(?:Bearer|bearer) (\S+)$/.exec(header ?? '')?.[1];
    if (!token) return false;
    try {
      await jwtVerify(token, this.publicKey, { algorithms: ['RS256'] });
      return true;
    } catch {
      return false;
    }
  }
}
