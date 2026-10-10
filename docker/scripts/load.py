#!/usr/bin/env python3
"""Sustained _bulk ingest for esticli, with per-index rate profiles. Stdlib only.

Each target index gets a profile and a weight. --rate (docs/s) is divided
between the targets by weight, then each target's share is shaped over time
by its profile. Every profile except idle averages 1.0x its share, so the
long-run total is close to --rate.

  steady   constant
  bursty   --burst-on seconds at a high rate, then --burst-off seconds of nothing
  sine     1 +/- 0.9 sine wave over --sine-period seconds
  ramp     sawtooth from 0 to 2x over --ramp-period seconds, then drops back
  idle     never writes (keeps an index in the target list at 0 docs/s)

Default targets (all created by seed.py; data streams are written with
op_type=create, so ILM rollovers happen under load):

  esticli-steady=steady  esticli-bursty=bursty  esticli-sine=sine
  esticli-ramp=ramp      esticli-logs=steady    esticli-metrics=sine:0.5
  esticli-churn=steady:0.5                      esticli-idle=idle

--churn periodically (every --churn-every seconds, rotating):
  1. delete_by_query half of --churn-index      (doc count and size shrink)
  2. delete and recreate --churn-index          (index_total counter resets)
  3. roll over the first data stream target     (new .ds- backing index)

--spread 'esticli-scale-*' also writes to up to --spread-max existing indices
matching the pattern, each with a random profile and phase, so a scale test
shows many rows moving at once.

Examples:
  load.py                                  # 1000 docs/s to the default targets
  load.py --rate 5000 --duration 60 --churn
  load.py --target esticli-steady=steady --target esticli-sine=sine:3
  load.py --rate 20000 --workers 8 --spread 'esticli-scale-*' --spread-max 2000
"""

from __future__ import annotations

import argparse
import concurrent.futures as cf
import itertools
import json
import math
import os
import random
import signal
import sys
import threading
import time
from dataclasses import dataclass, field

sys.dont_write_bytecode = True  # keep docker/scripts free of __pycache__
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from eslib import ESError, add_connection_args, connect_or_die, now_iso, q, short_error  # noqa: E402

DEFAULT_TARGETS = [
    "esticli-steady=steady",
    "esticli-bursty=bursty",
    "esticli-sine=sine",
    "esticli-ramp=ramp",
    "esticli-logs=steady",
    "esticli-metrics=sine:0.5",
    "esticli-churn=steady:0.5",
    "esticli-idle=idle",
]
PROFILES = ("steady", "bursty", "sine", "ramp", "idle")
TICK = 0.2  # seconds between scheduling rounds

LEVELS = ["debug", "info", "info", "info", "warn", "error"]
SERVICES = [f"svc-{i}" for i in range(12)]


@dataclass
class Target:
    name: str
    profile: str
    weight: float
    phase: float = 0.0          # seconds, shifts the profile so spread targets differ
    data_stream: bool = False
    owed: float = 0.0           # fractional docs carried between ticks
    sent: int = 0
    ok: int = 0
    failed: int = 0
    window_ok: int = 0
    window_sent: int = 0
    seq: itertools.count = field(default_factory=itertools.count)


class Shape:
    """Profile multipliers; every profile but idle averages 1.0 over its period."""

    def __init__(self, args):
        self.burst_on = args.burst_on
        self.burst_off = args.burst_off
        self.sine_period = args.sine_period
        self.ramp_period = args.ramp_period

    def __call__(self, profile: str, t: float) -> float:
        if profile == "steady":
            return 1.0
        if profile == "bursty":
            cycle = self.burst_on + self.burst_off
            return cycle / self.burst_on if (t % cycle) < self.burst_on else 0.0
        if profile == "sine":
            return 1.0 + 0.9 * math.sin(2 * math.pi * t / self.sine_period)
        if profile == "ramp":
            return 2.0 * ((t % self.ramp_period) / self.ramp_period)
        return 0.0  # idle


def parse_target(spec: str) -> Target:
    """NAME=PROFILE[:WEIGHT]"""
    name, sep, rest = spec.partition("=")
    if not sep or not name:
        raise SystemExit(f"bad --target {spec!r}: expected NAME=PROFILE[:WEIGHT]")
    profile, _, weight = rest.partition(":")
    if profile not in PROFILES:
        raise SystemExit(f"bad --target {spec!r}: profile must be one of {', '.join(PROFILES)}")
    try:
        w = float(weight) if weight else 1.0
    except ValueError:
        raise SystemExit(f"bad --target {spec!r}: weight must be a number")
    return Target(name=name, profile=profile, weight=w)


