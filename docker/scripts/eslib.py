"""Tiny stdlib-only Elasticsearch client shared by seed.py, load.py, scale.py
and apikey.py.

Why not urllib.request: it opens a new TCP (and TLS) connection per request,
which skews latency measurements and wastes time under sustained load. This
keeps one keep-alive http.client connection per thread instead, which is also
what esticli's reqwest connection pool does.

Connection settings come from the command line, falling back to environment
variables so `docker/esenv.sh` can point every script at a profile:

    --url        ES_URL          (default http://localhost:9200; a path prefix
                                  such as http://localhost:9280/es is kept)
    --user       ES_USER
    --password   ES_PASSWORD
    --api-key    ES_API_KEY      (the base64 "encoded" value, sent as
                                  "Authorization: ApiKey <value>")
    --ca-cert    ES_CA_CERT
    --insecure   ES_INSECURE=1
"""

from __future__ import annotations

import argparse
import base64
import gzip
import http.client
import json
import os
import ssl
import sys
import threading
import time
import urllib.parse
from typing import Any

DEFAULT_URL = "http://localhost:9200"


class ESError(Exception):
    """A non-2xx response (or a transport failure, with status 0)."""

    def __init__(self, status: int, body, method: str = "", path: str = ""):
        self.status = status
        self.body = body
        self.method = method
        self.path = path
        super().__init__(f"{method} {path} -> {status}: {short_error(body)}")

    @property
    def error_type(self) -> str:
        if isinstance(self.body, dict):
            err = self.body.get("error")
            if isinstance(err, dict):
                return err.get("type", "")
        return ""


def short_error(body) -> str:
    """One-line summary of an ES error body."""
    if isinstance(body, dict) and isinstance(body.get("error"), dict):
        err = body["error"]
        root = (err.get("root_cause") or [{}])[0]
        return f"{err.get('type')}: {err.get('reason') or root.get('reason')}"
    text = body if isinstance(body, str) else json.dumps(body)
    return text[:300]


def q(name: str) -> str:
    """Percent-encode one path segment (an index name) completely."""
    return urllib.parse.quote(name, safe="")


def add_connection_args(parser: argparse.ArgumentParser) -> None:
    env = os.environ.get
    g = parser.add_argument_group("connection (defaults from ES_* env vars)")
    g.add_argument("--url", default=env("ES_URL", DEFAULT_URL), help="base URL, may include a path prefix (default: %(default)s)")
    g.add_argument("--user", default=env("ES_USER"), help="basic auth user")
    g.add_argument("--password", default=env("ES_PASSWORD"), help="basic auth password")
    g.add_argument("--api-key", default=env("ES_API_KEY"), help='encoded API key, sent as "Authorization: ApiKey <key>"')
    g.add_argument("--ca-cert", default=env("ES_CA_CERT"), help="PEM CA certificate to trust")
    g.add_argument(
        "-k", "--insecure", action="store_true",
        default=env("ES_INSECURE", "") not in ("", "0", "false"),
        help="skip TLS certificate verification",
    )
    g.add_argument("--timeout", type=float, default=float(env("ES_TIMEOUT", "120")), help="per-request timeout in seconds")


def client_from_args(args: argparse.Namespace) -> "ES":
    return ES(
        args.url,
        user=args.user,
        password=args.password,
        api_key=args.api_key,
        ca_cert=args.ca_cert,
        insecure=args.insecure,
        timeout=args.timeout,
    )


