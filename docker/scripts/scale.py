#!/usr/bin/env python3
"""Scale fixtures and per-tick request measurements for esticli. Stdlib only.

Create N indices fast (explicit creates from parallel workers, which the
master batches; ~300 indices/s on one laptop node), optionally with docs:

  scale.py --count 10000                     # esticli-scale-00000 .. 09999
  scale.py --count 1000 --docs 50 --prefix esticli-big-

Measure exactly what esticli requests on every refresh tick, as JSON on
stdout: latency (median/p95 over --samples calls, gzip like esticli's
reqwest client) and response size with and without gzip:

  GET _stats/indexing,docs,store
  GET _cluster/health
  GET _nodes/stats/indices,jvm,process,breaker   (only while the nodes panel is open)

plus the wall time of the requests issued concurrently, the way
EsClient::fetch_tick joins them (with and without the nodes request):

  scale.py --measure --samples 20 > measure-10k.json
  scale.py --count 10000 --measure           # create, then measure

Remove them again (and the esticli-scale template):

  scale.py --delete

Creation is idempotent (existing names are skipped). If the new shards would
exceed cluster.max_shards_per_node, the limit is raised with a persistent
cluster setting (--delete removes that override again).
"""

from __future__ import annotations

import argparse
import concurrent.futures as cf
import json
import os
import random
import statistics
import sys
import threading
import time

sys.dont_write_bytecode = True  # keep docker/scripts free of __pycache__
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from eslib import ESError, add_connection_args, connect_or_die, human_bytes, now_iso, q, short_error  # noqa: E402

TEMPLATE = "esticli-scale"
TICK_REQUESTS = [
    "_stats/indexing,docs,store",
    "_cluster/health",
    "_nodes/stats/indices,jvm,process,breaker",
]


def log(msg: str) -> None:
    # Progress goes to stderr so --measure's JSON on stdout stays clean.
    print(msg, file=sys.stderr, flush=True)


# --------------------------------------------------------------------- create
def existing_indices(es, prefix: str) -> set[str]:
    rows = es.get(f"_cat/indices/{q(prefix)}*", params={"format": "json", "h": "index", "expand_wildcards": "all"},
                  ok=(404,))
    return {r["index"] for r in rows} if isinstance(rows, list) else set()


