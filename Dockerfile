# One image, four binaries. `engine`, `api`, `persist` and `ws` share a
# workspace and nearly all of their dependencies, so building an image per
# binary would compile the same crates four times over to produce four images
# that differ only in an argv. Each compose service picks its binary with
# `command:`.

FROM rust:1.97.1-slim-bookworm AS build
WORKDIR /app

# `rust-toolchain.toml` pins 1.97.1, the same version the base image carries.
# Copying it in means a future bump to the pin fails the build loudly instead
# of being silently compiled by whatever the image happens to ship.
COPY rust-toolchain.toml Cargo.toml Cargo.lock ./
COPY crates ./crates

# Cache mounts rather than the usual dummy-main dependency layer: the registry
# and the target directory survive between builds without a synthetic source
# tree to keep in sync, and neither ends up in the image. The binaries have to
# be copied out inside this same RUN, because /app/target stops existing the
# moment the mount is released.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    cargo build --release --locked \
      -p cex-engine -p cex-api -p cex-persist -p cex-ws \
    && mkdir -p /out \
    && cp target/release/engine \
          target/release/api \
          target/release/persist \
          target/release/ws /out/

FROM debian:bookworm-slim AS runtime

# Managed Postgres terminates TLS with a public CA and sqlx's rustls backend
# verifies against the system roots, so without these the connection dies at
# the handshake rather than anywhere informative.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# None of the four need root.
RUN useradd --system --uid 10001 --user-group cex

# Created here rather than left to the volume mount: a named volume inherits
# the ownership of the image path it covers, so making it now is what lets the
# engine write snapshots as a non-root user.
RUN mkdir -p /var/lib/cex/snapshots && chown -R cex:cex /var/lib/cex

COPY --from=build /out/ /usr/local/bin/

USER cex
