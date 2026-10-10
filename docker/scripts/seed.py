#!/usr/bin/env python3
"""Idempotent functional fixtures for esticli. Python 3 stdlib only.

Creates (everything named esticli-*, or .esticli-* for the dot index):

  regular indices   esticli-steady/-bursty/-sine/-ramp/-churn  (load.py targets)
                    esticli-idle, -empty, -small, -medium, -large, -multishard
  data streams      esticli-logs     ILM policy esticli-rollover (rollover on
                                     --rollover-docs, warm after 2m, delete 30m)
                    esticli-metrics  data stream lifecycle, 7d retention (8.11+)
                    -> hidden .ds-esticli-* backing indices
  health states     esticli-yellow   one replica more than there are data nodes
                    esticli-red      only with --with-red: primary pinned to a
                                     node that doesn't exist, so health is red
  other states      esticli-closed   closed index
                    .esticli-system  dot-prefixed, index.hidden=true
                    esticli-alias    alias over esticli-small + esticli-medium
  awkward names     every legal-but-URL-hostile character class ES accepts:
                    + & % = ; @ ! $ ( ) [ ] { } ~ ^ ' ` .. unicode emoji and
                    a 255-byte name (esticli puts names into URL paths)

Running it again converges to the same state: existing indices are kept and
only topped up to their target doc count. --reset deletes the fixtures first
(but not scale.py's esticli-scale-* indices).

Examples:
  seed.py                                  # against http://localhost:9200
  seed.py --reset --with-red
  seed.py --url https://localhost:9202 --user elastic --password esticli-dev \\
          --ca-cert docker/certs/ca.crt
  seed.py --probe-names                    # also report which names ES rejects
"""

from __future__ import annotations

import argparse
import concurrent.futures as cf
import os
import random
import sys
import json

sys.dont_write_bytecode = True  # keep docker/scripts free of __pycache__
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from eslib import ESError, add_connection_args, connect_or_die, human_bytes, now_iso, q, short_error  # noqa: E402

ILM_POLICY = "esticli-rollover"
SCALE_PREFIX = "esticli-scale-"

# name -> (target docs, payload bytes per doc, primary shards)
REGULAR = {
    "esticli-steady": (2000, 200, 1),
    "esticli-bursty": (1000, 200, 1),
    "esticli-sine": (1000, 200, 1),
    "esticli-ramp": (1000, 200, 1),
    "esticli-churn": (500, 200, 1),
    "esticli-idle": (5000, 200, 1),
    "esticli-empty": (0, 0, 1),
    "esticli-small": (100, 50, 1),
    "esticli-medium": (20000, 300, 1),
    "esticli-large": (100000, 1024, 1),
    "esticli-multishard": (10000, 200, 3),
}

# Legal in ES 7.17 - 9.x, but hostile in a URL path: reserved/sub-delim
# characters, a literal '%', non-ASCII, and the 255-byte maximum length.
AWKWARD = [
    "esticli-plus+sign",
    "esticli-amp&ersand",
    "esticli-100%-percent",
    "esticli-eq=semi;at@",
    "esticli-bang!dollar$",
    "esticli-paren(s)",
    "esticli-brackets[x]{y}",
    "esticli-tilde~caret^",
    "esticli-quote'apos",
    "esticli-backtick`tick",
    "esticli-dots..and.dots",
    "esticli-ünïcödé",
    "esticli-日本語",
    "esticli-emoji-🦀",
]
MAX_NAME = "esticli-max-" + "x" * (255 - len("esticli-max-"))  # exactly 255 bytes

# Only tried with --probe-names: names ES must reject, to document the rules
# of the version under test.
ILLEGAL_PROBES = [
    "esticli-colon:x", "esticli-hash#x", "esticli-space x", "esticli-UPPER", "esticli-comma,x",
    "esticli-pipe|x", "esticli-star*", "esticli-question?", "esticli-lt<", 'esticli-dquote"',
    "esticli-back\\slash", "esticli-slash/x", "+esticli-plus-start", "_esticli-underscore-start",
    "-esticli-dash-start", "esticli-max-" + "x" * (256 - len("esticli-max-")),
]

