# son collection

A gallery for the **"Son 😭😭😭😭😭"** meme and its endless variants — Sonion,
Capri-Son, Dy-Son, Sonflower, Sontato. Free uploads.

Pure Rust, front to back.

## Stack

| Layer      | Choice | Why |
| ---------- | ------ | --- |
| UI         | Leptos 0.8 (SSR + hydrate) | Reactive components in Rust; SSR keeps the OG/Twitter cards working, which is how a meme site actually spreads |
| Server     | Axum 0.8 | What `leptos_axum` runs on anyway |
| DB         | Cloudflare D1, over its HTTP API | No SQLite driver speaks D1's wire format, so `d1.rs` is a small hand-rolled client. Production has no local disk at all, so there is no local database either — see Deploy |
| Images     | `image` 0.25 | Decode, bound, thumbnail |
| Moderation | None (report queue only) | No content analysis at all — see Moderation. Screening is expected to move to an external API |
| Storage    | Cloudflare R2 (local disk in dev) | Behind a `Backend` trait; images served from media.soncollection.com, not proxied |

## Running it

```bash
rustup target add wasm32-unknown-unknown
cargo install cargo-leptos --locked

cp .env.example .env      # fill in CF_ACCOUNT_ID, CF_D1_DATABASE_ID, CF_D1_API_TOKEN
cargo leptos watch          # http://127.0.0.1:3100
```

Port 3100, not 3000 — 3000 was already taken by an unrelated dev server.

`SITE_ORIGIN` must be set to the real origin in production (e.g.
`https://soncollection.com`). Link unfurlers reject relative `og:image` URLs, so
without it Discord and Twitter previews render with no image.

**Local dev talks to the same D1 database as production.** There is no local
database at all, on purpose — see Deploy for why. This means local testing can
affect real data; `report`/`set_public` exist specifically to undo that.

### Styling

Tailwind, configured in `tailwind.config.js` (palette, radii, chrome heights,
content widths) with `style/tailwind.css` holding base element defaults and six
repeated primitives: `.btn`, `.icon-btn`, `.btn-quiet`, `.chip`, `.field`,
`.card`. None of them set layout — no flex, grid, width or margin. That is
deliberate: layout stays in utilities on the element, where there is no source
order for one rule to lose to another. The cascading stylesheet this replaced
produced nine separate bugs where two equal-specificity rules were resolved by
which happened to come last in the file.

Classes live inside `view!` macros, so the scanner reads `./src/**/*.rs`. A
class assembled from fragments at runtime is invisible to it — build the full
name as a literal (see `like.rs`, which swaps whole class strings).

### `LEPTOS_HASH_FILES` differs between dev and production, on purpose

Production serves content-hashed asset filenames, because Cloudflare's edge
caches `/pkg/*` per-POP for four hours and fixed filenames meant a coin-flip
between old and new CSS after a deploy. The Dockerfile sets
`LEPTOS_HASH_FILES=true` at **runtime**, which is required in addition to
`hash-files = true` in `Cargo.toml`: the build flag decides what filenames are
written to disk, the env var decides what filenames the HTML asks for. Setting
only one of the two ships a site whose HTML requests assets the origin doesn't
have.

`.env` sets it to `false` for local dev, because `cargo leptos watch`
regenerates `pkg/soncollection.css` on every edit but never refreshes the hashed
copy — so with hashing on, dev silently serves stale CSS and edits appear to do
nothing.

Note that `cargo-leptos` reads `.env` at **build** time too, so a local
`cargo leptos build --release` produces unhashed filenames. That is fine (CI
builds in a container with no `.env`), but it means the local release output is
not what gets deployed. To check the real artifact, build with `.env` moved
aside and run the binary with `LEPTOS_HASH_FILES=true`.

### Two SSR pitfalls, already fixed — don't reintroduce them

1. **`SsrMode::Async` on `/` and `/son/:id`.** With the default out-of-order
   streaming, `<head>` flushes before resources resolve, so `og:`/`twitter:` tags
   and all card markup landed in a `<template>` that only JS swaps in — invisible
   to crawlers and link unfurlers. Paired with `Resource::new_blocking`.
2. **Server functions need their own mount.** `.leptos_routes` registers page
   routes only. SSR still works without it (it calls the fns in-process), so the
   breakage only shows up on client-side calls, as a 404. See the
   `/api/{*fn_name}` route in `main.rs`.

## Layout

```
src/
  app.rs            document shell, router, nav
  api.rs            #[server] fns — gallery reads, likes, report
  upload_route.rs   POST /api/upload (plain Axum: multipart)
  d1.rs             Cloudflare D1 HTTP client (no sqlx driver exists for D1)
  db.rs             queries against d1.rs, keyset pagination
  storage.rs        decode → square → thumbnail → Backend (local | r2)
  dedupe.rs         SHA-256 of decoded pixels (exact duplicates only)
  gemini.rs         client for the screening sidecar
  jobs.rs           in-memory upload progress, polled by the upload page
  models.rs         types shared by server and wasm
  components/       gallery, card, detail, upload

sidecar/             Python: Gemini via browser cookies (screen + square)
keeper/              Python: a logged-in Chromium that keeps those cookies fresh
scripts/build-check.sh  builds the container images on a host that has Docker
Dockerfile           multi-stage build; pinned base digests, pinned Tailwind binary
docker-compose.yml   deployed as-is to bulky-server; app + cloudflared, no volumes
deploy/known_hosts   bulky-server's pinned SSH host key (public info, safe to commit)
```

