#!/usr/bin/env bash
# Offline render and registry-validation checks for tenant-platform.
#
# Nothing is installed on the host: Helm, helmfile and yq run in containers.
# No cluster access is needed or performed.
#
#   ./tenant-platform/tests/render-checks.sh
#
# Environment overrides: HELM_IMAGE, HELMFILE_IMAGE.
set -euo pipefail

COMPONENT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CHART_DIR="./charts/tenant-platform"
REGISTRY="./values/tenants.yaml"
FIXTURES="./tests/fixtures"
ACCEPTANCE="./tests/build-tenant-pipelinerun.yaml"

HELM_IMAGE="${HELM_IMAGE:-alpine/helm:latest}"
HELMFILE_IMAGE="${HELMFILE_IMAGE:-ghcr.io/helmfile/helmfile:latest}"

WORK="$(mktemp -d /tmp/tenant-platform-render.XXXXXX)"
trap 'rm -rf "${WORK}"' EXIT

failure=0

ok() {
  printf 'ok: %s\n' "$1"
}

bad() {
  failure=$((failure + 1))
  printf 'FAIL: %s\n' "$1" >&2
}

helm_run() {
  docker run --rm \
    -v "${COMPONENT_DIR}:/src" \
    -v "${WORK}:/work" \
    -w /src \
    "${HELM_IMAGE}" "$@"
}

# expect_failure <name> <values-file relative to the component> <message substring>
expect_failure() {
  local name="$1" values="$2" expected="$3"
  if helm_run template tenant-platform "${CHART_DIR}" -f "${values}" \
      >"${WORK}/${name}.out" 2>"${WORK}/${name}.err"; then
    bad "registry validation [${name}]: render succeeded, expected a failure"
    return
  fi
  if grep -qF "${expected}" "${WORK}/${name}.err"; then
    ok "registry validation [${name}]: fails with '${expected}'"
  else
    bad "registry validation [${name}]: error does not contain '${expected}'"
    sed -n '1,8p' "${WORK}/${name}.err" >&2
  fi
}

echo "== helm lint =="
if helm_run lint "${CHART_DIR}" >"${WORK}/lint-empty.out" 2>&1; then
  ok "helm lint (empty registry)"
else
  bad "helm lint (empty registry)"
  cat "${WORK}/lint-empty.out" >&2
fi
if helm_run lint "${CHART_DIR}" -f "${REGISTRY}" >"${WORK}/lint.out" 2>&1; then
  ok "helm lint (real registry)"
else
  bad "helm lint (real registry)"
  cat "${WORK}/lint.out" >&2
fi
if helm_run lint "${CHART_DIR}" -f "${FIXTURES}/valid-two-tenants.yaml" >"${WORK}/lint-multi.out" 2>&1; then
  ok "helm lint (two-tenant fixture)"
else
  bad "helm lint (two-tenant fixture)"
  cat "${WORK}/lint-multi.out" >&2
fi

echo "== helm template =="
if helm_run template tenant-platform "${CHART_DIR}" >"${WORK}/empty.yaml" 2>"${WORK}/empty.err"; then
  ok "helm template with an empty registry"
else
  bad "helm template with an empty registry"
  cat "${WORK}/empty.err" >&2
fi
if helm_run template tenant-platform "${CHART_DIR}" -f "${REGISTRY}" \
    >"${WORK}/base.yaml" 2>"${WORK}/base.err"; then
  ok "helm template with the real registry"
else
  bad "helm template with the real registry"
  cat "${WORK}/base.err" >&2
fi
if helm_run template tenant-platform "${CHART_DIR}" -f "${FIXTURES}/valid-two-tenants.yaml" \
    >"${WORK}/multi.yaml" 2>"${WORK}/multi.err"; then
  ok "helm template with the two-tenant fixture"
else
  bad "helm template with the two-tenant fixture"
  cat "${WORK}/multi.err" >&2
fi

echo "== registry validation failures =="
expect_failure duplicate-owner "${FIXTURES}/invalid-duplicate-owner.yaml" "duplicate owner"
expect_failure duplicate-namespace "${FIXTURES}/invalid-duplicate-namespace.yaml" "duplicate namespace"
expect_failure duplicate-repo-name "${FIXTURES}/invalid-duplicate-repo-name.yaml" "duplicate repo name"
expect_failure duplicate-repo-url "${FIXTURES}/invalid-duplicate-repo-url.yaml" "duplicate repo url"
expect_failure invalid-namespace "${FIXTURES}/invalid-namespace.yaml" "not a valid DNS-1123 label"
expect_failure invalid-repo-name "${FIXTURES}/invalid-repo-name.yaml" "not a valid DNS subdomain"
expect_failure invalid-tier "${FIXTURES}/invalid-tier.yaml" "tier must be one of"
expect_failure owner-too-long "${FIXTURES}/invalid-owner-too-long.yaml" "longer than 63 characters"
expect_failure invalid-concurrency "${FIXTURES}/invalid-concurrency.yaml" "concurrencyLimit must be a positive integer"
expect_failure multiple-tier "${FIXTURES}/invalid-multiple.yaml" "tier must be one of"
expect_failure multiple-namespace "${FIXTURES}/invalid-multiple.yaml" "duplicate namespace"
expect_failure multiple-repo-name "${FIXTURES}/invalid-multiple.yaml" "duplicate repo name"

echo "== helmfile template =="
if docker run --rm \
    -v "${COMPONENT_DIR}:/src" \
    -v "${WORK}:/work" \
    -w /src \
    --entrypoint /bin/sh \
    "${HELMFILE_IMAGE}" -c 'helmfile template > /work/helmfile-template.yaml' \
    >"${WORK}/helmfile.out" 2>"${WORK}/helmfile.err"; then
  ok "helmfile template"
else
  bad "helmfile template"
  cat "${WORK}/helmfile.err" >&2
fi

echo "== PaC auto-configure disabled =="
PAC_DIR="${COMPONENT_DIR}/../cicd-tekton-pipelines-as-code"
if grep -q 'auto-configure-new-github-repo: "false"' \
    "${PAC_DIR}/kustomizations/pipelines-as-code/patches/configmaps.yaml"; then
  ok "PaC config sets auto-configure-new-github-repo to false"
else
  bad "PaC config does not disable auto-configure-new-github-repo"
fi
if grep -qE '^  auto-configure-repo-(namespace|repository)-template: (null)?$' \
    "${PAC_DIR}/kustomizations/pipelines-as-code/patches/configmaps.yaml"; then
  ok "PaC config deletes the auto-configure-repo-* templates (null)"
else
  bad "PaC config still sets an auto-configure-repo-* template"
fi
if grep -q 'chart: ./charts/pipelines-as-code-repositories' "${PAC_DIR}/helmfile.yaml"; then
  bad "PaC helmfile still has a pipelines-as-code-repositories release"
else
  ok "PaC helmfile has no pipelines-as-code-repositories release"
