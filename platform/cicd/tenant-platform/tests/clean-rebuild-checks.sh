#!/usr/bin/env bash
# Clean-rebuild proof for ticket #8: the registry is the source of truth.
#
# Deletes the build tenant namespace (`stijnklomp-lave`) and rebuilds it from
# Git with `helmfile -l stage=tenancy sync`, then asserts every tenant object
# came back and re-runs the committed acceptance PipelineRun.
#
# Safety:
#   - only the build tenant is deleted; the deploy tenant (org) namespace and
#     its real deploy secret/PVC are never touched;
#   - the script refuses to run unless the control-plane checks pass
#     (SKIP_CONTROL_PLANE_CHECK=true overrides, for local experiments only);
#   - namespace deletion is irreversible: any Longhorn PVC in the build
#     namespace is deleted with it. The build tenant holds no deploy data.
#
# Usage:
#   KUBECONFIG_DIR=/path/to/home-cluster-1-config \
#     ./tenant-platform/tests/clean-rebuild-checks.sh
#
# Environment:
#   KUBECONFIG_DIR           directory holding the cluster kubeconfig (required)
#   KUBECTL_IMAGE            kubectl image (default registry.k8s.io/kubectl:v1.36.4)
#   HELMFILE_IMAGE           helmfile image (default ghcr.io/helmfile/helmfile:latest)
#   SKIP_CONTROL_PLANE_CHECK "true" to skip the preflight (default: run it)
#   SKIP_ACCEPTANCE          "true" to skip the PipelineRun (default: run it)
set -euo pipefail

COMPONENT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
KUBECTL_IMAGE="${KUBECTL_IMAGE:-registry.k8s.io/kubectl:v1.36.4}"
HELMFILE_IMAGE="${HELMFILE_IMAGE:-ghcr.io/helmfile/helmfile:latest}"
KUBECONFIG_DIR="${KUBECONFIG_DIR:-}"
SKIP_CONTROL_PLANE_CHECK="${SKIP_CONTROL_PLANE_CHECK:-false}"
SKIP_ACCEPTANCE="${SKIP_ACCEPTANCE:-false}"

BUILD_NS="stijnklomp-lave"
BUILD_OWNER="stijnklomp-LAVE"
PIPELINERUN="tenant-acceptance"

if [ -z "${KUBECONFIG_DIR}" ]; then
  echo "KUBECONFIG_DIR must point at the directory containing the cluster kubeconfig" >&2
  exit 2
fi
if [ ! -f "${KUBECONFIG_DIR}/kubeconfig" ]; then
  echo "no kubeconfig at ${KUBECONFIG_DIR}/kubeconfig" >&2
  exit 2
fi

WORK="$(mktemp -d /tmp/tenant-platform-rebuild.XXXXXX)"
trap 'rm -rf "${WORK}"' EXIT
cp "${COMPONENT_DIR}/tests/build-tenant-pipelinerun.yaml" "${WORK}/"

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

# get <namespace> <kind> <name> <jsonpath>
get() {
  kubectl -n "$1" get "$2" "$3" -o jsonpath="$4" 2>/dev/null || true
}

# snapshot - every registry-managed object in the build tenant
snapshot() {
  {
    kubectl get namespace "${BUILD_NS}" -o name 2>/dev/null || true
    kubectl -n "${BUILD_NS}" get \
      serviceaccount,configmap,ciliumnetworkpolicy,resourcequota,limitrange,repository,rolebinding \
      -o name 2>/dev/null || true
    kubectl get clusterrolebinding -o name 2>/dev/null \
      | grep '^clusterrolebinding.rbac.authorization.k8s.io/tekton-namespace-lister-' || true
  } | sort
}

if [ "${SKIP_CONTROL_PLANE_CHECK}" != "true" ]; then
  echo "== preflight: control plane must be clean =="
  if ! KUBECONFIG_DIR="${KUBECONFIG_DIR}" \
      KUBECTL_IMAGE="${KUBECTL_IMAGE}" \
      "${COMPONENT_DIR}/tests/retire-single-namespace-checks.sh"; then
    bad "control plane is not clean; refusing the rebuild"
    exit 1
  fi
fi

echo
echo "== before: registry-managed objects in ${BUILD_NS} =="
snapshot | tee "${WORK}/before.txt"

echo
echo "== delete ${BUILD_NS} =="
if kubectl delete namespace "${BUILD_NS}" --wait=true --timeout=300s >/dev/null 2>&1; then
  ok "namespace delete request completed"
