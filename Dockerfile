# syntax=docker/dockerfile:1
#
# The browser build of CyberAPI as one image: `cyberapi-bridge` in release
# form with the release page compiled in, as `make web-dist` builds it. Not
# the desktop app, which needs a window. The projects and accounts live in
# PostgreSQL or MySQL, so the image keeps nothing of its own:
#
#   docker run -p 7380:7380 -e CYBERAPI_BRIDGE_STORE=postgres://user:password@host/db vicanso/cyberapi-web
#
# and make the administrator on the page with the setup token it logs.
#
# Self-contained on purpose: `docker build .` on a fresh clone produces the
# image, with no page built beforehand. The price is that each architecture
# compiles the (architecture-independent) wasm for itself.

# Bookworm, for the glibc of the distroless image below.
FROM rust:1-bookworm AS builder

# Pinned, rather than whatever is newest when the image builds: an unpinned
# install resolves afresh and lets a bad upstream release break ours.
ARG WASM_PACK_VERSION=0.15.0
# From GitHub, not apt: Debian's binaryen is older than the wasm a current
# rustc emits, and `wasm-opt` refuses what it cannot parse.
ARG BINARYEN_VERSION=version_132

# clang/libclang: the C the page compiles to wasm, and build scripts that run
# bindgen. python3: scripts/web-bundle.sh. curl: binaryen.
RUN apt-get update \
    && apt-get install -y --no-install-recommends clang libclang-dev python3 curl ca-certificates \
    && rm -rf /var/lib/apt/lists/*

RUN arch="$(uname -m)" \
    && curl -fsSL "https://github.com/WebAssembly/binaryen/releases/download/${BINARYEN_VERSION}/binaryen-${BINARYEN_VERSION}-${arch}-linux.tar.gz" \
        | tar -xz -C /opt \
    && ln -s "/opt/binaryen-${BINARYEN_VERSION}/bin/wasm-opt" /usr/local/bin/wasm-opt \
    && wasm-opt --version

# A download that stalls is retried rather than fatal, and HTTP/2
# multiplexing is off: behind a flaky NAT it is what turns one slow stream
# into "transferred 0 bytes in 30s" for all of them. Harmless on a good line.
ENV CARGO_NET_RETRY=10 \
    CARGO_HTTP_TIMEOUT=120 \
    CARGO_HTTP_MULTIPLEXING=false

# Optional, for a network where crates.io and static.rust-lang.org crawl:
#   --build-arg CARGO_MIRROR=sparse+https://rsproxy.cn/index/
#   --build-arg RUSTUP_DIST_SERVER=https://mirrors.ustc.edu.cn/rust-static
# Unset, as CI builds, nothing changes. A mirror changes where crates come
# from, not what `Cargo.lock` pins: `--locked` means the same versions and
# checksums.
ARG CARGO_MIRROR=""
RUN if [ -n "$CARGO_MIRROR" ]; then \
        printf '[source.crates-io]\nreplace-with = "mirror"\n\n[source.mirror]\nregistry = "%s"\n' "$CARGO_MIRROR" \
            > "$CARGO_HOME/config.toml"; \
    fi

RUN cargo install --locked "wasm-pack@${WASM_PACK_VERSION}"

WORKDIR /src

# The two toolchains, before the sources, so that a source change does not
# download them again: the pinned stable for the bridge, and the nightly the
# browser backend needs, which `cyberapi-web/` selects for itself.
COPY rust-toolchain.toml ./
COPY cyberapi-web/rust-toolchain.toml cyberapi-web/
# Declared here and not with `CARGO_MIRROR` above: an ARG is part of the cache
# key of every step after it, and this one has no business invalidating the
# wasm-pack install. Exported only when given: an empty value is not "the
# default".
ARG RUSTUP_DIST_SERVER=""
RUN if [ -n "$RUSTUP_DIST_SERVER" ]; then export RUSTUP_DIST_SERVER; else unset RUSTUP_DIST_SERVER; fi \
    && rustup toolchain install "$(sed -n 's/^channel = "\(.*\)"/\1/p' rust-toolchain.toml)" \
        --profile minimal --no-self-update \
    && rustup toolchain install "$(sed -n 's/^channel = "\(.*\)"/\1/p' cyberapi-web/rust-toolchain.toml)" \
        --profile minimal --no-self-update --target wasm32-unknown-unknown

COPY . .

# What the bridge reports as its version (#54): a nightly's own and the
# commit, given by docker.yml; empty, Cargo.toml's version alone.
ARG CYBERAPI_VERSION=""
ARG CYBERAPI_COMMIT=""

# The page first, in its release form, because the bridge embeds whatever
# `cyberapi-web/www/` holds when it is compiled, and its `build.rs` refuses a
# release build that would embed no module. Cargo's downloads and target
# directory are cache mounts, kept between local builds and out of the
# layers: the binary is copied out of them.
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/src/target,sharing=locked \
    scripts/web-bundle.sh --release \
    && cargo build --release --locked -p cyberapi-bridge \
    && cp target/release/cyberapi-bridge /cyberapi-bridge

# glibc and libgcc and nothing else: no shell, no package manager, not root.
# Requests are checked against its certificate store (the binary's own
# Mozilla roots when there is none).
FROM gcr.io/distroless/cc-debian12:nonroot

COPY --from=builder /cyberapi-bridge /usr/local/bin/cyberapi-bridge

# Every interface, because inside a container loopback reaches nobody; what
# is published, and to whom, is `docker run -p`'s decision. An environment
# variable rather than an argument: arguments to `docker run` (such as
# `--insecure-cookie` behind plain http, where the Secure login cookie is
# dropped, or `--allow-target 10.0.0.0/8` for the APIs of your own network)
# are passed to the bridge and would replace it.
ENV CYBERAPI_BRIDGE_LISTEN=0.0.0.0:7380
EXPOSE 7380

ENTRYPOINT ["/usr/local/bin/cyberapi-bridge"]
