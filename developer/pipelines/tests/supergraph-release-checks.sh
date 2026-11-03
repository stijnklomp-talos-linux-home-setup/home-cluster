#!/usr/bin/env bash
# Offline checks for ticket #18 (schema change reaches the gateway) in the
# shared Tekton catalog.
#
# Asserts the rendered chart keeps its contract:
#   - the compose Task pins Rover/the ELv2 plugin, uses --skip-update, deletes a
#     stale artifact and composes to a temp file before moving it into place;
#   - the ConfigMap Task refuses an empty artifact, renders to a file (bash +
#     pipefail) and applies it, and only updates an existing object;
#   - the Pipeline wires clone *or* seed (when repo-url is empty) -> compose ->
#     publish, and publish runs only after compose;
#   - the seed Task unpacks the base64 tar ConfigMap into the source PVC;
#   - the ServiceAccount/Role/RoleBinding land in the ConfigMap namespace
#     (home-graphql by default), the Role is scoped to that ConfigMap and has no
#     namespace-wide `create`;
#   - every Task/Pipeline default (names, namespace, key, images, plugin) comes
#     from chart values, so an override cannot fork RBAC and publisher;
#   - the PipelineRun example binds the persistent seeded PVC, not a per-run
#     volumeClaimTemplate, and documents the seed command;
#   - helm lint and helmfile template pass.
#
# Usage (Docker-first, nothing installed on the host):
#   ./tests/supergraph-release-checks.sh
set -euo pipefail

REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
HELM_IMAGE="${HELM_IMAGE:-alpine/helm:latest}"
HELMFILE_IMAGE="${HELMFILE_IMAGE:-ghcr.io/helmfile/helmfile:latest}"
CHART="./charts/supergraph-release"

WORK="$(mktemp -d /tmp/cicd-supergraph-release.XXXXXX)"
trap 'rm -rf "${WORK}"' EXIT

failure=0
ok() { printf 'ok: %s\n' "$1"; }
bad() {
  failure=$((failure + 1))
  printf 'FAIL: %s\n' "$1" >&2
}

cp "${REPO_DIR}/charts/supergraph-release/Chart.yaml" "${WORK}/Chart.yaml"
cp "${REPO_DIR}/charts/supergraph-release/usage-examples/v1/pipelinerun.yaml" "${WORK}/pipelinerun.yaml"
cp "${REPO_DIR}/charts/supergraph-release/README.md" "${WORK}/README.md"

docker run --rm -v "${REPO_DIR}:/src:ro" -w /src \
  "${HELM_IMAGE}" template supergraph-release "${CHART}" \
  >"${WORK}/rendered.yaml"

# Same chart with every value that used to be dead overridden. Proves the
# Tasks/Pipeline/RBAC/PVC all read the values instead of hardcoding.
docker run --rm -v "${REPO_DIR}:/src:ro" -w /src \
  "${HELM_IMAGE}" template supergraph-release "${CHART}" \
  --set-string images.rover=example.test/rover:9.9.9 \
  --set-string rover.plugin=supergraph@=9.9.9 \
  --set-string images.kubectl=example.test/kubectl:9.9.9 \
  --set-string configMap.name=custom-supergraph \
  --set-string configMap.namespace=custom-ns \
  --set-string configMap.key=custom.graphql \
  --set-string serviceAccount.name=custom-sa \
  --set-string source.pvc.name=custom-source \
  --set-string source.seed.configMapName=custom-seed \
  >"${WORK}/rendered-override.yaml"

yq_query() {
  local expr="$1" file="$2"
  docker run --rm -i -v "${WORK}:/work:ro" -w /work \
    --entrypoint /bin/sh "${HELMFILE_IMAGE}" -c 'yq ea -r "$1" "$2"' sh "${expr}" "${file}"
}

echo "== Tasks and Pipeline render =="
for name in supergraph-seed-v1 supergraph-compose-v1 supergraph-configmap-v1 supergraph-release-v1; do
  if grep -q "name: ${name}$" "${WORK}/rendered.yaml"; then
    ok "${name}"
  else
    bad "${name} missing from the rendered chart"
  fi
done

chart_app_version="$(yq_query '.appVersion' Chart.yaml)"
if [ -n "${chart_app_version}" ] && \
   [ "$(yq_query 'select(.kind == "Task") | .metadata.labels."app.kubernetes.io/version"' rendered.yaml | grep -v '^---$' | sort -u)" = "${chart_app_version}" ]; then
  ok "Task labels match Chart appVersion (${chart_app_version})"
else
  bad "Task app.kubernetes.io/version labels do not match Chart appVersion (${chart_app_version:-<none>})"
