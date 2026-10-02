import { inspect } from 'node:util';

const REDACTED = '[redacted]';

/**
 * Wraps a sensitive value (token, key) so it cannot reach logs, spans, error messages or JSON
 * responses by accident. The only way out is the explicit `reveal()` call.
 */
export class Secret<T = string> {
  readonly #value: T;

  constructor(value: T) {
    this.#value = value;
  }

  reveal(): T {
    return this.#value;
  }

  toJSON(): string {
    return REDACTED;
  }

  toString(): string {
    return REDACTED;
  }

  [inspect.custom](): string {
    return `Secret(${REDACTED})`;
  }
}
