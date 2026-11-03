#!/usr/bin/env bash
# verify-ambient.sh — prove ambient mTLS works in the sandbox namespace.
#
# This is the acceptance test for ticket #10. It asserts all of:
#   1. istiod, ztunnel and the Istio CNI node agent are ready.
#   2. `istioctl analyze -A` reports no issues.
#   3. An in-mesh client reaches the in-mesh HTTP server (200).
#   4. The same request from an unlabeled namespace (plaintext) is denied.
#   5. ztunnel lists the server as an HBONE workload, i.e. the successful
#      request was tunnelled over mTLS rather than plaintext.
#
# The manifests live in kustomizations/ambient-sandbox and are applied by
# `helmfile apply` in this directory (release `ambient-sandbox`).
#
# Run from anywhere; the script locates the workspace kubeconfig relative to
# itself:
#
#   platform/cluster/home-istio/verify-ambient.sh
#
# Options:
#   --render   print the sandbox manifests to stdout and exit (no cluster)
#   --help     show usage
#
# Environment overrides:
#   CLUSTER_CONFIG_DIR  directory holding kubeconfig (default: <workspace>/home-cluster-1-config)
#   KUBECTL_IMAGE       kubectl image (default: registry.k8s.io/kubectl:v1.36.4)
#   ISTIOCTL_IMAGE      istioctl image (default: docker.io/istio/istioctl:1.31.3)

set -euo pipefail

NAMESPACE="${NAMESPACE:-ambient-sandbox}"
NOMESH_NAMESPACE="${NOMESH_NAMESPACE:-ambient-sandbox-nomesh}"
SERVER_URL="${SERVER_URL:-http://server.${NAMESPACE}.svc.cluster.local}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CLUSTER_CONFIG_DIR="${CLUSTER_CONFIG_DIR:-${SCRIPT_DIR}/../../../home-cluster-1-config}"
KUBECTL_IMAGE="${KUBECTL_IMAGE:-registry.k8s.io/kubectl:v1.36.4}"
ISTIOCTL_IMAGE="${ISTIOCTL_IMAGE:-docker.io/istio/istioctl:1.31.3}"

usage() {
  cat <<'EOF'
verify-ambient.sh — prove ambient mTLS works in the ambient-sandbox namespace.

Asserts: Istio components ready, `istioctl analyze` clean, in-mesh request
succeeds, plaintext request from an unlabeled namespace is denied, and ztunnel
reports the server workload as HBONE.

Run from anywhere; the script locates the workspace kubeconfig relative to
itself (or set CLUSTER_CONFIG_DIR):

  platform/cluster/home-istio/verify-ambient.sh

Options:
  --render   print the sandbox manifests to stdout and exit (no cluster)
  --help     show usage
EOF
}

die() {
  printf 'ERROR: %s\n' "$*" >&2
  exit 1
}

kubectl() {
  docker run --rm -i \
    --volume "${CLUSTER_CONFIG_DIR}:/cfg:ro" \
    "${KUBECTL_IMAGE}" \
    --kubeconfig /cfg/kubeconfig \
    "$@"
}

istioctl() {
  docker run --rm -i \
    --volume "${CLUSTER_CONFIG_DIR}:/cfg:ro" \
    "${ISTIOCTL_IMAGE}" \
    --kubeconfig /cfg/kubeconfig \
    "$@"
}

pass() { printf 'PASS %s\n' "$*"; }
fail() { printf 'FAIL %s\n' "$*"; FAILURES=$((FAILURES + 1)); }

# curl in a sandbox pod. The image is the same for both probes, so a failure
# cannot be blamed on a different client implementation.
exec_curl() {
  local namespace="$1"
  local url="$2"
  kubectl -n "${namespace}" exec deploy/curl-client -- \
    curl -sS -o /dev/null -w '%{http_code}' \
    --connect-timeout 5 --max-time 10 "${url}"
}

