#!/usr/bin/env bash
# Live acceptance checks for the deploy tenant (ticket #5).
#
# Proves on the real cluster:
#   - the operator org tenant namespace exists with the deploy-tier labels,
#     quota, LimitRange and Cilium policy (incl. `kube-apiserver` egress);
#   - exactly 17 Repository CRs live in the org namespace, the build tenant
#     keeps its one, and `tekton-pipelines` holds none;
#   - the deploy secrets and the terragrunt plan PVC/CronJob live in the org
#     namespace, and the build namespace has no deploy credentials;
#   - the build tenant's workload identity cannot read the org secrets;
#   - a PipelineRun in the org namespace can reach the Kubernetes API server,
#     cluster DNS, Verdaccio and the internet under `restricted` PSA.
#
# Usage:
#   KUBECONFIG_DIR=/path/to/home-cluster-1-config \
#     ./tenant-platform/tests/deploy-tenant-checks.sh
#
# Environment:
#   KUBECONFIG_DIR    directory holding the cluster kubeconfig (required)
#   KUBECTL_IMAGE     kubectl image (default registry.k8s.io/kubectl:v1.36.4)
#   RUN_PIPELINERUN   "false" to skip the live PipelineRun (default: run it)
set -euo pipefail

COMPONENT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
KUBECTL_IMAGE="${KUBECTL_IMAGE:-registry.k8s.io/kubectl:v1.36.4}"
KUBECONFIG_DIR="${KUBECONFIG_DIR:-}"
RUN_PIPELINERUN="${RUN_PIPELINERUN:-true}"

ORG_NS="stijnklomp-talos-linux-home-setup"
ORG_OWNER="stijnklomp-talos-linux-home-setup"
BUILD_NS="stijnklomp-lave"
BUILD_OWNER="stijnklomp-LAVE"
CONTROL_NS="tekton-pipelines"
PIPELINERUN="deploy-tenant-acceptance"

EXPECTED_REPOS="
home-cluster
home-cluster-internal
"

if [ -z "${KUBECONFIG_DIR}" ]; then
  echo "KUBECONFIG_DIR must point at the directory containing the cluster kubeconfig" >&2
  exit 2
fi
if [ ! -f "${KUBECONFIG_DIR}/kubeconfig" ]; then
  echo "no kubeconfig at ${KUBECONFIG_DIR}/kubeconfig" >&2
  exit 2
fi

WORK="$(mktemp -d /tmp/tenant-platform-deploy.XXXXXX)"
trap 'rm -rf "${WORK}"' EXIT
cp "${COMPONENT_DIR}/tests/deploy-tenant-pipelinerun.yaml" "${WORK}/"

kubectl() {
  docker run --rm \
    -v "${KUBECONFIG_DIR}:/cfg:ro" \
    -v "${WORK}:/m" \
    -w /m \
    "${KUBECTL_IMAGE}" --kubeconfig /cfg/kubeconfig "$@"
}

failure=0

ok() {
  printf 'ok: %s\n' "$1"
}

bad() {
  failure=$((failure + 1))
  printf 'FAIL: %s\n' "$1" >&2
}

# eq <label> <got> <want>
eq() {
  local label="$1" got="$2" want="$3"
  if [ "${got}" = "${want}" ]; then
    ok "${label}"
  else
    bad "${label} (got '${got}', want '${want}')"
  fi
}

# has <label> <got> <substring>
has() {
  local label="$1" got="$2" want="$3"
  case "${got}" in
    *"${want}"*) ok "${label}" ;;
    *) bad "${label} (got '${got}', want substring '${want}')" ;;
  esac
}

# get <namespace> <kind> <name> <jsonpath>
get() {
  kubectl -n "$1" get "$2" "$3" -o jsonpath="$4" 2>/dev/null || true
}

repo_count() {
  kubectl get repositories.pipelinesascode.tekton.dev -n "$1" -o name 2>/dev/null \
    | grep -c . || true
}

echo "== org namespace and deploy tier objects =="
if kubectl get namespace "${ORG_NS}" >/dev/null 2>&1; then
  ok "namespace ${ORG_NS} exists"
