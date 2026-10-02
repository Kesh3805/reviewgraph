import { createCipheriv, createDecipheriv, randomBytes } from 'node:crypto';

/**
 * AES-256-GCM envelope with a fresh 96-bit random nonce per message.
 * Wire format: `v1.<nonce>.<ciphertext>.<tag>` (base64url segments).
 * `aad` binds the ciphertext to its context (e.g. the Redis key) so a value cannot be moved.
 */
const VERSION = 'v1';
const NONCE_BYTES = 12;
const KEY_BYTES = 32;

export class AeadError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'AeadError';
  }
}

function assertKey(key: Buffer): void {
  if (key.length !== KEY_BYTES) throw new AeadError(`AEAD key must be ${KEY_BYTES} bytes`);
}

export function aeadEncrypt(key: Buffer, plaintext: string, aad = ''): string {
  assertKey(key);
  const nonce = randomBytes(NONCE_BYTES);
  const cipher = createCipheriv('aes-256-gcm', key, nonce);
  cipher.setAAD(Buffer.from(aad, 'utf8'));
  const ciphertext = Buffer.concat([cipher.update(plaintext, 'utf8'), cipher.final()]);
  const tag = cipher.getAuthTag();
  return [VERSION, nonce, ciphertext, tag]
    .map((part) => (typeof part === 'string' ? part : part.toString('base64url')))
    .join('.');
}

export function aeadDecrypt(key: Buffer, payload: string, aad = ''): string {
  assertKey(key);
  const [version, nonce, ciphertext, tag, ...rest] = payload.split('.');
  if (version !== VERSION || !nonce || ciphertext === undefined || !tag || rest.length > 0) {
    throw new AeadError('unsupported or malformed AEAD payload');
  }
  try {
    const decipher = createDecipheriv('aes-256-gcm', key, Buffer.from(nonce, 'base64url'));
    decipher.setAAD(Buffer.from(aad, 'utf8'));
    decipher.setAuthTag(Buffer.from(tag, 'base64url'));
    return Buffer.concat([
      decipher.update(Buffer.from(ciphertext, 'base64url')),
      decipher.final(),
    ]).toString('utf8');
  } catch {
    throw new AeadError('AEAD authentication failed');
  }
}
