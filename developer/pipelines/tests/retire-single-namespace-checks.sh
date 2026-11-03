#!/usr/bin/env bash
# Offline checks for ticket #8 in the shared Tekton catalog.
#
# Renders every chart and asserts the catalog no longer owns tenant objects:
#   - `charts/tekton-bot/` and its helmfile release are gone;
#   - no chart renders a `tekton-bot` ServiceAccount or Role/RoleBinding;
#   - no chart renders an `npm-npmrc` ConfigMap (tenant-platform owns it);
#   - `values/platform.yaml` has no `namespaces` key — only the deploy tenant
#     `namespace` remains;
#   - the shared Tasks/Pipelines still render and `helm lint` passes;
#   - the deploy-tenant resources still land in the deploy namespace.
#
# Usage (Docker-first, nothing installed on the host):
#   ./tests/retire-single-namespace-checks.sh
#
# Environment:
#   HELM_IMAGE      helm image (default alpine/helm:latest)
#   HELMFILE_IMAGE  helmfile image (default ghcr.io/helmfile/helmfile:latest)
set -euo pipefail

REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
HELM_IMAGE="${HELM_IMAGE:-alpine/helm:latest}"
HELMFILE_IMAGE="${HELMFILE_IMAGE:-ghcr.io/helmfile/helmfile:latest}"

WORK="$(mktemp -d /tmp/cicd-tekton-retire.XXXXXX)"
trap 'rm -rf "${WORK}"' EXIT

failure=0

ok() {
  printf 'ok: %s\n' "$1"
}

bad() {
  failure=$((failure + 1))
  printf 'FAIL: %s\n' "$1" >&2
}

render() {
  local chart="$1" release="$2"
  shift 2
  docker run --rm \
    -v "${REPO_DIR}:/src:ro" \
    -w /src \
    "${HELM_IMAGE}" template "${release}" "./charts/${chart}" "$@" \
    >"${WORK}/${chart}.yaml"
}

# yq_query <expression> <file-glob> — run yq inside the helmfile image
yq_query() {
  local expr="$1" files="$2"
  docker run --rm -i -v "${WORK}:/work:ro" -w /work \
    --entrypoint /bin/sh "${HELMFILE_IMAGE}" -c 'yq ea -r "$1" $2' sh "${expr}" "${files}"
}

# chart_values <chart> — secrets charts require the deploy namespace
chart_values() {
  case "$1" in
    *-secrets|terragrunt-secrets) printf '%s\n' --values values/platform.yaml ;;
  esac
}

echo "== the old single-namespace chart and release are gone =="
if [ -e "${REPO_DIR}/charts/tekton-bot" ]; then
  bad "charts/tekton-bot still exists"
else
  ok "charts/tekton-bot is deleted"
fi
if grep -qE 'name: cicd-tekton-tekton-bot|chart: ./charts/tekton-bot|stage: "tekton-bot"' "${REPO_DIR}/helmfile.yaml"; then
  bad "helmfile.yaml still has a tekton-bot release/stage"
else
  ok "helmfile.yaml has no tekton-bot release"
fi

echo "== values/platform.yaml carries only the deploy tenant namespace =="
if docker run --rm -i -v "${REPO_DIR}:/src:ro" -w /src \
    --entrypoint /bin/sh "${HELMFILE_IMAGE}" -c \
    'yq -e ".namespaces" values/platform.yaml >/dev/null 2>&1'; then
  bad "values/platform.yaml still has a namespaces key"
else
  ok "values/platform.yaml has no namespaces key"
fi
if docker run --rm -i -v "${REPO_DIR}:/src:ro" -w /src \
    --entrypoint /bin/sh "${HELMFILE_IMAGE}" -c \
    'test "$(yq -r .namespace values/platform.yaml)" = "stijnklomp-talos-linux-home-setup"'; then
  ok "values/platform.yaml still names the deploy tenant namespace"
else
  bad "values/platform.yaml deploy namespace is missing or wrong"
fi

echo "== render every chart =="
CHARTS="app-release docker-build-publish git-clone gitops-yaml-bump npm-install npm-install-secrets docker-build-publish-secrets gitops-yaml-bump-secrets supergraph-release terragrunt terragrunt-secrets"
for chart in ${CHARTS}; do
  # shellcheck disable=SC2046
  if render "${chart}" "${chart}" $(chart_values "${chart}") 2>"${WORK}/${chart}.err"; then
    ok "render ${chart}"
  else
    bad "render ${chart}"
    sed 's/^/    /' "${WORK}/${chart}.err" >&2
  fi
done

echo "== the catalog owns no tenant objects =="
if [ -n "$(yq_query 'select(.kind == "ConfigMap" and .metadata.name == "npm-npmrc") | .metadata.namespace' '*.yaml' 2>/dev/null)" ]; then
  bad "a shared chart still renders npm-npmrc"
