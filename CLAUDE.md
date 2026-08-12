# Working on son collection

Pure-Rust gallery for the "son" meme, Leptos UI included. Deployed as one Docker
image, no writable storage. Read this before changing anything.

## Run the full gate locally before pushing. Never use CI to find out.

CI takes ~23 minutes and the Docker build is the last thing in it, so a mistake
there costs half an hour to learn something a local command would have said in
three minutes. Every CI failure so far was reproducible locally first.

```bash
cargo fmt --all -- --check
cargo clippy --no-default-features --features ssr --all-targets -- -D warnings
cargo clippy --no-default-features --features hydrate --target wasm32-unknown-unknown -- -D warnings
cargo test  --no-default-features --features ssr
```

**Both feature sets, always.** `ssr` and `hydrate` compile different code from
the same files (`#[cfg(feature = "hydrate")]` is everywhere), so one passing
tells you nothing about the other.

### Dockerfiles cannot be tested here, so test them elsewhere

There is no container runtime on this machine and no sudo to install one, so a
`RUN` step is only exercised by CI — which is how three consecutive CI runs were
burned (a base digest that did not resolve, then `awscli` having no installation
candidate on the Playwright base).

`scripts/build-check.sh` pipes each build context over SSH to **bulky-server**,
which does have Docker, and builds there. ~30s for the sidecar, ~3min for the
keeper. Run it whenever a Dockerfile or a requirements file changes. It is not in
the pre-push hook on purpose: pulling a 1.9GB base is the wrong tax on every push.

The pre-push hook (`.githooks/pre-push`, enable with
`git config core.hooksPath .githooks`) covers everything else, including that
pinned image digests actually resolve — fetch the manifest **by digest** with the
manifest-list Accept headers, or you get a real-but-wrong digest that fails the
build with a bare "not found".

### Then reproduce the container, because your machine is not it

A plain local build passes with things the image does not have. This exact gap
has now broken CI once and production once:

```bash
mv node_modules ../node_modules.aside   # the image has no Node toolchain
mv .env .env.aside                      # cargo-leptos reads .env AT BUILD TIME
PATH="/path/to/standalone/tailwindcss/dir:$PATH" \
  env -u LEPTOS_HASH_FILES cargo leptos build --release
mv ../node_modules.aside node_modules && mv .env.aside .env
```

Then check the two things that silently disagree, because a broken build here
still returns HTTP 200 with no styles:

```bash
cat target/release/hash.txt          # every filename must exist in
ls  target/site/pkg/                 # target/site/pkg, hash for hash
```

For a real end-to-end check, copy `target/release/soncollection`,
`target/release/hash.txt` and `target/site` into one directory (the container's
`/app` layout), run it with `LEPTOS_HASH_FILES=true LEPTOS_SITE_ROOT=site`, and
confirm the assets the HTML asks for actually return 200.

## Traps that have already cost real time

**`recursion_limit` fails only in release.** `#![recursion_limit = "512"]` must
stay in **both** `lib.rs` and `main.rs`. Debug builds pass without it.

**Asset hashing is two switches, not one.** `hash-files = true` in `Cargo.toml`
decides what filenames land on disk. `LEPTOS_HASH_FILES` at runtime decides what
filenames the HTML requests. Set one without the other and the site serves 200s
with no CSS and no wasm. It shipped that way once and only looked fine because
Cloudflare still had the old files cached.

**Dev deliberately turns hashing off.** `cargo leptos watch` regenerates
`pkg/soncollection.css` but never the hashed copy, so with hashing on, every edit
is invisible in the browser and you will debug CSS that was never served.

**`cargo leptos serve` does not watch.** Use `watch`. And kill the old process
first — a stale binary squatting on 3100 means you review the previous build.
`pkill -f 'cargo leptos ser[v]e'` (brackets stop `pkill` matching its own shell,
which otherwise kills the command that ran it).

**Signal writes during render crash hydration.** Not a warning — the wasm module
dies and the whole page goes inert. Seeding state from props belongs in
`Effect::new`, which is client-only by construction.

**`SsrMode::Async` + `Resource::new_blocking` on `/` and `/son/:id`.** Default
out-of-order streaming flushes `<head>` before resources resolve, so `og:` tags
and card markup land in a `<template>` that only JS swaps in — invisible to
every crawler and link unfurler.

**Responsive layout is CSS-only.** Never branch on `window.innerWidth`: the
server has no window, so the two renders disagree and hydration breaks.

**`Action::new`'s closure must return a future.** A bare `return;` will not
compile; guards go inside the async block.

