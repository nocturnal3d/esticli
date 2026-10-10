#!/usr/bin/env bash
# One-command flows for the EstiCLI Elasticsearch test environment.
# Works with macOS's stock bash 3.2. Run with no arguments for help.
set -euo pipefail

DOCKER_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(cd "$DOCKER_DIR/.." && pwd)"
SCRIPTS="$DOCKER_DIR/scripts"

# docker/.env feeds Compose; read it here too so volume names, passwords and
# versions agree with what Compose uses.
if [ -f "$DOCKER_DIR/.env" ]; then
  set -a
  # shellcheck disable=SC1091
  . "$DOCKER_DIR/.env"
  set +a
fi
ES_VERSION="${ES_VERSION:-9.5.5}"
LEGACY_ES_VERSION="${LEGACY_ES_VERSION:-7.17.28}"
ELASTIC_PASSWORD="${ELASTIC_PASSWORD:-esticli-dev}"
CA_CERT="$DOCKER_DIR/certs/ca.crt"
ALL_PROFILES=(--profile single --profile cluster --profile secure --profile legacy --profile proxy)

usage() {
  cat <<EOF
usage: docker/esenv.sh <command> [args]

environment
  up <profile>...            start profile(s), wait until each answers >= yellow
  down [<profile>...|all]    stop and remove containers (data volumes are kept)
  wipe <profile>...|all      down + delete the data volumes (asks first; -y skips)
  status                     containers, cluster health and memory use
  logs <profile> [-f]        container logs

data (connection flags are filled in for the profile)
  seed <target> [args]       docker/scripts/seed.py    (e.g. seed single --reset --with-red)
  load <target> [args]       docker/scripts/load.py    (e.g. load single --rate 5000 --churn)
  scale <target> [args]      docker/scripts/scale.py   (e.g. scale single --count 10000)
  measure <target> [args]    scale.py --measure        (JSON on stdout)
  apikey [args]              docker/scripts/apikey.py against "secure", prints the encoded key
  toxics [args]              docker/scripts/toxics.py  (e.g. toxics latency 1500, toxics clear)

esticli
  esticli <target> [args]    run target/release/esticli against the target (builds it if missing)
  cmd <target>               print that esticli command line instead of running it
  curl <target> <path> [curl args]   curl with the target's URL, auth and CA
  env <target>               print ES_* exports for the scripts: eval "\$(docker/esenv.sh env secure)"

profiles: single (9200)  cluster (9201)  secure (9202, TLS+auth)  legacy (9207)  proxy (nginx 9280 + toxiproxy 9290)
targets:  single cluster secure legacy  proxy (http://localhost:9280/es)  toxi (http://localhost:9290)
          secure-apikey (secure, authenticating with a fresh API key)
EOF
}

die() { echo "esenv: $*" >&2; exit 1; }

compose() { docker compose -f "$DOCKER_DIR/compose.yml" "$@"; }

# ------------------------------------------------------------------ targets
# Sets URL, CURL_AUTH (array), SCRIPT_ENV (array of VAR=value) and
# ESTICLI_ARGS (array) for a target name.
resolve_target() {
  CURL_AUTH=()
  # Blank first, so ES_* variables exported in the caller's shell can't leak
  # credentials meant for another profile into the scripts.
  SCRIPT_ENV=(ES_USER= ES_PASSWORD= ES_API_KEY= ES_CA_CERT= ES_INSECURE=)
  ESTICLI_ARGS=()
  case "$1" in
    single)  URL=http://localhost:9200 ;;
    cluster) URL=http://localhost:9201 ;;
    legacy)  URL=http://localhost:9207 ;;
    proxy)   URL=http://localhost:9280/es ;;
    toxi|toxiproxy) URL=http://localhost:9290 ;;
    secure)
      URL=https://localhost:9202
      CURL_AUTH=(--cacert "$CA_CERT" -u "elastic:$ELASTIC_PASSWORD")
      SCRIPT_ENV=(ES_USER=elastic "ES_PASSWORD=$ELASTIC_PASSWORD" "ES_CA_CERT=$CA_CERT")
      ESTICLI_ARGS=(--ca-cert "$CA_CERT" --username elastic --password "$ELASTIC_PASSWORD")
      ;;
    secure-apikey)
      URL=https://localhost:9202
      local key
      key="$(env ES_URL="$URL" ES_USER=elastic "ES_PASSWORD=$ELASTIC_PASSWORD" "ES_CA_CERT=$CA_CERT" \
             python3 "$SCRIPTS/apikey.py" --name esticli-esenv --expiration 1d 2>/dev/null)" \
        || die "could not create an API key on secure (is it up?)"
      CURL_AUTH=(--cacert "$CA_CERT" -H "Authorization: ApiKey $key")
      SCRIPT_ENV=("ES_API_KEY=$key" "ES_CA_CERT=$CA_CERT")
      ESTICLI_ARGS=(--ca-cert "$CA_CERT" --api-key "$key")
      ;;
    *) die "unknown target '$1' (single, cluster, secure, secure-apikey, legacy, proxy, toxi)" ;;
  esac
  SCRIPT_ENV+=("ES_URL=$URL")
  ESTICLI_ARGS=(-u "$URL" "${ESTICLI_ARGS[@]+"${ESTICLI_ARGS[@]}"}")
}

