#!/usr/bin/env bash
set -eu

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=deploy-api-ip.sh
. "$SCRIPT_DIR/deploy-api-ip.sh"

SAMPLE='[{"NetworkSettings":{"Networks":{"cex_default":{"IPAddress":"172.22.0.6"},"bridge":{"IPAddress":"172.20.0.9"}}}}]'

result="$(CEX_NETWORK=cex_default cex_api_ip_from_inspect_json <<<"$SAMPLE")"
if [ "$result" != "172.22.0.6" ]; then
    echo "expected 172.22.0.6 on cex_default, got $result" >&2
    exit 1
fi

if CEX_NETWORK=missing cex_api_ip_from_inspect_json <<<"$SAMPLE" >/dev/null 2>&1; then
    echo "expected failure for missing network" >&2
    exit 1
fi

echo "deploy-api-ip.test.sh: ok"
