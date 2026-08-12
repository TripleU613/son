#!/usr/bin/env bash
# Normal operation is headless: exec the keeper and nothing else.
#
# In login mode a human needs to see and click a real browser, so this brings up
# the smallest stack that puts one in their tab: Xvfb for Chromium to draw into,
# x11vnc to export that display, websockify+noVNC to make it an HTTP page. None of
# it runs otherwise.
set -euo pipefail

if [ "${KEEPER_LOGIN_MODE:-}" != "1" ]; then
  exec python session_keeper.py
fi

export DISPLAY=:99

# -nolisten tcp: the X server is for this container only, never the network.
Xvfb :99 -screen 0 1440x900x24 -nolisten tcp &
# Wait for the socket rather than sleeping a guessed number of seconds.
for _ in $(seq 1 50); do
  [ -e /tmp/.X11-unix/X99 ] && break
  sleep 0.2
done

# A window manager, or Google's sign-in popups appear undecorated and unfocusable.
fluxbox >/dev/null 2>&1 &

# -localhost so VNC itself is not exposed; websockify below is the only way in,
# and that is reachable solely over the tailnet (see docker-compose.yml).
# -nopw is deliberate: the port is not published publicly, and a VNC password
# would be one more secret to pass around for a window that lives for minutes.
x11vnc -display :99 -forever -shared -localhost -nopw -quiet >/dev/null 2>&1 &

websockify --web /usr/share/novnc 6080 localhost:5900 >/dev/null 2>&1 &

echo "keeper: login mode up. Open http://<host>:6080/vnc.html and sign in."
exec python session_keeper.py
