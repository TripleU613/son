"""Keeps a logged-in Gemini browser session alive and feeds its cookies to the
sidecar.

The problem this solves: the Gemini web client authenticates as a browser
session, and a session that nothing uses expires. Pasting cookies by hand works
but has to be redone every time, and the failure is silent -- uploads just start
getting held.

A real browser does not have that problem. A logged-in Chromium that loads
gemini.google.com every so often refreshes its own `__Secure-1PSIDTS` the way any
open tab does, and its cookie jar is always current. This reads that jar and POSTs
the values to the sidecar's /cookies endpoint.

**Logging in is a human step, once.** Nothing here types a password or handles
credentials: it drives a browser profile that is already authenticated. See
`keeper/README.md` for how to produce that profile, and `LOGIN_CHECK_ONLY=1` to
verify one before deploying it.

Where the profile lives: R2, not the server. "No storage on the server" is a hard
rule for this project, and a browser profile is state. It is pulled into /tmp at
boot, and pushed back whenever the cookies change -- so the durable copy is in
Cloudflare like everything else, and a container restart resumes the same session
instead of needing a fresh login.
"""

from __future__ import annotations

import asyncio
import logging
import os
import shutil
import subprocess
import tarfile
import tempfile

import httpx
from playwright.async_api import async_playwright

log = logging.getLogger("session-keeper")
logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(message)s")

GEMINI_URL = "https://gemini.google.com/app"
SIDECAR = os.environ.get("SIDECAR_URL", "http://gemini:8099")
SIDECAR_KEY = os.environ.get("SIDECAR_KEY", "")

# Well inside the ~30 minute __Secure-1PSIDTS rotation window, so the jar is
# never more than one cycle stale.
REFRESH_SECONDS = int(os.environ.get("KEEPER_REFRESH_SECONDS", "600"))

PROFILE_DIR = "/tmp/profile"
PROFILE_KEY = os.environ.get("KEEPER_PROFILE_KEY", "keeper/profile.tar.gz")

# The two cookies gemini_webapi needs. The rest of the jar is irrelevant to it.
WANTED = ("__Secure-1PSID", "__Secure-1PSIDTS")


def _r2_env() -> dict[str, str] | None:
    keys = ("R2_ACCESS_KEY_ID", "R2_SECRET_ACCESS_KEY", "R2_BUCKET", "CF_ACCOUNT_ID")
    if not all(os.environ.get(k) for k in keys):
        return None
    return {k: os.environ[k] for k in keys}


def _aws(*args: str) -> subprocess.CompletedProcess:
    """R2 over the S3 API, via the aws CLI.

    The CLI rather than boto3 because this container needs it for exactly two
    operations and the CLI is one apt line, where boto3 is a dependency tree to
    keep pinned for a copy and a paste.
    """
    env = dict(os.environ)
    env["AWS_ACCESS_KEY_ID"] = os.environ["R2_ACCESS_KEY_ID"]
    env["AWS_SECRET_ACCESS_KEY"] = os.environ["R2_SECRET_ACCESS_KEY"]
    env["AWS_DEFAULT_REGION"] = "auto"
    endpoint = f"https://{os.environ['CF_ACCOUNT_ID']}.r2.cloudflarestorage.com"
    return subprocess.run(
        ["aws", "s3", *args, "--endpoint-url", endpoint],
        env=env,
        capture_output=True,
        text=True,
    )


def restore_profile() -> bool:
    """Pull the profile out of R2. False means "no profile yet, log in first"."""
    if not _r2_env():
        log.warning("R2 not configured; the profile will not survive a restart")
        return os.path.isdir(PROFILE_DIR)

    bucket = os.environ["R2_BUCKET"]
    with tempfile.NamedTemporaryFile(suffix=".tar.gz", delete=False) as tmp:
        dest = tmp.name
    res = _aws("cp", f"s3://{bucket}/{PROFILE_KEY}", dest)
    if res.returncode != 0:
        log.warning("no stored profile (%s)", res.stderr.strip()[:200])
        return False

    os.makedirs(PROFILE_DIR, exist_ok=True)
    with tarfile.open(dest, "r:gz") as tar:
        # filter="data" refuses absolute paths and traversal in member names. The
        # archive is ours, but an extract that trusts its input is a bad habit to
        # write down.
        tar.extractall(PROFILE_DIR, filter="data")
    os.unlink(dest)
    log.info("profile restored from R2")
    return True


