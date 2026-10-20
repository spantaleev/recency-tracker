FROM docker.io/node:24.16.0-alpine3.24@sha256:21f403ab171f2dc89bad4dd69d7721bfd15f084ccb46cdd225f31f2bc59b5c9a

LABEL org.opencontainers.image.source="https://github.com/spantaleev/recency-tracker" \
      org.opencontainers.image.licenses="BSD-3-Clause"

ENV NODE_ENV=production \
    RECENCY_TRACKER_CONFIG=/config/config.json

WORKDIR /app

COPY package.json package-lock.json ./
RUN npm ci --omit=dev --engine-strict --no-audit --no-fund \
    && npm cache clean --force \
    && mkdir /config /data \
    && chown node:node /data

COPY bin/ ./bin/
COPY lib/ ./lib/
COPY resources/static/ ./resources/static/
COPY LICENSE.txt ./

USER node
EXPOSE 9998 9999
ENTRYPOINT ["node"]
CMD ["/app/bin/server"]