fi

echo "== compose Task is pinned and publishes only on success =="
compose_task="$(yq_query 'select(.kind == "Task" and .metadata.name == "supergraph-compose-v1") | .spec' "rendered.yaml")"
for needle in 'supergraph@=2.9.3' 'ghcr.io/apollographql/rover:1.0.1' '--skip-update' '--elv2-license accept' 'mv ' 'mktemp -d' 'set -euo pipefail' 'rm -f "${artifact}"' '.rover-home' 'trap '; do
  if printf '%s' "${compose_task}" | grep -qF -- "${needle}"; then
    ok "compose task contains ${needle}"
  else
    bad "compose task is missing ${needle}"
  fi
done
if printf '%s' "${compose_task}" | grep -qF -- '--output "${work}/' ; then
  ok "compose writes to a temp file first"
else
  bad "compose does not compose into a temp file"
fi
if [ "$(yq_query 'select(.kind == "Task" and .metadata.name == "supergraph-compose-v1") | .spec.steps[0].securityContext.runAsUser' "rendered.yaml")" = "65532" ]; then
  ok "compose step runs as UID 65532"
else
  bad "compose step does not run as UID 65532"
fi

echo "== seed Task unpacks the checkout into the PVC =="
seed_task="$(yq_query 'select(.kind == "Task" and .metadata.name == "supergraph-seed-v1") | .spec' "rendered.yaml")"
for needle in 'base64 -d' 'tar -xzf' '.rover-home' 'set -euo pipefail' 'mktemp -d' 'trap '; do
  if printf '%s' "${seed_task}" | grep -qF -- "${needle}"; then
    ok "seed task contains ${needle}"
  else
    bad "seed task is missing ${needle}"
  fi
done
if [ "$(yq_query 'select(.kind == "Task" and .metadata.name == "supergraph-seed-v1") | .spec.params[] | select(.name == "SEED_CONFIGMAP") | .default' "rendered.yaml")" = "supergraph-source" ] \
   && [ "$(yq_query 'select(.kind == "Task" and .metadata.name == "supergraph-seed-v1") | .spec.volumes[] | select(.name == "seed") | .configMap.name' "rendered.yaml")" = '$(params.SEED_CONFIGMAP)' ]; then
  ok "seed volume mounts the supergraph-source ConfigMap via its parameter"
else
  bad "seed volume does not mount the supergraph-source ConfigMap"
fi

echo "== ConfigMap Task refuses empty artifacts and applies declaratively =="
configmap_task="$(yq_query 'select(.kind == "Task" and .metadata.name == "supergraph-configmap-v1") | .spec' "rendered.yaml")"
for needle in 'Refusing to touch' '--dry-run=client' 'kubectl apply -f' 'configmap-sha256' 'set -euo pipefail' 'mktemp -d' 'does not exist'; do
  if printf '%s' "${configmap_task}" | grep -qF -- "${needle}"; then
    ok "configmap task contains ${needle}"
  else
    bad "configmap task is missing ${needle}"
  fi
done

echo "== Pipeline ordering, seed alternative and manual trigger =="
pipeline="$(yq_query 'select(.kind == "Pipeline") | .spec' "rendered.yaml")"
if printf '%s' "${pipeline}" | grep -q 'operator: notin'; then
  ok "clone is skipped when repo-url is empty"
else
  bad "clone has no repo-url guard"
fi
if [ "$(yq_query 'select(.kind == "Pipeline") | .spec.tasks[] | select(.name == "seed") | .taskRef.params[] | select(.name == "name") | .value' "rendered.yaml")" = "supergraph-seed-v1" ]; then
  ok "pipeline runs the seed Task when repo-url is empty"
else
  bad "pipeline does not wire supergraph-seed-v1"
fi
seed_whens="$(yq_query 'select(.kind == "Pipeline") | .spec.tasks[] | select(.name == "seed") | .when[] | "\(.input) \(.operator) \(.values[0])"' "rendered.yaml")"
if printf '%s' "${seed_whens}" | grep -q 'repo-url) in ' \
   && printf '%s' "${seed_whens}" | grep -q 'source-configmap) notin '; then
  ok "seed runs only when repo-url is empty and source-configmap is set"
else
  bad "seed when-conditions are wrong: ${seed_whens}"
fi
publish_run_after="$(yq_query 'select(.kind == "Pipeline") | .spec.tasks[] | select(.name == "publish-configmap") | .runAfter[0]' "rendered.yaml")"
if [ "${publish_run_after}" = "compose-supergraph" ]; then
  ok "publish-configmap runs after compose-supergraph"