## Moderation: Gemini, out of process

Screening happens in **Gemini**, reached through the Python sidecar in `sidecar/`
(`HanaokaYuzu/Gemini-API`, driven with browser cookies rather than an API key).
Two endpoints, one Gemini call each:

| | takes | returns |
| --- | --- | --- |
| `POST /judge` | ~3s | `{"verdict": "PASS"\|"FAIL", "topic": "SON"\|"NOTSON"}` |
| `POST /square` | ~30-80s | the square image |

Two calls rather than one prompt doing both, because a single prompt asking it to
judge *and* generate makes the model answer in prose and refuse the image ("I
cannot generate, edit, or modify images").

Two *endpoints* rather than one, because judging is seconds and generating is most
of a minute: the upload page reports which phase it is in, and a refusal comes
back in ~4s instead of waiting behind a generation that was going to be discarded.

The policy lives in `Verdict::acceptable`, not in the sidecar's status codes. It
fails closed — anything that is not an explicit `PASS` is a refusal — and the
visitor-facing reasons are written in Rust rather than echoed from the model, so
nothing it generates is ever rendered on the page.

`GEMINI_URL` unset ⇒ the step is skipped entirely and uploads publish unscreened.
Gemini unreachable ⇒ **the original is published unscreened** rather than the
upload being lost, on the grounds that an outage should not stop contributions.
Either way `storage::to_square` runs, so every stored image is 1024×1024.

The local CLIP ViT-B/32 model that used to do this (via `candle`) has been
removed, along with the embedding-based near-duplicate check that rode on the
same inference. `src/moderation/` is gone. Regenerated images carry Gemini's
SynthID; that is left intact.

An upload takes ~50 seconds, nearly all of it the image generation, so
`POST /api/upload` returns a job id and the page polls
`/api/upload/status/:id` — see `jobs.rs`.

### What still stops a bad upload

- **Format and size limits** in `storage::decode` (`MAX_PIXELS` rejects
  decompression bombs before decode, `MAX_UPLOAD_BYTES` caps the request).
- **Exact duplicate rejection** — SHA-256 of the *decoded pixel buffer*, so the
  same image re-saved in another format is still caught. This is a hash, not
  analysis: it says nothing about content.
- **The report queue.** `db::report` flips `is_public` to 0 after three reports,
  and `/admin` can hide or delete anything. This is now the *primary* moderation
  mechanism rather than a backstop.

### The schema keeps its score columns

`sons.son_score` and `sons.nsfw_score` are still `NOT NULL` from migration 0001
and are written as `0`. They were left in place rather than dropped so scores from
an external screening API have somewhere to land, and so no existing row loses
data. Nothing reads them: no UI shows them, and no sort or filter depends on them.
Read a `0` as "not assessed", not as "scored zero". `sons.embedding` is likewise
retained but no longer written.

### If you want review-before-publish instead

One branch in `upload_route::upload`: insert with `is_public = 0` and let `/admin`
release them. That is the only change needed — the admin queue and the
hide/unhide/delete paths already exist.

## Deploy

Runs on `bulky-server`, reachable only over Tailscale — no public inbound port,
not even for SSH. **No storage is allowed on the server itself**: everything
that needs to persist lives in a Cloudflare-managed service instead.

| Needs to persist | Lives in |
| --- | --- |
| App data (sons, likes, reports) | D1 |
| Images | R2 |
| Ingress (soncollection.com → the box) | Cloudflare Tunnel |

Three containers: `son-app`, `gemini` (the screening sidecar), and `cloudflared`.
Only cloudflared is reachable from outside. The sidecar publishes no port at all
and is addressed as `gemini` on the private compose network — it holds Google
session cookies, so it stays off the host's interfaces entirely.

`docker-compose.yml` has no `volumes:` section — there is nowhere in it a
volume could even be declared. The app container also runs `read_only: true`
with only a `tmpfs` `/tmp`, so this is enforced at the container level too, not
just by "nothing happens to write there."

### Why D1 over its HTTP API, and what that costs

The app is a native binary on a plain server, not a Cloudflare Worker, so there
is no in-process binding to D1 — every query is one HTTPS round trip via
`d1.rs`, a small hand-rolled client (no sqlx driver exists for D1; it isn't a
wire protocol sqlx speaks).

The real cost: **D1 has no transaction spanning two separate HTTP calls.** A
`batch` is atomic, but it's a fixed list of statements decided before the call
— nothing in it can branch on a value read earlier in the same batch. This is
why `db::toggle_like` is *not* "read whether liked, then branch": it uses
`INSERT ... ON CONFLICT DO NOTHING RETURNING` as an atomic try-and-check in one
statement, and recomputes `sons.likes` from `COUNT(*)` on every toggle rather
than incrementing/decrementing — self-healing against any drift from a request
that failed partway through, since the next successful toggle corrects it.

### Why local dev also uses D1

The alternative was a SQLite fallback for dev, matching D1 for prod. Rejected:
two divergent code paths — one SQLite-shaped, one D1-shaped — could behave
differently at exactly the place that matters most (the transaction limitation
above), and a bug that only exists in the path nobody's local machine exercises
is the worst kind to ship. One code path, always talking to the real thing.

### Ingress: Cloudflare Tunnel, not an open port

`cloudflared` runs as a second container on the same Docker network as the app,
reaching it by Compose service name (`http://son-app:3100`) — the app container
publishes no host port at all. `soncollection.com` is a CNAME to
`<tunnel-id>.cfargotunnel.com`; Cloudflare's edge terminates TLS and reaches the
box only through the tunnel's outbound-only connection. Nothing needs to be
opened in a firewall for web traffic, the same way the box was already
reachable only over Tailscale for SSH.

### Deploy pipeline

`.github/workflows/ci.yml`'s `deploy` job (pushes to `main` only, never for
`pull_request`): build the image → push to GHCR → join the tailnet as `tag:ci`
→ SSH to `bulky-server` → `docker compose up -d`.

