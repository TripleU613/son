#!/usr/bin/env bash
# A phone-shaped virtual display running an ordinary Chromium, exported over VNC.
#
# Two things here are deliberate and both come from watching a real sign-in fail.
#
# 1. Chromium is launched *by this script*, not by Playwright. Playwright starts a
#    browser with --enable-automation and navigator.webdriver = true, and Google
#    refuses to sign in from one: "Couldn't sign you in / This browser or app may
#    not be secure." Nothing here spoofs those signals -- the browser simply is not
#    automated. The keeper attaches over CDP afterwards only to read the cookie jar
#    and navigate, which is not something Google's login check looks at.
#
# 2. The display is phone-shaped and the UA is a phone's, because this is looked at
#    from a phone over VNC. A 1280x800 desktop scaled onto a handset is unusable for
#    typing a password, and Google's mobile sign-in layout is built for exactly the
#    screen the person is holding.
# 3. It is restartable. This script is a supervised program in a single-container
#    deployment (see deploy/supervisord.conf), so when the keeper dies supervisor
#    runs it again -- and the first version of that leaked catastrophically: every
#    restart started a second Xvfb, x11vnc, websockify and Chromium while the
#    previous set kept running, orphaned to PID 1. Four restarts took the container
#    from 541MB to 1.13GB and 22 Chromium processes, on its way to an OOM kill that
#    would have taken the website down with it. Everything below is now torn down
#    before it is started, and torn down again on the way out.
set -euo pipefail

export DISPLAY=:99

log() { printf '%s keeper-entrypoint: %s\n' "$(date -u '+%Y-%m-%d %H:%M:%S')" "$*"; }

# Anything left from a previous run of this script, before starting our own. Matched
# on the exact command lines this script uses, not a bare "chrome", so a stray match
# cannot take out something unrelated in a container that now holds four processes.
cleanup() {
  pkill -f 'Xvfb :99' 2>/dev/null || true
  pkill -f 'x11vnc -display :99' 2>/dev/null || true
  pkill -f 'websockify --web /usr/share/novnc' 2>/dev/null || true
  pkill -f 'remote-debugging-port=9222' 2>/dev/null || true
  pkill -f 'fluxbox' 2>/dev/null || true
}
cleanup
# And on the way out, whether that is a clean stop or a crash. Without this, a
# crashing keeper leaves a browser behind that the next run cannot use and cannot
# see: the port is taken, so the new Chromium never opens its debugging socket, and
# the new keeper waits three minutes to be told nothing.
trap cleanup EXIT INT TERM

# The browser waits for the sidecar to finish initialising before it starts.
#
# Not politeness -- contention. The sidecar spends ~90s opening five Gemini sessions
# at start-up, and this container has 1.5 CPUs for everything in it. A cold Chromium
# loading the real Gemini SPA against that lost badly enough that its debugging port
# was not open before the keeper gave up waiting, which is what made the merged
# deployment crash-loop while the same code worked by hand on a quiet container.
#
# Bounded, and a timeout is not fatal: screening being slow to start must never mean
# no browser at all, because the browser is how somebody signs in to fix screening.
if [ -n "${SIDECAR_URL:-}" ]; then
  log "waiting for the sidecar to answer before starting the browser"
  for i in $(seq 1 60); do
    if python -c "import urllib.request;urllib.request.urlopen('${SIDECAR_URL}/health', timeout=2)" 2>/dev/null; then
      log "sidecar answered after ${i}0s; starting the display"
      break
    fi
    # A 503 means it is up and has no accounts yet -- which is exactly the state
    # this browser exists to fix, so stop waiting.
    if python -c "
import urllib.error, urllib.request, sys
try:
    urllib.request.urlopen('${SIDECAR_URL}/health', timeout=2)
except urllib.error.HTTPError:
    sys.exit(0)
except Exception:
    sys.exit(1)
sys.exit(0)" 2>/dev/null; then
      log "sidecar is up (no usable accounts yet); starting the display"
      break
    fi
    sleep 10
  done
fi

# Portrait, near enough to a phone's aspect that noVNC's scaling is close to 1:1.
Xvfb :99 -screen 0 "${KEEPER_SCREEN:-440x920x24}" -nolisten tcp &
for _ in $(seq 1 50); do
  [ -e /tmp/.X11-unix/X99 ] && break
  sleep 0.2
done

# A window manager, or Google's sign-in popups appear undecorated and unfocusable.
fluxbox >/dev/null 2>&1 &

# -localhost, so VNC is not on the network: websockify is the only reader, and it
# is reachable solely from the app container, which requires an admin session per
# request. -nopw for the same reason -- a password here would guard a port nothing
# can route to.
x11vnc -display :99 -forever -shared -localhost -nopw -quiet >/dev/null 2>&1 &

