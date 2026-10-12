# Web image (Next.js, standalone output). Built by CI only; local development runs `next dev`.
ARG NODE_VERSION=24
ARG DEBIAN_RELEASE=bookworm

FROM node:${NODE_VERSION}-${DEBIAN_RELEASE}-slim AS build
RUN corepack enable
WORKDIR /repo
COPY . .
RUN pnpm install --frozen-lockfile --ignore-scripts --filter @reviewgraph/web...
ENV NEXT_TELEMETRY_DISABLED=1
RUN pnpm --filter @reviewgraph/web build

FROM node:${NODE_VERSION}-${DEBIAN_RELEASE}-slim AS runtime
ARG GIT_SHA=unknown
ARG BUILD_TIME=unknown
LABEL org.opencontainers.image.title="reviewgraph-web" \
      org.opencontainers.image.revision="${GIT_SHA}" \
      org.opencontainers.image.created="${BUILD_TIME}" \
      org.opencontainers.image.source="https://github.com/Kesh3805/reviewgraph"
ENV NODE_ENV=production PORT=3001 HOSTNAME=0.0.0.0 NEXT_TELEMETRY_DISABLED=1
WORKDIR /app
COPY --from=build /repo/apps/web/.next/standalone ./
COPY --from=build /repo/apps/web/.next/static ./apps/web/.next/static
USER 10001:10001
EXPOSE 3001
CMD ["node", "apps/web/server.js"]
