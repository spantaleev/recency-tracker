FROM docker.io/rust:1.98.0-alpine3.24@sha256:a10e64dd139b7387337c7fbe8aca31b959b57b2fd4c8ae20a02cf1d6ea424dce AS build
WORKDIR /build
COPY rust/Cargo.toml rust/Cargo.lock ./rust/
COPY rust/src/ ./rust/src/
COPY resources/static/ ./resources/static/
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
