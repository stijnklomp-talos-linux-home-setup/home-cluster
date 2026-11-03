#!/usr/bin/env sh
# Compose supergraph.graphql from the pinned Rover config, then copy the
# artifacts the chart mounts into the chart's files/ directory.
#
# Everything runs through Docker. The named volume caches Rover's ELv2
# `supergraph` plugin so the run is reproducible and offline after the first
# install. Nothing is installed on the host.
set -eu

repo_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
rover_image="ghcr.io/apollographql/rover:1.0.1"
rover_home="home-graphql-rover"
# The image runs as uid 10001; run it as the invoking user instead so the
# composed artifacts in the checkout stay writable and correctly owned.
docker_user="$(id -u):$(id -g)"

cd "$repo_dir"

for schema in schemas/users.graphql schemas/orders.graphql; do
  if [ ! -f "$schema" ]; then
    echo "$schema is missing; fetch the subgraph SDL first (see README.md)" >&2
    exit 1
  fi
done

docker volume create "$rover_home" >/dev/null
# The named volume is created root-owned; hand it to the invoking user once.
docker run --rm --user 0 \
  -v "$rover_home:/home/rover" \
  --entrypoint chown \
  "$rover_image" -R "$docker_user" /home/rover

# Install the pinned ELv2 plugin once; the named volume keeps it.
if ! docker run --rm --user "$docker_user" \
  -e HOME=/home/rover \
  -v "$rover_home:/home/rover" \
  --entrypoint sh \
  "$rover_image" \
  -c 'test -x /home/rover/.rover/bin/supergraph-v2.9.3' >/dev/null 2>&1; then
  docker run --rm --user "$docker_user" \
    -e HOME=/home/rover \
    -v "$rover_home:/home/rover" \
    "$rover_image" \
    plugin install supergraph@=2.9.3 --elv2-license accept
fi

docker run --rm --user "$docker_user" \
  -e HOME=/home/rover \
  -v "$repo_dir:/src" \
  -v "$rover_home:/home/rover" \
  -w /src \
  "$rover_image" \
  supergraph compose \
  --config supergraph.yaml \
  --output supergraph.graphql \
  --elv2-license accept \
  --skip-update

# Helm's .Files can only read inside the chart directory, so keep the chart
# copies in sync with the canonical artifacts at the repository root.
cp gateway.config.mjs deploy/home-graphql/files/gateway.config.mjs
cp supergraph.graphql deploy/home-graphql/files/supergraph.graphql

echo "Composed supergraph.graphql and synced deploy/home-graphql/files/."
