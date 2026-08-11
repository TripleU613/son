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
| Moderation | trait `Moderator` | **Currently a stub — see below** |

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
  storage.rs        decode → bound → thumbnail → disk
  moderation/       Moderator trait + stub
  models.rs         types shared by server and wasm
  components/       gallery, card, detail, upload
```

## Moderation: read this before deploying

The site **auto-publishes anything that clears the thresholds.** There is no
review queue. Right now the only classifier is `moderation::stub`, which:

- does **not** detect NSFW content
- does **not** detect sons

It checks aspect ratio and minimum dimensions, and passes everything else with
`nsfw_score: 0.0`. The server logs a warning at startup saying so.

**Do not put this on the public internet as-is.** The intended replacement is
CLIP via [`candle`](https://github.com/huggingface/candle) (pure Rust, no C++
ONNX dependency), scoring each upload zero-shot against two prompt sets:

- NSFW prompts → `nsfw_score`
- `"the Anthony Mackie Son meme"`, `"a sunflower"`, … → `son_score`

Implement `Moderator` in `src/moderation/clip.rs` and swap the one line in
`main.rs`. Nothing else changes.

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

- CLIP moderation (above)
- Perceptual-hash dedupe on repost (`img_hash`)
- Admin view for the report queue
- S3/R2 storage — swap `storage::store`, it's the only place that touches disk
- The generator
