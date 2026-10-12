const { DATABASE_URL, REDIS_URL: redisUrl } = process.env;

export function port(): number {
  return Number(process.env.PORT ?? '3000');
}

export function secret(): string | undefined {
  return process.env['JWT_SECRET'];
}

export const urls = [DATABASE_URL, redisUrl];
