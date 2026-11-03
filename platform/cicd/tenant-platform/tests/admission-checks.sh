#!/usr/bin/env bash
# Live admission-gate checks for the tenant-platform Repository policy.
#
# Proves the ValidatingAdmissionPolicy behaviour on the real cluster:
#   - a Repository whose spec.url owner matches the namespace
#     `home.cluster/tenant` label (case-insensitively) is admitted;
#   - a Repository whose owner differs is denied;
#   - a Repository in a namespace without the label is denied;
#   - a Repository in `pipelines-as-code` is exempt (global CR namespace);
#   - changing the owner with an UPDATE is denied;
#   - the registry-managed tenant Repository still exists.
#
# The script only creates/deletes throwaway CRs named `admission-*` and uses
# URLs that match no real repository, so the PaC webhook's URL-uniqueness check
# does not interfere. It never touches real Repository CRs.
#
# Usage:
#   KUBECONFIG_DIR=/path/to/home-cluster-1-config \
#     ./tenant-platform/tests/admission-checks.sh
#
# Environment:
#   KUBECONFIG_DIR  directory holding the cluster kubeconfig (required)
#   KUBECTL_IMAGE   kubectl image (default registry.k8s.io/kubectl:v1.36.4)
set -euo pipefail

COMPONENT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
KUBECTL_IMAGE="${KUBECTL_IMAGE:-registry.k8s.io/kubectl:v1.36.4}"
KUBECONFIG_DIR="${KUBECONFIG_DIR:-}"
POLICY="tenant-platform-repository-owner"
TENANT_NAMESPACE="stijnklomp-lave"
EXEMPT_NAMESPACE="pipelines-as-code"

if [ -z "${KUBECONFIG_DIR}" ]; then
  echo "KUBECONFIG_DIR must point at the directory containing the cluster kubeconfig" >&2
  exit 2
fi
if [ ! -f "${KUBECONFIG_DIR}/kubeconfig" ]; then
  echo "no kubeconfig at ${KUBECONFIG_DIR}/kubeconfig" >&2
  exit 2
fi

WORK="$(mktemp -d /tmp/tenant-platform-admission.XXXXXX)"
trap 'rm -rf "${WORK}"' EXIT

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

# manifest <name> <namespace> <url>
manifest() {
  cat >"${WORK}/$1.yaml" <<EOF
apiVersion: pipelinesascode.tekton.dev/v1alpha1
kind: Repository
metadata:
  name: $1
  namespace: $2
spec:
  url: "$3"
EOF
}

# delete_quietly <name> <namespace>: the trap also runs this, so a failed test
# never leaves a CR behind.
delete_quietly() {
  kubectl -n "$2" delete repository.pipelinesascode.tekton.dev "$1" \
    --ignore-not-found >/dev/null 2>&1 || true
}

cleanup() {
  delete_quietly admission-ok "${TENANT_NAMESPACE}"
  delete_quietly admission-update "${TENANT_NAMESPACE}"
  delete_quietly admission-deny-owner "${TENANT_NAMESPACE}"
  delete_quietly admission-deny-nolabel tekton-pipelines
  delete_quietly admission-exempt "${EXEMPT_NAMESPACE}"
}

# admit <name> <namespace> <url> <label>
# A real create + delete: this is what a registry apply does.
admit() {
  local name="$1" namespace="$2" url="$3" label="$4"
  manifest "${name}" "${namespace}" "${url}"
  if kubectl apply -f "/m/${name}.yaml" >"${WORK}/${name}.out" 2>"${WORK}/${name}.err"; then
    ok "${label}"
  else
    bad "${label}"
    cat "${WORK}/${name}.err" >&2
  fi
  delete_quietly "${name}" "${namespace}"
}

