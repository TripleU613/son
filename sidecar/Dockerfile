# syntax=docker/dockerfile:1
#
# The Gemini sidecar. Pinned by digest for the same reason as the app's images:
# a public repo's build should not depend on trusting that a moving tag still
# points where it did yesterday. Re-pin deliberately:
#   docker pull python:3.13-slim-bookworm && docker inspect --format \
#     '{{index .RepoDigests 0}}' python:3.13-slim-bookworm
FROM python:3.13-slim-bookworm@sha256:00faa2debb87529f9f0764e9491d8ba400a3678976616c3bd7cb193745ac20d1

# curl-cffi (a gemini_webapi dependency) ships prebuilt wheels for this platform,
# so no compiler is needed here. ca-certificates is, because everything this does
# is an outbound TLS call to Google.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --uid 10002 gem

WORKDIR /app

# Requirements before the source, so a code edit does not reinstall the world.
COPY requirements.txt ./
RUN pip install --no-cache-dir -r requirements.txt

COPY gemini_service.py ./

# Non-root, like the app container. The only path this writes to is /tmp: the
# uploaded image on its way to Gemini, the generated image on its way back, and
# gemini_webapi's cache of the rotated __Secure-1PSIDTS cookie. All three are
# transient, which is why /tmp being a tmpfs in compose is correct rather than a
# limitation.
USER gem

ENV PYTHONUNBUFFERED=1 \
    HOME=/tmp

EXPOSE 8099

# One worker. Each GeminiClient holds a conversation-bearing session and the
# service serialises per client with a lock; multiple workers would each build
# their own client pool from the same cookies, multiplying the account's
# concurrent sessions for no throughput gain, since the bottleneck is Gemini.
CMD ["python", "-m", "uvicorn", "gemini_service:app", \
     "--host", "0.0.0.0", "--port", "8099", "--workers", "1"]

# /health reports how many accounts initialised. Unhealthy at zero is the point:
# with no usable account every upload would silently publish unscreened, and that
# should be visible in `docker ps` rather than only in the logs.
HEALTHCHECK --interval=60s --timeout=10s --start-period=90s --retries=3 \
    CMD python -c "import urllib.request,json,sys; \
d=json.load(urllib.request.urlopen('http://127.0.0.1:8099/health', timeout=8)); \
sys.exit(0 if d.get('accounts', 0) > 0 else 1)"
