FROM rust:1.99.1-bookworm AS builder
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY migrations ./migrations
COPY dashboard ./dashboard
COPY tracker/dist ./tracker/dist
COPY docs ./docs
RUN cargo build --release --locked -p click-server

FROM debian:12.12-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl && rm -rf /var/lib/apt/lists/* && useradd --uid 10001 --create-home click
COPY --from=builder /build/target/release/click-server /usr/local/bin/click-server
USER 10001:10001
ENV BIND=0.0.0.0:8082
EXPOSE 8082
ENTRYPOINT ["click-server"]
CMD ["serve"]
