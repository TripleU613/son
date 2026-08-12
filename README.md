# Son Collection

A gallery dedicated to the **“Son 😭😭😭😭😭”** meme and its endless variants:

**Sonion. Capri-Son. Dy-Son. Sonflower. Sontato.**

Uploads are free and displayed without public attribution.

## What is this?

The goal is to build the most complete collection of Son memes possible.

Once the collection is large enough, we plan to publish a cleaned and deduplicated dataset on Hugging Face so researchers, artists, and AI agents can study the format—and perhaps generate even more Sons.

## Upload rules

Please help keep the collection useful and safe:

- **No NSFW, illegal, hateful, violent, or otherwise harmful content.**
- **Upload Son memes only.**
- **Upload originals whenever possible.**
- **Check for duplicates before uploading.** We do not need a billion copies of the same Son floating around.
- **Crop images before uploading.** Please remove screenshots, browser chrome, social-media interfaces, notifications, and unrelated borders.
- **Only upload content you have the right to share.**
- Do not include personal information or images that violate someone’s privacy.

Uploads may be removed at any time if they break these rules or create legal, safety, privacy, or moderation concerns.

## Contributing

The best way to contribute is simple:

1. Find Sons in the wild.
2. Check whether they are already in the collection.
3. Crop and clean the image.
4. Upload it.

Code contributions are also welcome. The repository is public, and anyone brave enough to improve the infrastructure is appreciated.

## AI-generated Sons

Have time and a questionable idea?

Generate a Son with AI and submit it. We welcome tasteful AI slop here—just label AI-generated uploads clearly and follow the same content rules.

## Privacy

Uploads are shown without public attribution, but this does **not** necessarily mean complete technical anonymity. The service or its infrastructure may temporarily process information such as IP addresses, timestamps, and request logs for security, abuse prevention, and maintenance.

Do not upload anything that could identify you or another person.

## Rights and licensing

The license covering this repository’s **source code** does not automatically apply to user-uploaded images.

By uploading an image, you confirm that:

- you created it, have permission to share it, or reasonably believe its use is lawful;
- it does not violate another person’s copyright, trademark, privacy, or other rights; and
- you grant the project permission to store, display, process, moderate, and include it in downloadable research datasets.


## Commercial use

This project and its dataset are intended for personal, cultural, archival, and research use.

**Commercial use of the dataset or hosted collection is not permitted without prior written permission.**

See the repository license and dataset terms for the exact conditions.

## Infrastructure

The infrastructure is not perfect. Please do not scrape aggressively, bypass rate limits, automate abusive uploads, probe for vulnerabilities, or otherwise make operating the project harder than it already is.


## Disclaimer

This is an experimental community archive. Availability is not guaranteed, uploads may be removed without notice, and the collection may contain material submitted by third parties.

The maintainers do not endorse every upload.

---

<details>
<summary><b>Technical notes</b> — running it, developing it, deploying it</summary>

Pure Rust, including the UI: [Leptos](https://leptos.dev) 0.8 (SSR + hydration) on
Axum, Tailwind for styling, Cloudflare D1 for data and R2 for images. The only
non-Rust parts are a Python service that asks Gemini to screen and square each
upload, and the browser that keeps that service's session alive.

**No local database.** Dev talks to the same D1 and R2 as production — there is no
sqlite fallback to drift from. Clean up anything you upload while testing.

### Run it locally

```bash
cp .env.example .env          # fill in the Cloudflare values; the rest degrade
                              # gracefully when unset
npm install                   # Tailwind only; the image builds without Node
cargo install cargo-leptos --locked
cargo leptos watch            # http://127.0.0.1:3100
```

`watch`, not `serve` — `serve` doesn't watch anything. Kill the old process first,
or you'll review the previous build.

### The gate, before you push

CI takes ~20 minutes and the image build is last, so a mistake there costs half an
hour to learn something these say in three minutes:

```bash
cargo fmt --all -- --check
cargo clippy --no-default-features --features ssr --all-targets -- -D warnings
cargo clippy --no-default-features --features hydrate --target wasm32-unknown-unknown -- -D warnings
cargo test  --no-default-features --features ssr
scripts/audit-secrets.sh      # no credential in history, files or build output
```

Both feature sets, always: `ssr` and `hydrate` compile disjoint code from the same
files, so one passing tells you nothing about the other. `git config core.hooksPath
.githooks` runs all of it on every push.

`scripts/build-check.sh` builds the container on a host that has Docker (`--run`
starts it and checks all four processes come up, `--audit` scans its layers). The
Dockerfile is the one thing that cannot be tested without a container runtime, and
every CI failure this project has had was reproducible locally first.

### Deploy

One image, one container, nothing else: the site, the Gemini sidecar, the sign-in
browser and `cloudflared` are four processes under supervisor in a single package
(`Dockerfile`, `deploy/supervisord.conf`, `docker-compose.yml`). Push to `main` and
GitHub Actions builds it, pushes it to GHCR by digest, and runs `docker compose up`
on the host over Tailscale SSH. There is nothing to click.

Every credential is a GitHub Actions secret, injected as runtime environment — none
is a build argument, none is in an image layer, and there is no `.env` on the
server. `scripts/audit-secrets.sh` exists to keep that a fact rather than an
intention: it scans every object in git history, every tracked file, the compiled
binary, the wasm every visitor downloads, and (with `--image`) every layer and the
config of the built image, for every value in `.env` plus a set of credential
shapes. It runs in CI and in the pre-push hook.

The container publishes no port. `cloudflared` reaches the app on loopback inside
it, so the host needs no inbound rule for web traffic.

### Before you change anything

`CLAUDE.md` is the list of things that have already cost real time here — the
router that intercepts anchors and 404s them, the asset hashing that silently
serves a style-less site, the D1 behaviour that has to be verified against the live
database, the serde tagging that hydrates fine until it doesn't. It is not
documentation of how the code works; it is a list of traps, and it is short because
each entry was paid for.

</details>

Made for the Sons.