profile_services() {
  case "$1" in
    single)  echo "es-single nginx toxiproxy" ;;  # the proxies can't outlive their upstream
    cluster) echo "es01 es02 es03" ;;
    secure)  echo "es-secure secure-setup" ;;
    legacy)  echo "es-legacy" ;;
    proxy)   echo "nginx toxiproxy" ;;
    *) die "unknown profile '$1' (single, cluster, secure, legacy, proxy)" ;;
  esac
}

profile_volumes() {
  case "$1" in
    single)  echo "esticli_single-data-$ES_VERSION" ;;
    cluster) echo "esticli_cluster-data-01-$ES_VERSION esticli_cluster-data-02-$ES_VERSION esticli_cluster-data-03-$ES_VERSION" ;;
    secure)  echo "esticli_secure-data-$ES_VERSION esticli_secure-certs" ;;
    legacy)  echo "esticli_legacy-data-$LEGACY_ES_VERSION" ;;
    proxy)   echo "" ;;
  esac
}

# Polls _cluster/health until the target answers with yellow or green.
# A cluster that answers but stays red (seed.py --with-red) is reported, not fatal.
wait_ready() {
  local target="$1" expect_nodes="${2:-1}" deadline answered_at="" body status nodes
  resolve_target "$target"
  deadline=$((SECONDS + 300))
  printf 'waiting for %s (%s) ' "$target" "$URL"
  while [ $SECONDS -lt $deadline ]; do
    # An error body (401, 503 "master_not_discovered") also carries a numeric
    # "status", so only a body with cluster_name counts as an answer.
    if body="$(curl -sS --max-time 10 "${CURL_AUTH[@]+"${CURL_AUTH[@]}"}" \
                "$URL/_cluster/health?wait_for_status=yellow&timeout=3s" 2>/dev/null)" \
       && read -r status nodes < <(printf '%s' "$body" | python3 -c \
            'import json,sys; h=json.load(sys.stdin); print(h["status"], h["number_of_nodes"]) if "cluster_name" in h else sys.exit(1)' \
            2>/dev/null); then
      answered_at="${answered_at:-$SECONDS}"
      if [ "$nodes" -ge "$expect_nodes" ] && [ "$status" != red ]; then
        echo " $status, $nodes node(s)"
        return 0
      fi
      if [ "$status" = red ] && [ $((SECONDS - answered_at)) -gt 60 ]; then
        echo " RED after 60s ($nodes node(s)); continuing (esticli-red fixture?)"
        return 0
      fi
    fi
    printf '.'
    sleep 2
  done
  echo " timed out"
  return 1
}

# ----------------------------------------------------------------- commands
cmd_up() {
  [ $# -gt 0 ] || die "up needs a profile: single cluster secure legacy proxy"
  local p args=()
  for p in "$@"; do profile_services "$p" >/dev/null; args+=(--profile "$p"); done
  compose "${args[@]}" up -d
  for p in "$@"; do
    case "$p" in
      cluster) wait_ready cluster 3 ;;
      proxy)   wait_ready single; wait_ready proxy; wait_ready toxi ;;
      *)       wait_ready "$p" ;;
    esac
  done
}

cmd_down() {
  if [ $# -eq 0 ] || [ "$1" = all ]; then
    compose "${ALL_PROFILES[@]}" down
    return
  fi
  local p services=""
  for p in "$@"; do services="$services $(profile_services "$p")"; done
  # shellcheck disable=SC2086
  compose "${ALL_PROFILES[@]}" rm --stop --force $services
}

cmd_wipe() {
  local yes=0 profiles=() p volumes=""
  for p in "$@"; do
    case "$p" in
      -y|--yes) yes=1 ;;
      all) profiles+=(single cluster secure legacy) ;;
      *) profiles+=("$p") ;;
    esac
  done
  [ ${#profiles[@]} -gt 0 ] || die "wipe needs a profile (or all)"
  for p in "${profiles[@]}"; do volumes="$volumes $(profile_volumes "$p")"; done
  echo "this deletes containers and volumes:$volumes"
  if [ $yes -ne 1 ]; then
    read -r -p "continue? [y/N] " answer
    case "$answer" in y|Y|yes) ;; *) die "aborted" ;; esac
  fi
  cmd_down "${profiles[@]}"
  # shellcheck disable=SC2086
  docker volume rm $volumes 2>/dev/null || true
  for p in "${profiles[@]}"; do
    [ "$p" = secure ] && rm -f "$CA_CERT"
  done
  echo "wiped:$volumes"
}