# Bound to loopback explicitly. It used to listen on 0.0.0.0:6080, which was
# unreachable anyway because the container published no port -- but the app now
# lives in this same container, so "reachable from the app" and "reachable from the
# host network" are no longer the same thing, and only the first is wanted. The
# route to this browser is /admin/browser, which checks for an admin session on
# every request including the WebSocket upgrade.
websockify --web /usr/share/novnc 127.0.0.1:6080 localhost:5900 >/dev/null 2>&1 &

# The browser Playwright shipped, started as a normal browser. Asking Playwright
# for the path rather than globbing /ms-playwright, so an image update that moves
# it does not silently break this.
# stderr silenced: this spins up a Playwright connection purely to read a path, and
# tearing it down logs "Task was destroyed" and a TargetClosedError that look
# exactly like a real failure in the container's logs. Asking Playwright rather than
# globbing /ms-playwright, so an image update that moves the browser fails loudly
# here instead of silently launching nothing.
CHROME="$(python -c 'from playwright.sync_api import sync_playwright
with sync_playwright() as p: print(p.chromium.executable_path)' 2>/dev/null)"
if [ ! -x "$CHROME" ]; then
  echo "keeper: could not locate the Chromium Playwright shipped" >&2
  exit 1
fi

# The profile must be in place BEFORE the browser opens it.
#
# This used to happen afterwards, inside session_keeper.py, and the consequence was
# subtle and total: Chromium started on an empty profile, the restore then wrote
# cookies into a directory the browser already had open, and the browser -- which
# keeps its own state in memory and rewrites those files on exit -- never saw them.
# The log said "profile restored from R2" and the very next line said "not signed
# in", which is exactly what happened after a deploy: a session that was safely
# stored was not actually used.
mkdir -p /tmp/profile
python -c "from session_keeper import restore_profile; restore_profile()" || true
# --remote-debugging-port on loopback only. Flags mirror the footprint work in
# session_keeper.py: no GPU exists in this container, and a renderer cap plus a
# small JS heap keep one page from growing without bound.
#
# It opens about:blank, not gemini.google.com, and that is not cosmetic. Measured on
# the deploy host: the debugging port answers ~2s after launch, but
# `connect_over_cdp` against a browser that is loading the real Gemini SPA took
# **131 seconds** to return -- Playwright waits for the browser's targets, and the
# page load is what it is waiting behind. Playwright's default timeout is 180s, so
# this sat ~50s from failing, and on a cold container with the sidecar initialising
# alongside it, it did fail: three minutes of silence, a traceback with no cause, and
# a restart that leaked another browser. From about:blank the same attach takes about
# a second. session_keeper.py navigates to Gemini once it is attached, so the window
# a human signs in through still shows the login page.
"$CHROME" \
  --remote-debugging-port=9222 \
  --remote-debugging-address=127.0.0.1 \
  --user-data-dir=/tmp/profile \
  --no-sandbox \
  --disable-dev-shm-usage \
  --disable-gpu \
  --disable-software-rasterizer \
  --renderer-process-limit=2 \
  --js-flags=--max-old-space-size=192 \
  --disable-extensions \
  --no-first-run \
  --no-default-browser-check \
  --disable-features=TranslateUI \
  --window-position=0,0 \
  --window-size="${KEEPER_WINDOW:-440,880}" \
  --user-agent="${KEEPER_UA:-Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Mobile Safari/537.36}" \
  "about:blank" >/dev/null 2>&1 &

# Wait for CDP before the keeper tries to attach, rather than making it retry.
#
# 120s, not the 30s this used to allow, and it says what happened either way. The
# old version was silent on both paths: it waited 30 seconds, gave up without a
# word, and exec'd a keeper that then spent three minutes timing out against a
# browser that was still starting. Every symptom of that was somewhere else -- a
# Playwright traceback with no cause, a container quietly growing a second browser
# every four minutes -- and the one line that would have explained it did not exist.
#
# Measured: CDP took 17.5s on an idle container and did not open within 30s on a
# cold start with the sidecar initialising alongside it.
cdp_up=0
for i in $(seq 1 240); do
  if python -c "import urllib.request,sys; urllib.request.urlopen('http://127.0.0.1:9222/json/version', timeout=1)" 2>/dev/null; then
    log "CDP answered after ~$((i / 2))s"
    cdp_up=1
    break
  fi
  sleep 0.5
done
if [ "$cdp_up" = 0 ]; then
  # Deliberately fatal. supervisor restarts this script, the cleanup above kills the
  # half-started browser, and the next attempt gets a clean display -- which is a
  # better outcome than handing the keeper a browser that will never answer, and it
  # names itself in the log.
  log "Chromium never opened its debugging port; giving up so supervisor restarts cleanly"
  exit 1
fi

exec python session_keeper.py