LEVELS = ["debug", "info", "info", "info", "warn", "error"]
SERVICES = [f"svc-{i}" for i in range(12)]


def make_doc(i: int, payload: int, rng: random.Random) -> dict:
    doc = {
        "@timestamp": now_iso(),
        "n": i,
        "level": rng.choice(LEVELS),
        "service": rng.choice(SERVICES),
        "latency_ms": int(rng.expovariate(1 / 40)),
        "message": f"seeded document {i}",
    }
    if payload:
        # Random hex compresses poorly, so the payload shows up in store size.
        doc["pad"] = rng.randbytes(payload // 2).hex()
    return doc


def bulk_load(es, index: str, count: int, payload: int, op: str = "index", workers: int = 4, batch: int = 2000) -> int:
    """Indexes `count` generated docs into `index`; returns the number of item errors."""
    if count <= 0:
        return 0
    action = json.dumps({op: {"_index": index}})

    def one_batch(start: int, n: int) -> int:
        rng = random.Random(f"{index}-{start}")
        lines = []
        for i in range(start, start + n):
            lines.append(action)
            lines.append(json.dumps(make_doc(i, payload, rng)))
        resp = es.bulk(lines)
        if not resp.get("errors"):
            return 0
        failed = [it for it in resp["items"] if list(it.values())[0].get("error")]
        first = list(failed[0].values())[0]["error"]
        print(f"    ! {len(failed)} bulk errors in {index}: {first.get('type')}: {first.get('reason')}")
        return len(failed)

    errors = 0
    with cf.ThreadPoolExecutor(workers) as pool:
        futures = [pool.submit(one_batch, s, min(batch, count - s)) for s in range(0, count, batch)]
        for f in futures:
            errors += f.result()
    return errors


class Seeder:
    def __init__(self, es, args):
        self.es = es
        self.args = args
        self.version = es.version()
        health = es.get("_cluster/health")
        self.data_nodes = health["number_of_data_nodes"]
        self.replicas = (0 if self.data_nodes == 1 else 1) if args.replicas == "auto" else int(args.replicas)
        self.notes: list[str] = []

    # ------------------------------------------------------------ utilities
    def log(self, msg: str) -> None:
        print(msg, flush=True)

    def index_states(self, pattern: str = "esticli-*,.esticli-*,.ds-esticli-*") -> dict:
        """name -> {status, health, docs} for every matching index, hidden and closed included."""
        rows = self.es.get(
            f"_cat/indices/{pattern}",
            params={"format": "json", "h": "index,status,health,docs.count,store.size", "expand_wildcards": "all", "bytes": "b"},
            ok=(404,),
        )
        if not isinstance(rows, list):
            return {}
        return {r["index"]: r for r in rows}

    def doc_count(self, index: str) -> int:
        resp = self.es.get(f"{q(index)}/_count", ok=(404,))
        return resp.get("count", 0) if isinstance(resp, dict) else 0

    def ensure_index(self, name: str, settings: dict, mappings: dict | None = None, wait_active: bool = True) -> bool:
        """Creates the index unless it exists. Returns True when it was created."""
        if self.es.head_exists(q(name)):
            return False
        body = {"settings": settings}
        if mappings:
            body["mappings"] = mappings
        params = None if wait_active else {"wait_for_active_shards": "0"}
        try:
            self.es.put(q(name), body, params=params)
        except ESError as exc:
            if exc.error_type == "resource_already_exists_exception":
                return False
            raise
        return True

    def top_up(self, name: str, target: int, payload: int, op: str = "index") -> None:
        self.es.post(f"{q(name)}/_refresh", ok=(404,))
        have = self.doc_count(name)
        missing = target - have
        if missing > 0:
            self.log(f"  {name}: indexing {missing} docs ({have} present)")
            bulk_load(self.es, name, missing, payload, op=op)
        else:
            self.log(f"  {name}: {have} docs present, nothing to add")

    # ---------------------------------------------------------------- reset
    def reset(self) -> None:
        self.log("reset: deleting existing fixtures")
        streams = self.es.get("_data_stream/esticli-*", params={"expand_wildcards": "all"}, ok=(404,))
        names = [d["name"] for d in (streams or {}).get("data_streams", [])]
        if names:
            self.es.delete("_data_stream/" + ",".join(q(n) for n in names))
            self.log(f"  deleted data streams: {', '.join(names)}")
        keep_scale = not self.args.reset_scale
        doomed = [n for n in self.index_states("esticli-*,.esticli-*") if not (keep_scale and n.startswith(SCALE_PREFIX))]
        # Delete by explicit name (works without action.destructive_requires_name=false),
        # chunked to stay well under ES's 4KB request-line limit.
        chunk: list[str] = []
        for name in doomed + [None]:
            if name is not None:
                chunk.append(q(name))
            if chunk and (name is None or sum(len(c) + 1 for c in chunk) > 2500):
                self.es.delete(",".join(chunk), params={"expand_wildcards": "all"}, ok=(404,))
                chunk = []
        self.log(f"  deleted {len(doomed)} indices")
        for tpl in ("esticli-logs", "esticli-metrics", "esticli-defaults"):
            self.es.delete(f"_index_template/{tpl}", ok=(404,))
        self.es.delete(f"_ilm/policy/{ILM_POLICY}", ok=(404,))
        self.log("  deleted templates and ILM policy")

    # ------------------------------------------------------------- fixtures
    def templates(self) -> None:
        self.log("templates and policies")
        self.es.put(f"_ilm/policy/{ILM_POLICY}", {
            "policy": {
                "phases": {
                    "hot": {"actions": {"rollover": {"max_docs": self.args.rollover_docs, "max_age": "1h"}}},
                    "warm": {"min_age": "2m", "actions": {"set_priority": {"priority": 50}}},
                    "delete": {"min_age": "30m", "actions": {"delete": {}}},
                }
            }
        })
        self.log(f"  ILM policy {ILM_POLICY}: rollover at {self.args.rollover_docs} docs, warm after 2m, delete after 30m")

        # Catch-all so indices auto-created or re-created by load.py (--churn)
        # get the same shard/replica layout instead of ES's 1 replica default.
        self.es.put("_index_template/esticli-defaults", {
            "index_patterns": ["esticli-*"],
            "priority": 10,
            "template": {"settings": {"number_of_shards": 1, "number_of_replicas": self.replicas}},
            "_meta": {"managed_by": "esticli docker/scripts/seed.py"},
        })
        mappings = {
            "properties": {
                "@timestamp": {"type": "date"},
                "level": {"type": "keyword"},
                "service": {"type": "keyword"},
                "latency_ms": {"type": "long"},
                "n": {"type": "long"},
                "message": {"type": "text"},
                "pad": {"type": "keyword", "index": False, "doc_values": False},
            }
        }
        self.es.put("_index_template/esticli-logs", {
            "index_patterns": ["esticli-logs*"],
            "data_stream": {},
            # Above the built-in logs/metrics templates (priority 100) as a
            # matter of habit, even though the patterns don't overlap.
            "priority": 500,
            "template": {
                "settings": {
                    "number_of_shards": 1,
                    "number_of_replicas": self.replicas,
                    "index.lifecycle.name": ILM_POLICY,
                },
                "mappings": mappings,
            },
        })
        metrics_template = {"settings": {"number_of_shards": 1, "number_of_replicas": self.replicas}, "mappings": mappings}
        if self.version >= (8, 11, 0):
            metrics_template["lifecycle"] = {"data_retention": "7d"}
        else:
            self.notes.append("esticli-metrics has no data stream lifecycle (needs ES 8.11+)")
        self.es.put("_index_template/esticli-metrics", {
            "index_patterns": ["esticli-metrics*"],
            "data_stream": {},
            "priority": 500,
            "template": metrics_template,
        })
        self.log("  index templates esticli-defaults, esticli-logs (ILM), esticli-metrics (lifecycle)")

    def regular_indices(self) -> None:
        self.log("regular indices")
        scale = self.args.scale
        for name, (docs, payload, shards) in REGULAR.items():
            created = self.ensure_index(name, {"number_of_shards": shards, "number_of_replicas": self.replicas})
            if created:
                self.log(f"  created {name}")
            self.top_up(name, int(docs * scale), payload)

    def data_streams(self) -> None:
        self.log("data streams")
        existing = self.es.get("_data_stream/esticli-*", ok=(404,))
        have = {d["name"] for d in (existing or {}).get("data_streams", [])}
        for name, docs in (("esticli-logs", 2000), ("esticli-metrics", 500)):
            if name not in have:
                if self.es.head_exists(q(name)):
                    raise SystemExit(
                        f"error: {name} exists as a regular index (written before its template existed?); rerun with --reset")
                self.es.put(f"_data_stream/{q(name)}")
                self.log(f"  created data stream {name}")
            self.top_up(name, int(docs * self.args.scale), 200, op="create")

    def state_fixtures(self) -> None:
        self.log("health and state fixtures")
        # Yellow: one replica more than the cluster can place.
        if self.ensure_index("esticli-yellow", {"number_of_shards": 1, "number_of_replicas": self.data_nodes},
                             wait_active=True):
            self.log(f"  created esticli-yellow with {self.data_nodes} replica(s) on {self.data_nodes} data node(s)")
        self.top_up("esticli-yellow", 100, 100)

        # Hidden, dot-prefixed "system-like" index. Some versions warn about
        # dot-prefixed names; fall back to a non-dot hidden index if refused.
        try:
            if self.ensure_index(".esticli-system", {"number_of_shards": 1, "number_of_replicas": self.replicas, "index.hidden": True}):
                self.log("  created .esticli-system (hidden)")
            self.top_up(".esticli-system", 100, 100)
        except ESError as exc:
            self.notes.append(f".esticli-system refused ({short_error(exc.body)}); created esticli-hidden instead")
            self.ensure_index("esticli-hidden", {"number_of_shards": 1, "number_of_replicas": self.replicas, "index.hidden": True})
            self.top_up("esticli-hidden", 100, 100)

        # Closed index (with some data, so reopening it shows something).
        states = self.index_states("esticli-closed")
        if states.get("esticli-closed", {}).get("status") != "close":
            self.ensure_index("esticli-closed", {"number_of_shards": 1, "number_of_replicas": self.replicas})
            self.top_up("esticli-closed", 1000, 100)
            self.es.post("esticli-closed/_close")
            self.log("  closed esticli-closed")
        else:
            self.log("  esticli-closed already closed")

        # Alias over two regular indices.
        self.es.post("_aliases", {"actions": [
            {"add": {"index": "esticli-small", "alias": "esticli-alias"}},
            {"add": {"index": "esticli-medium", "alias": "esticli-alias"}},
        ]})
        self.log("  alias esticli-alias -> esticli-small, esticli-medium")

        # Red: a primary that can never be allocated.
        if self.args.with_red:
            created = self.ensure_index("esticli-red", {
                "number_of_shards": 1,
                "number_of_replicas": 0,
                "index.routing.allocation.require._name": "esticli-no-such-node",
            }, wait_active=False)
            self.log("  created esticli-red (unassignable primary -> cluster RED)" if created else "  esticli-red present (cluster RED)")
        elif self.es.head_exists("esticli-red"):
            self.es.delete("esticli-red")
            self.log("  deleted esticli-red (pass --with-red to keep the cluster red)")

    def awkward_names(self) -> None:
        self.log("awkward index names")
        for name in AWKWARD + [MAX_NAME]:
            label = name if len(name) < 60 else f"{name[:24]}...({len(name.encode())} bytes)"
            try:
                self.ensure_index(name, {"number_of_shards": 1, "number_of_replicas": self.replicas})
                self.es.post(f"{q(name)}/_refresh")
                if self.doc_count(name) < 20:
                    bulk_load(self.es, name, 20 - self.doc_count(name), 50)
                self.log(f"  ok       {label}")
            except ESError as exc:
                self.notes.append(f"index name {label!r} rejected: {short_error(exc.body)}")
                self.log(f"  REJECTED {label}: {short_error(exc.body)}")

    def probe_illegal_names(self) -> None:
        self.log("probing names ES should reject (nothing is created)")
        for name in ILLEGAL_PROBES:
            label = name if len(name) < 60 else f"{name[:24]}...({len(name.encode())} bytes)"
            try:
                self.es.put(q(name), {"settings": {"number_of_replicas": 0}})
                self.log(f"  ACCEPTED (unexpected!) {label} - deleting")
                self.es.delete(q(name))
            except ESError as exc:
                # The reason repeats the (possibly 256-byte) name first; keep
                # the rule at the end, e.g. "must not contain '#'" or
                # "index name is too long, (256 > 255)".
                text = short_error(exc.body)
                reason = text.split(", must ", 1)[1] if ", must " in text else text.rsplit("], ", 1)[-1]
                reason = reason[:90]
                self.log(f"  rejected  {label!r:42} {reason}")

    def summary(self) -> None:
        self.es.post("esticli-*,.esticli-*/_refresh", params={"expand_wildcards": "open,hidden"}, ok=(404,))
        health = self.es.get("_cluster/health")
        print()
        print(f"cluster {health['cluster_name']} ES {'.'.join(map(str, self.version))}: status={health['status'].upper()}"
              f" nodes={health['number_of_nodes']} unassigned_shards={health['unassigned_shards']}")
        rows = sorted(self.index_states().values(), key=lambda r: r["index"])
        print(f"{'index':58} {'health':7} {'status':6} {'docs':>8} {'size':>9}")
        for r in rows:
            name = r["index"] if len(r["index"]) <= 58 else r["index"][:46] + f"..({len(r['index'].encode())}B)"
            size = human_bytes(int(r["store.size"])) if r.get("store.size") else "-"
            print(f"{name:58} {r.get('health') or '-':7} {r.get('status'):6} {r.get('docs.count') or '-':>8} {size:>9}")
        streams = self.es.get("_data_stream/esticli-*", ok=(404,))
        for ds in (streams or {}).get("data_streams", []):
            print(f"data stream {ds['name']}: generation {ds.get('generation')}, {len(ds['indices'])} backing index(es),"
                  f" ilm_policy={ds.get('ilm_policy')}, lifecycle={ds.get('lifecycle')}")
        for note in self.notes:
            print(f"note: {note}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    add_connection_args(parser)
    parser.add_argument("--reset", action="store_true", help="delete all esticli fixtures first (not esticli-scale-*)")
    parser.add_argument("--reset-only", action="store_true", help="delete the fixtures and stop")
    parser.add_argument("--reset-scale", action="store_true", help="with --reset: also delete esticli-scale-* indices")
    parser.add_argument("--with-red", action="store_true", help="add esticli-red (unassignable primary): cluster goes RED")
    parser.add_argument("--replicas", default="auto", help="replicas for regular fixtures: auto (0 on one data node, else 1) or N")
    parser.add_argument("--scale", type=float, default=1.0, help="multiply fixture doc counts (default 1.0; esticli-large is ~100k x 1KB)")
    parser.add_argument("--rollover-docs", type=int, default=5000, help="ILM rollover max_docs for esticli-logs (default 5000)")
    parser.add_argument("--skip-awkward", action="store_true", help="don't create the awkward-name indices")
    parser.add_argument("--probe-names", action="store_true", help="also try names ES must reject and report the reasons")
    args = parser.parse_args()

    es = connect_or_die(args)
    seeder = Seeder(es, args)
    print(f"seeding {args.url} (ES {'.'.join(map(str, seeder.version))}, {seeder.data_nodes} data node(s), replicas={seeder.replicas})")
    if args.reset or args.reset_only:
        seeder.reset()
        if args.reset_only:
            return
    seeder.templates()
    seeder.regular_indices()
    seeder.data_streams()
    seeder.state_fixtures()
    if not args.skip_awkward:
        seeder.awkward_names()
    if args.probe_names:
        seeder.probe_illegal_names()
    seeder.summary()


if __name__ == "__main__":
    try:
        main()
    except ESError as exc:
        print(f"error: {exc}", file=sys.stderr)
        sys.exit(1)
    except KeyboardInterrupt:
        sys.exit(130)
