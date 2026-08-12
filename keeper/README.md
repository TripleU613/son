# Session keeper

Keeps a logged-in Gemini browser session alive so screening stops dying, and
pushes the current cookies to the sidecar whenever they rotate.

**Nothing here logs in.** It drives a browser profile that is *already* signed in.
Producing that profile is a one-time human step:

```bash
# On any machine with a display. Chromium opens; sign in to the burner Google
# account, open gemini.google.com, then close the window.
pip install playwright==1.62.0 && playwright install chromium
python - <<'PY'
from playwright.sync_api import sync_playwright
with sync_playwright() as p:
    ctx = p.chromium.launch_persistent_context("./profile", headless=False)
    ctx.pages[0].goto("https://gemini.google.com/app")
    input("Sign in, then press Enter here to save and close...")
    ctx.close()
PY

# Check it before trusting it, then upload it.
tar czf profile.tar.gz -C ./profile .
aws s3 cp profile.tar.gz "s3://$R2_BUCKET/keeper/profile.tar.gz" \
  --endpoint-url "https://$CF_ACCOUNT_ID.r2.cloudflarestorage.com"
```

Verify a profile without deploying it:

```bash
docker run --rm -e LOGIN_CHECK_ONLY=1 \
  -e R2_ACCESS_KEY_ID -e R2_SECRET_ACCESS_KEY -e R2_BUCKET -e CF_ACCOUNT_ID \
  ghcr.io/tripleu613/son-keeper:latest
#   signed in: True  (found ['__Secure-1PSID', '__Secure-1PSIDTS'])
```

## Why the profile lives in R2

"No storage on the server" is a hard rule here, and a browser profile is state. It
is pulled to `/tmp` at boot and pushed back when the cookies change, so the durable
copy sits in Cloudflare alongside the database and the images — and a container
restart resumes the same session instead of needing a fresh login.

## When it can't fix itself

If the profile gets signed out (password change, session revoked, Google deciding
otherwise), the log says so explicitly and no amount of retrying will help — redo
the one-time login above. Until then the sidecar's `/health` reports down, the
admin page says so, and uploads are **held** rather than published unscreened.

The paste box in `/admin` remains the manual override, and is the faster route for
a one-off.