else
  bad "namespace delete did not complete"
fi
if kubectl get namespace "${BUILD_NS}" >/dev/null 2>&1; then
  bad "namespace ${BUILD_NS} still exists"
else
  ok "namespace ${BUILD_NS} is gone"
fi

echo
echo "== rebuild from Git: helmfile -l stage=tenancy sync =="
if docker run --rm \
    -v "${COMPONENT_DIR}:/src" \
    -v "${KUBECONFIG_DIR}:/cfg:ro" \
    -e KUBECONFIG=/cfg/kubeconfig \
    -w /src \
    "${HELMFILE_IMAGE}" helmfile -l stage=tenancy sync; then
  ok "helmfile sync completed"
else
  bad "helmfile sync failed"
  exit 1
fi

echo
echo "== after: registry-managed objects in ${BUILD_NS} =="
snapshot | tee "${WORK}/after.txt"
if diff -u "${WORK}/before.txt" "${WORK}/after.txt" >"${WORK}/diff.txt"; then
  ok "before and after object lists are identical"
else
  printf 'note: before/after differ (expected if the pre-delete namespace had run objects):\n'
  sed 's/^/    /' "${WORK}/diff.txt"
fi

echo
echo "== per-tenant objects recreated =="
eq "namespace exists" "$(get "${BUILD_NS}" namespace "${BUILD_NS}" '{.metadata.name}')" "${BUILD_NS}"
eq "tenant label" \
  "$(get "${BUILD_NS}" namespace "${BUILD_NS}" '{.metadata.labels.home\.cluster/tenant}')" "${BUILD_OWNER}"
eq "tier label" \
  "$(get "${BUILD_NS}" namespace "${BUILD_NS}" '{.metadata.labels.home\.cluster/tier}')" "build"
eq "managed label" \
  "$(get "${BUILD_NS}" namespace "${BUILD_NS}" '{.metadata.labels.home\.cluster/managed}')" "true"
eq "PSA enforce label" \
  "$(get "${BUILD_NS}" namespace "${BUILD_NS}" '{.metadata.labels.pod-security\.kubernetes\.io/enforce}')" "restricted"
eq "PSA enforce-version label" \
  "$(get "${BUILD_NS}" namespace "${BUILD_NS}" '{.metadata.labels.pod-security\.kubernetes\.io/enforce-version}')" "latest"
eq "PSA audit label" \
  "$(get "${BUILD_NS}" namespace "${BUILD_NS}" '{.metadata.labels.pod-security\.kubernetes\.io/audit}')" "restricted"
eq "PSA warn label" \
  "$(get "${BUILD_NS}" namespace "${BUILD_NS}" '{.metadata.labels.pod-security\.kubernetes\.io/warn}')" "restricted"

eq "ResourceQuota exists" "$(get "${BUILD_NS}" resourcequota tenant-quota '{.metadata.name}')" "tenant-quota"
eq "ResourceQuota pods" "$(get "${BUILD_NS}" resourcequota tenant-quota '{.spec.hard.pods}')" "20"
eq "ResourceQuota requests.cpu" "$(get "${BUILD_NS}" resourcequota tenant-quota '{.spec.hard.requests\.cpu}')" "4"
eq "LimitRange exists" "$(get "${BUILD_NS}" limitrange tenant-limits '{.metadata.name}')" "tenant-limits"
eq "LimitRange default cpu" "$(get "${BUILD_NS}" limitrange tenant-limits '{.spec.limits[0].default.cpu}')" "2"

eq "CiliumNetworkPolicy exists" \
  "$(get "${BUILD_NS}" ciliumnetworkpolicy tenant-sandbox '{.metadata.name}')" "tenant-sandbox"
eq "build CNP world egress is the third rule" \
  "$(get "${BUILD_NS}" ciliumnetworkpolicy tenant-sandbox '{.spec.egress[2].toEntities[0]}')" "world"
eq "build CNP has no fourth (API) rule" \
  "$(get "${BUILD_NS}" ciliumnetworkpolicy tenant-sandbox '{.spec.egress[3].toEntities[0]}')" ""

eq "tekton-bot ServiceAccount exists" \
  "$(get "${BUILD_NS}" serviceaccount tekton-bot '{.metadata.name}')" "tekton-bot"
eq "tekton-bot does not automount a token" \
  "$(get "${BUILD_NS}" serviceaccount tekton-bot '{.automountServiceAccountToken}')" "false"