def save_profile() -> None:
    """Push the profile back, so the next container resumes this session."""
    if not _r2_env():
        return
    bucket = os.environ["R2_BUCKET"]
    with tempfile.NamedTemporaryFile(suffix=".tar.gz", delete=False) as tmp:
        dest = tmp.name
    with tarfile.open(dest, "w:gz") as tar:
        tar.add(PROFILE_DIR, arcname=".")
    res = _aws("cp", dest, f"s3://{bucket}/{PROFILE_KEY}")
    os.unlink(dest)
    if res.returncode == 0:
        log.info("profile saved to R2")
    else:
        log.error("could not save profile: %s", res.stderr.strip()[:200])


async def push_cookies(psid: str, psidts: str) -> bool:
    async with httpx.AsyncClient(timeout=120) as client:
        try:
            r = await client.post(
                f"{SIDECAR}/cookies",
                json={"cookies": f"{psid}:{psidts}"},
                headers={"X-Sidecar-Key": SIDECAR_KEY},
            )
        except Exception as e:  # noqa: BLE001
            log.error("could not reach the sidecar: %s", e)
            return False
    if r.status_code == 200:
        log.info("sidecar accepted the cookies: %s", r.text.strip())
        return True
    log.error("sidecar refused the cookies (%s): %s", r.status_code, r.text[:200])
    return False


async def run() -> None:
    had_profile = restore_profile()
    if not had_profile:
        log.error(
            "no logged-in profile available. Nothing here can log in -- produce a "
            "profile by signing in once (see keeper/README.md) and upload it to "
            "r2://%s. Sleeping rather than spinning.",
            PROFILE_KEY,
        )

    async with async_playwright() as pw:
        # A persistent context, not a fresh browser: the whole point is to keep
        # using one profile's session rather than starting a new anonymous one
        # that would need logging in.
        ctx = await pw.chromium.launch_persistent_context(
            PROFILE_DIR,
            headless=True,
            args=["--no-sandbox", "--disable-dev-shm-usage"],
        )
        page = ctx.pages[0] if ctx.pages else await ctx.new_page()
        last: tuple[str, str] | None = None

        while True:
            try:
                # A real navigation, which is what makes Google reissue the
                # rotating cookie -- reading the jar without loading anything
                # would never refresh it.
                await page.goto(GEMINI_URL, wait_until="domcontentloaded", timeout=90_000)
                await page.wait_for_timeout(4_000)

                jar = {c["name"]: c["value"] for c in await ctx.cookies()}
                missing = [n for n in WANTED if n not in jar]
                if missing:
                    # Signed out: the profile is no longer authenticated and only
                    # a human can fix that. Said plainly rather than retried
                    # silently forever.
                    log.error(
                        "profile is not signed in (missing %s). Re-do the one-time "
                        "login and re-upload the profile.",
                        ", ".join(missing),
                    )
                else:
                    pair = (jar["__Secure-1PSID"], jar["__Secure-1PSIDTS"])
                    if pair != last:
                        log.info("cookies changed; handing them to the sidecar")
                        if await push_cookies(*pair):
                            last = pair
                            save_profile()
                    else:
                        log.debug("cookies unchanged")
            except Exception as e:  # noqa: BLE001 - a bad cycle must not kill the loop
                log.error("refresh cycle failed: %s", e)

            await asyncio.sleep(REFRESH_SECONDS)


if __name__ == "__main__":
    if os.environ.get("LOGIN_CHECK_ONLY"):
        # Verify a profile before trusting it in production: restores, loads the
        # page once, and reports whether it is signed in.
        async def check() -> None:
            restore_profile()
            async with async_playwright() as pw:
                ctx = await pw.chromium.launch_persistent_context(
                    PROFILE_DIR, headless=True, args=["--no-sandbox"]
                )
                page = ctx.pages[0] if ctx.pages else await ctx.new_page()
                await page.goto(GEMINI_URL, wait_until="domcontentloaded", timeout=90_000)
                await page.wait_for_timeout(4_000)
                jar = {c["name"] for c in await ctx.cookies()}
                have = [n for n in WANTED if n in jar]
                print(f"  signed in: {len(have) == len(WANTED)}  (found {have})")
                await ctx.close()

        asyncio.run(check())
    else:
        try:
            asyncio.run(run())
        finally:
            shutil.rmtree(PROFILE_DIR, ignore_errors=True)
