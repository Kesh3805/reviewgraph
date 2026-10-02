import type { NextConfig } from 'next';

/** Where the Next server proxies `/api/*` to (the NestJS control plane). */
const apiInternalUrl = process.env.API_INTERNAL_URL ?? 'http://127.0.0.1:8080';

const nextConfig: NextConfig = {
  poweredByHeader: false,
  reactStrictMode: true,
  // The browser and the API share an origin, so SameSite=Lax session cookies just work.
  async rewrites() {
    return [{ source: '/api/:path*', destination: `${apiInternalUrl}/api/:path*` }];
  },
  async headers() {
    // The CSP itself is set per request (with a nonce) in middleware.ts.
    return [
      {
        source: '/:path*',
        headers: [
          { key: 'X-Content-Type-Options', value: 'nosniff' },
          { key: 'Referrer-Policy', value: 'strict-origin-when-cross-origin' },
          { key: 'X-Frame-Options', value: 'DENY' },
        ],
      },
    ];
  },
};

export default nextConfig;
