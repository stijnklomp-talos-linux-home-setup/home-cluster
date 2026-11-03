#!/usr/bin/env bash
# verify-isolation.sh — prove that Cilium enforces the tenant egress policy.
#
# This is the acceptance test for the CNI change (ticket #2). It is expected to
# FAIL on a Flannel cluster: Flannel ignores CiliumNetworkPolicy, so the probes
# that must be denied (API server, home LAN, another namespace) succeed.
#
# Run from the workspace root, i.e. the directory that contains
# home-cluster-1-config/:
#
#   platform/cluster/home-cilium/verify-isolation.sh
#
# Options:
#   --render   print the manifests to stdout and exit (no cluster access)
#   --keep     keep t2-verify / t2-verify-peer after the run for debugging
#   --help     show usage
#
# Environment overrides:
#   CLUSTER_CONFIG_DIR  directory holding kubeconfig (default: $PWD/home-cluster-1-config)
#   KUBECTL_IMAGE       kubectl image (default: registry.k8s.io/kubectl:v1.36.4)
#   PROBE_IMAGE         probe image (default: curlimages/curl:8.23.0)
#   PEER_IMAGE          peer image (default: busybox:1.36)
#   KEEP_NAMESPACES     set to 1 instead of --keep
#
# The script is idempotent: it recreates the scratch namespaces, policy, peer
# and Job, then deletes the namespaces on exit.

set -euo pipefail

NAMESPACE="${NAMESPACE:-t2-verify}"
PEER_NAMESPACE="${PEER_NAMESPACE:-t2-verify-peer}"
JOB_NAME="isolation-probe"
PEER_NAME="peer"

CLUSTER_CONFIG_DIR="${CLUSTER_CONFIG_DIR:-${PWD}/home-cluster-1-config}"
KUBECTL_IMAGE="${KUBECTL_IMAGE:-registry.k8s.io/kubectl:v1.36.4}"
PROBE_IMAGE="${PROBE_IMAGE:-curlimages/curl:8.23.0}"
PEER_IMAGE="${PEER_IMAGE:-busybox:1.36}"

KEEP_NAMESPACES="${KEEP_NAMESPACES:-0}"

# The node IPs the policy must deny. These are the real home-cluster-1 node
# addresses documented in platform/node/AGENTS.md.
CONTROL_PLANE_IP="${CONTROL_PLANE_IP:-192.168.1.107}"
INGRESS_IP="${INGRESS_IP:-192.168.1.200}"