else
  bad "namespace ${ORG_NS} does not exist (apply the tenant platform)"
fi
eq "tenant label keeps the owner verbatim" \
  "$(get "${ORG_NS}" namespace "${ORG_NS}" '{.metadata.labels.home\.cluster/tenant}')" \
  "${ORG_OWNER}"
eq "tier label is deploy" \
  "$(get "${ORG_NS}" namespace "${ORG_NS}" '{.metadata.labels.home\.cluster/tier}')" \
  "deploy"
eq "PSA enforce is restricted" \
  "$(get "${ORG_NS}" namespace "${ORG_NS}" '{.metadata.labels.pod-security\.kubernetes\.io/enforce}')" \
  "restricted"

eq "deploy quota requests.cpu" \
  "$(get "${ORG_NS}" resourcequota tenant-quota '{.spec.hard.requests\.cpu}')" "8"
eq "deploy quota pods" \
  "$(get "${ORG_NS}" resourcequota tenant-quota '{.spec.hard.pods}')" "40"
eq "deploy LimitRange default cpu" \
  "$(get "${ORG_NS}" limitrange tenant-limits '{.spec.limits[0].default.cpu}')" "4"
eq "deploy LimitRange default memory" \
  "$(get "${ORG_NS}" limitrange tenant-limits '{.spec.limits[0].default.memory}')" "4Gi"

eq "CiliumNetworkPolicy exists" \
  "$(get "${ORG_NS}" ciliumnetworkpolicy tenant-sandbox '{.metadata.name}')" \
  "tenant-sandbox"
eq "deploy CNP allows kube-apiserver (4th egress rule)" \
  "$(get "${ORG_NS}" ciliumnetworkpolicy tenant-sandbox '{.spec.egress[3].toEntities[0]}')" \
  "kube-apiserver"
eq "deploy CNP keeps world egress (3rd rule)" \
  "$(get "${ORG_NS}" ciliumnetworkpolicy tenant-sandbox '{.spec.egress[2].toEntities[0]}')" \
  "world"

eq "tekton-bot ServiceAccount exists" \
  "$(get "${ORG_NS}" serviceaccount tekton-bot '{.metadata.name}')" "tekton-bot"
eq "tekton-bot does not automount a token" \
  "$(get "${ORG_NS}" serviceaccount tekton-bot '{.automountServiceAccountToken}')" "false"
eq "npm-npmrc ConfigMap exists" \
  "$(get "${ORG_NS}" configmap npm-npmrc '{.metadata.name}')" "npm-npmrc"

echo "== Repository CR placement =="
eq "org Repository CR count" "$(repo_count "${ORG_NS}")" "17"
eq "build Repository CR count" "$(repo_count "${BUILD_NS}")" "1"
eq "control-plane Repository CR count" "$(repo_count "${CONTROL_NS}")" "0"

actual_repos="$(kubectl get repositories.pipelinesascode.tekton.dev -n "${ORG_NS}" \
  -o jsonpath='{range .items[*]}{.metadata.name}{"\n"}{end}' 2>/dev/null | sort || true)"
expected_repos="$(printf '%s\n' ${EXPECTED_REPOS} | sort)"
if [ "${actual_repos}" = "${expected_repos}" ]; then
  ok "org Repository names match the registry (17 names)"
else
  bad "org Repository names differ from the registry"
  diff <(printf '%s\n' "${expected_repos}") <(printf '%s\n' "${actual_repos}") >&2 || true
fi

mismatched=""
for name in ${EXPECTED_REPOS}; do
  url="$(get "${ORG_NS}" repository.pipelinesascode.tekton.dev "${name}" '{.spec.url}')"
  case "${url}" in
    "https://github.com/${ORG_OWNER}/${name}") ;;
    *) mismatched="${mismatched} ${name}=${url:-<missing>}" ;;
  esac
done
if [ -z "${mismatched}" ]; then
  ok "every org Repository URL matches the org owner and repo name"
else
  bad "org Repository URL mismatch:${mismatched}"
fi

