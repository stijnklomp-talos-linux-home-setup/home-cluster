#!/usr/bin/env sh
# Assert the chart copies of the gateway config and supergraph match the
# canonical files at the repository root.
#
# Helm `.Files` cannot read outside the chart directory, so
# `deploy/home-graphql/files/` holds generated copies. `scripts/compose.sh`
# writes them; this check fails if someone edited one copy by hand and the
# next `helm upgrade` would ship a different config than the repo root.
set -eu

repo_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
status=0

for pair in "gateway.config.mjs" "supergraph.graphql"; do
  if ! diff -u "$repo_dir/$pair" "$repo_dir/deploy/home-graphql/files/$pair"; then
    echo "$pair: chart copy is stale; run scripts/compose.sh" >&2
    status=1
  fi
done

if [ "$status" -eq 0 ]; then
  echo "gateway.config.mjs and supergraph.graphql are in sync with the chart copies."
fi
exit "$status"