else
  bad "publish-configmap ordering is wrong: ${publish_run_after}"
fi
compose_run_after="$(yq_query 'select(.kind == "Pipeline") | .spec.tasks[] | select(.name == "compose-supergraph") | .runAfter[]' "rendered.yaml" | sort | tr '\n' ' ')"
if [ "${compose_run_after}" = "clone seed " ]; then
  ok "compose-supergraph waits for clone and seed"
else
  bad "compose-supergraph runAfter is wrong: ${compose_run_after}"
fi
if grep -q 'pipelinesascode.tekton.dev/on-' "${WORK}/rendered.yaml"; then
  bad "chart renders a PaC trigger annotation (composition is manual)"
else
  ok "no PaC trigger annotations (manual only)"
fi
if printf '%s' "${pipeline}" | grep -qi 'breaking'; then
  ok "pipeline documents the missing breaking-change gate"
else
  bad "pipeline description does not mention the breaking-change gate"
fi
if printf '%s' "${pipeline}" | grep -qF 'config-map-namespace'; then
  ok "pipeline exposes the target namespace as a parameter"
else
  bad "pipeline does not expose config-map-namespace"
fi

echo "== RBAC stays in the ConfigMap namespace and is scoped =="
sa_ns="$(yq_query 'select(.kind == "ServiceAccount") | .metadata.namespace' "rendered.yaml")"
if [ "${sa_ns}" = "home-graphql" ]; then
  ok "ServiceAccount renders in home-graphql"
else
  bad "ServiceAccount namespace is ${sa_ns:-<none>}"
fi
if [ "$(yq_query 'select(.kind == "Role") | .metadata.name' "rendered.yaml")" = "supergraph-release" ]; then
  ok "Role is supergraph-release"
else
  bad "Role is missing or wrongly named"
fi
if [ "$(yq_query 'select(.kind == "Role") | .rules[] | select(.resourceNames != null) | .resourceNames[0]' "rendered.yaml")" = "home-graphql-supergraph" ]; then
  ok "get/update/patch are scoped to the supergraph ConfigMap"
else
  bad "Role is not scoped to the supergraph ConfigMap"
fi
if yq_query 'select(.kind == "Role") | .rules[].verbs[]' "rendered.yaml" | grep -qx 'create'; then
  bad "Role grants namespace-wide create on ConfigMaps"
else
  ok "Role has no create verb (cannot write other ConfigMaps)"
fi
if grep -q 'namespace: tekton-pipelines' "${WORK}/rendered.yaml" \
   && [ "$(yq_query 'select(.kind == "ServiceAccount") | .metadata.namespace' "rendered.yaml")" != "tekton-pipelines" ]; then
  ok "Tasks stay in tekton-pipelines, RBAC does not"
else
  bad "tenant/control-plane namespace separation is broken"
fi

echo "== source PVC and PipelineRun example agree =="
pvc_name="$(yq_query 'select(.kind == "PersistentVolumeClaim") | .metadata.name' "rendered.yaml")"
if [ "${pvc_name}" = "supergraph-release-source" ]; then
  ok "chart renders the supergraph-release-source PVC"
else
  bad "chart source PVC is ${pvc_name:-<none>}"
fi
if [ "$(yq_query 'select(.kind == "PersistentVolumeClaim") | .metadata.namespace' "rendered.yaml")" = "home-graphql" ]; then
  ok "source PVC renders in the ConfigMap namespace"
else
  bad "source PVC namespace is wrong"
fi
example_claim="$(yq_query '.spec.workspaces[] | select(.name == "source") | .persistentVolumeClaim.claimName' pipelinerun.yaml)"
if [ "${example_claim}" = "${pvc_name}" ]; then
  ok "PipelineRun example binds the chart PVC (${example_claim})"
else
  bad "PipelineRun example binds '${example_claim:-<none>}', chart renders '${pvc_name:-<none>}'"
fi
if [ "$(yq_query '.spec.workspaces[] | select(.name == "source") | has("volumeClaimTemplate")' pipelinerun.yaml)" = "false" ]; then
  ok "PipelineRun example does not use a per-run volumeClaimTemplate"
else
  bad "PipelineRun example uses a per-run volumeClaimTemplate (empty workspace, no seed, no cache)"
fi
if [ "$(yq_query '.spec.params[] | select(.name == "repo-url") | .value' pipelinerun.yaml)" = "" ] \
   && [ "$(yq_query '.spec.params[] | select(.name == "source-configmap") | .value' pipelinerun.yaml)" != "" ]; then
  ok "PipelineRun example documents the seeded path (repo-url empty + source-configmap set)"
