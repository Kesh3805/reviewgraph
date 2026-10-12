# API image (NestJS control plane). Built by CI only; local development runs the API natively.
ARG NODE_VERSION=24
ARG DEBIAN_RELEASE=bookworm

FROM node:${NODE_VERSION}-${DEBIAN_RELEASE}-slim AS build
RUN corepack enable
WORKDIR /repo
COPY . .
RUN pnpm install --frozen-lockfile --ignore-scripts --filter @reviewgraph/api...
RUN pnpm --filter @reviewgraph/api build
# `deploy` honors ignore files, so the compiled output is copied explicitly.
RUN pnpm --filter @reviewgraph/api deploy --prod --legacy /out \
 && cp -r apps/api/dist /out/dist

FROM node:${NODE_VERSION}-${DEBIAN_RELEASE}-slim AS runtime
ARG GIT_SHA=unknown
ARG BUILD_TIME=unknown
LABEL org.opencontainers.image.title="reviewgraph-api" \
      org.opencontainers.image.revision="${GIT_SHA}" \
      org.opencontainers.image.created="${BUILD_TIME}" \
      org.opencontainers.image.source="https://github.com/Kesh3805/reviewgraph"
ENV NODE_ENV=production PORT=3000
WORKDIR /app
COPY --from=build /out /app
USER 10001:10001
EXPOSE 3000
CMD ["node", "--import", "./dist/telemetry/instrumentation.js", "dist/main.js"]
