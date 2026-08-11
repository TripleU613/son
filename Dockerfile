# syntax=docker/dockerfile:1
#
# Base images are pinned by digest, not a movable tag: a public repo's supply
# chain should not depend on trusting that "rust:1-bookworm" still points at
# what it pointed at yesterday. Re-pin deliberately (not silently) when a
# rebuild is wanted: docker pull <image> && docker inspect --format
# '{{index .RepoDigests 0}}' <image>.

FROM rust:1-bookworm@sha256:0e2bcaef56d041a486784e54104a81aebe0da44bd03019bd70bc0401e42e4a97 AS builder

RUN rustup target add wasm32-unknown-unknown \
    && cargo install cargo-leptos --locked

# Tailwind's standalone binary, so the CSS build needs no Node toolchain in the
# image. Pinned to an exact version with its checksum verified, for the same
# reason the base images are pinned by digest: this runs in a public repo's
# build and must not depend on whatever a moving URL serves today.
#
# Installed explicitly rather than left to cargo-leptos's own tool download:
# that would resolve a version at build time, and Tailwind 4 reads
# tailwind.config.js only in a legacy compatibility path, so an implicit
# upgrade would quietly change what CSS this image ships.
#
# linux-x64 because CI builds for the runner's native platform (no `platforms:`
# in the workflow) and deploys to an amd64 host. A wrong-arch pull fails loudly
# here rather than at runtime.
ARG TAILWIND_VERSION=3.4.19
ARG TAILWIND_SHA256=4af3198c015616ea7d6617974ec3d70d987ecc00c1ca8463b0a30fd65cc7c06e
RUN curl -fsSL -o /usr/local/bin/tailwindcss \
      "https://github.com/tailwindlabs/tailwindcss/releases/download/v${TAILWIND_VERSION}/tailwindcss-linux-x64" \
    && echo "${TAILWIND_SHA256}  /usr/local/bin/tailwindcss" | sha256sum -c - \
    && chmod +x /usr/local/bin/tailwindcss \
    && tailwindcss --help | head -1

WORKDIR /build

# Dependencies compiled against a placeholder crate, in their own layer, before
# any real source is copied in. This is what actually survives across CI runs:
# CI's cache-from/cache-to is `type=gha`, which caches plain image layers, not
# BuildKit `--mount=type=cache` mounts -- those are scoped to a single build
# and do not persist to the next push at all (a documented upstream
# limitation, not a guess: docker/build-push-action#1011). A layer, on the
# other hand, is exactly what `type=gha` restores, and this layer only
# invalidates when Cargo.toml/Cargo.lock change -- not on every source edit --
# so aws-sdk-s3/leptos/image and cargo-leptos's own build only recompile from
# scratch when a dependency actually changes.
# Plain `cargo build`, not `cargo leptos build`, against the placeholder:
# cargo-leptos's build also runs wasm-bindgen post-processing, Tailwind CSS
# compilation, and asset bundling, which expect a real hydrate() entrypoint
# to exist -- an empty lib.rs makes cargo-leptos itself fail outright (this
# was tried and observed failing in CI, not assumed). Plain `cargo build`
# only needs to compile successfully, which an empty crate does trivially,
# and compiling IS the expensive, cacheable part -- the wasm-bindgen/asset
# steps are comparatively instant and only need to happen once, for real,
# after the actual source is copied in below.
COPY Cargo.toml Cargo.lock ./
RUN mkdir -p src/components src/storage \
    && echo 'fn main() {}' > src/main.rs \
    && echo '' > src/lib.rs \
    && cargo build --release --no-default-features --features ssr \
    && cargo build --release --target wasm32-unknown-unknown --no-default-features --features hydrate --lib \
    && rm -rf src

COPY src ./src
COPY style ./style
COPY public ./public
# tailwind.config.js is a build input, not tooling config to leave behind: it
# holds the palette, radii, chrome heights and content widths, and its `content`
# glob is what stops every utility being purged. Omitting it is what broke the
# first CI run after the Tailwind migration -- cargo-leptos is pointed at it by
# `tailwind-config-file` and fails outright when it is missing.
COPY tailwind.config.js ./
# Touch so the placeholder mtimes above don't make cargo skip the real build.
RUN touch src/main.rs src/lib.rs

RUN cargo leptos build --release

# ---- runtime ---------------------------------------------------------------
FROM debian:bookworm-slim@sha256:abd67ffcfa541b485a3dff59865ab629aa048a6c613e639d36e7456b0b229241 AS runtime

RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --uid 10001 son

WORKDIR /app
COPY --from=builder --chown=son:son /build/target/release/soncollection ./soncollection
COPY --from=builder --chown=son:son /build/target/site ./site
# hash.txt must sit beside the binary, not in site/: leptos_meta's
# HashedStylesheet resolves it from current_exe()'s directory. Without it the
# <link> falls back to the unhashed name, which 404s once hash-files is on --
# i.e. a silently style-less site.
COPY --from=builder --chown=son:son /build/target/release/hash.txt ./hash.txt

# No storage: the image ships with no writable data directory, and nothing
# under /app is written to at runtime. Uploads go to R2, all state lives in D1.
USER son

# LEPTOS_HASH_FILES must be set at RUNTIME, not just as `hash-files` in
# Cargo.toml. That metadata only tells cargo-leptos to emit hashed filenames at
# build time; LeptosOptions reads hash_files from the environment and defaults
# it to false, and no Cargo.toml ships in this image. Without it the build
# produces ONLY hashed assets while the served HTML asks for the unhashed names
# -- an HTTP 200 with no CSS and no wasm. This shipped exactly once: it looked
# fine only because Cloudflare still had the previous build's files cached, and
# would have gone unstyled the moment that expired.
ENV LEPTOS_SITE_ADDR=0.0.0.0:3100 \
    LEPTOS_SITE_ROOT=/app/site \
    LEPTOS_HASH_FILES=true

EXPOSE 3100

# A TCP-level liveness check via bash's /dev/tcp, not a full HTTP GET: it needs
# no curl/wget in the image, keeping the runtime stage to exactly
# ca-certificates plus the binary and its assets.
#
# start-period is back down to 40s: it was raised to 5m only because main.rs
# loaded a ~600MB CLIP model before binding the port, so nothing listened for
# minutes on a cold start. With no model to load, the server binds almost
# immediately and a 5m grace period would just delay noticing a real failure.
HEALTHCHECK --interval=30s --timeout=5s --start-period=40s --retries=3 \
    CMD bash -c 'echo > /dev/tcp/127.0.0.1/3100' || exit 1

ENTRYPOINT ["/app/soncollection"]
