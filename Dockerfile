FROM docker.io/node:24.22.0-alpine3.24@sha256:44fce696f38ba8c987c4db89954051000bbd05ffd8d8659ba15ed4a8602e937f

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
