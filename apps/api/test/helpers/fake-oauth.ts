import { createServer, type IncomingMessage, type Server, type ServerResponse } from 'node:http';
import type { AddressInfo } from 'node:net';

export const FAKE_USER_TOKEN = 'gho_fakeUserTokenForTests0123456789';

export interface FakeInstallation {
  id: number;
  account: { login: string; type: 'User' | 'Organization' };
}

/**
 * Fake GitHub OAuth + user endpoints (DEV-005 stand-in): `POST /login/oauth/access_token`,
 * `GET /user` and `GET /user/installations`. Codes are single use, like GitHub.
 */
export class FakeOAuth {
  readonly validCodes = new Set<string>();
  readonly tokenRequests: Record<string, string>[] = [];
  user = {
    id: 424242,
    login: 'octo-user',
    name: 'Octo User',
    email: null as string | null,
    avatar_url: 'https://avatars.example/u/1',
  };
  installations: FakeInstallation[] = [];
  /** When set, `/user` and `/user/installations` answer this status. */
  apiFailure?: number;
  private server?: Server;

  get url(): string {
    return `http://127.0.0.1:${(this.server!.address() as AddressInfo).port}`;
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

  newCode(): string {
    const code = `code-${Math.random().toString(36).slice(2)}`;
    this.validCodes.add(code);
    return code;
  }

  private async handle(req: IncomingMessage, res: ServerResponse): Promise<void> {
    const chunks: Buffer[] = [];
    for await (const chunk of req) chunks.push(chunk as Buffer);
    const raw = Buffer.concat(chunks).toString('utf8');
    const url = new URL(req.url ?? '/', 'http://x');
    const json = (status: number, body: unknown): void => {
      res.writeHead(status, { 'content-type': 'application/json' });
      res.end(JSON.stringify(body));
    };

    if (req.method === 'POST' && url.pathname === '/login/oauth/access_token') {
      const body = JSON.parse(raw) as Record<string, string>;
      this.tokenRequests.push(body);
      if (!body.code || !this.validCodes.delete(body.code) || !body.code_verifier) {
        json(200, { error: 'bad_verification_code' });
        return;
      }
      json(200, { access_token: FAKE_USER_TOKEN, token_type: 'bearer' });
      return;
    }
    if (req.headers.authorization !== `Bearer ${FAKE_USER_TOKEN}`) {
      json(401, { message: 'Bad credentials' });
      return;
    }
    if (this.apiFailure) {
      json(this.apiFailure, { message: 'boom' });
      return;
    }
    if (url.pathname === '/user') {
      json(200, this.user);
      return;
    }
    if (url.pathname === '/user/installations') {
      const page = Number(url.searchParams.get('page') ?? '1');
      const per = Number(url.searchParams.get('per_page') ?? '30');
      const slice = this.installations.slice((page - 1) * per, page * per);
      json(200, { total_count: this.installations.length, installations: slice });
      return;
    }
    json(404, { message: 'not found' });
  }
}
