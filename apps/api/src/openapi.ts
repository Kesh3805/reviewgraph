import type { INestApplication } from '@nestjs/common';
import { DocumentBuilder, SwaggerModule, type OpenAPIObject } from '@nestjs/swagger';
import { cleanupOpenApiDoc } from 'nestjs-zod';

/**
 * The OpenAPI document of the control plane. It is snapshotted to
 * `packages/contracts/openapi/api.json` (the web app generates its typed client from it);
 * `pnpm -F @reviewgraph/api test -- openapi -u` regenerates the file.
 */
export function buildOpenApiDocument(app: INestApplication): OpenAPIObject {
  const config = new DocumentBuilder()
    .setTitle('ReviewGraph API')
    .setDescription('Control-plane API of ReviewGraph. Errors are RFC 9457 problem+json.')
    .setVersion('1.0.0')
    .addCookieAuth('rg_session', { type: 'apiKey', in: 'cookie', name: 'rg_session' })
    .build();
  return cleanupOpenApiDoc(SwaggerModule.createDocument(app, config));
}
