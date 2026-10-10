#!/usr/bin/env python3
"""Creates an Elasticsearch API key for esticli and prints it. Stdlib only.

esticli sends `--api-key <value>` verbatim as `Authorization: ApiKey <value>`
(src/elasticsearch/client.rs), so <value> must be the base64 of "id:api_key",
which is what ES returns as "encoded". That is the only thing printed on
stdout, so it can be captured:

  KEY=$(apikey.py --url https://localhost:9202 --user elastic --password esticli-dev \\
                  --ca-cert docker/certs/ca.crt)
  esticli -u https://localhost:9202 --ca-cert docker/certs/ca.crt --api-key "$KEY"

By default the key inherits the creating user's privileges. --read-only
limits it to what a monitoring user would get (cluster: monitor; indices:
monitor + view_index_metadata), which is useful for seeing how the details
popup degrades when some of its requests are refused.
"""

from __future__ import annotations

import argparse
import base64
import os
import sys

sys.dont_write_bytecode = True  # keep docker/scripts free of __pycache__
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from eslib import ESError, add_connection_args, connect_or_die  # noqa: E402

READ_ONLY_ROLE = {
    "esticli-read-only": {
        "cluster": ["monitor"],
        "indices": [{"names": ["*"], "privileges": ["monitor", "view_index_metadata"]}],
    }
}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    add_connection_args(parser)
    parser.add_argument("--name", default="esticli-dev", help="API key name (default %(default)s)")
    parser.add_argument("--expiration", default="30d", help="expiry such as 1h or 30d; 'none' for never (default %(default)s)")
    parser.add_argument("--read-only", action="store_true", help="limit the key to monitoring privileges")
    parser.add_argument("--invalidate", action="store_true", help="invalidate all keys named --name and exit")
    args = parser.parse_args()

    es = connect_or_die(args)
    if args.invalidate:
        resp = es.request("DELETE", "_security/api_key", body={"name": args.name})
        invalidated = resp.get("invalidated_api_keys", []) if isinstance(resp, dict) else []
        print(f"invalidated {len(invalidated)} key(s) named {args.name}", file=sys.stderr)
        return

    body: dict = {"name": args.name, "metadata": {"created_by": "esticli docker/scripts/apikey.py"}}
    if args.expiration != "none":
        body["expiration"] = args.expiration
    if args.read_only:
        body["role_descriptors"] = READ_ONLY_ROLE
    try:
        resp = es.post("_security/api_key", body)
    except ESError as exc:
        if exc.status in (400, 500) and "security" in str(exc.body).lower():
            raise SystemExit(f"error: this cluster has security disabled, API keys need the secure profile ({exc})")
        raise
    if not isinstance(resp, dict) or "id" not in resp or "api_key" not in resp:
        raise SystemExit(f"error: unexpected _security/api_key response: {str(resp)[:300]}")
    # "encoded" exists on ES 7.16+; build it for anything older.
    encoded = resp.get("encoded") or base64.b64encode(f"{resp['id']}:{resp['api_key']}".encode()).decode()
    scope = "read-only" if args.read_only else "same privileges as the creating user"
    print(f"created API key {args.name!r} (id {resp['id']}, {scope}, expires: {args.expiration})", file=sys.stderr)
    print(encoded)


if __name__ == "__main__":
    try:
        main()
    except ESError as exc:
        print(f"error: {exc}", file=sys.stderr)
        sys.exit(1)
