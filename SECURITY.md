# Security

## Reporting a vulnerability

Open a [GitHub issue](https://github.com/TripleU613/son/issues) for anything
that isn't sensitive. For anything you'd rather not post publicly (a real
exploit against the live site, a credential leak, an auth bypass), email
usher@rndsoftwaregroup.com instead of filing an issue.

## What runs where

- **No secret is ever committed.** `.env` is gitignored; `.env.example` holds
  no real values. If you find one in the history anyway, treat it as
  compromised and say so in the report — it doesn't matter whether it's still
  "valid," it needs rotating.
- **No secret is baked into a Docker image layer.** Every credential the app
  needs (D1, R2, the tunnel token) arrives as a runtime environment variable
  injected by the CI deploy step, never an `ENV`/`ARG` in the `Dockerfile` and
  never a file written to `bulky-server`'s disk. `docker history` on either
  published image should show no secret material — if it ever does, that's a
  real finding, not a style nit.
- **The server holds no application data.** `bulky-server` has no volumes and
  runs the app container `read_only`. Sons live in D1, images live in R2. A
  compromised container has nothing local worth taking beyond whatever
  environment variables it was handed for that run.
- **Base images are pinned by digest**, not a movable tag — see the README's
  Deploy section for why and how to re-pin.
- **Moderation is unproven on real explicit content.** The CLIP-based NSFW
  gate has been checked for false positives (safe images wrongly rejected) but
  not against a real abuse benchmark — see the README's Moderation section.
  Don't treat `nsfw_score < 0.5` as a guarantee.

## Credential exposure history

This repository's Cloudflare, Tailscale, and R2 credentials were rotated
multiple times during initial setup after being pasted into a chat transcript
rather than set directly. If you're the one operating this going forward:
prefer writing credentials straight into `.env` or a secret manager over
typing them into any chat interface, including this one — a transcript is a
durable record whether or not the conversation feels ephemeral.
