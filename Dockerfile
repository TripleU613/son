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
#
# One image, one container, four processes. Previously four images (app, Gemini
# sidecar, session keeper, cloudflared) and four containers on a private Compose
# network; now a single package, because that is one thing to build, one thing to
# pull, one thing to roll back, and one version number that describes what is
# actually running. The four-container split had a real cost: three of the four
# outages in this project's history were a version skew or a name-resolution
# failure *between* containers, neither of which can happen inside one.
#
# Built on Playwright's image rather than adding a browser to Debian: it is the
# only one of the four bases that is hard to reproduce (Chromium plus ~90 matched
# shared libraries), and it already carries the Python the sidecar and keeper need.
# The other three contributed a static binary, two Python files, and a downloaded
# binary -- all cheap to move here, none cheap to move the other way.
#
# Re-pin by asking the registry and then *proving the answer resolves*, because a
# digest fetched with the wrong Accept headers is a different manifest's digest and
# fails the build with a bare "not found":
#
#   H='Accept: application/vnd.oci.image.index.v1+json, \
#      application/vnd.docker.distribution.manifest.list.v2+json'
#   D=$(curl -sI -H "$H" \
#       https://mcr.microsoft.com/v2/playwright/python/manifests/v1.62.0-noble \
#       | awk '/^docker-content-digest/{print $2}' | tr -d '\r')
#   curl -so /dev/null -w '%{http_code}\n' -H "$H" \
#       "https://mcr.microsoft.com/v2/playwright/python/manifests/$D"   # want 200
FROM mcr.microsoft.com/playwright/python:v1.62.0-noble@sha256:aa81288e738725378becba5b3e06cb0f3a7f012a610e87e8d767a090ea3f740d AS runtime