else
  ok "no shared chart renders npm-npmrc"
fi
unexpected_sas="$(yq_query 'select(.kind == "ServiceAccount") | select(.metadata.name != "supergraph-release") | .metadata.name' '*.yaml' 2>/dev/null)"
if [ -n "${unexpected_sas}" ]; then
  bad "a shared chart renders a per-tenant ServiceAccount: ${unexpected_sas}"
else
  ok "the only rendered ServiceAccount is supergraph-release (home-graphql RBAC)"
fi
if [ "$(yq_query 'select(.kind == "ServiceAccount") | .metadata.namespace' 'supergraph-release.yaml' 2>/dev/null)" = "home-graphql" ]; then
  ok "supergraph-release SA is in the gateway namespace, not a tenant"
else
  bad "supergraph-release SA is not in home-graphql"
fi
if [ -n "$(yq_query 'select(.metadata.name == "tekton-bot") | .kind' '*.yaml' 2>/dev/null)" ]; then
  bad "a shared chart still renders a tekton-bot object"
else
  ok "no shared chart renders a tekton-bot object"
fi

echo "== the shared Tasks/Pipelines still render =="
for task in git-clone-v1 npm-install-v1 build-push-v1 kaniko-v1 gitops-yaml-bump-v1 supergraph-compose-v1 supergraph-configmap-v1 terragrunt-plan-v1 terragrunt-apply-destroy-v1; do
  if grep -q "name: ${task}$" "${WORK}"/*.yaml 2>/dev/null; then
    ok "Task ${task}"
  else
    bad "Task ${task} missing from the rendered catalog"
  fi
done
for pipeline in app-release-v1 supergraph-release-v1 terragrunt-plan-v1 terragrunt-apply-destroy-v1; do
  if grep -q "name: ${pipeline}$" "${WORK}"/*.yaml 2>/dev/null; then
    ok "Pipeline ${pipeline}"
  else
    bad "Pipeline ${pipeline} missing from the rendered catalog"
  fi
done

echo "== deploy-tenant resources stay in the deploy namespace =="
if [ "$(yq_query 'select(.kind == "PersistentVolumeClaim") | .metadata.namespace' 'terragrunt-secrets.yaml' 2>/dev/null)" = "stijnklomp-talos-linux-home-setup" ]; then
  ok "home-terraform-plans PVC namespace"
else
  bad "home-terraform-plans PVC is not in the deploy tenant namespace"
fi
if [ "$(yq_query 'select(.kind == "CronJob") | .metadata.namespace' 'terragrunt-secrets.yaml' 2>/dev/null)" = "stijnklomp-talos-linux-home-setup" ]; then
  ok "home-terraform-plans-cleanup CronJob namespace"
else
  bad "cleanup CronJob is not in the deploy tenant namespace"
fi

echo "== helm lint =="
for chart in ${CHARTS}; do
  # shellcheck disable=SC2046
  if docker run --rm -v "${REPO_DIR}:/src:ro" -w /src \
      "${HELM_IMAGE}" lint "./charts/${chart}" $(chart_values "${chart}") \
      >/dev/null 2>"${WORK}/${chart}.lint"; then
    ok "lint ${chart}"
  else
    bad "lint ${chart}"
    sed 's/^/    /' "${WORK}/${chart}.lint" >&2
  fi
done

echo "== helmfile template still resolves every release =="
if docker run --rm \
    -v "${REPO_DIR}:/src:ro" -w /src \
    "${HELMFILE_IMAGE}" helmfile template --values values/platform.yaml \
    >"${WORK}/helmfile.yaml" 2>"${WORK}/helmfile.err"; then
  ok "helmfile template"
else
  bad "helmfile template"
  sed 's/^/    /' "${WORK}/helmfile.err" >&2
fi
if [ -n "$(yq_query 'select(.kind == "ConfigMap" and .metadata.name == "npm-npmrc") | .metadata.namespace' 'helmfile.yaml' 2>/dev/null)" ]; then
  bad "helmfile template still renders npm-npmrc"
else
  ok "helmfile template renders no npm-npmrc"
fi
unexpected_sas="$(yq_query 'select(.kind == "ServiceAccount") | select(.metadata.name != "supergraph-release") | .metadata.name' 'helmfile.yaml' 2>/dev/null)"
if [ -n "${unexpected_sas}" ]; then
  bad "helmfile template renders a per-tenant ServiceAccount: ${unexpected_sas}"
else
  ok "helmfile template renders no per-tenant ServiceAccount"
fi

echo
if [ "${failure}" -ne 0 ]; then
  printf 'FAILED: %d shared-catalog retire check(s) failed\n' "${failure}" >&2
  exit 1
fi
printf 'PASSED: shared catalog owns no tenant objects\n'
