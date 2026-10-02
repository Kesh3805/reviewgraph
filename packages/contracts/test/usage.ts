// Type-level test: compiled by `tsc -p tsconfig.test.json`. Fails if the generated barrel
// stops exporting SchemaInfo or its shape changes incompatibly.
import type { SchemaInfo } from '../src/index.js';
import { CONTRACTS_VERSION } from '../src/index.js';

export const sample: SchemaInfo = { contracts_version: CONTRACTS_VERSION, types: ['SchemaInfo'] };

// @ts-expect-error unknown fields are rejected by the generated type
export const bad: SchemaInfo = { contracts_version: 1, types: [], extra: true };
