#!/usr/bin/env bash
# Live checks for ticket #8: the single-namespace PaC path is retired and the
# registry is the only source of truth.
#
# Asserts on the real cluster:
#   - `tekton-pipelines` holds shared Tekton objects only: no tenant Repository
#     CRs, no `tekton-bot` SA/Role/RoleBinding, no `npm-npmrc` ConfigMap, no
#     deploy secrets and no `home-terraform-plans` PVC/CronJob;
#   - the shared Tasks/Pipelines and the Tekton config/dashboard remain;
#   - the obsolete Helm releases (`pipelines-as-code-repositories`,
#     `cicd-tekton-tekton-bot`) are gone from the `default` namespace;
#   - both registry tenants still have every per-tenant object;
#   - no `tekton-bot` SA or `npm-npmrc` ConfigMap exists outside a tenant
#     namespace (tenant-platform is the only owner).
#
# Usage:
#   KUBECONFIG_DIR=/path/to/home-cluster-1-config \
#     ./tenant-platform/tests/retire-single-namespace-checks.sh
#
# Environment:
#   KUBECONFIG_DIR  directory holding the cluster kubeconfig (required)
#   KUBECTL_IMAGE   kubectl image (default registry.k8s.io/kubectl:v1.36.4)
set -euo pipefail

KUBECTL_IMAGE="${KUBECTL_IMAGE:-registry.k8s.io/kubectl:v1.36.4}"
KUBECONFIG_DIR="${KUBECONFIG_DIR:-}"

CONTROL_NS="tekton-pipelines"
BUILD_NS="stijnklomp-lave"
BUILD_OWNER="stijnklomp-LAVE"
ORG_NS="stijnklomp-talos-linux-home-setup"
ORG_OWNER="stijnklomp-talos-linux-home-setup"
HELM_NS="default"

if [ -z "${KUBECONFIG_DIR}" ]; then
  echo "KUBECONFIG_DIR must point at the directory containing the cluster kubeconfig" >&2
  exit 2
fi
if [ ! -f "${KUBECONFIG_DIR}/kubeconfig" ]; then
  echo "no kubeconfig at ${KUBECONFIG_DIR}/kubeconfig" >&2
  exit 2
fi

