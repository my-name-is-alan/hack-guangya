FROM node:24-alpine AS ui-build
WORKDIR /app
RUN corepack enable
ENV PNPM_CONFIG_MINIMUM_RELEASE_AGE=0
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml ./
RUN pnpm install --frozen-lockfile --config.minimumReleaseAge=0
COPY vite.config.mjs ./
COPY src-tauri/icons/128x128.png ./src-tauri/icons/128x128.png
COPY ui ./ui
RUN pnpm ui:build

FROM alpine:3.22 AS rclone-bin
ARG RCLONE_VERSION=1.74.4
ARG TARGETARCH
RUN set -eux; \
    apk add --no-cache ca-certificates unzip wget; \
    case "${TARGETARCH}" in \
      amd64) archive_arch=amd64; expected=fe435e0c36228e7c2f116a8701f01127bb1f694005fc11d1f27186c8bca4115d ;; \
      arm64) archive_arch=arm64; expected=97685285c9ad6a0cf17d5844115d2a67245af6444db672187074bd9c358de419 ;; \
      386) archive_arch=386; expected=7feee086d7ff72652c5a91ef4b4a576941ccd33b2929772a2d70471904e516f0 ;; \
      *) echo "Unsupported rclone architecture: ${TARGETARCH}" >&2; exit 1 ;; \
    esac; \
    archive="rclone-v${RCLONE_VERSION}-linux-${archive_arch}.zip"; \
    wget -q "https://downloads.rclone.org/v${RCLONE_VERSION}/${archive}" -O /tmp/rclone.zip; \
    echo "${expected}  /tmp/rclone.zip" | sha256sum -c -; \
    unzip -q /tmp/rclone.zip -d /tmp/rclone; \
    install -m 0755 "/tmp/rclone/rclone-v${RCLONE_VERSION}-linux-${archive_arch}/rclone" /usr/local/bin/rclone; \
    install -D -m 0644 "/tmp/rclone/rclone-v${RCLONE_VERSION}-linux-${archive_arch}/COPYING" /usr/local/share/licenses/rclone/COPYING

FROM node:24-alpine
WORKDIR /app
RUN corepack enable
RUN apk add --no-cache fuse3
ENV PNPM_CONFIG_MINIMUM_RELEASE_AGE=0
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml ./
RUN pnpm install --prod --frozen-lockfile --config.minimumReleaseAge=0
COPY server ./server
COPY ui/shareLink.js ./ui/shareLink.js
COPY --from=ui-build /app/dist ./dist
COPY --from=rclone-bin /usr/local/bin/rclone /usr/local/bin/rclone
COPY --from=rclone-bin /usr/local/share/licenses/rclone/COPYING /usr/local/share/licenses/rclone/COPYING
ENV PORT=8080 DATA_DIR=/data GUANGYA_WATCH_ROOT=/watch GUANGYA_FILE_ROOTS=/watch,/archive GUANGYA_DEFAULT_MONITOR_MODE=polling GUANGYA_OSS_TIMEOUT_MS=600000 GUANGYA_OSS_RETRY_MAX=3 GUANGYA_OSS_PARALLEL=3 GUANGYA_CLOUD_CONFIRM_TIMEOUT_MS=600000 GUANGYA_CLOUD_CONFIRM_POLL_MS=1000 GUANGYA_AUTO_SHARE_QUIET_MS=30000
VOLUME ["/data", "/watch", "/archive"]
EXPOSE 8080 19090
CMD ["node", "server/server.mjs"]
