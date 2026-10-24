FROM rust:1.90.0-bookworm AS build
RUN apt-get update && apt-get install -y --no-install-recommends libcurl4-openssl-dev pkg-config && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates ./crates
RUN cargo build --locked --release -p buaalogin-web

FROM debian:bookworm-slim AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends libcurl4 ca-certificates && rm -rf /var/lib/apt/lists/*
RUN install -d -o 65532 -g 65532 -m 0700 /data
COPY --from=build /src/target/release/buaalogin-web /usr/local/bin/buaalogin-web
VOLUME ["/data"]
EXPOSE 18799
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 CMD ["/usr/local/bin/buaalogin-web", "healthcheck"]
USER 65532:65532
ENTRYPOINT ["/usr/local/bin/buaalogin-web"]