kubectl() {
  docker run --rm \
    -v "${KUBECONFIG_DIR}:/cfg:ro" \
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

# count <namespace> <kind>
count() {
  kubectl -n "$1" get "$2" -o name 2>/dev/null | grep -c . || true
}

# release_secrets <release> - any Helm release storage secret in the default ns
release_secrets() {
  kubectl -n "${HELM_NS}" get secret -l "owner=helm,name=$1" -o name 2>/dev/null \
    | grep -c . || true
}

# names_of <kind> <name> - namespaces of every object with that name
names_of() {
  kubectl get "$1" -A -o jsonpath="{range .items[?(@.metadata.name==\"$2\")]}{.metadata.namespace}{\"\\n\"}{end}" 2>/dev/null \
    | sort | paste -sd, - || true
}

echo "== control plane (${CONTROL_NS}) holds shared objects only =="
eq "no Repository CRs in the control plane" "$(count "${CONTROL_NS}" repositories.pipelinesascode.tekton.dev)" "0"
eq "no tekton-bot ServiceAccount" "$(get "${CONTROL_NS}" serviceaccount tekton-bot '{.metadata.name}')" ""
eq "no tekton-bot-developer Role" "$(get "${CONTROL_NS}" role tekton-bot-developer '{.metadata.name}')" ""
eq "no tekton-bot-developer RoleBinding" "$(get "${CONTROL_NS}" rolebinding tekton-bot-developer '{.metadata.name}')" ""
eq "no npm-npmrc ConfigMap" "$(get "${CONTROL_NS}" configmap npm-npmrc '{.metadata.name}')" ""
for secret in dockerhub-docker-config gitops-pat cloud-creds npm-secret; do
  eq "no deploy secret ${secret}" "$(get "${CONTROL_NS}" secret "${secret}" '{.metadata.name}')" ""
done
eq "no home-terraform-plans PVC" "$(get "${CONTROL_NS}" pvc home-terraform-plans '{.metadata.name}')" ""
eq "no home-terraform-plans-cleanup CronJob" \
  "$(get "${CONTROL_NS}" cronjob home-terraform-plans-cleanup '{.metadata.name}')" ""

echo "== shared catalog and Tekton control plane remain =="
for task in git-clone-v1 npm-install-v1 build-push-v1 kaniko-v1 gitops-yaml-bump-v1 terragrunt-plan-v1 terragrunt-apply-destroy-v1; do
  eq "Task ${task} exists" "$(get "${CONTROL_NS}" task "${task}" '{.metadata.name}')" "${task}"
done
for pipeline in app-release-v1 terragrunt-plan-v1 terragrunt-apply-destroy-v1; do
  eq "Pipeline ${pipeline} exists" "$(get "${CONTROL_NS}" pipeline "${pipeline}" '{.metadata.name}')" "${pipeline}"
done
eq "Tekton config ConfigMap exists" "$(get "${CONTROL_NS}" configmap config-defaults '{.metadata.name}')" "config-defaults"
for deployment in tekton-pipelines-controller tekton-pipelines-webhook tekton-dashboard; do
  eq "Deployment ${deployment} exists" \
    "$(get "${CONTROL_NS}" deployment "${deployment}" '{.metadata.name}')" "${deployment}"
done

echo "== obsolete Helm releases are uninstalled =="
eq "pipelines-as-code-repositories release is gone" \
  "$(release_secrets pipelines-as-code-repositories)" "0"
eq "cicd-tekton-tekton-bot release is gone" \
  "$(release_secrets cicd-tekton-tekton-bot)" "0"

echo "== registry tenants still own their objects =="
eq "build namespace exists" \
  "$(get "${BUILD_NS}" namespace "${BUILD_NS}" '{.metadata.name}')" "${BUILD_NS}"
eq "build tenant label" \
  "$(get "${BUILD_NS}" namespace "${BUILD_NS}" '{.metadata.labels.home\.cluster/tenant}')" "${BUILD_OWNER}"
eq "build tier label" \
  "$(get "${BUILD_NS}" namespace "${BUILD_NS}" '{.metadata.labels.home\.cluster/tier}')" "build"
eq "org namespace exists" \
  "$(get "${ORG_NS}" namespace "${ORG_NS}" '{.metadata.name}')" "${ORG_NS}"
eq "org tenant label" \
  "$(get "${ORG_NS}" namespace "${ORG_NS}" '{.metadata.labels.home\.cluster/tenant}')" "${ORG_OWNER}"
eq "org tier label" \
  "$(get "${ORG_NS}" namespace "${ORG_NS}" '{.metadata.labels.home\.cluster/tier}')" "deploy"
eq "build Repository CR count" "$(count "${BUILD_NS}" repositories.pipelinesascode.tekton.dev)" "1"
eq "org Repository CR count" "$(count "${ORG_NS}" repositories.pipelinesascode.tekton.dev)" "17"

for ns in "${BUILD_NS}" "${ORG_NS}"; do
  eq "${ns}: tekton-bot SA" "$(get "${ns}" serviceaccount tekton-bot '{.metadata.name}')" "tekton-bot"
  eq "${ns}: SA does not automount a token" \
    "$(get "${ns}" serviceaccount tekton-bot '{.automountServiceAccountToken}')" "false"
  eq "${ns}: npm-npmrc ConfigMap" "$(get "${ns}" configmap npm-npmrc '{.metadata.name}')" "npm-npmrc"
  eq "${ns}: CiliumNetworkPolicy" \
    "$(get "${ns}" ciliumnetworkpolicy tenant-sandbox '{.metadata.name}')" "tenant-sandbox"
  eq "${ns}: ResourceQuota" "$(get "${ns}" resourcequota tenant-quota '{.metadata.name}')" "tenant-quota"
  eq "${ns}: LimitRange" "$(get "${ns}" limitrange tenant-limits '{.metadata.name}')" "tenant-limits"
  eq "${ns}: developer RoleBinding" \
    "$(get "${ns}" rolebinding tekton-developer-stijnklomp1-hotmail-com '{.roleRef.name}')" \
    "tekton-developer"
done
eq "deduped namespace-lister ClusterRoleBinding" \
  "$(kubectl get clusterrolebinding tekton-namespace-lister-stijnklomp1-hotmail-com \
      -o jsonpath='{.roleRef.name}' 2>/dev/null || true)" \
  "tekton-namespace-lister"

echo "== tenant objects exist only in tenant namespaces =="
eq "tekton-bot SA namespaces" "$(names_of serviceaccount tekton-bot)" \
  "${BUILD_NS},${ORG_NS}"
eq "npm-npmrc ConfigMap namespaces" "$(names_of configmap npm-npmrc)" \
  "${BUILD_NS},${ORG_NS}"
eq "Repository CR namespaces" "$(names_of repositories.pipelinesascode.tekton.dev test-app)" \
  "${BUILD_NS}"
eq "all Repository CRs are in the two tenant namespaces" \
  "$(kubectl get repositories.pipelinesascode.tekton.dev -A \
      -o jsonpath='{range .items[*]}{.metadata.namespace}{"\n"}{end}' 2>/dev/null \
      | sort -u | paste -sd, - || true)" \
  "${BUILD_NS},${ORG_NS}"

echo
if [ "${failure}" -ne 0 ]; then
  printf 'FAILED: %d retire-single-namespace check(s) failed\n' "${failure}" >&2
  exit 1
fi
printf 'PASSED: single-namespace path is retired; registry tenants intact\n'
