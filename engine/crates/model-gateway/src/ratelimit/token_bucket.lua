-- Atomic token bucket. Uses Redis TIME so every worker shares one clock.
-- KEYS[1] = bucket key
-- ARGV[1] = capacity, ARGV[2] = refill per ms, ARGV[3] = cost
-- Returns {1, 0} when granted, {0, wait_ms} when denied.
local t = redis.call('TIME')
local now = t[1] * 1000 + math.floor(t[2] / 1000)
local b = redis.call('HMGET', KEYS[1], 'tokens', 'ts')
local capacity = tonumber(ARGV[1])
local tokens = tonumber(b[1]) or capacity
local ts = tonumber(b[2]) or now
tokens = math.min(capacity, tokens + (now - ts) * tonumber(ARGV[2]))
local cost = tonumber(ARGV[3])
if tokens >= cost then
  tokens = tokens - cost
  redis.call('HSET', KEYS[1], 'tokens', tokens, 'ts', now)
  redis.call('PEXPIRE', KEYS[1], 120000)
  return {1, 0}
else
  redis.call('HSET', KEYS[1], 'tokens', tokens, 'ts', now)
  redis.call('PEXPIRE', KEYS[1], 120000)
  return {0, math.ceil((cost - tokens) / tonumber(ARGV[2]))}
end
