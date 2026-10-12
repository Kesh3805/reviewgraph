export class DomainError extends Error {
  constructor(message: string, readonly cause?: unknown) {
    super(message);
  }
}

export class NotFoundError extends DomainError {}

export class BillingError extends DomainError {}

export class ForbiddenError extends DomainError {}
