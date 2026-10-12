import { lookup } from 'node:dns/promises';
import { isIP } from 'node:net';
import { ProviderError } from '../ports';

/**
 * SSRF protection for provider base URLs that an operator can point anywhere (self-managed
 * GitLab, Bitbucket Data Center). HTTPS only; no credentials in the URL; and by default no
 * private, loopback or link-local destination, checked on the literal host AND on what the
 * name resolves to. `allowPrivateNetworks` is the explicit operator opt-in for an on-premises
 * server (and for tests that talk to a loopback fake).
 */
export interface BaseUrlPolicy {
  /** When non-empty, the host must be one of these (lower-case, no port). */
  allowedHosts?: readonly string[];
  /** Permit private ranges and plain `http:`; off unless an operator turns it on. */
  allowPrivateNetworks?: boolean;
}

export type HostResolver = (host: string) => Promise<string[]>;

const defaultResolver: HostResolver = async (host) =>
  (await lookup(host, { all: true, verbatim: true })).map((a) => a.address);

/** True for loopback, private, link-local, unspecified and unique-local addresses. */
export function isPrivateAddress(address: string): boolean {
  const ip = address.startsWith('[') ? address.slice(1, -1) : address;
  const version = isIP(ip);
  if (version === 4) return isPrivateV4(ip);
  if (version === 6) return isPrivateV6(ip.toLowerCase());
  return false;
}

function isPrivateV4(ip: string): boolean {
  const [a = 0, b = 0] = ip.split('.').map(Number);
  return (
    a === 0 ||
    a === 10 ||
    a === 127 ||
    (a === 100 && b >= 64 && b <= 127) ||
    (a === 169 && b === 254) ||
    (a === 172 && b >= 16 && b <= 31) ||
    (a === 192 && b === 168) ||
    (a === 198 && (b === 18 || b === 19))
  );
}

function isPrivateV6(ip: string): boolean {
  if (ip === '::' || ip === '::1') return true;
  const mapped = /^::ffff:(\d+\.\d+\.\d+\.\d+)$/.exec(ip);
  if (mapped?.[1]) return isPrivateV4(mapped[1]);
  const first = Number.parseInt(ip.split(':')[0] || '0', 16);
  // fc00::/7 unique local, fe80::/10 link local.
  return (first & 0xfe00) === 0xfc00 || (first & 0xffc0) === 0xfe80;
}

const NAME_BLOCKLIST = [/^localhost$/, /\.localhost$/, /\.local$/, /\.internal$/, /\.lan$/];

/**
 * Validates `raw` and returns the normalized base URL (no trailing slash). Synchronous checks
 * only; `assertResolvesPublic` adds the DNS check.
 */
export function parseBaseUrl(raw: string, policy: BaseUrlPolicy = {}): URL {
  let url: URL;
  try {
    url = new URL(raw);
  } catch {
    throw new ProviderError('invalid', 'provider base URL is not a valid URL');
  }
  const privateOk = policy.allowPrivateNetworks === true;
  if (url.protocol !== 'https:' && !(privateOk && url.protocol === 'http:')) {
    throw new ProviderError('invalid', 'provider base URL must use https');
  }
  if (url.username || url.password) {
    throw new ProviderError('invalid', 'provider base URL must not carry credentials');
  }
  const host = url.hostname.toLowerCase().replace(/^\[|\]$/g, '');
  if (policy.allowedHosts && policy.allowedHosts.length > 0) {
    if (!policy.allowedHosts.includes(host)) {
      throw new ProviderError('invalid', 'provider base URL host is not on the allow-list');
    }
  }
  if (!privateOk) {
    if (isPrivateAddress(host) || NAME_BLOCKLIST.some((re) => re.test(host))) {
      throw new ProviderError('invalid', 'provider base URL points at a private network');
    }
  }
  return url;
}

/** Rejects a host whose name resolves to a private address (DNS rebinding to the intranet). */
export async function assertResolvesPublic(
  url: URL,
  policy: BaseUrlPolicy = {},
  resolve: HostResolver = defaultResolver,
): Promise<void> {
  if (policy.allowPrivateNetworks === true) return;
  const host = url.hostname.replace(/^\[|\]$/g, '');
  if (isIP(host)) return;
  let addresses: string[];
  try {
    addresses = await resolve(host);
  } catch {
    throw new ProviderError('transient', 'provider host did not resolve');
  }
  if (addresses.length === 0 || addresses.some(isPrivateAddress)) {
    throw new ProviderError('invalid', 'provider base URL resolves to a private network');
  }
}
