FROM node:24-alpine AS ui-build
WORKDIR /app
RUN corepack enable
ENV PNPM_CONFIG_MINIMUM_RELEASE_AGE=0
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml ./
RUN pnpm install --frozen-lockfile --config.minimumReleaseAge=0
COPY vite.config.mjs ./
COPY ui ./ui
RUN pnpm ui:build

FROM node:24-alpine
WORKDIR /app
RUN corepack enable
ENV PNPM_CONFIG_MINIMUM_RELEASE_AGE=0
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml ./
RUN pnpm install --prod --frozen-lockfile --config.minimumReleaseAge=0
COPY server ./server
COPY --from=ui-build /app/dist ./dist
ENV PORT=8080 DATA_DIR=/data GUANGYA_WATCH_ROOT=/watch GUANGYA_FILE_ROOTS=/watch,/archive GUANGYA_OSS_TIMEOUT_MS=600000 GUANGYA_OSS_RETRY_MAX=3 GUANGYA_OSS_PARALLEL=3 GUANGYA_CLOUD_CONFIRM_TIMEOUT_MS=600000 GUANGYA_CLOUD_CONFIRM_POLL_MS=1000 GUANGYA_AUTO_SHARE_QUIET_MS=30000
VOLUME ["/data", "/watch"]
EXPOSE 8080
CMD ["node", "server/server.mjs"]