cmd_status() {
  compose "${ALL_PROFILES[@]}" ps --format 'table {{.Service}}\t{{.Status}}\t{{.Ports}}'
  echo
  local t
  for t in single cluster secure legacy proxy toxi; do
    resolve_target "$t"
    local body
    if body="$(curl -sS --max-time 3 "${CURL_AUTH[@]+"${CURL_AUTH[@]}"}" "$URL/_cluster/health" 2>/dev/null)" \
       && printf '%s' "$body" | python3 -c 'import json,sys; sys.exit("cluster_name" not in json.load(sys.stdin))' >/dev/null 2>&1; then
      printf '%-8s %-28s ' "$t" "$URL"
      printf '%s' "$body" | python3 -c 'import json,sys; h=json.load(sys.stdin); print("%-7s nodes=%s shards=%s unassigned=%s" % (h["status"], h["number_of_nodes"], h["active_shards"], h["unassigned_shards"]))'
    fi
  done
  echo
  local ids
  ids="$(compose "${ALL_PROFILES[@]}" ps -q)"
  if [ -n "$ids" ]; then
    # shellcheck disable=SC2086
    docker stats --no-stream --format 'table {{.Name}}\t{{.MemUsage}}\t{{.CPUPerc}}' $ids
  fi
}

run_script() {
  local target="$1" script="$2"
  shift 2
  resolve_target "$target"
  exec env "${SCRIPT_ENV[@]}" python3 "$SCRIPTS/$script" "$@"
}

esticli_bin() {
  local bin="$REPO_DIR/target/release/esticli"
  if [ ! -x "$bin" ]; then
    echo "building esticli (cargo build --release) ..." >&2
    (cd "$REPO_DIR" && cargo build --release >&2)
  fi
  echo "$bin"
}

quote_cmd() {
  local out="" a
  for a in "$@"; do out="$out $(printf '%q' "$a")"; done
  echo "${out# }"
}

main() {
  [ $# -gt 0 ] || { usage; exit 0; }
  local cmd="$1"
  shift
  case "$cmd" in
    up)      cmd_up "$@" ;;
    down)    cmd_down "$@" ;;
    wipe)    cmd_wipe "$@" ;;
    status)  cmd_status ;;
    logs)
      [ $# -gt 0 ] || die "logs needs a profile"
      local p="$1"; shift
      # shellcheck disable=SC2046
      compose "${ALL_PROFILES[@]}" logs "$@" $(profile_services "$p") ;;
    seed|load|scale)
      [ $# -gt 0 ] || die "$cmd needs a target"
      local t="$1"; shift
      run_script "$t" "$cmd.py" "$@" ;;
    measure)
      [ $# -gt 0 ] || die "measure needs a target"
      local t="$1"; shift
      run_script "$t" scale.py --measure "$@" ;;
    apikey)  run_script secure apikey.py "$@" ;;
    toxics)  exec python3 "$SCRIPTS/toxics.py" "$@" ;;
    esticli|cmd)
      [ $# -gt 0 ] || die "$cmd needs a target"
      local t="$1"; shift
      resolve_target "$t"
      if [ "$cmd" = cmd ]; then
        quote_cmd "$REPO_DIR/target/release/esticli" "${ESTICLI_ARGS[@]}" "$@"
      else
        local bin
        bin="$(esticli_bin)"
        exec "$bin" "${ESTICLI_ARGS[@]}" "$@"
      fi ;;
    curl)
      [ $# -ge 2 ] || die "usage: curl <target> <path> [curl args]"
      local t="$1" path="$2"; shift 2
      resolve_target "$t"
      exec curl -sS "${CURL_AUTH[@]+"${CURL_AUTH[@]}"}" "$@" "$URL/${path#/}" ;;
    env)
      [ $# -gt 0 ] || die "env needs a target"
      resolve_target "$1"
      local kv
      for kv in "${SCRIPT_ENV[@]}"; do echo "export $(printf '%q' "$kv")"; done ;;
    -h|--help|help) usage ;;
    *) usage >&2; exit 2 ;;
  esac
}

main "$@"