fi
if [ -e "${PAC_DIR}/charts/pipelines-as-code-repositories" ]; then
  bad "PaC chart directory pipelines-as-code-repositories still exists"
else
  ok "PaC chart directory pipelines-as-code-repositories is deleted"
fi

echo "== TektonConfig (operator) =="
if docker run --rm \
    -v "${COMPONENT_DIR}/../cicd-tekton-operator:/src:ro" \
    registry.k8s.io/kubectl:v1.36.4 kustomize /src/kustomizations/tekton-operator-config \
    >"${WORK}/tektonconfig.yaml" 2>"${WORK}/tektonconfig.err"; then
  ok "kubectl kustomize tekton-operator-config"
else
  bad "kubectl kustomize tekton-operator-config"
  cat "${WORK}/tektonconfig.err" >&2
fi

echo "== YAML parse checks =="
docker run --rm -i \
  -v "${COMPONENT_DIR}:/src:ro" \
  -v "${WORK}:/work:ro" \
  -w /src \
  --entrypoint /bin/sh \
  "${HELMFILE_IMAGE}" <<'EOS' || failure=$((failure + 1))
set -eu
for f in \
  values/tenants.yaml \
  charts/tenant-platform/values.yaml \
  charts/tenant-platform/Chart.yaml \
  helmfile.yaml \
  tests/build-tenant-pipelinerun.yaml \
  tests/deploy-tenant-pipelinerun.yaml \
  tests/fixtures/*.yaml
do
  yq -e '.' "$f" >/dev/null
  echo "ok: yq parses $f"
done
for f in /work/base.yaml /work/multi.yaml /work/empty.yaml /work/helmfile-template.yaml /work/tektonconfig.yaml; do
  yq -e '.' "$f" >/dev/null
  echo "ok: yq parses $f"
done
EOS

echo "== registry is the only source of tenant names =="
if grep -q "stijnklomp" "${WORK}/multi.yaml" 2>/dev/null; then
  bad "rendering the two-tenant fixture leaked a name from values/tenants.yaml"
else
  ok "no tenant name hardcoded in the chart (fixture render is clean)"
fi
if grep -R -n "kind: Secret" "${COMPONENT_DIR}/charts" >/dev/null 2>&1; then
  bad "chart renders a Secret (operator credentials must never enter a tenant namespace)"
else
  ok "chart renders no Secret"
fi

echo "== rendered base registry =="
docker run --rm -i \
  -v "${WORK}:/work:ro" \
  -w /work \
  --entrypoint /bin/sh \
  "${HELMFILE_IMAGE}" <<'EOS' || failure=$((failure + 1))
set -eu
fail=0
q() { yq ea -r "$1" "$2"; }
eq() {
  expr=$1; file=$2; want=$3; label=$4
  got=$(q "$expr" "$file" 2>/dev/null || true)
  if [ "$got" = "$want" ]; then
    echo "ok: $label"
  else
    echo "FAIL: $label" >&2
    echo "      expr: $expr" >&2
    echo "      got:  '$got'" >&2
    echo "      want: '$want'" >&2
    fail=1
  fi
}

BUILD_NS=stijnklomp-lave
ORG_NS=stijnklomp-talos-linux-home-setup
ORG_REPOS="home-cluster,home-cluster-internal"

# --- Namespace (two tenants: build + deploy) ---
eq '[select(.kind == "Namespace")] | length' base.yaml 2 "base: two Namespaces"
eq '[select(.kind == "Namespace")] | map(.metadata.name) | sort | join(",")' base.yaml "${BUILD_NS},${ORG_NS}" "base: namespace names"
eq '[select(.kind == "Namespace" and .metadata.name == "stijnklomp-lave")] | .[0].metadata.labels["home.cluster/tenant"]' base.yaml stijnklomp-LAVE "base: build tenant label keeps the owner verbatim"
eq '[select(.kind == "Namespace" and .metadata.name == "stijnklomp-talos-linux-home-setup")] | .[0].metadata.labels["home.cluster/tenant"]' base.yaml stijnklomp-talos-linux-home-setup "base: deploy tenant label is the org owner"
eq '[select(.kind == "Namespace" and .metadata.name == "stijnklomp-lave")] | .[0].metadata.labels["home.cluster/tier"]' base.yaml build "base: build tier label"
eq '[select(.kind == "Namespace" and .metadata.name == "stijnklomp-talos-linux-home-setup")] | .[0].metadata.labels["home.cluster/tier"]' base.yaml deploy "base: deploy tier label"
eq '[select(.kind == "Namespace" and .metadata.name == "stijnklomp-lave")] | .[0].metadata.labels["home.cluster/managed"]' base.yaml true "base: build managed label for the pruner"
eq '[select(.kind == "Namespace" and .metadata.name == "stijnklomp-talos-linux-home-setup")] | .[0].metadata.labels["home.cluster/managed"]' base.yaml true "base: deploy managed label for the pruner"
for ns in "${BUILD_NS}" "${ORG_NS}"; do
  for label in enforce audit warn; do
    eq "[select(.kind == \"Namespace\" and .metadata.name == \"${ns}\")] | .[0].metadata.labels[\"pod-security.kubernetes.io/${label}\"]" base.yaml restricted "base: ${ns} PSA ${label}"
  done
  eq "[select(.kind == \"Namespace\" and .metadata.name == \"${ns}\")] | .[0].metadata.labels[\"pod-security.kubernetes.io/enforce-version\"]" base.yaml latest "base: ${ns} PSA enforce-version"
done

# --- ResourceQuota ---
eq '[select(.kind == "ResourceQuota")] | length' base.yaml 2 "base: two ResourceQuotas"
eq '[select(.kind == "ResourceQuota")] | map(.metadata.namespace) | sort | join(",")' base.yaml "${BUILD_NS},${ORG_NS}" "base: quota namespaces"
for pair in \
  'requests.cpu|4' 'limits.cpu|8' \
  'requests.memory|8Gi' 'limits.memory|16Gi' \
  'requests.ephemeral-storage|10Gi' 'limits.ephemeral-storage|20Gi' \
  'pods|20' 'persistentvolumeclaims|4' 'requests.storage|40Gi'
do
  key=${pair%%|*}
  want=${pair##*|}
  eq "[select(.kind == \"ResourceQuota\" and .metadata.namespace == \"${BUILD_NS}\")] | .[0].spec.hard[\"${key}\"]" base.yaml "$want" "base: build quota ${key}=${want}"
done
for pair in \
  'requests.cpu|8' 'limits.cpu|16' \
  'requests.memory|16Gi' 'limits.memory|32Gi' \
  'requests.ephemeral-storage|20Gi' 'limits.ephemeral-storage|40Gi' \
  'pods|40' 'persistentvolumeclaims|8' 'requests.storage|100Gi'
do
  key=${pair%%|*}
  want=${pair##*|}
  eq "[select(.kind == \"ResourceQuota\" and .metadata.namespace == \"${ORG_NS}\")] | .[0].spec.hard[\"${key}\"]" base.yaml "$want" "base: deploy quota ${key}=${want}"
done

# --- LimitRange ---
eq '[select(.kind == "LimitRange")] | length' base.yaml 2 "base: two LimitRanges"
eq "[select(.kind == \"LimitRange\" and .metadata.namespace == \"${BUILD_NS}\")] | .[0].spec.limits[0].type" base.yaml Container "base: build LimitRange type"
eq "[select(.kind == \"LimitRange\" and .metadata.namespace == \"${BUILD_NS}\")] | .[0].spec.limits[0].default.cpu" base.yaml 2 "base: build LimitRange default cpu"
eq "[select(.kind == \"LimitRange\" and .metadata.namespace == \"${BUILD_NS}\")] | .[0].spec.limits[0].default.memory" base.yaml 2Gi "base: build LimitRange default memory"
eq "[select(.kind == \"LimitRange\" and .metadata.namespace == \"${BUILD_NS}\")] | .[0].spec.limits[0].default[\"ephemeral-storage\"]" base.yaml 5Gi "base: build LimitRange default ephemeral-storage"
eq "[select(.kind == \"LimitRange\" and .metadata.namespace == \"${BUILD_NS}\")] | .[0].spec.limits[0].defaultRequest.cpu" base.yaml 200m "base: build LimitRange defaultRequest cpu"
eq "[select(.kind == \"LimitRange\" and .metadata.namespace == \"${BUILD_NS}\")] | .[0].spec.limits[0].defaultRequest.memory" base.yaml 256Mi "base: build LimitRange defaultRequest memory"
eq "[select(.kind == \"LimitRange\" and .metadata.namespace == \"${BUILD_NS}\")] | .[0].spec.limits[0].defaultRequest[\"ephemeral-storage\"]" base.yaml 1Gi "base: build LimitRange defaultRequest ephemeral-storage"
eq "[select(.kind == \"LimitRange\" and .metadata.namespace == \"${ORG_NS}\")] | .[0].spec.limits[0].default.cpu" base.yaml 4 "base: deploy LimitRange default cpu"
eq "[select(.kind == \"LimitRange\" and .metadata.namespace == \"${ORG_NS}\")] | .[0].spec.limits[0].default.memory" base.yaml 4Gi "base: deploy LimitRange default memory"
eq "[select(.kind == \"LimitRange\" and .metadata.namespace == \"${ORG_NS}\")] | .[0].spec.limits[0].default[\"ephemeral-storage\"]" base.yaml 10Gi "base: deploy LimitRange default ephemeral-storage"
eq "[select(.kind == \"LimitRange\" and .metadata.namespace == \"${ORG_NS}\")] | .[0].spec.limits[0].defaultRequest.cpu" base.yaml 250m "base: deploy LimitRange defaultRequest cpu"
eq "[select(.kind == \"LimitRange\" and .metadata.namespace == \"${ORG_NS}\")] | .[0].spec.limits[0].defaultRequest.memory" base.yaml 512Mi "base: deploy LimitRange defaultRequest memory"
eq "[select(.kind == \"LimitRange\" and .metadata.namespace == \"${ORG_NS}\")] | .[0].spec.limits[0].defaultRequest[\"ephemeral-storage\"]" base.yaml 1Gi "base: deploy LimitRange defaultRequest ephemeral-storage"

# --- CiliumNetworkPolicy (build tier) ---
eq '[select(.kind == "CiliumNetworkPolicy")] | length' base.yaml 2 "base: two CiliumNetworkPolicies"
eq '[select(.kind == "CiliumNetworkPolicy")] | map(.metadata.namespace) | sort | join(",")' base.yaml "${BUILD_NS},${ORG_NS}" "base: CNP namespaces"
eq "[select(.kind == \"CiliumNetworkPolicy\" and .metadata.namespace == \"${BUILD_NS}\")] | .[0].metadata.name" base.yaml tenant-sandbox "base: build CNP name"
eq "[select(.kind == \"CiliumNetworkPolicy\" and .metadata.namespace == \"${BUILD_NS}\")] | .[0].spec.ingress | length" base.yaml 1 "base: build CNP ingress rules"
eq "[select(.kind == \"CiliumNetworkPolicy\" and .metadata.namespace == \"${BUILD_NS}\")] | .[0].spec.ingress[0].fromEntities | join(\",\")" base.yaml host "base: build CNP ingress only from host"
eq "[select(.kind == \"CiliumNetworkPolicy\" and .metadata.namespace == \"${BUILD_NS}\")] | .[0].spec.egress | length" base.yaml 3 "base: build CNP three egress allows"
eq "[select(.kind == \"CiliumNetworkPolicy\" and .metadata.namespace == \"${BUILD_NS}\")] | .[0].spec.egress[0].toEndpoints[0].matchLabels[\"k8s:io.kubernetes.pod.namespace\"]" base.yaml kube-system "base: build CNP DNS namespace"
eq "[select(.kind == \"CiliumNetworkPolicy\" and .metadata.namespace == \"${BUILD_NS}\")] | .[0].spec.egress[0].toEndpoints[0].matchLabels[\"k8s:k8s-app\"]" base.yaml kube-dns "base: build CNP DNS app"
eq "[select(.kind == \"CiliumNetworkPolicy\" and .metadata.namespace == \"${BUILD_NS}\")] | .[0].spec.egress[0].toPorts[0].ports | map(.protocol) | sort | join(\",\")" base.yaml TCP,UDP "base: build CNP DNS TCP+UDP"
eq "[select(.kind == \"CiliumNetworkPolicy\" and .metadata.namespace == \"${BUILD_NS}\")] | .[0].spec.egress[0].toPorts[0].ports | map(.port) | unique | join(\",\")" base.yaml 53 "base: build CNP DNS port 53"
eq "[select(.kind == \"CiliumNetworkPolicy\" and .metadata.namespace == \"${BUILD_NS}\")] | .[0].spec.egress[1].toEndpoints[0].matchLabels[\"k8s:io.kubernetes.pod.namespace\"]" base.yaml verdaccio "base: build CNP Verdaccio namespace"
eq "[select(.kind == \"CiliumNetworkPolicy\" and .metadata.namespace == \"${BUILD_NS}\")] | .[0].spec.egress[1].toPorts[0].ports[0].port" base.yaml 4873 "base: build CNP Verdaccio port"
eq "[select(.kind == \"CiliumNetworkPolicy\" and .metadata.namespace == \"${BUILD_NS}\")] | .[0].spec.egress[2].toEntities | join(\",\")" base.yaml world "base: build CNP world egress"
eq "[select(.kind == \"CiliumNetworkPolicy\" and .metadata.namespace == \"${BUILD_NS}\")] | .[0].spec.egress | any_c(.toEntities != null and .toEntities[0] == \"kube-apiserver\")" base.yaml false "base: build CNP excludes kube-apiserver"
eq "[select(.kind == \"CiliumNetworkPolicy\" and .metadata.namespace == \"${BUILD_NS}\")] | .[0].spec.egressDeny | map(.toCIDR[]) | join(\",\")" base.yaml "192.168.0.0/16,172.16.0.0/12,169.254.0.0/16" "base: build CNP denies RFC1918 and link-local"

# --- CiliumNetworkPolicy (deploy tier) ---
eq "[select(.kind == \"CiliumNetworkPolicy\" and .metadata.namespace == \"${ORG_NS}\")] | .[0].metadata.name" base.yaml tenant-sandbox "base: deploy CNP name"
eq "[select(.kind == \"CiliumNetworkPolicy\" and .metadata.namespace == \"${ORG_NS}\")] | .[0].spec.egress | length" base.yaml 4 "base: deploy CNP adds the API allow"
eq "[select(.kind == \"CiliumNetworkPolicy\" and .metadata.namespace == \"${ORG_NS}\")] | .[0].spec.egress[0].toEndpoints[0].matchLabels[\"k8s:io.kubernetes.pod.namespace\"]" base.yaml kube-system "base: deploy CNP DNS namespace"
eq "[select(.kind == \"CiliumNetworkPolicy\" and .metadata.namespace == \"${ORG_NS}\")] | .[0].spec.egress[1].toEndpoints[0].matchLabels[\"k8s:io.kubernetes.pod.namespace\"]" base.yaml verdaccio "base: deploy CNP Verdaccio namespace"
eq "[select(.kind == \"CiliumNetworkPolicy\" and .metadata.namespace == \"${ORG_NS}\")] | .[0].spec.egress[2].toEntities | join(\",\")" base.yaml world "base: deploy CNP world egress"
eq "[select(.kind == \"CiliumNetworkPolicy\" and .metadata.namespace == \"${ORG_NS}\")] | .[0].spec.egress[3].toEntities | join(\",\")" base.yaml kube-apiserver "base: deploy CNP allows kube-apiserver"
eq "[select(.kind == \"CiliumNetworkPolicy\" and .metadata.namespace == \"${ORG_NS}\")] | .[0].spec.egressDeny | map(.toCIDR[]) | join(\",\")" base.yaml "192.168.0.0/16,172.16.0.0/12,169.254.0.0/16" "base: deploy CNP denies RFC1918 and link-local"

# --- ServiceAccount / ConfigMap ---
eq '[select(.kind == "ServiceAccount" and .metadata.name == "tekton-bot")] | length' base.yaml 2 "base: one tekton-bot SA per tenant"
eq '[select(.kind == "ServiceAccount" and .metadata.name == "tekton-bot")] | map(.metadata.namespace) | sort | join(",")' base.yaml "${BUILD_NS},${ORG_NS}" "base: SA namespaces"
eq '[select(.kind == "ServiceAccount" and .metadata.name == "tekton-bot")] | map(.automountServiceAccountToken) | unique | join(",")' base.yaml false "base: no SA automounts a token"
eq '[select(.kind == "ConfigMap" and .metadata.name == "npm-npmrc")] | length' base.yaml 2 "base: one npm-npmrc ConfigMap per tenant"
eq '[select(.kind == "ConfigMap" and .metadata.name == "npm-npmrc")] | map(.metadata.namespace) | sort | join(",")' base.yaml "${BUILD_NS},${ORG_NS}" "base: npm-npmrc namespaces"
eq '[select(.kind == "ConfigMap" and .metadata.name == "npm-npmrc")] | map(.data[".npmrc"] | contains("registry=http://verdaccio.verdaccio.svc.cluster.local:4873/")) | unique | join(",")' base.yaml true "base: npmrc registry line"
eq '[select(.kind == "ConfigMap" and .metadata.name == "npm-npmrc")] | map(.data[".npmrc"] | contains("# Cache-only private registry (cicd-verdaccio) proxying registry.npmjs.org.")) | unique | join(",")' base.yaml true "base: npmrc keeps the live comment"

# --- Repository ---
eq '[select(.kind == "Repository")] | length' base.yaml 3 "base: 3 Repository CRs"
eq '[select(.kind == "Repository" and .metadata.namespace == "stijnklomp-lave")] | length' base.yaml 1 "base: build tenant keeps its Repository"
eq '[select(.kind == "Repository" and .metadata.namespace == "stijnklomp-talos-linux-home-setup")] | length' base.yaml 2 "base: deploy tenant has 2 Repositories"
eq '[select(.kind == "Repository" and .metadata.namespace == "stijnklomp-talos-linux-home-setup")] | map(.metadata.name) | sort | join(",")' base.yaml "${ORG_REPOS}" "base: deploy Repository names"
eq '[select(.kind == "Repository" and .metadata.namespace == "stijnklomp-talos-linux-home-setup")] | map(.spec.url) | unique | length' base.yaml 2 "base: deploy Repository URLs are unique"
eq '[select(.kind == "Repository" and .metadata.namespace == "stijnklomp-talos-linux-home-setup")] | map(select(.spec.url | test("^https://github.com/stijnklomp-talos-linux-home-setup/"))) | length' base.yaml 2 "base: every deploy Repository URL has the org owner"
eq '[select(.kind == "Repository" and .metadata.name == "test-app")] | .[0].metadata.namespace' base.yaml stijnklomp-lave "base: build Repository namespace is the run namespace"
eq '[select(.kind == "Repository" and .metadata.name == "test-app")] | .[0].spec.url' base.yaml https://github.com/stijnklomp-LAVE/test-app "base: build Repository URL"
eq '[select(.kind == "Repository" and .metadata.name == "test-app")] | .[0].spec.concurrency_limit' base.yaml 2 "base: Repository default concurrency_limit"
eq '[select(.kind == "Repository" and .metadata.name == "test-app")] | .[0].spec.settings.pipelinerun_provenance' base.yaml source "base: Repository source provenance"
eq '[select(.kind == "Repository" and .metadata.name == "home-cluster")] | .[0].metadata.namespace' base.yaml stijnklomp-talos-linux-home-setup "base: deploy Repository namespace is the org namespace"
eq '[select(.kind == "Repository" and .metadata.name == "home-cluster")] | .[0].spec.url' base.yaml https://github.com/stijnklomp-talos-linux-home-setup/home-cluster "base: deploy Repository URL"
eq '[select(.kind == "Repository" and .metadata.name == "home-cluster")] | .[0].spec.concurrency_limit' base.yaml 2 "base: deploy Repository concurrency_limit"
eq '[select(.kind == "Repository" and .metadata.name == "home-cluster")] | .[0].spec.settings.pipelinerun_provenance' base.yaml source "base: deploy Repository source provenance"

# --- Repository admission gate (ValidatingAdmissionPolicy) ---
eq '[select(.kind == "ValidatingAdmissionPolicy")] | length' base.yaml 1 "base: one ValidatingAdmissionPolicy"
eq '[select(.kind == "ValidatingAdmissionPolicy")] | .[0].metadata.name' base.yaml tenant-platform-repository-owner "base: policy name"
eq '[select(.kind == "ValidatingAdmissionPolicy")] | .[0].spec.failurePolicy' base.yaml Fail "base: policy fails closed"
eq '[select(.kind == "ValidatingAdmissionPolicy")] | .[0].spec.matchConstraints.resourceRules[0].apiGroups | join(",")' base.yaml pipelinesascode.tekton.dev "base: policy apiGroup"
eq '[select(.kind == "ValidatingAdmissionPolicy")] | .[0].spec.matchConstraints.resourceRules[0].apiVersions | join(",")' base.yaml v1alpha1 "base: policy apiVersion"
eq '[select(.kind == "ValidatingAdmissionPolicy")] | .[0].spec.matchConstraints.resourceRules[0].operations | join(",")' base.yaml CREATE,UPDATE "base: policy matches CREATE and UPDATE"
eq '[select(.kind == "ValidatingAdmissionPolicy")] | .[0].spec.matchConstraints.resourceRules[0].resources | join(",")' base.yaml repositories "base: policy matches repositories"
eq '[select(.kind == "ValidatingAdmissionPolicy")] | .[0].spec.matchConstraints.resourceRules[0].scope' base.yaml Namespaced "base: policy scope is namespaced"
eq '[select(.kind == "ValidatingAdmissionPolicy")] | .[0].spec.matchConstraints.namespaceSelector.matchExpressions[0].key' base.yaml kubernetes.io/metadata.name "base: exemption selector key"
eq '[select(.kind == "ValidatingAdmissionPolicy")] | .[0].spec.matchConstraints.namespaceSelector.matchExpressions[0].operator' base.yaml NotIn "base: exemption selector operator"
eq '[select(.kind == "ValidatingAdmissionPolicy")] | .[0].spec.matchConstraints.namespaceSelector.matchExpressions[0].values | join(",")' base.yaml pipelines-as-code "base: exempts the PaC namespace"
eq '[select(.kind == "ValidatingAdmissionPolicy")] | .[0].spec.variables | map(.expression) | join(";") | contains("home.cluster/tenant")' base.yaml true "base: policy reads the tenant label"
eq '[select(.kind == "ValidatingAdmissionPolicy")] | .[0].spec.validations[0].expression | contains("lowerAscii")' base.yaml true "base: owner comparison is case-insensitive"
eq '[select(.kind == "ValidatingAdmissionPolicy")] | .[0].spec.variables | map(.expression) | join(";") | contains("size(variables.urlParts) >= 4")' base.yaml true "base: policy guards short URLs"
eq '[select(.kind == "ValidatingAdmissionPolicy")] | .[0].spec.validations[0].message | contains("home.cluster/tenant")' base.yaml true "base: denial message names the label"
eq '[select(.kind == "ValidatingAdmissionPolicyBinding")] | length' base.yaml 1 "base: one ValidatingAdmissionPolicyBinding"
eq '[select(.kind == "ValidatingAdmissionPolicyBinding")] | .[0].metadata.name' base.yaml tenant-platform-repository-owner "base: binding name"
eq '[select(.kind == "ValidatingAdmissionPolicyBinding")] | .[0].spec.policyName' base.yaml tenant-platform-repository-owner "base: binding policy name"
eq '[select(.kind == "ValidatingAdmissionPolicyBinding")] | .[0].spec.validationActions | join(",")' base.yaml Deny "base: binding denies"

# --- Developer RBAC (same developer listed in both tenants) ---
eq '[select(.kind == "RoleBinding")] | length' base.yaml 2 "base: one RoleBinding per developer per tenant"
eq '[select(.kind == "RoleBinding")] | map(.metadata.namespace) | sort | join(",")' base.yaml "${BUILD_NS},${ORG_NS}" "base: RoleBinding namespaces"
eq '[select(.kind == "RoleBinding" and .metadata.namespace == "stijnklomp-lave")] | .[0].metadata.name' base.yaml tekton-developer-stijnklomp1-hotmail-com "base: build RoleBinding name from the email slug"
eq '[select(.kind == "RoleBinding" and .metadata.namespace == "stijnklomp-talos-linux-home-setup")] | .[0].metadata.name' base.yaml tekton-developer-stijnklomp1-hotmail-com "base: deploy RoleBinding name from the email slug"
eq '[select(.kind == "RoleBinding")] | map(.roleRef.kind) | unique | join(",")' base.yaml ClusterRole "base: RoleBindings target a ClusterRole"
eq '[select(.kind == "RoleBinding")] | map(.roleRef.name) | unique | join(",")' base.yaml tekton-developer "base: RoleBindings target tekton-developer"
eq '[select(.kind == "RoleBinding")] | map(.subjects[0].kind) | unique | join(",")' base.yaml User "base: RoleBinding subjects are Users"
eq '[select(.kind == "RoleBinding")] | map(.subjects[0].name) | unique | join(",")' base.yaml stijnklomp1@hotmail.com "base: RoleBinding subject is the developer email"
eq '[select(.kind == "ClusterRoleBinding")] | map(select(.metadata.name | test("^tekton-namespace-lister-"))) | length' base.yaml 1 "base: one namespace-lister CRB per unique developer"
eq '[select(.kind == "ClusterRoleBinding")] | map(select(.metadata.name | test("^tekton-namespace-lister-"))) | .[0].metadata.name' base.yaml tekton-namespace-lister-stijnklomp1-hotmail-com "base: namespace-lister CRB name"
eq '[select(.kind == "ClusterRoleBinding")] | map(select(.metadata.name | test("^tekton-namespace-lister-"))) | .[0].roleRef.name' base.yaml tekton-namespace-lister "base: namespace-lister CRB targets tekton-namespace-lister"

# --- Prune hook + RBAC ---
eq '[select(.kind == "Job")] | length' base.yaml 1 "base: one prune hook Job"
eq '[select(.kind == "Job")] | .[0].metadata.name' base.yaml tenant-platform-prune "base: prune Job name"
eq '[select(.kind == "Job")] | .[0].metadata.namespace' base.yaml tekton-pipelines "base: prune Job namespace"
eq '[select(.kind == "Job")] | .[0].metadata.annotations["helm.sh/hook"]' base.yaml post-install,post-upgrade "base: prune runs post-install/post-upgrade"
eq '[select(.kind == "Job")] | .[0].metadata.annotations["helm.sh/hook-delete-policy"]' base.yaml "before-hook-creation,hook-succeeded" "base: prune hook delete policy"
eq '[select(.kind == "Job")] | .[0].spec.template.spec.containers[0].image' base.yaml docker.io/alpine/kubectl:1.36.4 "base: pinned kubectl image"
eq '[select(.kind == "Job")] | .[0].spec.template.spec.securityContext.runAsNonRoot' base.yaml true "base: prune pod non-root"
eq '[select(.kind == "Job")] | .[0].spec.template.spec.securityContext.runAsUser' base.yaml 65532 "base: prune pod UID"
eq '[select(.kind == "Job")] | .[0].spec.template.spec.containers[0].securityContext.allowPrivilegeEscalation' base.yaml false "base: prune container no privilege escalation"
eq '[select(.kind == "Job")] | .[0].spec.template.spec.containers[0].securityContext.readOnlyRootFilesystem' base.yaml true "base: prune container read-only rootfs"
eq '[select(.kind == "Job")] | .[0].spec.template.spec.containers[0].securityContext.capabilities.drop | join(",")' base.yaml ALL "base: prune container drops ALL"
eq '[select(.kind == "ClusterRole" and .metadata.name == "tenant-platform-pruner")] | .[0].rules | length' base.yaml 2 "base: pruner ClusterRole has two rules"
eq '[select(.kind == "ClusterRole" and .metadata.name == "tenant-platform-pruner")] | .[0].rules | map(.verbs | sort | join(",")) | sort | join(";")' base.yaml "delete,list;delete,list" "base: pruner verbs are list+delete only"
eq '[select(.kind == "ClusterRole" and .metadata.name == "tenant-platform-pruner")] | .[0].rules | map(.resources | join(",")) | sort | join(";")' base.yaml "clusterrolebindings;namespaces" "base: pruner resources are namespaces+CRBs only"
eq '[select(.kind == "ClusterRoleBinding" and .metadata.name == "tenant-platform-pruner")] | .[0].subjects[0].namespace' base.yaml tekton-pipelines "base: pruner CRB subject namespace"
eq '[select(.kind == "ServiceAccount" and .metadata.name == "tenant-platform-pruner")] | .[0].metadata.namespace' base.yaml tekton-pipelines "base: pruner SA namespace"
eq '[select(.kind == "Job")] | .[0].spec.template.spec.containers[0].env | map(select(.name == "KEEP_NAMESPACES")) | .[0].value' base.yaml "stijnklomp-lave,stijnklomp-talos-linux-home-setup" "base: pruner keeps both registry namespaces"
eq '[select(.kind == "Job")] | .[0].spec.template.spec.containers[0].env | map(select(.name == "KEEP_DEVELOPERS")) | .[0].value' base.yaml stijnklomp1@hotmail.com "base: pruner keeps the deduped developer set"
eq '[select(.kind == "Secret")] | length' base.yaml 0 "base: no Secrets"

# --- Empty registry: removals are valid, only the pruner remains ---
eq '[select(.kind == "Namespace")] | length' empty.yaml 0 "empty: no tenant Namespace"
eq '[select(.kind == "Repository")] | length' empty.yaml 0 "empty: no Repository CR"
eq '[select(.kind == "Job")] | length' empty.yaml 1 "empty: pruner Job still renders"
eq '[select(.kind == "ValidatingAdmissionPolicy")] | length' empty.yaml 1 "empty: admission gate still renders"
eq '[select(.kind == "RoleBinding")] | length' empty.yaml 0 "empty: no developer RoleBindings"
eq '[select(.kind == "ClusterRoleBinding")] | map(select(.metadata.name | test("^tekton-namespace-lister-"))) | length' empty.yaml 0 "empty: no namespace-lister CRBs"

# --- TektonConfig (operator component) ---
eq 'select(.kind == "TektonConfig") | .spec.pipeline.set-security-context' tektonconfig.yaml true "operator: restricted security contexts enabled"
eq 'select(.kind == "TektonConfig") | .spec.pipeline["default-pod-template"] | from_yaml | .securityContext.fsGroup' tektonconfig.yaml 65532 "operator: pod fsGroup 65532"
eq 'select(.kind == "TektonConfig") | .spec.pipeline["default-pod-template"] | from_yaml | .securityContext.fsGroupChangePolicy' tektonconfig.yaml OnRootMismatch "operator: fsGroupChangePolicy"
eq 'select(.kind == "TektonConfig") | .spec.pipeline["default-pod-template"] | from_yaml | .securityContext | has("runAsNonRoot")' tektonconfig.yaml false "operator: no pod-level runAsNonRoot"
eq 'select(.kind == "TektonConfig") | .spec.pipeline["default-pod-template"] | from_yaml | .securityContext | has("runAsUser")' tektonconfig.yaml false "operator: no pod-level runAsUser"

# --- Multi-tenant fixture ---
eq '[select(.kind == "Namespace")] | length' multi.yaml 2 "multi: two Namespaces"
eq '[select(.kind == "Namespace")] | map(.metadata.name) | sort | join(",")' multi.yaml "acme-build,acme-deploy" "multi: namespace names"
eq '[select(.kind == "ResourceQuota" and .metadata.namespace == "acme-build")] | .[0].spec.hard.pods' multi.yaml 5 "multi: tenant quota override"
eq '[select(.kind == "ResourceQuota" and .metadata.namespace == "acme-build")] | .[0].spec.hard["requests.cpu"]' multi.yaml 4 "multi: build tier default quota"
eq '[select(.kind == "LimitRange" and .metadata.namespace == "acme-build")] | .[0].spec.limits[0].default.cpu' multi.yaml 1 "multi: tenant LimitRange override"
eq '[select(.kind == "LimitRange" and .metadata.namespace == "acme-build")] | .[0].spec.limits[0].default.memory' multi.yaml 2Gi "multi: LimitRange override deep-merges tier defaults"
eq '[select(.kind == "ResourceQuota" and .metadata.namespace == "acme-deploy")] | .[0].spec.hard["requests.cpu"]' multi.yaml 8 "multi: deploy tier default quota"
eq '[select(.kind == "ResourceQuota" and .metadata.namespace == "acme-deploy")] | .[0].spec.hard.pods' multi.yaml 40 "multi: deploy tier default pods"
eq '[select(.kind == "CiliumNetworkPolicy" and .metadata.namespace == "acme-build")] | .[0].spec.egress | length' multi.yaml 3 "multi: build CNP has no API allow"
eq '[select(.kind == "CiliumNetworkPolicy" and .metadata.namespace == "acme-deploy")] | .[0].spec.egress | length' multi.yaml 4 "multi: deploy CNP adds one allow"
eq '[select(.kind == "CiliumNetworkPolicy" and .metadata.namespace == "acme-deploy")] | .[0].spec.egress | any_c(.toEntities != null and .toEntities[0] == "kube-apiserver")' multi.yaml true "multi: deploy CNP allows kube-apiserver"
eq '[select(.kind == "Repository")] | length' multi.yaml 3 "multi: one Repository per repo"
eq '[select(.kind == "Repository" and .metadata.name == "infra")] | .[0].spec.concurrency_limit' multi.yaml 3 "multi: tenant concurrencyLimit applies to each repo"
eq '[select(.kind == "Repository" and .metadata.name == "widgets")] | .[0].spec.concurrency_limit' multi.yaml 2 "multi: default concurrencyLimit is 2"
eq '[select(.kind == "RoleBinding")] | length' multi.yaml 3 "multi: one RoleBinding per developer per tenant"
eq '[select(.kind == "RoleBinding" and .metadata.name == "tekton-developer-dev-example-com")] | length' multi.yaml 2 "multi: same developer bound in both tenants"
eq '[select(.kind == "ClusterRoleBinding")] | map(select(.metadata.name | test("^tekton-namespace-lister-"))) | length' multi.yaml 2 "multi: namespace-lister CRBs are deduped"
eq '[select(.kind == "ClusterRoleBinding")] | map(select(.metadata.name | test("^tekton-namespace-lister-"))) | map(.metadata.name) | sort | join(",")' multi.yaml "tekton-namespace-lister-dev-example-com,tekton-namespace-lister-ops-example-com" "multi: CRB names"
eq '[select(.kind == "ClusterRoleBinding")] | map(select(.metadata.name | test("^tekton-namespace-lister-"))) | .[0].subjects[0].kind' multi.yaml User "multi: CRB subject is a User"
eq '[select(.kind == "Job")] | .[0].spec.template.spec.containers[0].env | map(select(.name == "KEEP_NAMESPACES")) | .[0].value' multi.yaml "acme-build,acme-deploy" "multi: pruner keep-namespaces"
eq '[select(.kind == "Job")] | .[0].spec.template.spec.containers[0].env | map(select(.name == "KEEP_DEVELOPERS")) | .[0].value' multi.yaml "dev@example.com,ops@example.com" "multi: pruner keep-developers is deduped and sorted"
eq '[select(.kind == "Secret")] | length' multi.yaml 0 "multi: no Secrets"

exit "$fail"
EOS

echo "== helmfile output =="
docker run --rm -i \
  -v "${WORK}:/work:ro" \
  -w /work \
  --entrypoint /bin/sh \
  "${HELMFILE_IMAGE}" <<'EOS' || failure=$((failure + 1))
set -eu
fail=0
q() { yq ea -r "$1" "$2"; }
eq() {
  expr=$1; file=$2; want=$3; label=$4
  got=$(q "$expr" "$file" 2>/dev/null || true)
  if [ "$got" = "$want" ]; then
    echo "ok: $label"
  else
    echo "FAIL: $label (got '$got', want '$want')" >&2
    fail=1
  fi
}
eq '[select(.kind == "Namespace")] | length' helmfile-template.yaml 2 "helmfile: renders both registry namespaces"
eq '[select(.kind == "Repository")] | length' helmfile-template.yaml 3 "helmfile: renders one Repository per registry repo"
eq '[select(.kind == "Repository" and .metadata.namespace == "stijnklomp-lave")] | length' helmfile-template.yaml 1 "helmfile: build Repository in the build namespace"
eq '[select(.kind == "Repository" and .metadata.namespace == "stijnklomp-talos-linux-home-setup")] | length' helmfile-template.yaml 2 "helmfile: org Repositories in the deploy namespace"
exit "$fail"
EOS

echo "== prune hook logic (stub kubectl, no cluster) =="
PRUNER_IMAGE="${PRUNER_IMAGE:-docker.io/alpine/kubectl:1.36.4}"
mkdir -p "${WORK}/prune/bin"
cat >"${WORK}/prune/bin/kubectl" <<'STUB'
#!/bin/sh
set -eu
case "$1 $2" in
  "get namespaces")
    printf 'stijnklomp-lave\nstale-tenant\n'
    ;;
  "get clusterrolebindings")
    printf 'tekton-namespace-lister-dev-example-com|dev@example.com,\n'
    printf 'tekton-namespace-lister-gone-example-com|gone@example.com,\n'
    printf 'tenant-platform-pruner|,\n'
    ;;
  "delete namespace")
    echo "namespace $3" >>"${DELETE_LOG:?}"
    ;;
  "delete clusterrolebinding")
    echo "clusterrolebinding $3" >>"${DELETE_LOG:?}"
    ;;
  *)
    echo "stub kubectl: unexpected invocation: $*" >&2
    exit 99
    ;;
esac
STUB
chmod +x "${WORK}/prune/bin/kubectl"

docker run --rm -i \
  -v "${WORK}:/work" \
  -w /work \
  --entrypoint /bin/sh \
  "${HELMFILE_IMAGE}" <<'EOS'
set -eu
yq ea -r 'select(.kind == "Job") | .spec.template.spec.containers[0].args[0]' base.yaml > prune/prune.sh
chmod +x prune/prune.sh
EOS

run_pruner() {
  local keep_developers="$1" log="$2" keep_namespaces="${3-stijnklomp-lave}" allow_empty="${4:-false}" expect_fail="${5:-false}"
  : >"${WORK}/prune/${log}"
  docker run --rm \
    -v "${WORK}:/work" \
    -w /work/prune \
    -e DELETE_LOG="/work/prune/${log}" \
    -e KEEP_NAMESPACES="${keep_namespaces}" \
    -e ALLOW_EMPTY_REGISTRY="${allow_empty}" \
    -e KEEP_DEVELOPERS="${keep_developers}" \
    -e PATH="/work/prune/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin" \
    --entrypoint /bin/sh \
    "${PRUNER_IMAGE}" /work/prune/prune.sh \
    >"${WORK}/prune/${log}.out" 2>&1 || {
      if [ "${expect_fail}" != "true" ]; then
        bad "pruner run '${log}' failed"
        cat "${WORK}/prune/${log}.out" >&2
      fi
      return 1
    }
}

assert_deletes() {
  local label="$1" file="$2" expected="$3"
  local actual
  actual="$(sort "${WORK}/prune/${file}" 2>/dev/null || true)"
  if [ -n "${expected}" ]; then
    expected="$(printf '%s\n' "${expected}" | sort)"
  fi
  if [ "${actual}" = "${expected}" ]; then
    ok "${label}"
  else
    bad "${label}"
    printf '  got:\n%s\n  want:\n%s\n' "${actual}" "${expected}" >&2
  fi
}

if run_pruner "dev@example.com" deletes-with-developer; then
  assert_deletes "pruner keeps the listed namespace and developer" deletes-with-developer \
"$(printf 'clusterrolebinding tekton-namespace-lister-gone-example-com\nnamespace stale-tenant')"
fi
if run_pruner "" deletes-no-developers; then
  assert_deletes "pruner deletes every namespace-lister CRB when no developers remain" deletes-no-developers \
"$(printf 'clusterrolebinding tekton-namespace-lister-dev-example-com\nclusterrolebinding tekton-namespace-lister-gone-example-com\nnamespace stale-tenant')"
fi
if grep -qE 'stijnklomp-lave|tenant-platform-pruner$' "${WORK}/prune/deletes-no-developers" 2>/dev/null; then
  bad "pruner touched a kept namespace or an unrelated ClusterRoleBinding"
else
  ok "pruner leaves kept namespaces and unrelated CRBs alone"
fi

# An empty registry must be refused unless explicitly allowed (safety guard).
if run_pruner "" empty-refused "" false true; then
  bad "pruner refused an empty registry by default"
else
  ok "pruner refuses an empty registry by default"
fi
assert_deletes "pruner deleted nothing when refusing an empty registry" empty-refused ""

if run_pruner "" empty-allowed "" true; then
  assert_deletes "pruner can delete every managed namespace when explicitly allowed" empty-allowed \
"$(printf 'clusterrolebinding tekton-namespace-lister-dev-example-com\nclusterrolebinding tekton-namespace-lister-gone-example-com\nnamespace stijnklomp-lave\nnamespace stale-tenant')"
fi

echo "== acceptance manifest (static) =="
docker run --rm -i \
  -v "${COMPONENT_DIR}:/src:ro" \
  -w /src \
  --entrypoint /bin/sh \
  "${HELMFILE_IMAGE}" <<'EOS' || failure=$((failure + 1))
set -eu
fail=0
q() { yq ea -r "$1" "$2"; }
eq() {
  expr=$1; file=$2; want=$3; label=$4
  got=$(q "$expr" "$file" 2>/dev/null || true)
  if [ "$got" = "$want" ]; then
    echo "ok: $label"
  else
    echo "FAIL: $label" >&2
    echo "      got:  '$got'" >&2
    echo "      want: '$want'" >&2
    fail=1
  fi
}
f=tests/build-tenant-pipelinerun.yaml
eq 'select(.kind == "PipelineRun") | .metadata.namespace' "$f" stijnklomp-lave "acceptance: runs in the tenant namespace"
eq 'select(.kind == "PipelineRun") | .spec.taskRunTemplate.serviceAccountName' "$f" tekton-bot "acceptance: uses tekton-bot"
eq 'select(.kind == "PipelineRun") | .spec.taskRunTemplate.podTemplate.securityContext.fsGroup' "$f" 65532 "acceptance: workspace fsGroup"
eq 'select(.kind == "PipelineRun") | has("pipelineRef")' "$f" false "acceptance: no shared Pipeline dependency"
eq 'select(.kind == "PipelineRun") | .spec.pipelineSpec.tasks | length' "$f" 2 "acceptance: clone + probe"
eq 'select(.kind == "PipelineRun") | .spec.pipelineSpec.tasks[0].taskRef.resolver' "$f" cluster "acceptance: clone uses the cluster resolver"
eq 'select(.kind == "PipelineRun") | .spec.pipelineSpec.tasks[0].taskRef.params | map(select(.name == "name")) | .[0].value' "$f" git-clone-v1 "acceptance: clone uses git-clone-v1"
eq 'select(.kind == "PipelineRun") | .spec.pipelineSpec.tasks[0].taskRef.params | map(select(.name == "namespace")) | .[0].value' "$f" tekton-pipelines "acceptance: tasks resolve from tekton-pipelines"
eq 'select(.kind == "PipelineRun") | .spec.pipelineSpec.tasks[1] | has("taskSpec")' "$f" true "acceptance: probe is inline"
eq 'select(.kind == "PipelineRun") | .spec.pipelineSpec.tasks[1] | has("taskRef")' "$f" false "acceptance: probe does not use a shared task"
eq 'select(.kind == "PipelineRun") | .spec.pipelineSpec.tasks[1].taskSpec.steps[0].image' "$f" docker.io/curlimages/curl:8.23.0 "acceptance: pinned probe image"
eq 'select(.kind == "PipelineRun") | .spec.pipelineSpec.tasks[1].taskSpec.steps[0].securityContext.runAsNonRoot' "$f" true "acceptance: probe is non-root"
eq 'select(.kind == "PipelineRun") | .spec.pipelineSpec.tasks[1].taskSpec.steps[0].securityContext.allowPrivilegeEscalation' "$f" false "acceptance: probe cannot escalate"
eq 'select(.kind == "PipelineRun") | .spec.pipelineSpec.tasks[1].taskSpec.steps[0].securityContext.capabilities.drop | join(",")' "$f" ALL "acceptance: probe drops ALL"
eq 'select(.kind == "PipelineRun") | .spec.workspaces[0] | has("emptyDir")' "$f" true "acceptance: emptyDir workspace, no PVC"
for marker in \
  "cluster DNS resolves" \
  "Verdaccio" \
  "internet" \
  "Kubernetes API" \
  "home LAN" \
  "another namespace"
do
  eq "select(.kind == \"PipelineRun\") | .spec.pipelineSpec.tasks[1].taskSpec.steps[0].script | contains(\"${marker}\")" "$f" true "acceptance: probe asserts '${marker}'"
done

f=tests/deploy-tenant-pipelinerun.yaml
eq 'select(.kind == "PipelineRun") | .metadata.namespace' "$f" stijnklomp-talos-linux-home-setup "deploy acceptance: runs in the org namespace"
eq 'select(.kind == "PipelineRun") | .spec.taskRunTemplate.serviceAccountName' "$f" tekton-bot "deploy acceptance: uses tekton-bot"
eq 'select(.kind == "PipelineRun") | .spec.taskRunTemplate.podTemplate.securityContext.fsGroup' "$f" 65532 "deploy acceptance: workspace fsGroup"
eq 'select(.kind == "PipelineRun") | has("pipelineRef")' "$f" false "deploy acceptance: no shared Pipeline dependency"
eq 'select(.kind == "PipelineRun") | .spec.pipelineSpec.tasks | length' "$f" 1 "deploy acceptance: one inline probe"
eq 'select(.kind == "PipelineRun") | .spec.pipelineSpec.tasks[0] | has("taskSpec")' "$f" true "deploy acceptance: probe is inline"
eq 'select(.kind == "PipelineRun") | .spec.pipelineSpec.tasks[0] | has("taskRef")' "$f" false "deploy acceptance: probe does not use a shared task"
eq 'select(.kind == "PipelineRun") | .spec.pipelineSpec.tasks[0].taskSpec.steps[0].image' "$f" docker.io/curlimages/curl:8.23.0 "deploy acceptance: pinned probe image"
eq 'select(.kind == "PipelineRun") | .spec.pipelineSpec.tasks[0].taskSpec.steps[0].securityContext.runAsNonRoot' "$f" true "deploy acceptance: probe is non-root"
eq 'select(.kind == "PipelineRun") | .spec.pipelineSpec.tasks[0].taskSpec.steps[0].securityContext.allowPrivilegeEscalation' "$f" false "deploy acceptance: probe cannot escalate"
eq 'select(.kind == "PipelineRun") | .spec.pipelineSpec.tasks[0].taskSpec.steps[0].securityContext.capabilities.drop | join(",")' "$f" ALL "deploy acceptance: probe drops ALL"
for marker in \
  "Kubernetes API reachable" \
  "cluster DNS resolves" \
  "Verdaccio" \
  "internet"
do
  eq "select(.kind == \"PipelineRun\") | .spec.pipelineSpec.tasks[0].taskSpec.steps[0].script | contains(\"${marker}\")" "$f" true "deploy acceptance: probe asserts '${marker}'"
done
exit "$fail"
EOS

echo
if [ "${failure}" -ne 0 ]; then
  printf 'FAILED: %d check group(s) reported failures\n' "${failure}" >&2
  exit 1
fi
printf 'PASSED: all offline checks\n'