# deny <name> <namespace> <url> <label> <expected-message-substring>
# A deny must come from the ValidatingAdmissionPolicy, not from the PaC
# validating webhook (which would mean the test URL collided or was invalid).
deny() {
  local name="$1" namespace="$2" url="$3" label="$4" expected="$5"
  manifest "${name}" "${namespace}" "${url}"
  if kubectl apply -f "/m/${name}.yaml" >"${WORK}/${name}.out" 2>"${WORK}/${name}.err"; then
    bad "${label}: request was admitted (expected deny)"
    delete_quietly "${name}" "${namespace}"
    return
  fi
  if ! grep -qF "ValidatingAdmissionPolicy '${POLICY}'" "${WORK}/${name}.err"; then
    bad "${label}: denied by something other than the ${POLICY} policy"
    sed -n '1,3p' "${WORK}/${name}.err" >&2
    return
  fi
  if ! grep -qF "${expected}" "${WORK}/${name}.err"; then
    bad "${label}: denial message does not contain '${expected}'"
    sed -n '1,3p' "${WORK}/${name}.err" >&2
    return
  fi
  ok "${label}: $(sed -n 's/.*denied request: //p' "${WORK}/${name}.err" | head -1)"
}

echo "== policy installed =="
if kubectl get validatingadmissionpolicy "${POLICY}" >/dev/null 2>&1; then
  ok "ValidatingAdmissionPolicy ${POLICY} exists"
else
  bad "ValidatingAdmissionPolicy ${POLICY} does not exist (apply the tenant-platform chart)"
fi
if kubectl get validatingadmissionpolicybinding "${POLICY}" >/dev/null 2>&1; then
  ok "ValidatingAdmissionPolicyBinding ${POLICY} exists"
else
  bad "ValidatingAdmissionPolicyBinding ${POLICY} does not exist"
fi

echo "== admission cases =="
# Same owner as the namespace label but different case: must match.
admit admission-ok "${TENANT_NAMESPACE}" \
  "https://github.com/stijnklomp-LAVE/admission-ok" \
  "admitted: spec.url owner matches the label case-insensitively"

admit admission-exempt "${EXEMPT_NAMESPACE}" \
  "https://github.com/someone-else/admission-exempt" \
  "admitted: pipelines-as-code namespace is exempt"

deny admission-deny-owner "${TENANT_NAMESPACE}" \
  "https://github.com/someone-else/admission-deny-owner" \
  "denied: spec.url owner differs from the namespace label" \
  "home.cluster/tenant"

deny admission-deny-nolabel tekton-pipelines \
  "https://github.com/someone-else/admission-deny-nolabel" \
  "denied: namespace has no home.cluster/tenant label" \
  "home.cluster/tenant"

# UPDATE is matched too: create a valid CR, then try to point it at another
# owner. The URL is unique, so only the policy can reject the update.
manifest admission-update "${TENANT_NAMESPACE}" \
  "https://github.com/stijnklomp-LAVE/admission-update"
if kubectl apply -f "/m/admission-update.yaml" >/dev/null 2>&1; then
  if kubectl -n "${TENANT_NAMESPACE}" patch repository.pipelinesascode.tekton.dev admission-update \
      --type merge -p '{"spec":{"url":"https://github.com/someone-else/admission-update"}}' \
      >"${WORK}/admission-update.out" 2>"${WORK}/admission-update.err"; then
    bad "denied: UPDATE changing the owner (update was admitted)"
  elif grep -qF "ValidatingAdmissionPolicy '${POLICY}'" "${WORK}/admission-update.err"; then
    ok "denied: UPDATE changing the owner: $(sed -n 's/.*denied request: //p' "${WORK}/admission-update.err" | head -1)"
  else
    bad "denied: UPDATE changing the owner was rejected by something other than ${POLICY}"
    sed -n '1,3p' "${WORK}/admission-update.err" >&2
  fi
else
  bad "denied: UPDATE setup could not create admission-update (see the admitted case above)"
fi
delete_quietly admission-update "${TENANT_NAMESPACE}"

echo "== registry-managed Repository untouched =="
if [ "$(kubectl -n "${TENANT_NAMESPACE}" get repository.pipelinesascode.tekton.dev test-app \
    -o jsonpath='{.spec.url}' 2>/dev/null || true)" = "https://github.com/stijnklomp-LAVE/test-app" ]; then
  ok "existing ${TENANT_NAMESPACE}/test-app Repository still exists with its URL"
else
  bad "existing ${TENANT_NAMESPACE}/test-app Repository is missing or changed"
fi

cleanup

echo
if [ "${failure}" -ne 0 ]; then
  printf 'FAILED: %d admission check(s) failed\n' "${failure}" >&2
  exit 1
fi
printf 'PASSED: all admission checks\n'
