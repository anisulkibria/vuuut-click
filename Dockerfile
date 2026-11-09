FROM rust:1.99.1-bookworm AS builder
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY migrations ./migrations
COPY dashboard ./dashboard
COPY tracker/dist ./tracker/dist
COPY docs ./docs
ENV CARGO_BUILD_JOBS=1
RUN cargo build --release --locked -p click-server

FROM gcr.io/distroless/cc-debian13:nonroot
COPY --from=builder /build/target/release/click-server /usr/local/bin/click-server
WORKDIR /app
USER 10001:10001
ENV BIND=0.0.0.0:8082
EXPOSE 8082
ENTRYPOINT ["click-server"]
CMD ["serve"]
