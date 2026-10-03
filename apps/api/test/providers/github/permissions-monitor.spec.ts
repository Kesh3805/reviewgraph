import { Logger } from '@nestjs/common';
import RedisMock from 'ioredis-mock';
import request from 'supertest';
import { gaugeValue } from '../../../src/common/metrics';
import { GithubAppAuth } from '../../../src/providers/github/app-auth.service';
import { APP_PERMISSIONS } from '../../../src/providers/github/app-manifest';
import { GithubPermissionsMonitor } from '../../../src/providers/github/permissions-monitor';
import { InstallationTokenCache } from '../../../src/providers/github/token-cache';
import { createTestApp } from '../../helpers';
import { FakeGithub, generateAppKey } from '../../helpers/fake-github';

const key = generateAppKey();

describe('GitHub App permission check', () => {
  let github: FakeGithub;
  let auth: GithubAppAuth;

  beforeAll(async () => {
    github = new FakeGithub(key.publicKey);
    await github.start();
    auth = new GithubAppAuth({
      appId: '1',
      privateKey: key.privateKey,
      apiUrl: github.url,
      cache: new InstallationTokenCache(new RedisMock(), Buffer.alloc(32, 5)),
      retries: false,
    });
  });
  afterAll(async () => {
    await github.stop();
  });
  beforeEach(() => {
    github.appPermissions = { ...APP_PERMISSIONS };
  });

  it('accepts exactly the manifest permissions', async () => {
    const monitor = new GithubPermissionsMonitor(auth);
    expect(monitor.canPublish()).toBe(false);
    expect(await monitor.verify()).toBe('valid');
    expect(monitor.canPublish()).toBe(true);
    expect(gaugeValue('github_app_permissions_valid')).toBe(1);
  });

  it('boot_rejects_contents_write_permission', async () => {
    const errors: string[] = [];
    const spy = jest.spyOn(Logger.prototype, 'error').mockImplementation((m: unknown) => {
      errors.push(String(m));
    });
    try {
      github.appPermissions = { ...APP_PERMISSIONS, contents: 'write' };
      const monitor = new GithubPermissionsMonitor(auth);
      await monitor.onApplicationBootstrap();
      expect(monitor.status()).toBe('invalid');
      expect(monitor.canPublish()).toBe(false);
      expect(gaugeValue('github_app_permissions_valid')).toBe(0);
      expect(errors.join('\n')).toMatch(/CRITICAL.*contents:write/);

      github.appPermissions = { ...APP_PERMISSIONS, administration: 'write' };
      expect(await monitor.verify()).toBe('invalid');
      // Recovers once the App is fixed.
      github.appPermissions = { ...APP_PERMISSIONS };
      expect(await monitor.verify()).toBe('valid');
    } finally {
      spy.mockRestore();
    }
  });

  it('stays unknown, with publishing off, when GitHub cannot be reached', async () => {
    const spy = jest.spyOn(Logger.prototype, 'error').mockImplementation(() => undefined);
    try {
      const dead = new GithubAppAuth({
        appId: '1',
        privateKey: key.privateKey,
        apiUrl: 'http://127.0.0.1:1',
        cache: new InstallationTokenCache(new RedisMock(), Buffer.alloc(32, 5)),
        retries: false,
      });
      const monitor = new GithubPermissionsMonitor(dead);
      expect(await monitor.verify()).toBe('unknown');
      expect(monitor.canPublish()).toBe(false);
    } finally {
      spy.mockRestore();
    }
  });

  it('is inert when GitHub is disabled', async () => {
    const monitor = new GithubPermissionsMonitor(null);
    expect(monitor.enabled).toBe(false);
    await monitor.onApplicationBootstrap();
    expect(monitor.canPublish()).toBe(false);
  });

  it('/health/ready reports github_permissions: invalid with 503', async () => {
    const app = await createTestApp(undefined, [], {
      configure: (builder) =>
        builder
          .overrideProvider(GithubPermissionsMonitor)
          .useValue({ enabled: true, status: () => 'invalid' }),
    });
    await app.init();
    try {
      const res = await request(app.getHttpServer()).get('/api/v1/health/ready');
      expect(res.status).toBe(503);
      expect(res.body.checks.github_permissions).toBe('invalid');
    } finally {
      await app.close();
    }
  });
});
