# son collection

A gallery for the **"Son 😭😭😭😭😭"** meme and its endless variants — Sonion,
Capri-Son, Dy-Son, Sonflower, Sontato. Free uploads, no accounts.

Pure Rust, front to back.

## Stack

| Layer      | Choice | Why |
| ---------- | ------ | --- |
| UI         | Leptos 0.8 (SSR + hydrate) | Reactive components in Rust; SSR keeps the OG/Twitter cards working, which is how a meme site actually spreads |
| Server     | Axum 0.8 | What `leptos_axum` runs on anyway |
| DB         | SQLite via `sqlx` 0.8 | Zero ops, WAL for concurrent reads during uploads. Runtime-checked queries so a fresh clone builds without a live DB |
| Images     | `image` 0.25 | Decode, bound, thumbnail |
| Moderation | CLIP ViT-B/32 via `candle` | Pure Rust, CPU. One pass gives NSFW score, son score, and the embedding |
| Storage    | Cloudflare R2 (local disk in dev) | Behind a `Backend` trait; images served from media.soncollection.com, not proxied |

## Running it

```bash
rustup target add wasm32-unknown-unknown
cargo install cargo-leptos --locked

cp .env.example .env
cargo leptos watch          # http://127.0.0.1:3100
```

Port 3100, not 3000 — 3000 was already taken by an unrelated dev server.

`SITE_ORIGIN` must be set to the real origin in production (e.g.
`https://soncollection.com`). Link unfurlers reject relative `og:image` URLs, so
without it Discord and Twitter previews render with no image.

To reset local state: `rm -f sons.db* uploads/orig/* uploads/thumb/*`

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
  api.rs            #[server] fns — gallery reads, report
  upload_route.rs   POST /api/upload (plain Axum: multipart)
  db.rs             SQLite, keyset pagination
  storage.rs        decode → bound → thumbnail → Backend (local | r2)
  moderation/       Moderator trait: clip | stub | deny
  models.rs         types shared by server and wasm
  components/       gallery, card, detail, upload
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

## Not built yet

- Evaluating the NSFW gate against a real benchmark (see above)
- Perceptual-hash dedupe on repost (`img_hash`)
- Admin view for the report queue
- Google sign-in — needs an OAuth 2.0 Web Client ID; a service account cannot
  do user login
- Similarity search over the stored embeddings
- The generator