# supervisor, because four processes in one container need something that restarts
# one of them without taking the others down. Compose used to provide that per
# container (`restart: unless-stopped`); inside one container it has to be explicit,
# and a bash script with `wait -n` gives up the per-program backoff and log routing
# that make a crash-looping browser survivable.
#
# The display stack is for the login browser: a virtual X server for Chromium to
# draw into, a VNC server to export it, websockify+noVNC so it opens in an ordinary
# browser tab, and a window manager -- without one, Google's sign-in popups render
# undecorated and cannot be focused.
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates supervisor python3-venv \
        xvfb x11vnc fluxbox websockify novnc \
    && rm -rf /var/lib/apt/lists/*

# cloudflared as a pinned binary rather than its own container.
#
# Checksum computed from the downloaded release asset and recorded here, the same
# discipline as tailwindcss above: this process has direct control over what
# reaches soncollection.com, so it does not get to be whatever a URL serves today.
# Bump deliberately -- download the asset, sha256sum it, change both lines.
#
# linux-amd64 because CI builds for the runner's native platform and deploys to an
# amd64 host; a wrong-arch pull fails at this checksum rather than at runtime.
ARG CLOUDFLARED_VERSION=2026.7.3
ARG CLOUDFLARED_SHA256=9d71c677db00134c1bd4144b7783486b654ad281b1ea62b4972098d19f770f17
RUN curl -fsSL -o /usr/local/bin/cloudflared \
      "https://github.com/cloudflare/cloudflared/releases/download/${CLOUDFLARED_VERSION}/cloudflared-linux-amd64" \
    && echo "${CLOUDFLARED_SHA256}  /usr/local/bin/cloudflared" | sha256sum -c - \
    && chmod +x /usr/local/bin/cloudflared \
    && cloudflared --version

WORKDIR /app

# Two Python environments in one image, and not for tidiness.
#
# This base image's Python is Debian-managed, and fastapi/pydantic want a newer
# typing_extensions than the distro ships. pip cannot replace a dpkg-installed
# package -- "Cannot uninstall typing_extensions 4.10.0: no RECORD file was found"
# -- so a single combined install fails outright. It was never a problem before
# because the sidecar had its own python:3.13-slim image, with no distro packages
# to collide with; the collision is a genuine cost of merging, and a venv is the
# supported way to pay it rather than forcing the issue with
# --break-system-packages and hoping the browser still works afterwards.
#
# The sidecar goes in a clean venv: it needs nothing from the base image. (Debian
# splits `venv` out of the stdlib, hence python3-venv in the apt list above -- the
# error without it is `ensurepip is not available`, which reads like a pip problem
# and is a packaging one.)
COPY sidecar/requirements.txt ./requirements-sidecar.txt
RUN python -m venv /app/venv \
    && /app/venv/bin/pip install --no-cache-dir --upgrade pip \
    && /app/venv/bin/pip install --no-cache-dir -r requirements-sidecar.txt \
    && /app/venv/bin/pip check
# The keeper installs into the base environment on purpose: its playwright pin must
# stay the exact version whose browsers this image bundles, and that version is
# already installed here. A venv for it would either shadow that with a second copy
# or need --system-site-packages, which puts the collision above right back.
COPY keeper/requirements.txt ./requirements-keeper.txt
# tzdata is not a keeper dependency -- it satisfies a complaint the base image
# already had ("oslo-serialization requires tzdata, which is not installed"), which
# would otherwise fail the `pip check` below on a problem that predates this file.
# Installed rather than skipping the check: a pip check that is commented out
# because of someone else's inconsistency stops catching mine.
RUN pip install --no-cache-dir -r requirements-keeper.txt tzdata \
    && pip check \
    && python -c "import playwright, boto3, httpx; print('keeper deps ok')"

# The app: a static binary plus its assets, straight out of the builder stage.
COPY --from=builder /build/target/release/soncollection ./soncollection
COPY --from=builder /build/target/site ./site
# hash.txt must sit beside the binary, not in site/: leptos_meta's HashedStylesheet
# resolves it from current_exe()'s directory. Without it the <link> falls back to
# the unhashed name, which 404s once hash-files is on -- i.e. a silently style-less
# site.
COPY --from=builder /build/target/release/hash.txt ./hash.txt

COPY sidecar/gemini_service.py ./
COPY keeper/session_keeper.py ./
COPY keeper/entrypoint.sh ./keeper-entrypoint.sh
COPY deploy/supervisord.conf /etc/supervisor/supervisord.conf

# Non-root, and the same user for all four processes. `pwuser` rather than a new
# one: it already owns the browser and its caches in this image, so reusing it
# avoids re-chowning ~1GB of /ms-playwright to say the same thing.
#
# Nothing here runs as root, including supervisor -- so its socket, pid file and
# logs are configured into /tmp (see deploy/supervisord.conf), which is also what
# lets the container keep running with a read-only root filesystem.
RUN chown -R pwuser:pwuser /app
USER pwuser

# HOME=/tmp for every process: fluxbox, Chromium and gemini_webapi all write under
# it, and /tmp is the one writable path (a tmpfs, per docker-compose.yml).
ENV HOME=/tmp \
    PYTHONUNBUFFERED=1 \
    PYTHONPATH=/app

# LEPTOS_HASH_FILES must be set at RUNTIME, not just as `hash-files` in
# Cargo.toml. That metadata only tells cargo-leptos to emit hashed filenames at
# build time; LeptosOptions reads hash_files from the environment and defaults it to
# false, and no Cargo.toml ships in this image. Without it the build produces ONLY
# hashed assets while the served HTML asks for the unhashed names -- an HTTP 200
# with no CSS and no wasm. This shipped exactly once: it looked fine only because
# Cloudflare still had the previous build's files cached, and would have gone
# unstyled the moment that expired.
#
# The two internal URLs are loopback now rather than Compose service names. That is
# the merge's one real simplification: `http://gemini:8099` depended on Docker's
# embedded DNS, and a transient failure there is what took out an upload and the
# sidecar's own outbound resolution in the same second.
ENV LEPTOS_SITE_ADDR=0.0.0.0:3100 \
    LEPTOS_SITE_ROOT=/app/site \
    LEPTOS_HASH_FILES=true \
    GEMINI_URL=http://127.0.0.1:8099 \
    KEEPER_URL=http://127.0.0.1:6080 \
    SIDECAR_URL=http://127.0.0.1:8099

# Only the app's port, and only to cloudflared inside this same container --
# docker-compose.yml publishes nothing. The sidecar (8099), noVNC (6080) and VNC
# (5900) bind to loopback and are not listed here at all: they hold Google session
# state, and the only route to the browser is /admin/browser, which requires an
# admin session per request.
EXPOSE 3100

# "Is the site serving?" and nothing else.
#
# The first version of this also required the sidecar to report a usable Gemini
# account, on the reasoning that unscreened uploads should be visible in
# `docker ps`. That is the wrong signal in the wrong place, and it would have been
# actively harmful: Google's cookies expire on their own schedule, the keeper
# repairs them a minute or two later, and in that window a perfectly healthy site
# would report unhealthy -- which the deploy now reads to decide whether to roll
# back. Screening status has a home already: the panel at the top of /admin, which
# says "Down" and why, and the upload page, which tells the uploader their son is
# held for review. Container health means the site answers.
#
# A TCP connect via bash's /dev/tcp rather than an HTTP GET: it needs no curl in the
# image, and a listening socket is exactly the claim being made.
HEALTHCHECK --interval=30s --timeout=5s --start-period=45s --retries=3 \
    CMD bash -c 'echo > /dev/tcp/127.0.0.1/3100' || exit 1

ENTRYPOINT ["supervisord", "-c", "/etc/supervisor/supervisord.conf"]