**`<Show>` needs a genuinely reactive `when`.** For a condition fixed at mount
(does this son have tags?) use `.then()` instead — `<Show>` there produces a real
Fn-vs-FnOnce error from nested `<For>` closures.

**`leptos_router::A` has an `exact` prop**, not `attr:exact`. Without it every
route matches "/" and the Gallery link is `aria-current` everywhere.

## Styling: Tailwind, and why the cascade is banned

`style/main.scss` was deleted. It produced nine bugs where two equal-specificity
rules were resolved by whichever came last in the file. Utilities have no source
order to lose to.

- `tailwind.config.js` holds the palette, radii, chrome heights, content widths.
  It is a **build input** and must be `COPY`'d into the Docker builder.
- `style/tailwind.css` holds base element defaults and six primitives: `.btn`,
  `.icon-btn`, `.btn-quiet`, `.chip`, `.field`, `.card`. **None set layout** — no
  flex, grid, width or margin. Layout stays on the element.
- Classes live in `view!` macros, so `content` scans `./src/**/*.rs`. A class
  assembled from fragments at runtime is invisible to the scanner: build the full
  literal (see `like.rs`, which swaps whole class strings rather than toggling
  utilities on top of conflicting base ones).
- Don't toggle a utility against a base class that sets the same property. Both
  land, equal specificity, and the winner depends on stylesheet order — the exact
  thing this migration removed.

### CSS facts learned the hard way

- `aspect-ratio` does not clamp in-flow content. An `<img>` with `height: 100%`
  falls back to its intrinsic ratio. Use `position: absolute; inset: 0`.
- `margin: 0 auto` on a flex item cancels cross-axis stretch. Add `width: 100%`.
- An implicit grid track is sized by its widest item's **min-content**, which can
  exceed the container and push the page sideways. `grid-cols-[minmax(0,1fr)]`
  or `min-w-0` on the item.
- `<input type="file">` has ~344px of native chrome that `max-width` will not
  shrink. Hide it (`sr-only`) behind a label that is the real drop zone, and move
  the focus ring with `has-[:focus-visible]`.

## Cloudflare D1

No transactions over HTTP. Counters self-heal via `COUNT(*)` rather than
increments; `INSERT ... ON CONFLICT DO NOTHING RETURNING` is the atomic
test-and-set. `COLLATE NOCASE` is byte-wise (space `0x20` sorts before hyphen
`0x2D`). `sqlite_version()` is blocked; probe features by trying them.

**Verify every D1-specific mechanism against the live database before relying on
it.** Create a throwaway `zz_*` table, test, drop it, confirm it is gone.

Search uses `sons_search`, FTS5 with `tokenize='trigram'` over title **and** tag
names (migration 0007). Trigram matches substrings, so "flower" finds Sonflower —
which matters here, since the entire joke is words with "son" buried inside.
Terms shorter than 3 characters produce no trigrams and match nothing, silently,
so those take a LIKE path. A bare `-` in an FTS query means NOT and errors with
`no such column`, so user terms are always quoted as phrases. The old word-token
index `sons_fts` is left in place, unused, so search can be rolled back by
reverting code alone.

**There is no migration runner.** Migrations are applied by hand against live D1,
statement by statement (trigger bodies contain semicolons, so naive splitting on
`;` sends fragments). Validate a trigger's body as a standalone SELECT before
installing it — a bad trigger on `sons` breaks every future upload.

## Slugs, not ids, in URLs

`/son/:slug` (migration 0008). `db::get` matches slug **or** id in one query, so
every link shared before slugs existed still resolves. Build links from
`son.slug`, never `son.id`. `db::unique_slug` appends -2, -3 on collision and the
UNIQUE index is the real guard.

## Two routers, and the anchors that fall between them

`main.rs` owns the Axum routes (`/auth/*`, `/admin/browser`, `/son/:slug/download`,
`/api/*`, `/embed/*`, `/oembed`, `robots.txt`, `sitemap.xml`, `llms.txt`).
Everything else is a Leptos route in `app.rs`.

