FROM docker.io/rust:1.99.1-alpine3.24@sha256:f432d4b8663fbc50b29e33846bf6a1fb9f17f142023598899d641cddf57c6b22 AS build
WORKDIR /build
COPY rust/Cargo.toml rust/Cargo.lock ./rust/
COPY rust/src/ ./rust/src/
COPY resources/static/ ./resources/static/
ARG RECENCY_TRACKER_VERSION=dev
RUN cargo build --release --locked --manifest-path rust/Cargo.toml \
    && mkdir /config /data /app

FROM scratch
LABEL org.opencontainers.image.source="https://github.com/spantaleev/recency-tracker" \
      org.opencontainers.image.licenses="BSD-3-Clause"
ENV RECENCY_TRACKER_CONFIG=/config/config.json
COPY --from=build /build/rust/target/release/recency-tracker /recency-tracker
COPY rust/container/passwd /etc/passwd
COPY rust/container/group /etc/group
COPY --from=build --chown=1000:1000 /data /data
COPY --from=build /config /config
COPY --from=build /app /app
COPY LICENSE.txt /LICENSE.txt
WORKDIR /app
USER node
EXPOSE 9998 9999
ENTRYPOINT ["/recency-tracker"]