usage() {
  cat <<'EOF'
verify-isolation.sh — prove that Cilium enforces the tenant egress policy.

This is the acceptance test for the CNI change (ticket #2). It is expected to
FAIL on a Flannel cluster: Flannel ignores CiliumNetworkPolicy, so the probes
that must be denied (API server, home LAN, another namespace) succeed.

Run from the workspace root, i.e. the directory that contains
home-cluster-1-config/:

  platform/cluster/home-cilium/verify-isolation.sh

Options:
  --render   print the manifests to stdout and exit (no cluster access)
  --keep     keep t2-verify / t2-verify-peer after the run for debugging
  --help     show usage

Environment overrides:
  CLUSTER_CONFIG_DIR  directory holding kubeconfig (default: $PWD/home-cluster-1-config)
  KUBECTL_IMAGE       kubectl image (default: registry.k8s.io/kubectl:v1.36.4)
  PROBE_IMAGE         probe image (default: curlimages/curl:8.23.0)
  PEER_IMAGE          peer image (default: busybox:1.36)
  KEEP_NAMESPACES     set to 1 instead of --keep

The script is idempotent: it recreates the scratch namespaces, policy, peer
and Job, then deletes the namespaces on exit.
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

render_setup() {
  sed -e "s|__NAMESPACE__|${NAMESPACE}|g" \
      -e "s|__PEER_NAMESPACE__|${PEER_NAMESPACE}|g" \
      -e "s|__PEER_NAME__|${PEER_NAME}|g" \
      -e "s|__PEER_IMAGE__|${PEER_IMAGE}|g" <<'YAML'
---
apiVersion: v1
kind: Namespace
metadata:
  name: __NAMESPACE__
  labels:
    pod-security.kubernetes.io/enforce: restricted
    pod-security.kubernetes.io/enforce-version: latest
    pod-security.kubernetes.io/audit: restricted
    pod-security.kubernetes.io/audit-version: latest
    pod-security.kubernetes.io/warn: restricted
    pod-security.kubernetes.io/warn-version: latest
    home.cluster/verify: "true"
---
apiVersion: v1
kind: Namespace
metadata:
  name: __PEER_NAMESPACE__
  labels:
    pod-security.kubernetes.io/enforce: restricted
    pod-security.kubernetes.io/enforce-version: latest
    pod-security.kubernetes.io/audit: restricted
    pod-security.kubernetes.io/audit-version: latest
    pod-security.kubernetes.io/warn: restricted
    pod-security.kubernetes.io/warn-version: latest
    home.cluster/verify: "true"
---
# Verification copy of the tenant policy rendered by
# platform/cicd/tenant-platform (chart
# templates/networkpolicy.yaml):
# ingress only from the host, egress only DNS/Verdaccio/world, and explicit
# denies for RFC1918 and link-local. No kube-apiserver allow: this is a
# `build` tenant.
apiVersion: cilium.io/v2
kind: CiliumNetworkPolicy
metadata:
  name: tenant-isolation
  namespace: __NAMESPACE__
spec:
  description: >-
    Tenant isolation probe: allow DNS, Verdaccio and world; deny RFC1918,
    link-local and everything else (including the Kubernetes API).
  endpointSelector: {}
  ingress:
    - fromEntities:
        - host
  egress:
    - toEndpoints:
        - matchLabels:
            k8s:io.kubernetes.pod.namespace: kube-system
            k8s:k8s-app: kube-dns
      toPorts:
        - ports:
            - port: "53"
              protocol: UDP
            - port: "53"
              protocol: TCP
    - toEndpoints:
        - matchLabels:
            k8s:io.kubernetes.pod.namespace: verdaccio
      toPorts:
        - ports:
            - port: "4873"
              protocol: TCP
    - toEntities:
        - world
  egressDeny:
    - toCIDR:
        - 192.168.0.0/16
    - toCIDR:
        - 172.16.0.0/12
    - toCIDR:
        - 169.254.0.0/16
---
# The peer answers HTTP on 8080 so a reachable peer is unambiguous (a bare
# TCP listener would make curl time out even when the packet got through).
apiVersion: apps/v1
kind: Deployment
metadata:
  name: __PEER_NAME__
  namespace: __PEER_NAMESPACE__
  labels:
    app: __PEER_NAME__
spec:
  replicas: 1
  selector:
    matchLabels:
      app: __PEER_NAME__
  template:
    metadata:
      labels:
        app: __PEER_NAME__
    spec:
      automountServiceAccountToken: false
      securityContext:
        runAsNonRoot: true
        runAsUser: 65534
        runAsGroup: 65534
        seccompProfile:
          type: RuntimeDefault
      containers:
        - name: http
          image: __PEER_IMAGE__
          imagePullPolicy: IfNotPresent
          command:
            - sh
            - -c
            - exec httpd -f -p 8080 -h /tmp
          ports:
            - name: http
              containerPort: 8080
          readinessProbe:
            tcpSocket:
              port: 8080
            periodSeconds: 2
          resources:
            requests:
              cpu: 10m
              memory: 16Mi
            limits:
              cpu: 100m
              memory: 64Mi
          securityContext:
            allowPrivilegeEscalation: false
            readOnlyRootFilesystem: true
            capabilities:
              drop:
                - ALL
          volumeMounts:
            - name: tmp
              mountPath: /tmp
      volumes:
        - name: tmp
          emptyDir: {}
---
apiVersion: v1
kind: Service
metadata:
  name: __PEER_NAME__
  namespace: __PEER_NAMESPACE__
spec:
  selector:
    app: __PEER_NAME__
  ports:
    - name: http
      port: 8080
      targetPort: http
YAML
}

render_probe() {
  sed -e "s|__NAMESPACE__|${NAMESPACE}|g" \
      -e "s|__PEER_NAMESPACE__|${PEER_NAMESPACE}|g" \
      -e "s|__PROBE_IMAGE__|${PROBE_IMAGE}|g" \
      -e "s|__CONTROL_PLANE_IP__|${CONTROL_PLANE_IP}|g" \
      -e "s|__INGRESS_IP__|${INGRESS_IP}|g" <<'YAML'
---
apiVersion: batch/v1
kind: Job
metadata:
  name: isolation-probe
  namespace: __NAMESPACE__
spec:
  backoffLimit: 0
  activeDeadlineSeconds: 120
  ttlSecondsAfterFinished: 600
  template:
    metadata:
      labels:
        app: isolation-probe
    spec:
      automountServiceAccountToken: false
      restartPolicy: Never
      securityContext:
        runAsNonRoot: true
        runAsUser: 100
        runAsGroup: 101
        seccompProfile:
          type: RuntimeDefault
      containers:
        - name: probe
          image: __PROBE_IMAGE__
          imagePullPolicy: IfNotPresent
          command:
            - /bin/sh
            - -c
          args:
            - |
              set -u
              fail=0
              pass() { printf 'PASS %s\n' "$*"; }
              bad()  { printf 'FAIL %s\n' "$*"; fail=1; }

              echo "== must be reachable =="
              if nslookup verdaccio.verdaccio.svc.cluster.local >/dev/null 2>&1; then
                pass "DNS resolves verdaccio.verdaccio.svc.cluster.local"
              else
                bad "DNS does not resolve verdaccio.verdaccio.svc.cluster.local"
              fi

              if curl -sS --connect-timeout 5 --max-time 10 -o /dev/null \
                  http://verdaccio.verdaccio.svc.cluster.local:4873/; then
                pass "Verdaccio TCP 4873 reachable"
              else
                bad "Verdaccio TCP 4873 NOT reachable"
              fi

              if curl -sS --connect-timeout 5 --max-time 15 -o /dev/null \
                  https://registry.npmjs.org/; then
                pass "public HTTPS registry.npmjs.org reachable"
              else
                bad "public HTTPS registry.npmjs.org NOT reachable"
              fi

              echo
              echo "== must be unreachable =="
              must_fail() {
                label="$1"
                shift
                if "$@" >/dev/null 2>&1; then
                  bad "${label} reachable (policy NOT enforced)"
                else
                  pass "${label} denied"
                fi
              }

              must_fail "Kubernetes API service kubernetes.default.svc:443" \
                curl -k -sS --connect-timeout 5 --max-time 8 -o /dev/null \
                  https://kubernetes.default.svc/healthz
              must_fail "Kubernetes API node __CONTROL_PLANE_IP__:6443" \
                curl -k -sS --connect-timeout 5 --max-time 8 -o /dev/null \
                  https://__CONTROL_PLANE_IP__:6443/healthz
              must_fail "Talos API __CONTROL_PLANE_IP__:50000" \
                nc -z -w 3 __CONTROL_PLANE_IP__ 50000
              must_fail "home LAN node __CONTROL_PLANE_IP__:443" \
                curl -k -sS --connect-timeout 5 --max-time 8 -o /dev/null \
                  https://__CONTROL_PLANE_IP__/
              must_fail "home LAN ingress __INGRESS_IP__:443" \
                curl -k -sS --connect-timeout 5 --max-time 8 -o /dev/null \
                  https://__INGRESS_IP__/
              must_fail "link-local metadata 169.254.169.254" \
                curl -sS --connect-timeout 5 --max-time 8 -o /dev/null \
                  http://169.254.169.254/latest/meta-data/
              must_fail "pod in peer namespace" \
                curl -sS --connect-timeout 5 --max-time 8 -o /dev/null \
                  http://peer.__PEER_NAMESPACE__.svc.cluster.local:8080/

              echo
              if [ "$fail" -eq 0 ]; then
                echo "RESULT: PASS - tenant isolation holds"
                exit 0
              fi
              echo "RESULT: FAIL - tenant isolation is NOT enforced"
              exit 1
          resources:
            requests:
              cpu: 50m
              memory: 32Mi
            limits:
              cpu: 200m
              memory: 128Mi
          securityContext:
            allowPrivilegeEscalation: false
            readOnlyRootFilesystem: true
            capabilities:
              drop:
                - ALL
          volumeMounts:
            - name: tmp
              mountPath: /tmp
      volumes:
        - name: tmp
          emptyDir: {}
YAML
}

render_all() {
  render_setup
  render_probe
}

wait_for_job() {
  local deadline=$((SECONDS + 180))
  local complete failed
  while ((SECONDS < deadline)); do
    complete="$(kubectl -n "${NAMESPACE}" get job "${JOB_NAME}" \
      -o jsonpath='{range .status.conditions[?(@.type=="Complete")]}{.status}{end}' 2>/dev/null || true)"
    failed="$(kubectl -n "${NAMESPACE}" get job "${JOB_NAME}" \
      -o jsonpath='{range .status.conditions[?(@.type=="Failed")]}{.status}{end}' 2>/dev/null || true)"
    if [[ "${complete}" == "True" ]]; then
      return 0
    fi
    if [[ "${failed}" == "True" ]]; then
      return 1
    fi
    sleep 3
  done
  return 1
}

wait_for_namespace_gone() {
  local ns="$1"
  if kubectl get namespace "${ns}" >/dev/null 2>&1; then
    local phase
    phase="$(kubectl get namespace "${ns}" -o jsonpath='{.status.phase}' 2>/dev/null || true)"
    if [[ "${phase}" == "Terminating" ]]; then
      echo "==> waiting for namespace ${ns} to finish terminating"
      kubectl wait --for=delete "namespace/${ns}" --timeout=180s >/dev/null 2>&1 || true
    fi
  fi
}

cleanup() {
  if [[ "${KEEP_NAMESPACES}" == "1" ]]; then
    echo "==> KEEP_NAMESPACES=1: leaving ${NAMESPACE} and ${PEER_NAMESPACE} in place" >&2
    return
  fi
  echo "==> cleaning up ${NAMESPACE} and ${PEER_NAMESPACE}"
  kubectl delete namespace "${NAMESPACE}" "${PEER_NAMESPACE}" \
    --ignore-not-found=true --wait=false >/dev/null 2>&1 || true
}

main() {
  command -v docker >/dev/null 2>&1 || die "docker is required to run kubectl"

  CLUSTER_CONFIG_DIR="$(cd "${CLUSTER_CONFIG_DIR}" 2>/dev/null && pwd)" \
    || die "cluster config directory not found: ${CLUSTER_CONFIG_DIR}"
  [[ -f "${CLUSTER_CONFIG_DIR}/kubeconfig" ]] \
    || die "kubeconfig not found at ${CLUSTER_CONFIG_DIR}/kubeconfig (run from the workspace root or set CLUSTER_CONFIG_DIR)"

  echo "==> checking CiliumNetworkPolicy CRD"
  if ! kubectl get crd ciliumnetworkpolicies.cilium.io >/dev/null 2>&1; then
    die "CiliumNetworkPolicy CRD is not installed; install Cilium first (see home-cilium/README.md). On Flannel this test can never pass."
  fi

  echo "==> checking Cilium agent DaemonSet"
  kubectl -n kube-system rollout status daemonset/cilium --timeout=120s \
    || die "cilium DaemonSet is not ready"
  kubectl -n kube-system rollout status deployment/cilium-operator --timeout=120s \
    || die "cilium-operator Deployment is not ready"

  wait_for_namespace_gone "${NAMESPACE}"
  wait_for_namespace_gone "${PEER_NAMESPACE}"

  trap cleanup EXIT

  echo "==> creating scratch namespaces, policy and peer"
  render_setup | kubectl apply -f -

  echo "==> waiting for the peer"
  kubectl -n "${PEER_NAMESPACE}" rollout status deployment/"${PEER_NAME}" --timeout=120s

  echo "==> running the isolation probe Job"
  kubectl -n "${NAMESPACE}" delete job "${JOB_NAME}" --ignore-not-found=true >/dev/null
  render_probe | kubectl apply -f -

  if ! wait_for_job; then
    echo
    echo "==> probe failed; pod logs:"
    kubectl -n "${NAMESPACE}" logs "job/${JOB_NAME}" --all-containers=true || true
    echo
    echo "==> Job status:"
    kubectl -n "${NAMESPACE}" describe job "${JOB_NAME}" || true
    exit 1
  fi

  kubectl -n "${NAMESPACE}" logs "job/${JOB_NAME}" --all-containers=true
  echo "==> isolation verification passed"
}

case "${1:-}" in
  --render)
    render_all
    ;;
  --keep)
    KEEP_NAMESPACES=1
    main
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