`leptos_router` intercepts **every** same-origin `<a>` click and resolves it
against its own table unless the anchor has `download` or `rel="external"`
(`leptos_router`'s `location/mod.rs`). So a plain anchor to an Axum route renders
the 404 page while the endpoint itself is perfectly fine. This has happened four
times: the sign-in link (three separate call sites), the download button, and the
`/admin/browser` link. It is invisible in review and compiles clean.

- Linking to an Axum route: add `rel="external"`, or `download` if it is a file.
- Linking to a Leptos route: use `<A>` and add nothing.
- Sign-in specifically: use `components::sign_in::SignInLink`, which owns the
  attribute. Do not hand-roll the href.
- `tests/router_links.rs` parses the route list out of `main.rs` and fails on any
  anchor that would be intercepted. A new Axum route is covered the moment it is
  registered.

## Hidden sons are visible to admins only, and only through the app

`is_public = 0` means held (screening was down) or auto-hidden (3 reports). The
report queue links to those sons, so `api::get_son` and `public_route::download`
serve them to an admin and 404 for everyone else — an admin has to see what they
are deleting. The detail page badges them "Hidden". The public API, sitemap,
search, gallery and embed exclude them for **everyone**, admin included: those are
publication surfaces, not review tools.

## serde tagging and server-fn payloads

`#[serde(tag = "...")]` cannot encode a newtype variant holding a `Vec` — it needs
a place to put the tag key and an array has none. It compiles, server-renders
correctly, and then panics inside `leptos_server`'s resource serializer while
preparing the value for hydration: the page arrives complete and never hydrates,
so the browser hangs with nothing on screen and curl reports a perfect 200. Use
`tag = "..." , content = "..."` (adjacent tagging), and round-trip every variant
through `serde_json` in a test — see `AdminQueue`.

## Admin is an env var, not a database flag

`ADMIN_EMAILS` (comma-separated) is re-checked at every Google login and written
to `users.is_admin`. A manual `UPDATE users SET is_admin = 1` is reverted the next
time that person signs in — the env var is the source of truth. A hardcoded list
would be a disclosure in a public repo.

## Local dev writes to the production database

There is no local database, deliberately. Local testing touches real data. Clean
up any seeded test sons from D1 and R2 afterwards.

## Verify by measuring, not by looking

Claims about layout need numbers from a real browser: `getBoundingClientRect`,
computed styles, `grep` against the *compiled* CSS. Three separate "fixes" in
this project silently did nothing and were only caught by measuring. And confirm
you are measuring the file the browser actually loaded — check the `<link>` href
against what is on disk.

Screenshot at 320 / 360 / 393 / 768 / 1024 / 1440 / 2560. The recurring failures
are horizontal overflow, header/content width mismatch, and a footer floating
above the fold on short pages.

## Moderation lives in Gemini, in a Python sidecar

`sidecar/gemini_service.py` judges and squares every upload; `src/gemini.rs` is
the Rust client. Things that will bite you there:

- **Two Gemini calls, never one.** A prompt that asks it to judge *and* generate
  makes it answer in prose: "I cannot generate, edit, or modify images."
- **Two endpoints, never one.** `/judge` is ~3s, `/square` is ~30-80s. Behind one
  endpoint the app can only label the whole wait "Scanning", and a refusal has to
  wait out a generation it is about to throw away.
- **The accept/refuse policy is `Verdict::acceptable`**, not HTTP status codes.
  Fail closed, and never render the model's own words at a visitor.
- **Upload files by path, not `BytesIO`.** `gemini_webapi`'s uploader needs an
  explicit filename for in-memory data and `generate_content` gives no way to
  pass one, so a BytesIO silently attaches nothing and the model then describes
  an image it never saw.
- **Flash judges, Pro generates.** Both read images; Pro is the one that reliably
  returns one.
- `GEMINI_URL` unset ⇒ skipped. Unreachable ⇒ original published unscreened, on
  purpose: an outage must not stop uploads. Distinguish `Rejected` (a decision)
  from `Unavailable` (a malfunction) — they must never be collapsed.
- Uploads take ~50s, so they are jobs (`jobs.rs`, in-memory) that the page polls.
  A restart loses in-flight jobs; the browser is told, rather than hanging.

- The only non-Gemini pre-publish checks are format/size limits in
  `storage::decode` and the exact-duplicate pixel hash in `dedupe`.
- `sons.son_score` / `sons.nsfw_score` are still `NOT NULL` in the schema and are
  written as literal `0`. They are kept so an external screening API has somewhere
  to write. A `0` means "not assessed" — do not build anything that reads them as
  a score, and do not sort or filter on them.
- `/admin` and the three-report auto-hide are now the *primary* moderation
  mechanism, not a backstop. Treat them as load-bearing.
- `main.rs` logs `content moderation: NONE` at every start. Keep that, or
  something like it — the absence of screening should never be a surprise.

## Open risks, not fixed

- A site that auto-publishes with no screening at all. This is a deliberate,
  stated choice, not an oversight — but it is the risk.
- `/api/v1/sons/:id` still exposes `reports` and `is_public` on a public
  endpoint.