def ensure_shard_headroom(es, new_shards: int, allow_raise: bool) -> None:
    health = es.get("_cluster/health")
    current = health["active_shards"] + health["unassigned_shards"] + health["initializing_shards"]
    settings = es.get("_cluster/settings", params={"include_defaults": "true", "flat_settings": "true"})
    limit_per_node = None
    for scope in ("transient", "persistent", "defaults"):
        value = settings.get(scope, {}).get("cluster.max_shards_per_node")
        if value is not None:
            limit_per_node = int(value)
            break
    if limit_per_node is None:
        return
    data_nodes = max(health["number_of_data_nodes"], 1)
    capacity = limit_per_node * data_nodes
    needed = current + new_shards
    if needed <= capacity:
        return
    # Leave room for seed fixtures and data stream rollovers.
    new_limit = -(-(needed + 1000) // data_nodes)
    if not allow_raise:
        raise SystemExit(f"error: {needed} shards would exceed cluster.max_shards_per_node={limit_per_node} x "
                         f"{data_nodes} data node(s); raise ES_MAX_SHARDS_PER_NODE or drop --no-raise-limit")
    es.put("_cluster/settings", {"persistent": {"cluster.max_shards_per_node": new_limit}})
    log(f"raised cluster.max_shards_per_node {limit_per_node} -> {new_limit} (persistent; --delete resets it)")


def put_template(es, args) -> None:
    es.put(f"_index_template/{TEMPLATE}", {
        "index_patterns": [f"{args.prefix}*"],
        # Above seed.py's catch-all esticli-* template (priority 10).
        "priority": 20,
        "template": {
            "settings": {
                "number_of_shards": args.shards,
                "number_of_replicas": args.replicas,
            },
            # One small, identical mapping: ES deduplicates identical mappings,
            # so 50k of these cost far less heap than 50k distinct ones.
            "mappings": {
                "dynamic": "false",
                "properties": {
                    "@timestamp": {"type": "date"},
                    "n": {"type": "long"},
                    "level": {"type": "keyword"},
                    "message": {"type": "text"},
                },
            },
        },
        "_meta": {"managed_by": "esticli docker/scripts/scale.py"},
    })


def create(es, args) -> list[str]:
    width = max(5, len(str(args.start + args.count - 1)))
    names = [f"{args.prefix}{i:0{width}d}" for i in range(args.start, args.start + args.count)]
    have = existing_indices(es, args.prefix)
    todo = [n for n in names if n not in have]
    log(f"{len(names)} indices requested, {len(names) - len(todo)} already exist, creating {len(todo)} "
        f"({args.shards} shard(s), {args.replicas} replica(s), {args.workers} workers)")
    put_template(es, args)
    if todo:
        ensure_shard_headroom(es, len(todo) * args.shards * (1 + args.replicas), not args.no_raise_limit)

    done = 0
    errors: list[str] = []
    lock = threading.Lock()

    def make(name: str) -> None:
        nonlocal done
        try:
            # Don't wait for the shard to start: allocation catches up after,
            # and waiting per request would serialise the master's batching.
            es.put(q(name), params={"wait_for_active_shards": "0", "master_timeout": "120s"})
        except ESError as exc:
            if exc.error_type != "resource_already_exists_exception":
                with lock:
                    errors.append(f"{name}: {short_error(exc.body)}")
        with lock:
            done += 1

    start = time.monotonic()
    with cf.ThreadPoolExecutor(args.workers) as pool:
        futures = [pool.submit(make, n) for n in todo]
        while not all(f.done() for f in futures):
            time.sleep(2)
            elapsed = time.monotonic() - start
            log(f"  created {done}/{len(todo)}  ({done / elapsed:.0f}/s, {len(errors)} errors)")
    if todo:
        elapsed = time.monotonic() - start
        log(f"created {len(todo) - len(errors)} indices in {elapsed:.1f}s ({len(todo) / elapsed:.0f}/s)")
    for err in errors[:10]:
        log(f"  ! {err}")
    if len(errors) > 10:
        log(f"  ! ... and {len(errors) - 10} more errors")

    wait_for_allocation(es, args)
    if args.docs > 0:
        fill(es, args, names)
    return names


def wait_for_allocation(es, args, timeout_s: float = 900) -> None:
    """Waits until every --prefix shard that can be assigned is started.

    Cluster-wide "no initializing shards" isn't enough: right after a burst of
    creates most new primaries are still *unassigned* (not yet initializing),
    and ES reports a never-allocated new primary as yellow, not red. So this
    asks for the prefix's own health: green, or yellow when the replicas
    can't all be placed on the available data nodes.
    """
    data_nodes = es.get("_cluster/health")["number_of_data_nodes"]
    want = "green" if args.replicas < data_nodes else "yellow"
    log(f"waiting for {args.prefix}* to reach {want} ...")
    deadline = time.monotonic() + timeout_s
    while True:
        health = es.get(f"_cluster/health/{q(args.prefix)}*",
                        params={"wait_for_status": want, "wait_for_no_initializing_shards": "true",
                                "timeout": "30s", "expand_wildcards": "all"},
                        ok=(408,))
        if not health.get("timed_out"):
            break
        if time.monotonic() > deadline:
            log("  giving up waiting; check `_cluster/allocation/explain`")
            break
        log(f"  {health['status']}: {health['active_shards']} active, {health['initializing_shards']} initializing, "
            f"{health['unassigned_shards']} unassigned")
    overall = es.get("_cluster/health")
    log(f"{args.prefix}* is {health['status']} ({health['active_shards']} active shards); cluster is "
        f"{overall['status']} with {overall['active_shards']} active shards")


def fill(es, args, names: list[str]) -> None:
    """Indexes --docs documents into every index, many indices per _bulk."""
    log(f"indexing {args.docs} docs into each of {len(names)} indices ...")
    rng = random.Random(1)
    per_request = max(1, 5000 // args.docs)  # indices per bulk, ~5000 docs per request

    def one(chunk: list[str]) -> int:
        ts = now_iso()
        lines = []
        for name in chunk:
            action = json.dumps({"index": {"_index": name}})
            for n in range(args.docs):
                lines.append(action)
                lines.append(json.dumps({"@timestamp": ts, "n": n, "level": rng.choice(["info", "warn"]),
                                         "message": f"scale doc {n}"}))
        resp = es.bulk(lines)
        if resp.get("errors"):
            return sum(1 for it in resp["items"] if next(iter(it.values())).get("error"))
        return 0

    chunks = [names[i:i + per_request] for i in range(0, len(names), per_request)]
    failed = 0
    start = time.monotonic()
    with cf.ThreadPoolExecutor(min(args.workers, 8)) as pool:
        for i, n_failed in enumerate(pool.map(one, chunks), 1):
            failed += n_failed
            if i % 20 == 0:
                log(f"  {i}/{len(chunks)} bulk requests")
    es.post(f"{q(args.prefix)}*/_refresh", ok=(404,))
    log(f"indexed {len(names) * args.docs} docs in {time.monotonic() - start:.1f}s ({failed} failed)")


# -------------------------------------------------------------------- measure
def percentile(values: list[float], pct: float) -> float:
    """Nearest-rank percentile (no interpolation: with 20 samples p95 is the 19th)."""
    ordered = sorted(values)
    rank = max(1, -(-len(ordered) * pct // 100))
    return ordered[int(rank) - 1]


def summarise(samples: list[float]) -> dict:
    return {
        "median": round(statistics.median(samples), 1),
        "p95": round(percentile(samples, 95), 1),
        "min": round(min(samples), 1),
        "max": round(max(samples), 1),
    }


def timed_get(es, path: str, encoding: str) -> tuple[float, int, str]:
    """(milliseconds, response bytes on the wire, Content-Encoding) of one GET."""
    t0 = time.perf_counter()
    status, headers, data = es.raw("GET", path, headers={"Accept-Encoding": encoding})
    elapsed = (time.perf_counter() - t0) * 1000
    if status >= 300:
        raise ESError(status, data.decode("utf-8", "replace")[:300], "GET", path)
    return elapsed, len(data), headers.get("Content-Encoding", "identity")


def measure(es, args) -> dict:
    health = es.get("_cluster/health")
    count = es.get("_cat/count", params={"format": "json"}, ok=(404,))
    indices_visible = len((es.get("_stats/docs") or {}).get("indices", {}))
    result = {
        "url": args.url,
        "es_version": ".".join(map(str, es.version())),
        "measured_at": now_iso(),
        "samples": args.samples,
        "cluster": {
            "status": health["status"],
            "nodes": health["number_of_nodes"],
            "indices_in_stats": indices_visible,
            "active_shards": health["active_shards"],
            "docs": int(count[0]["count"]) if isinstance(count, list) and count else None,
        },
        "requests": {},
        "tick": {},
    }

    for path in TICK_REQUESTS:
        timed_get(es, path, "gzip")  # warm-up: caches, JIT, connection
        latencies = []
        for _ in range(args.samples):
            ms, _, _ = timed_get(es, path, "gzip")
            latencies.append(ms)
        _, plain_bytes, _ = timed_get(es, path, "identity")
        _, gzip_bytes, encoding = timed_get(es, path, "gzip")
        if encoding != "gzip":
            # http.compression off (or not offered over TLS): say so rather
            # than report the identity size as if it were compressed.
            gzip_bytes_note = f"server answered with Content-Encoding: {encoding}"
        else:
            gzip_bytes_note = None
        entry = {
            "latency_ms_gzip": summarise(latencies),
            "bytes_identity": plain_bytes,
            "bytes_gzip": gzip_bytes,
            "bytes_identity_human": human_bytes(plain_bytes),
            "bytes_gzip_human": human_bytes(gzip_bytes),
            "gzip_ratio": round(plain_bytes / gzip_bytes, 1) if gzip_bytes else None,
        }
        if gzip_bytes_note:
            entry["note"] = gzip_bytes_note
        result["requests"][path] = entry
        log(f"  {path:45} median {entry['latency_ms_gzip']['median']:8.1f}ms  p95 {entry['latency_ms_gzip']['p95']:8.1f}ms"
            f"  {human_bytes(plain_bytes):>9} plain  {human_bytes(gzip_bytes):>9} gzip")

    # Concurrent, like EsClient::fetch_tick's tokio::join!. eslib keeps one
    # keep-alive connection per thread, so each request gets its own.
    for label, paths in (("with_nodes_panel", TICK_REQUESTS), ("without_nodes_panel", TICK_REQUESTS[:2])):
        walls = []
        with cf.ThreadPoolExecutor(len(paths)) as pool:
            list(pool.map(lambda p: timed_get(es, p, "gzip"), paths))  # warm-up
            for _ in range(args.samples):
                t0 = time.perf_counter()
                list(pool.map(lambda p: timed_get(es, p, "gzip"), paths))
                walls.append((time.perf_counter() - t0) * 1000)
        result["tick"][label] = {"wall_ms": summarise(walls)}
        log(f"  tick {label:40} median {statistics.median(walls):8.1f}ms  p95 {percentile(walls, 95):8.1f}ms")
    return result


# --------------------------------------------------------------------- delete
def delete(es, args) -> None:
    names = sorted(existing_indices(es, args.prefix))
    log(f"deleting {len(names)} indices matching {args.prefix}* ...")
    start = time.monotonic()
    if names:
        try:
            es.request("DELETE", f"{q(args.prefix)}*",
                       params={"expand_wildcards": "all", "master_timeout": "5m", "timeout": "5m"})
        except ESError as exc:
            if "destructive" not in short_error(exc.body) and exc.error_type != "illegal_argument_exception":
                raise
            # action.destructive_requires_name=true: delete by explicit names,
            # chunked to stay under the 4KB request-line limit.
            for i in range(0, len(names), 100):
                es.delete(",".join(q(n) for n in names[i:i + 100]), ok=(404,))
    es.delete(f"_index_template/{TEMPLATE}", ok=(404,))
    persistent = es.get("_cluster/settings", params={"flat_settings": "true"}).get("persistent", {})
    if "cluster.max_shards_per_node" in persistent:
        es.put("_cluster/settings", {"persistent": {"cluster.max_shards_per_node": None}})
        log("removed the persistent cluster.max_shards_per_node override")
    log(f"deleted in {time.monotonic() - start:.1f}s")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    add_connection_args(parser)
    parser.add_argument("--count", type=int, default=0, help="create this many indices")
    parser.add_argument("--start", type=int, default=0, help="first index number (default %(default)s)")
    parser.add_argument("--prefix", default="esticli-scale-", help="index name prefix (default %(default)s)")
    parser.add_argument("--shards", type=int, default=1, help="primary shards per index (default %(default)s)")
    parser.add_argument("--replicas", type=int, default=0, help="replicas per index (default %(default)s)")
    parser.add_argument("--workers", type=int, default=32, help="parallel create requests (default %(default)s)")
    parser.add_argument("--docs", type=int, default=0, help="docs to index into each created index (default 0)")
    parser.add_argument("--no-raise-limit", action="store_true",
                        help="fail instead of raising cluster.max_shards_per_node")
    parser.add_argument("--measure", action="store_true", help="measure esticli's per-tick requests (JSON on stdout)")
    parser.add_argument("--samples", type=int, default=20, help="calls per request for --measure (default %(default)s)")
    parser.add_argument("--delete", action="store_true", help="delete every index matching --prefix")
    args = parser.parse_args()
    if not (args.count or args.measure or args.delete):
        parser.error("nothing to do: pass --count N, --measure and/or --delete")
    if not args.prefix.startswith("esticli-"):
        parser.error("--prefix must start with 'esticli-' (keeps --delete from touching anything else)")

    # Index creation and deletion of thousands of indices can take minutes.
    args.timeout = max(args.timeout, 600)
    es = connect_or_die(args)
    if args.delete:
        delete(es, args)
    if args.count:
        create(es, args)
    if args.measure:
        log(f"measuring {args.url} ({args.samples} samples per request)")
        print(json.dumps(measure(es, args), indent=2))


if __name__ == "__main__":
    try:
        main()
    except ESError as exc:
        print(f"error: {exc}", file=sys.stderr)
        sys.exit(1)
    except KeyboardInterrupt:
        sys.exit(130)