eq "npm-npmrc ConfigMap exists" "$(get "${BUILD_NS}" configmap npm-npmrc '{.metadata.name}')" "npm-npmrc"
case "$(get "${BUILD_NS}" configmap npm-npmrc '{.data}')" in
  *"registry=http://verdaccio.verdaccio.svc.cluster.local:4873/"*)
    ok "npm-npmrc points at Verdaccio" ;;
  *)
    bad "npm-npmrc does not point at Verdaccio" ;;
esac

eq "Repository CR exists" \
  "$(get "${BUILD_NS}" repository.pipelinesascode.tekton.dev test-app '{.metadata.name}')" "test-app"
eq "Repository URL" \
  "$(get "${BUILD_NS}" repository.pipelinesascode.tekton.dev test-app '{.spec.url}')" \
  "https://github.com/${BUILD_OWNER}/test-app"
eq "Repository provenance" \
  "$(get "${BUILD_NS}" repository.pipelinesascode.tekton.dev test-app '{.spec.settings.pipelinerun_provenance}')" \
  "source"
eq "Repository concurrency_limit" \
  "$(get "${BUILD_NS}" repository.pipelinesascode.tekton.dev test-app '{.spec.concurrency_limit}')" "2"

eq "developer RoleBinding exists" \
  "$(get "${BUILD_NS}" rolebinding tekton-developer-stijnklomp1-hotmail-com '{.metadata.name}')" \
  "tekton-developer-stijnklomp1-hotmail-com"
eq "developer RoleBinding roleRef" \
  "$(get "${BUILD_NS}" rolebinding tekton-developer-stijnklomp1-hotmail-com '{.roleRef.name}')" \
  "tekton-developer"
eq "developer RoleBinding subject" \
  "$(get "${BUILD_NS}" rolebinding tekton-developer-stijnklomp1-hotmail-com '{.subjects[0].name}')" \
  "stijnklomp1@hotmail.com"
eq "namespace-lister ClusterRoleBinding" \
  "$(kubectl get clusterrolebinding tekton-namespace-lister-stijnklomp1-hotmail-com \
      -o jsonpath='{.roleRef.name}' 2>/dev/null || true)" \
  "tekton-namespace-lister"

echo "== the rebuild did not reintroduce control-plane tenant objects =="
eq "no Repository CR in tekton-pipelines" \
  "$(kubectl -n tekton-pipelines get repositories.pipelinesascode.tekton.dev -o name 2>/dev/null | grep -c . || true)" "0"
eq "no tekton-bot SA in tekton-pipelines" \
  "$(get tekton-pipelines serviceaccount tekton-bot '{.metadata.name}')" ""
eq "no npm-npmrc in tekton-pipelines" \
  "$(get tekton-pipelines configmap npm-npmrc '{.metadata.name}')" ""

if [ "${SKIP_ACCEPTANCE}" != "true" ]; then
  echo
  echo "== acceptance PipelineRun under restricted PSA =="
  kubectl -n "${BUILD_NS}" delete pipelinerun "${PIPELINERUN}" --ignore-not-found >/dev/null 2>&1 || true
  if kubectl apply -f "/m/build-tenant-pipelinerun.yaml" >/dev/null 2>&1; then
    ok "PipelineRun manifest admitted"
  else
    bad "PipelineRun manifest was not admitted"
  fi
  if kubectl wait --for=condition=Succeeded --timeout=20m \
      -n "${BUILD_NS}" "pipelinerun/${PIPELINERUN}" >/dev/null 2>&1; then
    ok "PipelineRun ${PIPELINERUN} succeeded"
  else
    bad "PipelineRun ${PIPELINERUN} did not succeed"
    kubectl -n "${BUILD_NS}" describe "pipelinerun/${PIPELINERUN}" >&2 || true
  fi
  if kubectl logs -n "${BUILD_NS}" -l "tekton.dev/pipelineRun=${PIPELINERUN}" \
      --all-containers --prefix >"${WORK}/acceptance.log" 2>&1; then
    sed 's/^/    /' "${WORK}/acceptance.log"
  fi
  if grep -q "all network assertions passed" "${WORK}/acceptance.log" 2>/dev/null; then
    ok "acceptance probe passed every network assertion"
  else
    bad "acceptance probe did not report success"
  fi
fi

echo
if [ "${failure}" -ne 0 ]; then
  printf 'FAILED: %d clean-rebuild check(s) failed\n' "${failure}" >&2
  exit 1
fi
printf 'PASSED: clean rebuild recreated the build tenant and the acceptance run passed\n'
