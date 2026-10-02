declare module 'ioredis-mock' {
  import type { Redis } from 'ioredis';
  const RedisMock: new (options?: unknown) => Redis;
  export default RedisMock;
}
