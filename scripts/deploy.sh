#!/usr/bin/env bash
# Build and deploy the live cex stack from a clean worktree of origin/main.
# Run by the owner only. Never run against project cex except via --dry-run
# in development.

set -eu

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=deploy-api-ip.sh
. "$SCRIPT_DIR/deploy-api-ip.sh"

REPO="$(cd "$SCRIPT_DIR/.." && pwd)"
DEPLOY_DIR="${CEX_DEPLOY_DIR:-/srv/claude/deploy/cex}"
ENV_FILE="${CEX_ENV_FILE:-$REPO/.env}"
DRY_RUN=0

usage() {
    echo "usage: $0 [--dry-run]" >&2
    exit 1
}

while [ $# -gt 0 ]; do
    case "$1" in
        --dry-run)
            DRY_RUN=1
            ;;
        -h | --help)
            usage
            ;;
        *)
            echo "unknown argument: $1" >&2
            usage
            ;;
    esac
    shift
done

run() {
    if [ "$DRY_RUN" -eq 1 ]; then
        printf '+'
        printf ' %q' "$@"
        printf '\n'
    else
        "$@"
    fi
}

wait_for_http() {
    url="$1"
    label="$2"
    deadline=$(( $(date +%s) + 60 ))
    while [ "$(date +%s)" -lt "$deadline" ]; do
        if curl -sf -m 5 "$url" >/dev/null 2>&1; then
            return 0
        fi
        sleep 1
    done
    echo "timed out after 60s waiting for $label at $url" >&2
    return 1
}

run git -C "$REPO" fetch origin main

if [ ! -d "$DEPLOY_DIR" ]; then
    run git -C "$REPO" worktree add --detach "$DEPLOY_DIR" origin/main
else
    if [ -n "$(git -C "$DEPLOY_DIR" status --porcelain)" ]; then
        echo "deploy directory has local changes: $DEPLOY_DIR" >&2
        exit 1
    fi
    run git -C "$DEPLOY_DIR" checkout --detach origin/main
fi

if [ ! -f "$ENV_FILE" ]; then
    echo "env file not found: $ENV_FILE" >&2
    exit 1
fi

COMPOSE=(docker compose -p cex --project-directory "$DEPLOY_DIR" -f "$DEPLOY_DIR/docker-compose.yml" --env-file "$ENV_FILE")

run "${COMPOSE[@]}" build api
run "${COMPOSE[@]}" up -d --no-build postgres redis engine api ws persist

if [ "$DRY_RUN" -eq 1 ]; then
    echo "dry run: would wait for http://<cex-api container ip>:8080/health"
    echo "dry run: would wait for http://<cex-api container ip>:8080/markets"
    run git -C "$DEPLOY_DIR" rev-parse HEAD
    exit 0
fi

CEX_NETWORK="$(cex_project_network cex)"
API_IP="$(docker inspect cex-api | CEX_NETWORK="$CEX_NETWORK" cex_api_ip_from_inspect_json)" || {
    echo "could not resolve cex-api container ip on network $CEX_NETWORK" >&2
    exit 1
}

wait_for_http "http://${API_IP}:8080/health" "GET /health"
wait_for_http "http://${API_IP}:8080/markets" "GET /markets"

git -C "$DEPLOY_DIR" rev-parse HEAD