class DocFactory:
    """Cheap document generation: payloads come from a precomputed pool."""

    def __init__(self, payload_bytes: int):
        rng = random.Random(42)
        self.rng = random.Random()
        self.pads = [rng.randbytes(max(payload_bytes, 2) // 2).hex() for _ in range(256)] if payload_bytes else None

    def doc(self, n: int, ts: str) -> str:
        r = self.rng
        d = {
            "@timestamp": ts,
            "n": n,
            "level": r.choice(LEVELS),
            "service": r.choice(SERVICES),
            "latency_ms": int(r.expovariate(1 / 40)),
            "message": f"load document {n}",
        }
        if self.pads:
            d["pad"] = r.choice(self.pads)
        return json.dumps(d)


class Loader:
    def __init__(self, es, args, targets: list[Target]):
        self.es = es
        self.args = args
        self.targets = targets
        self.shape = Shape(args)
        self.docs = DocFactory(args.doc_bytes)
        total_weight = sum(t.weight for t in targets if t.profile != "idle") or 1.0
        self.base = {id(t): 0.0 if t.profile == "idle" else args.rate * t.weight / total_weight for t in targets}
        self.pool = cf.ThreadPoolExecutor(args.workers, thread_name_prefix="bulk")
        self.inflight = 0
        self.lock = threading.Lock()
        self.stop = threading.Event()
        self.shed = 0           # docs not sent because every worker was busy
        self.rejected = 0       # bulk-level failures (429, timeouts, ...)
        self.last_error = ""
        self.window_shed = 0
        self.churn_step = 0

    # ---------------------------------------------------------------- bulk
    def _send(self, batch: list[tuple[Target, str]]) -> None:
        lines = []
        for target, doc in batch:
            op = "create" if target.data_stream else "index"
            lines.append(json.dumps({op: {"_index": target.name}}))
            lines.append(doc)
        try:
            resp = self.es.request(
                "POST", "_bulk", body="\n".join(lines) + "\n",
                params={"filter_path": "errors,items.*.status,items.*.error.type,items.*.error.reason"},
                headers={"Content-Type": "application/x-ndjson"},
            )
            items = resp.get("items", []) if isinstance(resp, dict) else []
            failures = {}
            if isinstance(resp, dict) and resp.get("errors"):
                for pos, item in enumerate(items):
                    result = next(iter(item.values()))
                    if result.get("error"):
                        failures[pos] = result["error"]
            with self.lock:
                for pos, (target, _) in enumerate(batch):
                    if pos in failures:
                        target.failed += 1
                    else:
                        target.ok += 1
                        target.window_ok += 1
                if failures:
                    err = next(iter(failures.values()))
                    self.last_error = f"{err.get('type')}: {str(err.get('reason'))[:120]}"
        except ESError as exc:
            with self.lock:
                self.rejected += len(batch)
                for target, _ in batch:
                    target.failed += 1
                self.last_error = str(exc)[:160]
        finally:
            with self.lock:
                self.inflight -= 1

    def _submit(self, batch: list[tuple[Target, str]]) -> bool:
        with self.lock:
            if self.inflight >= self.args.workers * 2:
                return False
            self.inflight += 1
        self.pool.submit(self._send, batch)
        return True

    def schedule(self, t: float, dt: float) -> None:
        """Works out what is due this tick and submits it in --batch chunks."""
        ts = now_iso()
        due: list[tuple[Target, str]] = []
        for target in self.targets:
            rate = self.base[id(target)] * self.shape(target.profile, t + target.phase)
            target.owed += rate * dt
            n = int(target.owed)
            target.owed -= n
            for _ in range(n):
                due.append((target, self.docs.doc(next(target.seq), ts)))
            target.sent += n
            target.window_sent += n
        # Interleave targets within a batch, like a real shipper would.
        random.shuffle(due)
        for i in range(0, len(due), self.args.batch):
            chunk = due[i:i + self.args.batch]
            if not self._submit(chunk):
                dropped = sum(1 for _ in due[i:])
                with self.lock:
                    self.shed += dropped
                    self.window_shed += dropped
                for target, _ in due[i:]:
                    target.sent -= 1
                    target.window_sent -= 1
                break

    # --------------------------------------------------------------- churn
    def churn(self) -> None:
        actions = [self._churn_delete_by_query, self._churn_recreate, self._churn_rollover]
        action = actions[self.churn_step % len(actions)]
        self.churn_step += 1
        try:
            action()
        except ESError as exc:
            print(f"  churn: {action.__name__.removeprefix('_churn_')} failed: {short_error(exc.body)}", flush=True)

    def _churn_delete_by_query(self) -> None:
        name = self.args.churn_index
        self.es.post(f"{q(name)}/_refresh", ok=(404,))
        count = (self.es.get(f"{q(name)}/_count", ok=(404,)) or {}).get("count", 0)
        if not count:
            print(f"  churn: {name} is empty, nothing to delete", flush=True)
            return
        resp = self.es.post(f"{q(name)}/_delete_by_query",
                            {"query": {"match_all": {}}},
                            params={"max_docs": max(count // 2, 1), "conflicts": "proceed", "refresh": "true"})
        # Expunging the deletes is what makes the store size drop too.
        self.es.post(f"{q(name)}/_forcemerge", params={"only_expunge_deletes": "true"})
        print(f"  churn: delete_by_query removed {resp.get('deleted')} of {count} docs from {name} (+ expunge)", flush=True)

    def _churn_recreate(self) -> None:
        name = self.args.churn_index
        self.es.delete(q(name), ok=(404,))
        try:
            self.es.put(q(name), {"settings": {"number_of_shards": 1, "number_of_replicas": self.args.replicas}})
        except ESError as exc:
            # A concurrent bulk may have auto-created it already; that's still a reset.
            if exc.error_type != "resource_already_exists_exception":
                raise
        print(f"  churn: deleted and recreated {name} (index_total counter reset)", flush=True)

    def _churn_rollover(self) -> None:
        stream = next((t.name for t in self.targets if t.data_stream), None)
        if stream is None:
            print("  churn: no data stream target to roll over", flush=True)
            return
        resp = self.es.post(f"{q(stream)}/_rollover")
        print(f"  churn: rolled over {stream}: {resp.get('old_index')} -> {resp.get('new_index')}", flush=True)

    # ---------------------------------------------------------------- loop
    def report(self, elapsed: float, window: float) -> None:
        with self.lock:
            achieved = sum(t.window_ok for t in self.targets) / window
            scheduled = sum(t.window_sent for t in self.targets) / window
            per_index = []
            if len(self.targets) <= 12:
                for t in self.targets:
                    short = t.name.removeprefix("esticli-")
                    per_index.append(f"{short} {t.window_ok / window:.0f}")
            for t in self.targets:
                t.window_ok = 0
                t.window_sent = 0
            shed, self.window_shed = self.window_shed, 0
            failed = sum(t.failed for t in self.targets)
            total = sum(t.ok for t in self.targets)
            err = self.last_error
            self.last_error = ""
        line = (f"[{elapsed:6.0f}s] scheduled {scheduled:7.0f}/s  indexed {achieved:7.0f}/s  "
                f"total {total:>9}  failed {failed}  in-flight {self.inflight}")
        if shed:
            line += f"  SHED {shed} (ES or this client can't keep up)"
        print(line, flush=True)
        if per_index:
            print("          " + "  ".join(per_index), flush=True)
        if err:
            print(f"          last error: {err}", flush=True)

    def run(self) -> None:
        args = self.args
        start = time.monotonic()
        next_tick = start
        last_report = start
        next_churn = start + args.churn_every if args.churn else math.inf
        while not self.stop.is_set():
            now = time.monotonic()
            elapsed = now - start
            if args.duration and elapsed >= args.duration:
                break
            if now >= next_tick:
                self.schedule(elapsed, TICK)
                next_tick += TICK
                if next_tick < now:  # fell behind (laptop slept, GC...): don't burst to catch up
                    next_tick = now + TICK
            if now - last_report >= args.report_every:
                self.report(elapsed, now - last_report)
                last_report = now
            if now >= next_churn:
                # Off the scheduling thread: delete_by_query/forcemerge can take a while.
                threading.Thread(target=self.churn, daemon=True).start()
                next_churn += args.churn_every
            self.stop.wait(max(0.0, min(next_tick, last_report + args.report_every) - time.monotonic()))

    def finish(self, elapsed: float) -> None:
        self.pool.shutdown(wait=True, cancel_futures=True)
        ok = sum(t.ok for t in self.targets)
        failed = sum(t.failed for t in self.targets)
        print(f"\nstopped after {elapsed:.0f}s: {ok} docs indexed ({ok / max(elapsed, 1e-9):.0f}/s average), "
              f"{failed} failed, {self.shed} shed")
        for t in self.targets:
            if t.ok or t.failed:
                print(f"  {t.name:40} {t.profile:7} ok {t.ok:>8}  failed {t.failed}")


def resolve_targets(es, args) -> list[Target]:
    targets = [parse_target(s) for s in (args.target or DEFAULT_TARGETS)]
    if args.spread:
        rows = es.get(f"_cat/indices/{args.spread}", params={"format": "json", "h": "index,status"}, ok=(404,))
        names = sorted(r["index"] for r in (rows if isinstance(rows, list) else []) if r.get("status") == "open")
        rng = random.Random(7)
        if len(names) > args.spread_max:
            names = sorted(rng.sample(names, args.spread_max))
        if not names:
            print(f"warning: --spread {args.spread!r} matched no open index", file=sys.stderr)
        for name in names:
            profile = rng.choice(["steady", "steady", "sine", "bursty", "ramp"])
            targets.append(Target(name=name, profile=profile, weight=args.spread_weight / max(len(names), 1),
                                  phase=rng.uniform(0, 600)))
    # Which targets are data streams (they need op_type=create)?
    streams = es.get("_data_stream", ok=(404,)) or {}
    stream_names = {d["name"] for d in streams.get("data_streams", [])} if isinstance(streams, dict) else set()
    missing = []
    for t in targets:
        t.data_stream = t.name in stream_names
        if not t.data_stream and t.profile != "idle" and not es.head_exists(q(t.name)):
            missing.append(t.name)
    if missing and len(missing) <= 10:
        print(f"note: {', '.join(missing)} don't exist yet and will be auto-created (run seed.py first for the real fixtures)")
    elif missing:
        print(f"note: {len(missing)} targets don't exist yet and will be auto-created")
    return targets


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    add_connection_args(parser)
    parser.add_argument("--rate", type=float, default=1000, help="total docs/s target (default %(default)s)")
    parser.add_argument("--target", action="append", metavar="NAME=PROFILE[:WEIGHT]",
                        help="replace the default targets (repeatable)")
    parser.add_argument("--duration", type=float, default=0, help="stop after N seconds (default: until Ctrl+C)")
    parser.add_argument("--workers", type=int, default=4, help="concurrent _bulk requests (default %(default)s)")
    parser.add_argument("--batch", type=int, default=1000, help="max docs per _bulk request (default %(default)s)")
    parser.add_argument("--doc-bytes", type=int, default=200, help="random payload per doc (default %(default)s)")
    parser.add_argument("--report-every", type=float, default=5, help="seconds between progress lines (default %(default)s)")
    parser.add_argument("--burst-on", type=float, default=5, help="bursty: seconds on (default %(default)s)")
    parser.add_argument("--burst-off", type=float, default=10, help="bursty: seconds off (default %(default)s)")
    parser.add_argument("--sine-period", type=float, default=60, help="sine: period in seconds (default %(default)s)")
    parser.add_argument("--ramp-period", type=float, default=60, help="ramp: period in seconds (default %(default)s)")
    parser.add_argument("--churn", action="store_true", help="periodically shrink/recreate/roll over (see above)")
    parser.add_argument("--churn-every", type=float, default=10, help="seconds between churn actions (default %(default)s)")
    parser.add_argument("--churn-index", default="esticli-churn", help="index churn shrinks and recreates (default %(default)s)")
    parser.add_argument("--replicas", type=int, default=0, help="replicas when churn recreates the index (default %(default)s)")
    parser.add_argument("--spread", metavar="PATTERN", help="also write to existing indices matching PATTERN")
    parser.add_argument("--spread-max", type=int, default=500, help="cap on --spread indices (default %(default)s)")
    parser.add_argument("--spread-weight", type=float, default=4.0,
                        help="combined weight of all --spread indices (default %(default)s)")
    args = parser.parse_args()
    if args.burst_on <= 0 or args.sine_period <= 0 or args.ramp_period <= 0:
        parser.error("--burst-on, --sine-period and --ramp-period must be positive")

    es = connect_or_die(args)
    targets = resolve_targets(es, args)
    loader = Loader(es, args, targets)

    # SIGTERM (timeout, esenv.sh, tmux kill) stops as cleanly as Ctrl+C.
    signal.signal(signal.SIGTERM, lambda *_: loader.stop.set())
    print(f"loading {args.url}: {args.rate:.0f} docs/s over {len(targets)} targets, "
          f"{args.workers} workers, batch {args.batch}"
          + (f", churn every {args.churn_every:.0f}s on {args.churn_index}" if args.churn else "")
          + (f", for {args.duration:.0f}s" if args.duration else ", Ctrl+C to stop"), flush=True)
    if len(targets) <= 12:
        for t in targets:
            kind = "data stream" if t.data_stream else "index"
            print(f"  {t.name:24} {t.profile:7} {loader.base[id(t)]:7.0f}/s avg  ({kind})")
    start = time.monotonic()
    try:
        loader.run()
    except KeyboardInterrupt:
        pass
    loader.stop.set()
    loader.finish(time.monotonic() - start)


if __name__ == "__main__":
    try:
        main()
    except ESError as exc:
        print(f"error: {exc}", file=sys.stderr)
        sys.exit(1)
    except KeyboardInterrupt:
        sys.exit(130)
