#!/usr/bin/env python3
"""Fault injection for the "proxy" profile, via Toxiproxy's HTTP API. Stdlib only.

Toxiproxy sits between localhost:9290 and the "single" node. Point esticli at
http://localhost:9290 and add toxics while it runs:

  toxics.py status                      # proxy state and active toxics
  toxics.py latency 1500                # +1500ms on every response
  toxics.py latency 300 --jitter 200    # 100..500ms
  toxics.py timeout 0                   # stall all data until removed (0 = forever)
  toxics.py timeout 5000                # close connections after 5s of silence
  toxics.py reset-peer                  # TCP RST on every connection
  toxics.py reset-peer --after 2000     # RST 2s into each connection
  toxics.py bandwidth 64                # cap responses at 64 KB/s
  toxics.py down / up                   # refuse connections entirely / restore
  toxics.py remove latency_downstream   # remove one toxic by name
  toxics.py clear                       # remove every toxic and re-enable the proxy

Adding a toxic of a type that's already active updates it in place. Toxics
apply to the downstream (ES -> client) direction unless --upstream is given.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import urllib.error
import urllib.request


class Toxiproxy:
    def __init__(self, api: str, proxy: str):
        self.api = api.rstrip("/")
        self.proxy = proxy

    def call(self, method: str, path: str, body=None, ok_missing: bool = False):
        data = json.dumps(body).encode() if body is not None else None
        req = urllib.request.Request(self.api + path, data=data, method=method,
                                     headers={"Content-Type": "application/json"})
        try:
            with urllib.request.urlopen(req, timeout=10) as resp:
                raw = resp.read()
                return json.loads(raw) if raw else None
        except urllib.error.HTTPError as exc:
            if ok_missing and exc.code == 404:
                return None
            raise SystemExit(f"error: {method} {path} -> {exc.code}: {exc.read().decode(errors='replace').strip()}")
        except urllib.error.URLError as exc:
            raise SystemExit(f"error: cannot reach Toxiproxy at {self.api} ({exc.reason}); "
                             "is the proxy profile up? (docker/esenv.sh up proxy)")

    def upsert(self, toxic_type: str, attributes: dict, stream: str, toxicity: float) -> None:
        name = f"{toxic_type}_{stream}"
        base = f"/proxies/{self.proxy}/toxics"
        existing = {t["name"] for t in self.call("GET", base) or []}
        body = {"name": name, "type": toxic_type, "stream": stream, "toxicity": toxicity, "attributes": attributes}
        if name in existing:
            self.call("POST", f"{base}/{name}", body)
            verb = "updated"
        else:
            self.call("POST", base, body)
            verb = "added"
        print(f"{verb} {name}: {json.dumps(attributes)} (toxicity {toxicity})")

    def status(self) -> None:
        proxy = self.call("GET", f"/proxies/{self.proxy}", ok_missing=True)
        if proxy is None:
            raise SystemExit(f"error: no proxy named {self.proxy!r}")
        state = "enabled" if proxy.get("enabled") else "DISABLED (refusing connections)"
        print(f"proxy {proxy['name']}: {proxy['listen']} -> {proxy['upstream']}, {state}")
        toxics = proxy.get("toxics") or []
        if not toxics:
            print("  no toxics")
        for t in toxics:
            print(f"  {t['name']:24} {t['type']:10} {t['stream']:10} toxicity {t['toxicity']}  {json.dumps(t['attributes'])}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--api", default=os.environ.get("TOXIPROXY_URL", "http://localhost:8474"),
                        help="Toxiproxy API (default %(default)s)")
    parser.add_argument("--proxy", default="es", help="proxy name (default %(default)s)")
    parser.add_argument("--upstream", action="store_true", help="apply to client -> ES instead of ES -> client")
    parser.add_argument("--toxicity", type=float, default=1.0, help="fraction of connections affected (default 1.0)")
    sub = parser.add_subparsers(dest="cmd", required=True)
    sub.add_parser("status", help="show the proxy and its toxics")
    p = sub.add_parser("latency", help="delay data by MS milliseconds")
    p.add_argument("ms", type=int)
    p.add_argument("--jitter", type=int, default=0, help="+/- random milliseconds")
    p = sub.add_parser("timeout", help="stop data; close the connection after MS (0 = never)")
    p.add_argument("ms", type=int)
    p = sub.add_parser("reset-peer", help="reset connections with a TCP RST")
    p.add_argument("--after", type=int, default=0, help="milliseconds before the reset (default 0)")
    p = sub.add_parser("bandwidth", help="limit throughput to KB per second")
    p.add_argument("kb_per_s", type=int)
    p = sub.add_parser("slow-close", help="delay the TCP close by MS")
    p.add_argument("ms", type=int)
    p = sub.add_parser("remove", help="remove one toxic by name (see status)")
    p.add_argument("name")
    sub.add_parser("clear", help="remove all toxics and re-enable the proxy")
    sub.add_parser("down", help="disable the proxy: new connections are refused")
    sub.add_parser("up", help="re-enable the proxy")
    args = parser.parse_args()

    tp = Toxiproxy(args.api, args.proxy)
    stream = "upstream" if args.upstream else "downstream"
    if args.cmd == "status":
        tp.status()
    elif args.cmd == "latency":
        tp.upsert("latency", {"latency": args.ms, "jitter": args.jitter}, stream, args.toxicity)
    elif args.cmd == "timeout":
        tp.upsert("timeout", {"timeout": args.ms}, stream, args.toxicity)
    elif args.cmd == "reset-peer":
        tp.upsert("reset_peer", {"timeout": args.after}, stream, args.toxicity)
    elif args.cmd == "bandwidth":
        tp.upsert("bandwidth", {"rate": args.kb_per_s}, stream, args.toxicity)
    elif args.cmd == "slow-close":
        tp.upsert("slow_close", {"delay": args.ms}, stream, args.toxicity)
    elif args.cmd == "remove":
        tp.call("DELETE", f"/proxies/{args.proxy}/toxics/{args.name}")
        print(f"removed {args.name}")
    elif args.cmd == "clear":
        for t in tp.call("GET", f"/proxies/{args.proxy}/toxics") or []:
            tp.call("DELETE", f"/proxies/{args.proxy}/toxics/{t['name']}")
            print(f"removed {t['name']}")
        tp.call("POST", f"/proxies/{args.proxy}", {"enabled": True})
        print("proxy enabled, no toxics")
    elif args.cmd in ("down", "up"):
        tp.call("POST", f"/proxies/{args.proxy}", {"enabled": args.cmd == "up"})
        print(f"proxy {args.proxy} {'enabled' if args.cmd == 'up' else 'disabled: connections refused'}")


if __name__ == "__main__":
    main()
