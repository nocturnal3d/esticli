# Local Elasticsearch test environment

Docker Compose profiles plus a few stdlib-only Python scripts for exercising
esticli against real clusters: every panel and popup, every health colour, each
auth mode, three major versions, path-prefixed and slow networks, and clusters
from a handful of indices to tens of thousands.

Nothing here is needed to build or unit-test esticli (`cargo test` never
touches it). It needs Docker with Compose v2, Python 3.9+ and, for the TUI
checks, tmux.

## Quick start

```bash
docker/esenv.sh up single              # ES 9.5.5 on :9200, waits for yellow/green
docker/esenv.sh seed single            # fixtures: indices, data streams, odd names…
docker/esenv.sh load single --churn &  # sustained ingest so rates aren't all 0
docker/esenv.sh esticli single         # builds target/release/esticli if needed
```

`docker/esenv.sh` with no arguments lists every command. It fills in URLs,
credentials and the CA file for whichever profile you name, so
`docker/esenv.sh cmd secure` prints the exact esticli command line for it.

## Profiles

| Profile   | What                                         | URL                          | Auth                          | Heap (default) |
|-----------|----------------------------------------------|------------------------------|-------------------------------|----------------|
| `single`  | one ES 9.5.5 node                            | `http://localhost:9200`      | none                          | 2g (4g cap)    |
| `cluster` | three nodes `es01`–`es03`                    | `http://localhost:9201`      | none                          | 1g each        |
| `secure`  | one node, HTTPS + security                   | `https://localhost:9202`     | `elastic` / `esticli-dev`, CA in `docker/certs/ca.crt` | 1g |
| `legacy`  | one ES 7.17.28 node                          | `http://localhost:9207`      | none                          | 1g             |
| `proxy`   | nginx serving `single` under a path prefix   | `http://localhost:9280/es`   | none                          | –              |
|           | toxiproxy in front of `single`               | `http://localhost:9290` (API `:8474`) | none                 | –              |

All ports bind to `127.0.0.1` only. `ES_VERSION` / `LEGACY_ES_VERSION` pick
other image tags (e.g. `ES_VERSION=8.19.23 docker/esenv.sh up single`); data
volumes are named per version, so switching never starts an old node on newer
data. Every tunable is listed at the top of `compose.yml` and can go in
`docker/.env` (gitignored).

**Memory.** Docker Desktop's VM defaults to 8 GB. `single` alone, or
`single` + `proxy` + `secure` or `legacy`, fit; `cluster` needs ~6 GB on its
own, so stop the others first (`docker/esenv.sh down all`). Scale tiers above
~10k indices want `ES_HEAP=4g ES_MEM_LIMIT=6g` and nothing else running.

## Pointing esticli at each profile

```bash
target/release/esticli -u http://localhost:9200                    # single
target/release/esticli -u http://localhost:9201                    # cluster (press n)
target/release/esticli -u http://localhost:9207                    # legacy 7.17
target/release/esticli -u http://localhost:9280/es                 # path prefix
target/release/esticli -u http://localhost:9290                    # through toxiproxy

# secure: basic auth, API key, or skipping verification
ESTICLI_PASSWORD=esticli-dev target/release/esticli -u https://localhost:9202 \
    --username elastic --ca-cert docker/certs/ca.crt
ESTICLI_API_KEY="$(docker/esenv.sh apikey | tail -1)" \
    target/release/esticli -u https://localhost:9202 --ca-cert docker/certs/ca.crt
ESTICLI_PASSWORD=esticli-dev target/release/esticli -u https://localhost:9202 --username elastic -k
```

### The macOS certificate policy

On macOS, esticli's TLS verifier applies Apple's server-certificate policy even
to a CA passed with `--ca-cert`: a certificate valid for more than 825 days, or
without the `serverAuth` extended key usage, is rejected. Certificates made with
`elasticsearch-certutil`'s defaults — what many self-managed clusters serve —
fail both checks; `curl` and Python accept them. `secure` therefore serves a
compliant certificate by default, and `ES_SECURE_CERT=certutil` switches it to
the certutil default to reproduce the failure. The CA is the same either way.

## What `seed.py` creates

Everything is named `esticli-*`, so `seed.py --reset` (or `--reset-only`)
removes exactly what it made:

- **Load targets:** `esticli-steady`, `-bursty`, `-sine`, `-ramp`, `-churn`
- **Sizes:** `esticli-idle`, `-empty`, `-small`, `-medium`, `-large`, `-multishard`
- **Data streams:** `esticli-logs` (ILM, rolls over every few thousand docs under
  load, so `.ds-*` backing indices keep appearing) and `esticli-metrics`
  (data stream lifecycle, 8.11+)
- **Health:** `esticli-yellow` (an unassignable replica); `--with-red` adds an
  index whose primary can't allocate, turning the whole cluster red
- **States:** `esticli-closed`, the hidden `.esticli-system`, and an alias
- **Awkward names:** every character class Elasticsearch allows that is hostile
  in a URL path (`+ & % = ; @ ! $ ( ) [ ] { } ~ ^ ' \``, `..`, non-ASCII, emoji,
  and a 255-byte name)

`load.py` writes to the load targets with per-index profiles (steady, bursty,
sine, ramp, idle). `--churn` also deletes documents, deletes and recreates an
index under the same name (a counter reset), and forces rollovers.

## Performance

```bash
docker/esenv.sh scale single --count 10000         # esticli-scale-00000…09999, ~300/s
docker/esenv.sh measure single > tick.json         # latency + bytes of each per-tick request
docker/esenv.sh esticli single                     # watch the fetch time in the panel title
docker/esenv.sh scale single --delete
```

`measure` times exactly what esticli requests each refresh (median/p95, with
and without gzip, and the joined wall time like `EsClient::fetch_tick`). Rough
tiers on a laptop: 100 and 1k indices need nothing special; 10k wants
`ES_HEAP=4g`; 50k needs the `ES_MAX_SHARDS_PER_NODE` default (30000) raised and
most of the VM. For render-side numbers without a cluster there is also
`cargo test --release perf -- --ignored --nocapture`.

## Fault injection

With the `proxy` profile up, point esticli at `http://localhost:9290` and change
the network under it:

```bash
docker/scripts/toxics.py latency 1500      # slow cluster: watch the fetch time
docker/scripts/toxics.py timeout 0         # hang every request (30s client timeout)
docker/scripts/toxics.py reset-peer        # connections reset
docker/scripts/toxics.py down              # refused; `up` restores
docker/scripts/toxics.py clear
```

## Cleanup

```bash
docker/esenv.sh down all        # stop everything, keep the data volumes
docker/esenv.sh wipe all        # …and delete the volumes (asks first)
```