main() {
  command -v docker >/dev/null 2>&1 || die "docker is required to run kubectl/istioctl"

  CLUSTER_CONFIG_DIR="$(cd "${CLUSTER_CONFIG_DIR}" 2>/dev/null && pwd)" \
    || die "cluster config directory not found (set CLUSTER_CONFIG_DIR; default is <workspace>/home-cluster-1-config)"
  [[ -f "${CLUSTER_CONFIG_DIR}/kubeconfig" ]] \
    || die "kubeconfig not found at ${CLUSTER_CONFIG_DIR}/kubeconfig (set CLUSTER_CONFIG_DIR)"

  FAILURES=0

  echo "==> control plane and data plane"
  kubectl -n istio-system rollout status deployment/istiod --timeout=180s \
    || die "istiod is not ready"
  kubectl -n istio-system rollout status daemonset/ztunnel --timeout=180s \
    || die "ztunnel is not ready"
  kubectl -n istio-system rollout status daemonset/istio-cni-node --timeout=180s \
    || die "istio-cni-node is not ready"
  pass "istiod, ztunnel and istio-cni-node are ready"

  echo
  echo "==> istioctl analyze"
  analyze_output="$(istioctl analyze -A 2>&1)" || true
  printf '%s\n' "${analyze_output}"
  if grep -qE '^(Error|Warning)' <<<"${analyze_output}"; then
    fail "istioctl analyze reported issues"
  else
    pass "istioctl analyze is clean"
  fi

  echo
  echo "==> in-mesh request (must succeed over mTLS)"
  code=""
  for _ in $(seq 1 20); do
    if code="$(exec_curl "${NAMESPACE}" "${SERVER_URL}/" 2>/dev/null)" && [[ "${code}" == "200" ]]; then
      break
    fi
    sleep 3
  done
  if [[ "${code}" == "200" ]]; then
    pass "in-mesh client reached ${SERVER_URL} (HTTP ${code})"
  else
    fail "in-mesh client did not reach ${SERVER_URL} (last HTTP code: ${code:-none})"
  fi

  echo
  echo "==> ztunnel workload state"
  server_pod="$(kubectl -n "${NAMESPACE}" get pods -l app=ambient-server \
    -o jsonpath='{.items[0].metadata.name}' 2>/dev/null || true)"
  if [[ -z "${server_pod}" ]]; then
    fail "no server pod found in ${NAMESPACE}"
  else
    # The pod form resolves the ztunnel on that pod's node, so the result does
    # not depend on which ztunnel instance is picked by default.
    workloads="$(istioctl ztunnel-config workload "${server_pod}.${NAMESPACE}" 2>&1)" || true
    printf '%s\n' "${workloads}"
    if grep -q "${server_pod}" <<<"${workloads}" && grep -q 'HBONE' <<<"${workloads}"; then
      pass "ztunnel lists ${server_pod} as an HBONE workload"
    else
      fail "ztunnel does not report ${server_pod} as HBONE"
    fi
  fi

  echo
  echo "==> plaintext request from a non-mesh namespace (must be denied)"
  if kubectl -n "${NOMESH_NAMESPACE}" exec deploy/curl-client -- \
      nslookup "server.${NAMESPACE}.svc.cluster.local" >/dev/null 2>&1; then
    pass "DNS resolves from the non-mesh namespace (denial is not a DNS failure)"
  else
    fail "DNS does not resolve from the non-mesh namespace; the denial is inconclusive"
  fi

  if exec_curl "${NOMESH_NAMESPACE}" "${SERVER_URL}/" >/dev/null 2>&1; then
    fail "plaintext request from ${NOMESH_NAMESPACE} SUCCEEDED (mTLS not enforced)"
  else
    pass "plaintext request from ${NOMESH_NAMESPACE} was denied"
  fi

  echo
  if ((FAILURES == 0)); then
    echo "RESULT: PASS - ambient mTLS is enforced in ${NAMESPACE}"
    exit 0
  fi
  echo "RESULT: FAIL - ${FAILURES} check(s) failed"
  exit 1
}

case "${1:-}" in
  --render)
    docker run --rm -v "${SCRIPT_DIR}:/src:ro" -w /src \
      "${KUBECTL_IMAGE}" kustomize build kustomizations/ambient-sandbox
    ;;
  --help | -h)
    usage
    ;;
  "")
    main
    ;;
  *)
    die "unknown option: $1 (try --help)"
    ;;
esac