# The admission gate is what let these CRs through; the build tenant CR must
# still be admitted too (unchanged).
eq "build tenant Repository URL unchanged" \
  "$(get "${BUILD_NS}" repository.pipelinesascode.tekton.dev test-app '{.spec.url}')" \
  "https://github.com/${BUILD_OWNER}/test-app"

echo "== secret placement (namespace is the boundary) =="
eq "dockerhub-docker-config exists in the org namespace" \
  "$(get "${ORG_NS}" secret dockerhub-docker-config '{.metadata.name}')" \
  "dockerhub-docker-config"
for secret in dockerhub-docker-config gitops-pat cloud-creds npm-secret; do
  eq "build namespace has no ${secret}" \
    "$(get "${BUILD_NS}" secret "${secret}" '{.metadata.name}')" ""
done
eq "build tenant SA cannot read org secrets" \
  "$(kubectl auth can-i get secrets -n "${ORG_NS}" \
      --as="system:serviceaccount:${BUILD_NS}:tekton-bot" 2>/dev/null || true)" \
  "no"

for secret in gitops-pat cloud-creds; do
  if [ -n "$(get "${ORG_NS}" secret "${secret}" '{.metadata.name}')" ]; then
    ok "out-of-band ${secret} exists in the org namespace"
  else
    printf 'note: out-of-band %s is not created yet (operator action, see README)\n' "${secret}"
  fi
done

echo "== terragrunt plan storage =="
eq "home-terraform-plans PVC is in the org namespace" \
  "$(get "${ORG_NS}" pvc home-terraform-plans '{.metadata.name}')" \
  "home-terraform-plans"
eq "home-terraform-plans PVC is Bound" \
  "$(get "${ORG_NS}" pvc home-terraform-plans '{.status.phase}')" "Bound"
eq "home-terraform-plans-cleanup CronJob is in the org namespace" \
  "$(get "${ORG_NS}" cronjob home-terraform-plans-cleanup '{.metadata.name}')" \
  "home-terraform-plans-cleanup"
eq "old PVC is gone from tekton-pipelines" \
  "$(get "${CONTROL_NS}" pvc home-terraform-plans '{.metadata.name}')" ""
eq "old cleanup CronJob is gone from tekton-pipelines" \
  "$(get "${CONTROL_NS}" cronjob home-terraform-plans-cleanup '{.metadata.name}')" ""

echo "== build tenant still isolated =="
eq "build tier label unchanged" \
  "$(get "${BUILD_NS}" namespace "${BUILD_NS}" '{.metadata.labels.home\.cluster/tier}')" \
  "build"
eq "build CNP still has no kube-apiserver allow" \
  "$(get "${BUILD_NS}" ciliumnetworkpolicy tenant-sandbox '{.spec.egress[3].toEntities[0]}')" \
  ""

if [ "${RUN_PIPELINERUN}" = "true" ]; then
  echo "== live PipelineRun in the org namespace =="
  kubectl -n "${ORG_NS}" delete pipelinerun "${PIPELINERUN}" --ignore-not-found >/dev/null 2>&1 || true
  if kubectl apply -f "/m/deploy-tenant-pipelinerun.yaml" >/dev/null 2>&1; then
    ok "PipelineRun manifest admitted"
  else
    bad "PipelineRun manifest was not admitted"
  fi
  # The manifest is mounted from the component directory below.
  if kubectl wait --for=condition=Succeeded --timeout=20m \
      -n "${ORG_NS}" "pipelinerun/${PIPELINERUN}" >/dev/null 2>&1; then
    ok "PipelineRun ${PIPELINERUN} succeeded"
  else
    bad "PipelineRun ${PIPELINERUN} did not succeed"
    kubectl -n "${ORG_NS}" describe "pipelinerun/${PIPELINERUN}" >&2 || true
  fi
  kubectl logs -n "${ORG_NS}" -l "tekton.dev/pipelineRun=${PIPELINERUN}" \
    --all-containers --prefix 2>/dev/null | sed 's/^/    /' || true
fi

echo
if [ "${failure}" -ne 0 ]; then
  printf 'FAILED: %d deploy-tenant check(s) failed\n' "${failure}" >&2
  exit 1
fi
printf 'PASSED: all deploy-tenant checks\n'