**Authentication has two independent layers, easy to conflate:**
1. **Network**: the runner must be *on the tailnet* at all (via
   `tailscale/github-action` and a tagged, ephemeral pre-auth key).
2. **SSH**: `bulky-server` has Tailscale SSH enabled — the connecting node's
   tailnet identity, checked against the tailnet's ACL, *is* the credential.
   There is no keypair, no `authorized_keys` entry, nothing to rotate on the
   SSH side. `deploy/known_hosts` pins the box's host key anyway, for the
   classic transport-layer guarantee, but it is not the actual auth boundary.

The tailnet ACL had to be changed for this to work at all: `bulky-server` was
an untagged personal device, and Tailscale's SSH `dst` field only accepts a
tag, `autogroup:self`, or a same-named user — **never a raw IP** — so a tagged
CI runner could not have reached it no matter what SSH credential it carried.
It now carries `tag:bulky` for exactly this reason.

**Secrets are GitHub Actions secrets, injected as `docker run`/`compose`
environment variables at deploy time — never written to a file on
`bulky-server`, and never baked into either image's layers.** A public image
in a public repo's registry is not a safe place for a credential regardless of
the registry's declared visibility (layer history persists, and history has a
way of becoming public even when the current state isn't). "All secrets set in
GH secrets" is compatible with "must be secure" only if they never touch disk
or a layer on the way to the running container — this is why.

The GitHub Actions secrets this depends on: `CF_ACCOUNT_ID`,
`CF_D1_DATABASE_ID`, `CF_D1_API_TOKEN`, `R2_ACCESS_KEY_ID`,
`R2_SECRET_ACCESS_KEY`, `R2_BUCKET`, `R2_PUBLIC_BASE`, `CF_TUNNEL_TOKEN`,
`TS_CI_AUTHKEY`. All were scoped to the minimum each needed (a D1-only token, an
R2-bucket-only token, a tunnel-only token) except the D1 token, which the
Cloudflare API can only scope to *all* D1 databases on the account — there is
no per-database resource identifier for D1 tokens today.

`TS_CI_AUTHKEY` expires **2026-11-09** (90-day key). Deploys will start failing
with a Tailscale auth error after that date until it's rotated.

### Base images are pinned by digest, not a tag

`rust:1-bookworm` and `debian:bookworm-slim` move; a public repo's build should
not silently pull in whatever those tags point to on a given day. Re-pin
deliberately: `docker pull <image> && docker inspect --format '{{index
.RepoDigests 0}}' <image>`. Same reasoning applies to `cloudflared`'s image in
`docker-compose.yml` — it has direct control over what reaches
soncollection.com, so it isn't exempted just for living in a different file.

## Not built yet

- **Content screening.** Nothing checks what an upload depicts. The intended
  replacement is an external API called from `upload_route::upload`, writing its
  verdict into the `son_score`/`nsfw_score` columns that are still there waiting
  for it.
- Near-duplicate detection (a resize or recompress of an existing son). The
  exact-hash check does not catch these; a perceptual hash (`img_hash`) would,
  without any content analysis.
- Google sign-in is built but dormant — the redirect URIs still need registering
  in the Google console.
- Cloudflare Web Analytics is wired but dormant: `CF_ANALYTICS_TOKEN` needs a
  site created in the Cloudflare dashboard (the D1-scoped API token cannot do
  it). Edge-side analytics — requests, bandwidth, cache ratio, threats — are
  already on for the proxied domain and need nothing.
- The generator
