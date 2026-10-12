import { createPrivateKey, type KeyObject } from 'node:crypto';
import { readFileSync } from 'node:fs';

/**
 * Where a secret comes from (SEC-005). The App private key is read once at boot through one of
 * these, parsed into a `KeyObject`, and the raw string is dropped: nothing else in the process
 * holds the PEM. Errors never contain secret material, only the source description.
 */
export interface SecretSource {
  /** `env:NAME`, `file:NAME` or `secret-manager:<name>`: safe to log. */
  readonly description: string;
  load(): Promise<string>;
}

/** A source that can be read during synchronous module construction (env and file). */
export interface SyncSecretSource extends SecretSource {
  loadSync(): string;
}

export class SecretSourceError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'SecretSourceError';
  }
}

/** The secret value is the content of an environment variable. */
export class EnvSecretSource implements SyncSecretSource {
  readonly description: string;

  constructor(
    private readonly name: string,
    private readonly env: NodeJS.ProcessEnv | Record<string, string | undefined> = process.env,
  ) {
    this.description = `env:${name}`;
  }

  loadSync(): string {
    const value = this.env[this.name];
    if (!value) throw new SecretSourceError(`${this.description} is not set`);
    return value;
  }

  load(): Promise<string> {
    try {
      return Promise.resolve(this.loadSync());
    } catch (err) {
      return Promise.reject(err as Error);
    }
  }
}

/** The secret value is the content of the file an environment variable points to. */
export class FileSecretSource implements SyncSecretSource {
  readonly description: string;

  constructor(
    private readonly path: string,
    envName = 'path',
  ) {
    this.description = `file:${envName}`;
  }

  loadSync(): string {
    try {
      return readFileSync(this.path, 'utf8');
    } catch {
      // Report the source only: fs errors carry the path and errno, which is noise here.
      throw new SecretSourceError(`${this.description} could not be read`);
    }
  }

  load(): Promise<string> {
    try {
      return Promise.resolve(this.loadSync());
    } catch (err) {
      return Promise.reject(err as Error);
    }
  }
}

/**
 * Adapter boundary for a cloud secret manager (GCP Secret Manager, Vault, ...). No real KMS client
 * ships in the MVP; deployments provide one by implementing this interface.
 */
export interface SecretManagerClient {
  access(name: string): Promise<string>;
}

export class SecretManagerSource implements SecretSource {
  readonly description: string;

  constructor(
    private readonly client: SecretManagerClient,
    private readonly name: string,
  ) {
    this.description = `secret-manager:${name}`;
  }

  async load(): Promise<string> {
    try {
      return await this.client.access(this.name);
    } catch {
      throw new SecretSourceError(`${this.description} could not be accessed`);
    }
  }
}

export interface PrivateKeyEnv {
  GITHUB_APP_PRIVATE_KEY?: string;
  GITHUB_APP_PRIVATE_KEY_FILE?: string;
}

/** `GITHUB_APP_PRIVATE_KEY` (content) wins over `GITHUB_APP_PRIVATE_KEY_FILE` (path). */
export function privateKeySource(env: PrivateKeyEnv): SyncSecretSource {
  if (env.GITHUB_APP_PRIVATE_KEY) {
    return new EnvSecretSource('GITHUB_APP_PRIVATE_KEY', {
      GITHUB_APP_PRIVATE_KEY: env.GITHUB_APP_PRIVATE_KEY,
    });
  }
  if (env.GITHUB_APP_PRIVATE_KEY_FILE) {
    return new FileSecretSource(env.GITHUB_APP_PRIVATE_KEY_FILE, 'GITHUB_APP_PRIVATE_KEY_FILE');
  }
  throw new SecretSourceError('GitHub App private key is not configured');
}

/** RS256 needs an RSA key; GitHub issues 2048-bit keys, anything weaker is refused. */
export const MIN_RSA_MODULUS_BITS = 2048;

/**
 * Parses and validates an App private key. Literal `\n` sequences (single-line env values) are
 * expanded. Failure messages never include key material.
 */
export function parseAppPrivateKey(pem: string): KeyObject {
  let key: KeyObject;
  try {
    key = createPrivateKey(pem.replace(/\\n/g, '\n'));
  } catch {
    throw new SecretSourceError('GitHub App private key could not be parsed (expected an RSA PEM)');
  }
  if (key.asymmetricKeyType !== 'rsa') {
    throw new SecretSourceError('GitHub App private key could not be parsed (expected an RSA PEM)');
  }
  const bits = key.asymmetricKeyDetails?.modulusLength ?? 0;
  if (bits < MIN_RSA_MODULUS_BITS) {
    throw new SecretSourceError(
      `GitHub App private key is too weak (${bits} bits, at least ${MIN_RSA_MODULUS_BITS} required)`,
    );
  }
  return key;
}

/** Loads, validates and returns the key object; the PEM string goes out of scope here. */
export async function loadAppPrivateKey(source: SecretSource): Promise<KeyObject> {
  return parseAppPrivateKey(await source.load());
}

/**
 * Removes the inline private key from `process.env` once it has been parsed, so environment dumps
 * (crash reporters, `/proc/<pid>/environ` readers in the same container, debug endpoints) cannot
 * show it. The file variant only holds a path and stays.
 */
export function scrubPrivateKeyFromEnv(env: NodeJS.ProcessEnv = process.env): void {
  delete env.GITHUB_APP_PRIVATE_KEY;
}
