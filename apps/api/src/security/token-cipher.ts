import { createCipheriv, createDecipheriv, randomBytes } from 'node:crypto';

/**
 * AES-256-GCM cipher for cached provider tokens (SEC-005).
 *
 * Stored value: JSON `{"kid","iv","ciphertext","tag"}` with base64url fields. The `kid` selects the
 * key on decrypt, so `TOKEN_CACHE_KEY` can rotate: new values are written with the current key,
 * values written under a previous key (`TOKEN_CACHE_KEY_PREVIOUS`) still decrypt until they
 * expire. The caller passes associated data (the installation id and cache key), so a value copied
 * to another installation's key fails authentication.
 */

const KEY_BYTES = 32;
const IV_BYTES = 12;
const KID = /^[A-Za-z0-9._-]{1,32}$/;
export const DEFAULT_TOKEN_KEY_ID = 'k1';

export interface TokenCipherKey {
  kid: string;
  key: Buffer;
}

export interface SealedToken {
  kid: string;
  iv: string;
  ciphertext: string;
  tag: string;
}

/** Reasons never include key material or plaintext. */
export class TokenCipherError extends Error {
  constructor(
    readonly reason: 'config' | 'malformed' | 'unknown_kid' | 'authentication',
    message: string,
  ) {
    super(message);
    this.name = 'TokenCipherError';
  }
}

/** Parses `base64` (kid `k1`) or `kid:base64`. */
export function parseTokenKey(spec: string, defaultKid = DEFAULT_TOKEN_KEY_ID): TokenCipherKey {
  const trimmed = spec.trim();
  const idx = trimmed.indexOf(':');
  const kid = idx > 0 ? trimmed.slice(0, idx) : defaultKid;
  const encoded = idx > 0 ? trimmed.slice(idx + 1) : trimmed;
  if (!KID.test(kid)) throw new TokenCipherError('config', 'token cache key id is invalid');
  const key = Buffer.from(encoded, 'base64');
  if (key.length !== KEY_BYTES) {
    throw new TokenCipherError('config', `token cache key "${kid}" must be 32 bytes, base64`);
  }
  return { kid, key };
}

/** True when `spec` parses as a single token cache key (env validation). */
export function isValidTokenKeySpec(spec: string): boolean {
  try {
    parseTokenKey(spec);
    return true;
  } catch {
    return false;
  }
}

/** True when `spec` is a comma list of `kid:base64` keys (env validation). */
export function isValidPreviousKeysSpec(spec: string): boolean {
  try {
    parsePreviousKeys(spec);
    return true;
  } catch {
    return false;
  }
}

function parsePreviousKeys(spec: string): TokenCipherKey[] {
  return spec
    .split(',')
    .map((entry) => entry.trim())
    .filter(Boolean)
    .map((entry) => {
      if (entry.indexOf(':') <= 0) {
        throw new TokenCipherError('config', 'previous token cache keys must look like kid:base64');
      }
      return parseTokenKey(entry);
    });
}

export class TokenCipher {
  private readonly current: TokenCipherKey;
  private readonly byKid = new Map<string, Buffer>();

  constructor(current: TokenCipherKey, previous: TokenCipherKey[] = []) {
    for (const k of [current, ...previous]) {
      if (k.key.length !== KEY_BYTES) {
        throw new TokenCipherError('config', `token cache key "${k.kid}" must be 32 bytes`);
      }
      if (this.byKid.has(k.kid)) {
        throw new TokenCipherError('config', `duplicate token cache key id "${k.kid}"`);
      }
      this.byKid.set(k.kid, k.key);
    }
    this.current = current;
  }

  /** `TOKEN_CACHE_KEY` (current) plus optional `TOKEN_CACHE_KEY_PREVIOUS` (decrypt only). */
  static fromConfig(current: string, previous?: string): TokenCipher {
    return new TokenCipher(parseTokenKey(current), previous ? parsePreviousKeys(previous) : []);
  }

  get currentKid(): string {
    return this.current.kid;
  }

  seal(plaintext: string, aad: string): SealedToken {
    const iv = randomBytes(IV_BYTES);
    const cipher = createCipheriv('aes-256-gcm', this.current.key, iv);
    cipher.setAAD(Buffer.from(aad, 'utf8'));
    const ciphertext = Buffer.concat([cipher.update(plaintext, 'utf8'), cipher.final()]);
    return {
      kid: this.current.kid,
      iv: iv.toString('base64url'),
      ciphertext: ciphertext.toString('base64url'),
      tag: cipher.getAuthTag().toString('base64url'),
    };
  }

  open(sealed: SealedToken, aad: string): string {
    const key = this.byKid.get(sealed.kid);
    if (!key) throw new TokenCipherError('unknown_kid', 'token cache key id is not configured');
    try {
      const decipher = createDecipheriv('aes-256-gcm', key, Buffer.from(sealed.iv, 'base64url'));
      decipher.setAAD(Buffer.from(aad, 'utf8'));
      decipher.setAuthTag(Buffer.from(sealed.tag, 'base64url'));
      return Buffer.concat([
        decipher.update(Buffer.from(sealed.ciphertext, 'base64url')),
        decipher.final(),
      ]).toString('utf8');
    } catch {
      throw new TokenCipherError('authentication', 'token cache value failed authentication');
    }
  }

  /** Serialized form stored in Redis. */
  encrypt(plaintext: string, aad: string): string {
    return JSON.stringify(this.seal(plaintext, aad));
  }

  decrypt(stored: string, aad: string): string {
    let sealed: unknown;
    try {
      sealed = JSON.parse(stored);
    } catch {
      throw new TokenCipherError('malformed', 'token cache value is not a sealed token');
    }
    if (!isSealed(sealed)) {
      throw new TokenCipherError('malformed', 'token cache value is not a sealed token');
    }
    return this.open(sealed, aad);
  }
}

function isSealed(v: unknown): v is SealedToken {
  if (!v || typeof v !== 'object') return false;
  const o = v as Record<string, unknown>;
  return ['kid', 'iv', 'ciphertext', 'tag'].every((k) => typeof o[k] === 'string');
}