else
  bad "PipelineRun example has no consistent seeded path"
fi
if grep -q 'create configmap supergraph-source' "${WORK}/pipelinerun.yaml" \
   && grep -q 'create configmap supergraph-source' "${WORK}/README.md"; then
  ok "seed command documented in the example and the README"
else
  bad "seed command is not documented"
fi
if grep -q 'GitOps deviation' "${WORK}/README.md" && grep -q 'OutOfSync' "${WORK}/README.md"; then
  ok "README documents the ArgoCD OutOfSync deviation"
else
  bad "README does not document the GitOps/ArgoCD deviation"
fi

echo "== chart values drive every Task/Pipeline default =="
override_compose_image="$(yq_query 'select(.kind == "Task" and .metadata.name == "supergraph-compose-v1") | .spec.params[] | select(.name == "ROVER_IMAGE") | .default' "rendered-override.yaml")"
override_plugin="$(yq_query 'select(.kind == "Task" and .metadata.name == "supergraph-compose-v1") | .spec.params[] | select(.name == "ROVER_PLUGIN") | .default' "rendered-override.yaml")"
override_key="$(yq_query 'select(.kind == "Task" and .metadata.name == "supergraph-configmap-v1") | .spec.params[] | select(.name == "KEY") | .default' "rendered-override.yaml")"
override_pipeline_name="$(yq_query 'select(.kind == "Pipeline") | .spec.params[] | select(.name == "config-map-name") | .default' "rendered-override.yaml")"
override_pipeline_ns="$(yq_query 'select(.kind == "Pipeline") | .spec.params[] | select(.name == "config-map-namespace") | .default' "rendered-override.yaml")"
override_role="$(yq_query 'select(.kind == "Role") | .rules[] | select(.resourceNames != null) | .resourceNames[0]' "rendered-override.yaml")"
override_pvc="$(yq_query 'select(.kind == "PersistentVolumeClaim") | .metadata.name' "rendered-override.yaml")"
override_seed="$(yq_query 'select(.kind == "Task" and .metadata.name == "supergraph-seed-v1") | .spec.params[] | select(.name == "SEED_CONFIGMAP") | .default' "rendered-override.yaml")"
override_sa="$(yq_query 'select(.kind == "ServiceAccount") | .metadata.name' "rendered-override.yaml")"
if [ "${override_compose_image}" = "example.test/rover:9.9.9" ] \
   && [ "${override_plugin}" = "supergraph@=9.9.9" ] \
   && [ "${override_key}" = "custom.graphql" ]; then
  ok "compose Task image/plugin and ConfigMap key follow values"
else
  bad "compose/ConfigMap Task defaults do not follow values"
fi
if [ "${override_pipeline_name}" = "custom-supergraph" ] \
   && [ "${override_pipeline_ns}" = "custom-ns" ] \
   && [ "${override_role}" = "custom-supergraph" ]; then
  ok "Pipeline default and RBAC resourceNames follow configMap.* values"
else
  bad "Pipeline default and RBAC resourceNames diverge (name=${override_pipeline_name}, ns=${override_pipeline_ns}, role=${override_role})"
fi
if [ "${override_pvc}" = "custom-source" ] \
   && [ "${override_seed}" = "custom-seed" ] \
   && [ "${override_sa}" = "custom-sa" ]; then
  ok "PVC, seed ConfigMap and ServiceAccount names follow values"
else
  bad "PVC/seed/ServiceAccount names do not follow values"
fi

echo "== helm lint =="
if docker run --rm -v "${REPO_DIR}:/src:ro" -w /src "${HELM_IMAGE}" lint "${CHART}" >/dev/null; then
  ok "helm lint"
else
  bad "helm lint"
fi

echo "== helmfile template resolves the release =="
if docker run --rm -v "${REPO_DIR}:/src:ro" -w /src \
    "${HELMFILE_IMAGE}" helmfile template --values values/platform.yaml \
    >"${WORK}/helmfile.yaml" 2>"${WORK}/helmfile.err"; then
  ok "helmfile template"
else
  bad "helmfile template"
  sed 's/^/    /' "${WORK}/helmfile.err" >&2
fi
if grep -q 'name: supergraph-release-v1' "${WORK}/helmfile.yaml"; then
  ok "helmfile renders the supergraph-release pipeline"
else
  bad "helmfile does not render the supergraph-release pipeline"
fi

echo
if [ "${failure}" -ne 0 ]; then
  printf 'FAILED: %d supergraph-release check(s) failed\n' "${failure}" >&2
  exit 1
fi
printf 'PASSED: supergraph-release chart contract holds\n'
