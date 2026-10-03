#!/usr/bin/env bash
# Helpers for resolving the api container IP on the compose project network.

cex_project_network() {
    local project="${1:-cex}"
    echo "${project}_default"
}

cex_api_ip_from_inspect_json() {
    python3 -c '
import json
import os
import sys

network = os.environ["CEX_NETWORK"]
data = json.load(sys.stdin)
ip = (data[0].get("NetworkSettings", {}).get("Networks", {}).get(network) or {}).get("IPAddress") or ""
if not ip:
    sys.stderr.write(f"no ip on network {network}\n")
    sys.exit(1)
print(ip)
'
}
