# syntax=docker/dockerfile:1.7
FROM node:24.14.0-bookworm-slim AS builder
ENV PNPM_HOME=/pnpm
ENV PATH=$PNPM_HOME:$PATH
RUN corepack enable && corepack prepare pnpm@10.31.0 --activate
WORKDIR /workspace
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml tsconfig.base.json ./
COPY clients/typescript/package.json clients/typescript/package.json
COPY packages/bff/package.json packages/bff/package.json
COPY apps/web/package.json apps/web/package.json
RUN --mount=type=cache,id=ayni-orchestrator-web-pnpm,target=/pnpm/store \
    pnpm install --frozen-lockfile
COPY clients/typescript clients/typescript
COPY packages/bff packages/bff
COPY apps/web apps/web
RUN pnpm build:web \
    && pnpm --filter @ayni-orchestrator/web deploy --prod --legacy /app

FROM node:24.14.0-bookworm-slim
ENV NODE_ENV=production
ENV HOST=0.0.0.0
ENV PORT=3000
ENV PROTOCOL_HEADER=x-forwarded-proto
WORKDIR /app
RUN useradd --system --uid 10002 --create-home web
COPY --from=builder --chown=10002:10002 /app ./
USER 10002
EXPOSE 3000
CMD ["node", "server.mjs"]