class ES:
    def __init__(self, url, user=None, password=None, api_key=None, ca_cert=None, insecure=False, timeout=120.0):
        parts = urllib.parse.urlsplit(url)
        if parts.scheme not in ("http", "https"):
            raise SystemExit(f"unsupported URL scheme in {url!r}")
        self.url = url
        self.scheme = parts.scheme
        self.host = parts.hostname or "localhost"
        self.port = parts.port or (443 if self.scheme == "https" else 80)
        # A path prefix (gateway / reverse proxy) is kept and every request
        # path is appended to it.
        self.prefix = parts.path.rstrip("/")
        self.timeout = timeout
        self.headers = {"Content-Type": "application/json", "User-Agent": "esticli-docker-scripts"}
        if api_key:
            self.headers["Authorization"] = f"ApiKey {api_key}"
        elif user:
            token = base64.b64encode(f"{user}:{password or ''}".encode()).decode()
            self.headers["Authorization"] = f"Basic {token}"
        self.ssl_context = None
        if self.scheme == "https":
            if insecure:
                ctx = ssl.create_default_context()
                ctx.check_hostname = False
                ctx.verify_mode = ssl.CERT_NONE
            else:
                ctx = ssl.create_default_context(cafile=ca_cert) if ca_cert else ssl.create_default_context()
            self.ssl_context = ctx
        self._local = threading.local()
        self._version: tuple[int, int, int] | None = None

    # ------------------------------------------------------------ transport
    def _connection(self) -> http.client.HTTPConnection:
        conn = getattr(self._local, "conn", None)
        if conn is None:
            if self.scheme == "https":
                conn = http.client.HTTPSConnection(self.host, self.port, timeout=self.timeout, context=self.ssl_context)
            else:
                conn = http.client.HTTPConnection(self.host, self.port, timeout=self.timeout)
            self._local.conn = conn
        return conn

    def _drop_connection(self) -> None:
        conn = getattr(self._local, "conn", None)
        if conn is not None:
            try:
                conn.close()
            except Exception:
                pass
        self._local.conn = None

    def raw(self, method: str, path: str, body=None, params=None, headers=None):
        """Sends one request; returns (status, response_headers, raw_bytes).

        Never raises for HTTP status codes. Retries once on a dropped
        keep-alive connection. `body` may be bytes, str, or a JSON-able object.
        """
        full = self.prefix + "/" + path.lstrip("/")
        if params:
            full += ("&" if "?" in full else "?") + urllib.parse.urlencode(params)
        if body is not None and not isinstance(body, (bytes, str)):
            body = json.dumps(body)
        if isinstance(body, str):
            body = body.encode()
        hdrs = dict(self.headers)
        if headers:
            hdrs.update(headers)
        last_exc = None
        for attempt in range(2):
            conn = self._connection()
            try:
                conn.request(method, full, body=body, headers=hdrs)
                resp = conn.getresponse()
                data = resp.read()
                if resp.getheader("Connection", "").lower() == "close":
                    self._drop_connection()
                return resp.status, resp.headers, data
            except (http.client.RemoteDisconnected, http.client.CannotSendRequest,
                    http.client.BadStatusLine, BrokenPipeError, ConnectionResetError) as exc:
                # Typically a keep-alive connection the server already closed.
                self._drop_connection()
                last_exc = exc
                continue
            except Exception as exc:
                self._drop_connection()
                raise ESError(0, f"{type(exc).__name__}: {exc}", method, full) from exc
        raise ESError(0, f"{type(last_exc).__name__}: {last_exc}", method, full)

    def request(self, method: str, path: str, body=None, params=None, ok=(), headers=None) -> Any:
        """Sends a request and returns the decoded JSON (or text) body.

        Typed Any on purpose: the result is whatever JSON the endpoint returns
        (usually a dict, a list for _cat APIs, None for HEAD-like empty bodies),
        and callers that depend on a shape check it with isinstance.

        Raises ESError for non-2xx statuses that are not listed in `ok`
        (e.g. ok=(404,) to treat "missing" as a normal answer).
        """
        status, resp_headers, data = self.raw(method, path, body=body, params=params, headers=headers)
        if resp_headers.get("Content-Encoding", "") == "gzip":
            data = gzip.decompress(data)
        text = data.decode("utf-8", "replace")
        try:
            parsed = json.loads(text) if text else None
        except ValueError:
            parsed = text
        if 200 <= status < 300 or status in ok:
            return parsed
        raise ESError(status, parsed, method, self.prefix + "/" + path.lstrip("/"))

    def get(self, path, **kw) -> Any:
        return self.request("GET", path, **kw)

    def put(self, path, body=None, **kw) -> Any:
        return self.request("PUT", path, body=body, **kw)

    def post(self, path, body=None, **kw) -> Any:
        return self.request("POST", path, body=body, **kw)

    def delete(self, path, **kw) -> Any:
        return self.request("DELETE", path, **kw)

    def head_exists(self, path) -> bool:
        status, _, _ = self.raw("HEAD", path)
        return status == 200

    # -------------------------------------------------------------- helpers
    def bulk(self, lines, refresh=None):
        """Sends NDJSON action/source lines (already JSON strings)."""
        body = "\n".join(lines) + "\n"
        params = {"refresh": refresh} if refresh else None
        return self.request("POST", "_bulk", body=body, params=params,
                            headers={"Content-Type": "application/x-ndjson"})

    def version(self) -> tuple[int, int, int]:
        """The cluster's (major, minor, patch), fetched once from GET /."""
        if self._version is None:
            self._version = parse_version(self.get("/"))
        return self._version

    def wait_for_status(self, status="yellow", timeout="60s", index=None):
        path = f"_cluster/health/{q(index)}" if index else "_cluster/health"
        return self.get(path, params={"wait_for_status": status, "timeout": timeout}, ok=(408,))


def connect_or_die(args) -> ES:
    """Builds a client and checks it can reach the cluster, with a clear message."""
    es = client_from_args(args)
    try:
        info = es.get("/")
    except ESError as exc:
        print(f"error: cannot reach Elasticsearch at {args.url}: {exc}", file=sys.stderr)
        if exc.status == 401:
            print("hint: pass --user/--password or --api-key (or use docker/esenv.sh <cmd> <profile>)", file=sys.stderr)
        raise SystemExit(2)
    try:
        es._version = parse_version(info)
    except ValueError as exc:
        print(f"error: {args.url} does not look like Elasticsearch: {exc}", file=sys.stderr)
        raise SystemExit(2)
    return es


def parse_version(info) -> tuple[int, int, int]:
    """Extracts (major, minor, patch) from a GET / response body.

    Raises ValueError when the body isn't an Elasticsearch root response (a
    proxy's HTML error page, say), instead of a confusing TypeError/KeyError.
    """
    version = info.get("version") if isinstance(info, dict) else None
    number = version.get("number") if isinstance(version, dict) else None
    if not isinstance(number, str):
        raise ValueError(f"no version.number in response: {str(info)[:200]!r}")
    # "9.5.5", "8.19.0-SNAPSHOT", "9.0.0-beta1": keep the numeric part.
    parts = [int(p) for p in number.split("-", 1)[0].split(".")[:3] if p.isdigit()]
    if not parts:
        raise ValueError(f"unparseable version {number!r}")
    parts += [0] * (3 - len(parts))
    return parts[0], parts[1], parts[2]


def human_bytes(n: float) -> str:
    for unit in ("B", "KB", "MB", "GB", "TB"):
        if abs(n) < 1024 or unit == "TB":
            return f"{n:.1f}{unit}" if unit != "B" else f"{int(n)}B"
        n /= 1024
    return f"{n:.1f}TB"


def now_iso() -> str:
    t = time.time()
    return time.strftime("%Y-%m-%dT%H:%M:%S", time.gmtime(t)) + f".{int((t % 1) * 1000):03d}Z"
