# syntax=docker/dockerfile:1.7
# Development-only image. The production image remains in ../web.
FROM node:24.14.0-bookworm-slim

ENV PNPM_HOME=/pnpm
ENV PATH=$PNPM_HOME:$PATH
RUN corepack enable && corepack prepare pnpm@10.31.0 --activate
WORKDIR /opt/source
RUN useradd --system --uid 10002 --create-home web
COPY --chown=10002:10002 package.json pnpm-lock.yaml pnpm-workspace.yaml tsconfig.base.json ./
COPY --chown=10002:10002 clients/typescript/package.json clients/typescript/package.json
COPY --chown=10002:10002 packages/bff/package.json packages/bff/package.json
COPY --chown=10002:10002 apps/web/package.json apps/web/package.json
# pnpm creates node_modules in the workspace, whose WORKDIR was created as root.
RUN chown --recursive 10002:10002 /opt/source
USER 10002
RUN pnpm install --frozen-lockfile
COPY --chown=10002:10002 clients/typescript clients/typescript
COPY --chown=10002:10002 packages/bff packages/bff
COPY --chown=10002:10002 apps/web apps/web

EXPOSE 3000
CMD ["pnpm", "--filter", "@ayni-orchestrator/web", "dev", "--host", "0.0.0.0", "--port", "3000"]
