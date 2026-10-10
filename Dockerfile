FROM rust:1.90.0-bookworm AS build
RUN apt-get update && apt-get install -y --no-install-recommends libcurl4-openssl-dev pkg-config && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates ./crates
RUN cargo build --locked --release -p buaalogin

FROM build AS test
RUN cargo test --locked -p buaa-core -p buaalogin

FROM debian:bookworm-slim AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends libcurl4 ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/buaalogin /usr/local/bin/buaalogin
USER 65532:65532
ENTRYPOINT ["/usr/local/bin/buaalogin"]
