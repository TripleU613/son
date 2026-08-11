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
| Moderation | CLIP ViT-B/32 via `candle` | Pure Rust, CPU. One pass gives NSFW score, son score, and the embedding |
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
  storage.rs        decode → bound → thumbnail → Backend (local | r2)
  moderation/       Moderator trait: clip | stub | deny
  models.rs         types shared by server and wasm
  components/       gallery, card, detail, upload

Dockerfile           multi-stage build; CLIP weights baked in, not fetched at runtime
docker-compose.yml   deployed as-is to bulky-server; app + cloudflared, no volumes
deploy/known_hosts   bulky-server's pinned SSH host key (public info, safe to commit)
```

## Moderation

The site **auto-publishes anything that clears the thresholds.** There is no
review queue, so `src/moderation/` is the only thing between an upload and the
front page.

The real classifier is **CLIP ViT-B/32 running on CPU through
[`candle`](https://github.com/huggingface/candle)** — pure Rust, no ONNX C++
dependency. One image forward pass answers both questions and yields the
embedding, which is why CLIP was chosen over a dedicated NSFW model.

Scoring is **contrastive, not absolute**. A raw cosine similarity to "porn"
means nothing on its own, so each question is a softmax over competing captions
and the score is the probability mass on the positive ones.

That detail matters. The first version used four benign captions and **a plain
yellow square scored 0.57 NSFW** — images resembling no benign caption had
nowhere to put their probability mass. The benign caption list is now
deliberately much longer than the explicit one. Measured after the fix:

| image | son | nsfw |
| --- | --- | --- |
| meme (caption + face) | 0.999 | 0.062 |
| plain colour square | 0.007 | 0.009 |
| spreadsheet screenshot | 0.002 | 0.033 |
| outdoor scene | 0.003 | 0.006 |
| advertising banner | 0.019 | 0.017 |
| random noise | 0.000 | 0.002 |

**If you add or reorder captions, re-run those numbers.** `NSFW_POSITIVE` and
`SON_POSITIVE` are counts of leading entries, so inserting a caption in the
wrong place silently reclassifies it.

### What has NOT been verified

The table above only shows **false positives** — safe images are not being
flagged. Nobody has tested this against actual explicit content, so the
**false-negative rate is unknown**: whether real NSFW material trips
`NSFW_MAX = 0.5` is unmeasured. Treat the NSFW gate as unproven until it is
evaluated against a real benchmark.

### Backends

`MODERATION_BACKEND` selects: `clip` (default), `stub`, `deny`.

If CLIP fails to load, the app falls back to **`deny`, not `stub`** — every
upload is refused while the gallery keeps serving. A model-loading failure must
not become an open door. `stub` detects nothing and has to be asked for by name.

### Weights

`CLIP_MODEL_DIR` (default `models/clip-vit-base-patch32`) must contain
`pytorch_model.bin` and `tokenizer.json`. The app does **not** download them: an
`hf-hub` dependency pulled in ureq → rustls 0.21 → rustls-webpki 0.101.7 and a
HIGH advisory, to serve a path only used once per machine. Fetch them yourself,
and ship them with the deploy rather than pulling 600MB at boot.

That repo publishes **no safetensors** — only `pytorch_model.bin`, loaded via
`VarBuilder::from_pth`. Preferred over a third-party safetensors mirror for a
file that decides what gets published.

```bash
mkdir -p models/clip-vit-base-patch32 && cd $_
for f in pytorch_model.bin tokenizer.json; do
  curl -sLO "https://huggingface.co/openai/clip-vit-base-patch32/resolve/main/$f"
done
```

### Why every upload stores an embedding

`sons.embedding` is populated from the first upload even though nothing reads it
yet. It is the dataset for dedupe, "similar sons", and eventually a generator —
and it cannot be backfilled for images that were never embedded.

### Safety valve

Auto-publish is survivable because of `db::report`: three reports flip
`is_public` to 0 automatically, and any son can be pulled with one UPDATE. If you'd
rather hold uploads for review instead, it's one branch in `upload_route::upload`
(insert with `is_public = 0`).

## Deploy

Runs on `bulky-server`, reachable only over Tailscale — no public inbound port,
not even for SSH. **No storage is allowed on the server itself**: everything
that needs to persist lives in a Cloudflare-managed service instead.

| Needs to persist | Lives in |
| --- | --- |
| App data (sons, likes, reports) | D1 |
| Images | R2 |
| Ingress (soncollection.com → the box) | Cloudflare Tunnel |

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

- Evaluating the NSFW gate against a real benchmark (see above)
- Perceptual-hash dedupe on repost (`img_hash`)
- Admin view for the report queue
- Google sign-in — needs an OAuth 2.0 Web Client ID; a service account cannot
  do user login
- Similarity search over the stored embeddings
- The generator
