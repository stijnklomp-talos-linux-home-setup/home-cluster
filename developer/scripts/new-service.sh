#!/usr/bin/env bash
# Scaffold a new service:
#   1. declaration: developer/apps/<cluster>/<app>.yaml (generates the
#      <app>-dev / -staging / -prod Applications on the next ArgoCD poll)
#   2. optional: the app-repo .tekton/ PipelineRuns (ci, cd-dev, cd-staging)
#      copied from developer/pipelines usage examples
#
# Usage:
#   developer/scripts/new-service.sh --app test-app \
#     --repo https://github.com/stijnklomp-LAVE/test-app \
#     --image docker.io/stijnklomp/test-app \
#     [--chart-path deploy/test-app] [--cluster baremetal|eks] \
#     [--app-repo-dir /path/to/app-repo]
#
# After running: commit+push home-cluster, then merge to main
# for dev, push a v* tag for staging, and sync prod manually in ArgoCD.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
EXAMPLES="$ROOT/developer/pipelines/charts/app-release/usage-examples/v1"

APP=""
REPO_URL=""
IMAGE=""
CLUSTER="baremetal"
CHART_PATH=""
APP_REPO_DIR=""

usage() {
  sed -n '2,16p' "$0" | sed 's/^# \{0,1\}//'
  exit 1
}

while [ $# -gt 0 ]; do
  case "$1" in
    --app) APP="$2"; shift 2 ;;
    --repo) REPO_URL="$2"; shift 2 ;;
    --image) IMAGE="$2"; shift 2 ;;
    --cluster) CLUSTER="$2"; shift 2 ;;
    --chart-path) CHART_PATH="$2"; shift 2 ;;
    --app-repo-dir) APP_REPO_DIR="$2"; shift 2 ;;
    -h|--help) usage ;;
    *) echo "unknown argument: $1" >&2; usage ;;
  esac
done

[ -n "$APP" ] || { echo "ERROR: --app is required" >&2; usage; }
[ -n "$REPO_URL" ] || { echo "ERROR: --repo is required" >&2; usage; }
[ -n "$IMAGE" ] || { echo "ERROR: --image is required (e.g. docker.io/<user>/<app>)" >&2; usage; }
case "$CLUSTER" in
  baremetal|eks) ;;
  *) echo "ERROR: --cluster must be baremetal or eks" >&2; exit 1 ;;
esac
CHART_PATH="${CHART_PATH:-deploy/$APP}"

DECL="$ROOT/developer/apps/$CLUSTER/$APP.yaml"
if [ -e "$DECL" ]; then
  echo "ERROR: $DECL already exists" >&2
  exit 1
fi

mkdir -p "$(dirname "$DECL")"
sed -e "s|<APP>|$APP|g" \
    -e "s|<CLUSTER>|$CLUSTER|g" \
    -e "s|<REPO_URL>|$REPO_URL|g" \
    -e "s|<CHART_PATH>|$CHART_PATH|g" \
    -e "s|<IMAGE_REPOSITORY>|$IMAGE|g" \
    "$ROOT/platform/gitops/templates/service-declaration.yaml.tpl" > "$DECL"
echo "created $DECL"

if [ "$CLUSTER" = "eks" ]; then
  echo "NOTE: EKS target — register the cluster first: argocd cluster add <context> --name eks"
fi

if [ -n "$APP_REPO_DIR" ]; then
  if [ ! -d "$EXAMPLES" ]; then
    echo "WARN: usage examples not found at $EXAMPLES" >&2
    echo "      copy the app-release usage examples into $APP_REPO_DIR/.tekton/ manually" >&2
  else
    mkdir -p "$APP_REPO_DIR/.tekton"
    for pair in "ci-pull-request-pipelinerun.yaml:ci.yaml" \
                "cd-dev-pipelinerun.yaml:cd-dev.yaml" \
                "cd-staging-pipelinerun.yaml:cd-staging.yaml"; do
      src="$EXAMPLES/${pair%%:*}"
      dst="$APP_REPO_DIR/.tekton/${pair##*:}"
      if [ -e "$dst" ]; then
        echo "WARN: $dst exists, skipping" >&2
        continue
      fi
      sed -e "s|docker.io/<DOCKERHUB_USERNAME>/<APP>|$IMAGE|g" \
          -e "s|developer/apps/baremetal/<APP>.yaml|developer/apps/$CLUSTER/$APP.yaml|g" \
          -e '/^# Copy to <app-repo>\/\.tekton\//d' \
          -e '/TODO: replace <DOCKERHUB_USERNAME> and <APP>/d' \
          -e '/TODO: replace <APP>/d' \
          "$src" > "$dst"
      echo "created $dst"
    done
  fi
fi

cat <<EOF

Next steps:
  1. home-cluster: commit + push $DECL
     ArgoCD generates $APP-dev, $APP-staging, $APP-prod and auto-syncs dev/staging.
  2. App repo: commit + push the .tekton/ PipelineRuns (if copied).
     Merge to main -> dev build + imageTagDev bump.
     Push a v* tag on main -> staging build + staging/prod tag record.
  3. ArgoCD: click Sync on $APP-prod when you are ready to release (manual gate).
  4. In the monorepo every app shares one .tekton/ directory, so add the
     on-path-change annotation (pipelinesascode.tekton.dev/on-path-change)
     with your app directory to each PipelineRun, or every push would run
     every app's pipeline.
  5. Prerequisites (once, per cluster): the gitops-pat secret (contents:write
     on home-cluster) and ArgoCD repo credentials must be in place.
EOF
