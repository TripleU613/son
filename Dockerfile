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

WORKDIR /build

# Dependencies compiled against a placeholder crate, in their own layer, before
# any real source is copied in. This is what actually survives across CI runs:
# CI's cache-from/cache-to is `type=gha`, which caches plain image layers, not
# BuildKit `--mount=type=cache` mounts -- those are scoped to a single build
# and do not persist to the next push at all (a documented upstream
# limitation, not a guess: docker/build-push-action#1011). A layer, on the
# other hand, is exactly what `type=gha` restores, and this layer only
# invalidates when Cargo.toml/Cargo.lock change -- not on every source edit --
# so candle/aws-sdk-s3/leptos and cargo-leptos's own build only recompile from
# scratch when a dependency actually changes.
COPY Cargo.toml Cargo.lock ./
RUN mkdir -p src/components src/storage src/moderation \
    && echo 'fn main() {}' > src/main.rs \
    && echo '' > src/lib.rs \
    && cargo leptos build --release \
    && rm -rf src

COPY src ./src
COPY style ./style
COPY public ./public
# Touch so the placeholder mtimes above don't make cargo skip the real build.
RUN touch src/main.rs src/lib.rs

RUN cargo leptos build --release

# ---- fetch CLIP weights in their own stage -------------------------------
# Kept separate from both the builder (which has no reason to need curl) and
# the runtime image (which has no reason to keep it after this copy), so a
# public reading of the final image's layers shows only what actually runs.
FROM debian:bookworm-slim@sha256:abd67ffcfa541b485a3dff59865ab629aa048a6c613e639d36e7456b0b229241 AS model
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /model
RUN curl -fsSLO https://huggingface.co/openai/clip-vit-base-patch32/resolve/main/pytorch_model.bin \
    && curl -fsSLO https://huggingface.co/openai/clip-vit-base-patch32/resolve/main/tokenizer.json

# ---- runtime ---------------------------------------------------------------
FROM debian:bookworm-slim@sha256:abd67ffcfa541b485a3dff59865ab629aa048a6c613e639d36e7456b0b229241 AS runtime

RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --uid 10001 son

WORKDIR /app
COPY --from=builder --chown=son:son /build/target/release/soncollection ./soncollection
COPY --from=builder --chown=son:son /build/target/site ./site
COPY --from=model --chown=son:son /model /app/models/clip-vit-base-patch32

# No storage: the image ships with no writable data directory, and nothing
# under /app is written to at runtime. Uploads go to R2, all state lives in D1.
USER son

ENV LEPTOS_SITE_ADDR=0.0.0.0:3100 \
    LEPTOS_SITE_ROOT=/app/site \
    CLIP_MODEL_DIR=/app/models/clip-vit-base-patch32

EXPOSE 3100

# A TCP-level liveness check via bash's /dev/tcp, not a full HTTP GET: it needs
# no curl/wget in the image, keeping the runtime stage to exactly
# ca-certificates plus the binary and its assets.
HEALTHCHECK --interval=30s --timeout=5s --start-period=40s --retries=3 \
    CMD bash -c 'echo > /dev/tcp/127.0.0.1/3100' || exit 1

ENTRYPOINT ["/app/soncollection"]
