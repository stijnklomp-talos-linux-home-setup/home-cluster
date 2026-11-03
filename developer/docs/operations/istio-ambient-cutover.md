# Istio edge + ambient cutover (big-bang)

Replace `ingress-nginx` with an Istio 1.31.x internal gateway, enable ambient mTLS, and migrate ArgoCD, Tekton Dashboard, PaC and Pinniped in one window. Public access and observability are deferred.

> This is a public document: use `<lb-ip>` for the MetalLB address. The real address lives in the cluster configuration, not here.

## Status

**Executed 2026-11-01** (tickets #10–#12). Ambient mTLS is live and the
internal Gateway `istio-internal` serves ArgoCD, Tekton Dashboard, PaC and
Pinniped on the address ingress-nginx used to own. `ingress-nginx` is removed
from the cluster and from `home-cluster-kubernetes-base`. Evidence and the
exact sequence are in [Phase 1](#phase-1--cutover-window-executed-2026-11-01)
and [Cutover record](#cutover-record-2026-11-01). One acceptance is explicitly
**manual/waived**: the interactive `pinniped login oidc` + `kubectl get nodes`
sign-in (see the cutover record).

## Preconditions

- [x] `home-istio` manifests rendered and reviewed (Istio base, istiod, cni, ztunnel; Gateway API CRDs ≥ 1.5). The Gateway API `Gateway` provisions its own Deployment/Service.
- [x] Mixed HTTPS termination + TLS passthrough on one `:443` port rehearsed on a spare MetalLB address (Phase 0b, Istio 1.31.3).
- [x] Cilium values updated: `cni.exclusive: false`; keep `kubeProxyReplacement: false`, `bpf.masquerade: false`, VXLAN.
- [x] `CiliumClusterwideNetworkPolicy` allowing ambient host probes from `169.254.7.127/32` prepared.
- [x] `PeerAuthentication` `STRICT` prepared for ambient namespaces.
- [x] Certificates prepared for `argocd.<lb-ip>.sslip.io`, `tekton.<lb-ip>.sslip.io` and `pipelines-as-code.<lb-ip>.sslip.io` from `home-issuer`.
- [x] Pinniped `TLSRoute` manifests prepared for the supervisor and impersonation proxy; SNI hostnames recorded.
- [x] Backup: `kubectl get ingress -A -o yaml`, `kubectl get svc -n ingress-nginx -o yaml`, the current LoadBalancer address, Cognito callback URLs, ArgoCD external URL, oauth2-proxy redirect URLs, and the Pinniped kubeconfig. (No Pinniped kubeconfig file exists in the repos; the impersonation-proxy CA is unchanged and no kubeconfig rewrite was needed.)
- [x] No active users; downtime is accepted.
- [x] Talos `/opt/cni/bin` confirmed writable (Cilium already installs there; verify the Istio CNI installer can too).

## Phase 0 — pre-stage (no traffic change)

1. Add `home-istio` to `home-cluster-kubernetes-base` and vendor the Istio 1.31.3 charts. Include the Gateway API CRDs in the same component.
2. Prepare the gateway manifests:
   - `Gateway` `istio-internal`, with HTTP :80 and HTTPS :443 listeners for ArgoCD, Tekton and PaC.
   - A TLS passthrough listener on :443 for Pinniped. The SNI hostnames must be distinct from the HTTPS hosts. Mixed mode is verified in Phase 0b; fall back to a separate address/port or an Istio-native `Gateway` only if a later Istio upgrade changes the behaviour.
   - `Gateway.spec.infrastructure.annotations` with `metallb.io/loadBalancerIPs: <lb-ip>` so the generated Service requests the existing address.
3. Prepare the `HTTPRoute` objects and the Pinniped `TLSRoute` objects. Attach them only to `istio-internal`.
4. Label the ambient namespaces only: application namespaces, `home-graphql`, and the ingress-gateway namespace. Do **not** label tenant namespaces, `argo`, `tekton-pipelines`, or `pinniped-system` with `istio.io/dataplane-mode: ambient`. (This is separate from the route-admission label `istio.io/edge-route: enabled`, which *is* set on `istio-ingress`, `argo`, `home-sso-auth`, `pipelines-as-code` and `pinniped-system`; it only controls which namespaces may attach routes.)
5. Do not apply any of this yet.

## Phase 0b — mixed 443 rehearsal (ticket #11)

Prove the riskiest listener combination on a spare MetalLB address while
ingress-nginx keeps every production host on `<lb-ip>`.

`home-istio/charts/edge-temp-gateway` deploys one `Gateway` `istio-temp` with
three listeners on `:443`, all with distinct hostnames:

| Listener | Protocol | Hostname | Backend |
|---|---|---|---|
| `https-hello` | HTTPS Terminate | `hello.<temp-lb-ip>.sslip.io` | nginx in `istio-temp-test` |
| `tls-login` | TLS Passthrough | `login.<lb-ip>.sslip.io` | `pinniped-supervisor:443` |
| `tls-kubectl` | TLS Passthrough | `kubectl.<lb-ip>.sslip.io` | `pinniped-concierge-impersonation-proxy-cluster-ip:443` |

The HTTPS listener uses an explicit `Certificate` from `home-issuer` (the
cert-manager Gateway shim is not enabled on this cluster). The Pinniped
`TLSRoute`s live in `pinniped-system`, reuse the existing services and serving
certificates, and attach by `sectionName`. The gateway manifest is a separate
helmfile, so the normal `helmfile apply` never creates it:

```sh
cd home-istio
EDGE_TEMP_LB_IP=<temp-lb-ip> \
EDGE_HOST_SUFFIX=<temp-lb-ip>.sslip.io \
EDGE_PINNIPED_HOST_SUFFIX=<lb-ip>.sslip.io \
helmfile -f helmfile-temp-gateway.yaml.gotmpl apply
```

**Verified 2026-11-01 on Istio 1.31.3: mixed HTTPS-Terminate + TLS-Passthrough
on one `:443` port works.** All three listeners report `Programmed=True` and
`Conflicted=False`; every route reports `Accepted=True` and
`ResolvedRefs=True`.

```sh
kubectl wait --for=condition=Programmed gtw/istio-temp -n istio-ingress
kubectl -n cert-manager get secret home-root-ca -o jsonpath='{.data.tls\.crt}' | base64 -d > home-ca.crt

# 200: TLS terminated at the gateway, chain verifies against the cluster root CA
curl -sS -o /dev/null -w '%{http_code}\n' --cacert home-ca.crt \
  --resolve hello.<temp-lb-ip>.sslip.io:443:<temp-lb-ip> \
  https://hello.<temp-lb-ip>.sslip.io/

# 200 + discovery JSON: passthrough to the supervisor
curl -sS --cacert home-ca.crt \
  --resolve login.<lb-ip>.sslip.io:443:<temp-lb-ip> \
  https://login.<lb-ip>.sslip.io/.well-known/openid-configuration

# 401: passthrough reaches the impersonation proxy (it serves its own CA, hence -k)
curl -sS -k -o /dev/null -w '%{http_code}\n' \
  --resolve kubectl.<lb-ip>.sslip.io:443:<temp-lb-ip> \
  https://kubectl.<lb-ip>.sslip.io/version
```

`pinniped login oidc` needs a browser and Cognito credentials. With the issuer
host forced to `<temp-lb-ip>`, the CLI completes OIDC discovery and prints the
supervisor authorize URL, which returns a `303` to the Cognito hosted UI. The
sign-in itself is the manual step in Phase 1; an automated successful login is
not part of this rehearsal.

Cleanup before the cutover window:

```sh
helmfile -f helmfile-temp-gateway.yaml.gotmpl destroy
```

This removes the Gateway, its generated Deployment/Service, the Certificate,
the routes, the `istio-temp-test` namespace, and releases the address back to
MetalLB. The fallback below was not needed.

**Retired 2026-11-01:** the temporary gateway was destroyed after the
production `istio-internal` gateway was verified, and `charts/edge-temp-gateway`
plus `helmfile-temp-gateway.yaml.gotmpl` were removed from the base repo. The
proof lives in git history; do not reintroduce a second Gateway.

## Phase 1 — cutover window (executed 2026-11-01)

The executed order differs from the original plan in one deliberate way: the
Gateway was applied on a spare MetalLB address while ingress-nginx still owned
`<lb-ip>`, every host was verified through that spare address, and only then
was nginx removed and the Gateway moved onto `<lb-ip>`. MetalLB never assigns
one VIP to two Services; this kept both edges serving until the new one was
proven.

1. [x] Snapshot the old state: `kubectl get ingress -A -o yaml`,
   `kubectl get svc -n ingress-nginx -o yaml`, the certificate inventory and
   the LoadBalancer address. Confirm the Cognito callbacks, ArgoCD URL,
   oauth2-proxy redirect URLs and the Pinniped hostnames.
2. [x] Apply the production edge with `EDGE_LB_IP=<spare>` and
   `EDGE_HOST_SUFFIX=<lb-ip>.sslip.io` (`home-istio`,
   `helmfile -f helmfile-edge.yaml.gotmpl`). The presync hook labels the route
   namespaces `istio.io/edge-route: enabled`.
3. [x] Wait for `Gateway Programmed=True`, `Certificate internal-tls Ready`,
   and every route `Accepted=True` / `ResolvedRefs=True`.
4. [x] Upgrade the owning components: ArgoCD (drop its Ingress and server
   certificate), PaC (HTTPRoute), Tekton Dashboard (HTTPRoute), Pinniped
   (TLSRoutes). Old `ingress-nginx` Ingresses disappear as each release is
   upgraded.
5. [x] Renew the supervisor serving certificate (Go/CA encoding defect) and
   re-render `expose-kubeconfig` so `JWTAuthenticator` carries the current root.
6. [x] Verify through the spare address: ArgoCD 200 and `/auth/login` 303 to
   Cognito; Tekton returns the same oauth2-proxy response as before; PaC 200;
   login discovery 200; impersonation proxy 401; HTTP 301 to HTTPS; OpenSSL
   **and** Go chain verification.
7. [x] `helmfile destroy` ingress-nginx. Confirm its Service and namespace are
   gone, `kubectl get ingress -A` is empty, and `<lb-ip>` is back in the pool.
8. [x] Re-apply the edge with `EDGE_LB_IP=<lb-ip>`. The generated Service takes
   `<lb-ip>`; reverify every host.
9. [x] `istioctl analyze -A`: no Errors or Warnings (only `IST0102` injection
   and `IST0118` port-name Info false positives).
10. [x] Destroy the ticket #11 temporary gateway; remove `home-ingress-nginx`
    and the temp chart from the base repo.
11. [x] Update the access guide, the CA-trust page, component AGENTS/README
    files and the workspace docs.

### Cutover record (2026-11-01)

| Check | Result |
|---|---|
| `Gateway istio-internal` | `Programmed=True`, address `<lb-ip>` |
| ArgoCD | `/` 200; `/auth/login` 303 to the same Cognito client and callback |
| Tekton Dashboard | oauth2-proxy response unchanged (403 unauthenticated) |
| PaC | `/` 200, `{"status":200,"message":"ok"}` |
| Pinniped supervisor | discovery 200; authorize 303 -> identity chooser -> 303 to Cognito |
| Impersonation proxy | `/version` 401 |
| `kubectl get ingress -A` | empty |
| ingress-nginx | Service and namespace gone; address reclaimed by the Gateway |
| `istioctl analyze -A` | no Errors/Warnings |
| `pinniped login oidc` + `kubectl get nodes` | **manual / waived** — see below |

`pinniped login oidc` and `kubectl get nodes` need Cognito credentials and a
browser. **This acceptance is manual and was waived for the automated cutover
verification:** the discovery, authorize redirect and impersonation-proxy legs
were verified instead. No automated success claim is made for the interactive
sign-in; run it by hand once, from a workstation with a browser, before
declaring the Pinniped path fully accepted.

### Ambient proof (ticket #10, verified 2026-11-01)

`home-istio/verify-ambient.sh` passed in full on 2026-11-01:

| Check | Result |
|---|---|
| `istiod`, `ztunnel`, `istio-cni-node` | Ready (all nodes) |
| `istioctl analyze -A` | clean (only `IST0102`/`IST0118` Info false positives) |
| In-mesh client -> nginx server | HTTP 200 |
| `ambient-sandbox-nomesh` (unlabeled) -> same server | denied |
| ztunnel workload list | server listed as an `HBONE` workload |

Together with the live edge checks above, this is the evidence that ambient
mTLS and the internal gateway work; the script is the regression test.

The edge certificate (`internal-tls`) and the renewed supervisor leaf both
verify against the current `home-root-ca` in OpenSSL **and** Go; the
impersonation-proxy chain verifies against its own CA. See
[ca-trust.md](../cicd/access/ca-trust.md#go-clients-pinniped-cli-client-go-after-the-istio-cutover).

## Phase 2 — GraphQL gateway

**Status 2026-11-01:** the gateway and both reference subgraphs are live
(tickets #15–#16); ticket #17 adds JWT authentication/authorization end to end.
The token flow, Cognito access-token caveats and the manual Cognito step are in
[graphql-auth.md](graphql-auth.md).

1. Create `home-graphql`: `supergraph.yaml`, gateway config, image/chart, a Tekton compose pipeline, and the `home-cluster` declaration.
2. Add the optional `graphql` feature to the Rust service template: `async-graphql` + `async-graphql-axum`, a `src/graphql` module, Federation v2 entity resolvers, a JWT extractor, and federation SDL tests. REST/OpenAPI/Pact remain. *(Historical: the standalone template was retired in 2026-11; `developer/examples/home-users` is now the starting point and carries the feature.)*
3. Build the `users` + `orders` reference subgraphs with the feature enabled. Deploy them only in ambient namespaces.
4. Run `rover supergraph compose` manually, commit the supergraph as a ConfigMap, and deploy Hive Gateway.
5. Verify:
   - one query resolves fields from both subgraphs; `_service` returns the SDL; `_entities` resolves representations
   - missing/invalid JWTs are rejected; the `Authorization` header reaches each subgraph and each service validates it
   - the gateway calls subgraphs over ambient mTLS; direct subgraph access is denied by Istio/Cilium policy
   - `/healthcheck` and `/readiness`
6. Composition is manual in iteration 1; no breaking-change gate.

### Reference federation query (#15–#16)

The supergraph composed from `developer/examples/home-graphql/supergraph.yaml` contains two
subgraphs: `users` (owns `User`, `@key(fields: "id")`) and `orders` (owns
`Order`, and references `User` as `@key(fields: "id", resolvable: false)`).
This is the reference cross-service query:

```graphql
{
  orders {
    id
    total
    status
    user {
      id
      name
      email
    }
  }
}
```

`orders` resolves the order rows locally and returns a minimal `User`
representation (`{ __typename: "User", id: userId }`); the query planner sends
one `_entities` request to `users`, which resolves `name` and `email` through
its own repository. Query plans are batched: a page of N orders produces one
orders query and one users `_entities` request whose DataLoader performs a
single `WHERE id = ANY($1)` lookup — not one request per order. The live
verification records the subgraph request logs and the users `_entities`
payload as the evidence.

Both subgraphs validate the same bearer token independently (the gateway
forwards the raw `Authorization` header; there are no trusted identity
headers). A call to the subgraph Service DNS names from an ambient sandbox
namespace is denied by Cilium, and `PeerAuthentication` is `STRICT` in both
namespaces: only the `home-graphql` gateway namespace is admitted, over HBONE.

### JWT authn/z (#17)

The verification overlay uses the Cognito **access-token** shape: RS256, the
test issuer `https://test-issuer.home.test`, the in-cluster test JWKS, **no
`aud`**, and a `client_id` claim (`home-test-client`). The gateway rejects a
missing, malformed, invalid or wrong-client token with 401 before query
planning and enforces `@requiresScopes` (`users:read`, `orders:read`,
`orders:admin`) through `genericAuth`; each service validates the forwarded
token again and applies self/owner rules. A token without `users:read` can read
only the caller's own verified `User.email`; another user's email is denied.
User 1 cannot read user 2's order, and spoofed `X-User`/`X-Email` headers have
no effect. Production values keep `<region>`/`<pool>`/`<client-id>`
placeholders (a dedicated GraphQL app client is pending); the switch, claim
requirements and the manual real-login step are in
[graphql-auth.md](graphql-auth.md).

### Schema change without a gateway rebuild (#18)

A schema change is a **ConfigMap update**, not a gateway deploy. The gateway
reads the supergraph through a function source (`() => readFile(path)`) and
polls the mounted ConfigMap every 10 s; the Deployment mounts it as a
**directory** (never `subPath`), so the kubelet's atomic symlink swap is
visible to the next fresh `open()`. The pod's restart count does not change.

Procedure:

1. Change the subgraph schema and release the subgraph image as usual.
2. Fetch the new SDL **verbatim** from the running subgraph into
   `developer/examples/home-graphql/schemas/` (the per-subgraph `kubectl exec … _service { sdl }`
   commands are in `developer/examples/home-graphql/README.md`).
3. Seed the composition inputs (home-graphql has no reachable git remote), then
   run the shared release pipeline — **manual, no PaC trigger and no
   breaking-change gate**:

   ```sh
   tar -czf - supergraph.yaml schemas | base64 | tr -d '\n' > /tmp/supergraph-source.b64
   kubectl -n home-graphql create configmap supergraph-source \
     --from-file=source.tgz.b64=/tmp/supergraph-source.b64 \
     --dry-run=client -o yaml | kubectl apply -f -
   rm -f /tmp/supergraph-source.b64
   kubectl create -f \
     developer/pipelines/charts/supergraph-release/usage-examples/v1/pipelinerun.yaml
   ```

   `supergraph-release-v1` = `supergraph-seed-v1` (unpacks the ConfigMap into
   the `supergraph-release-source` PVC; skipped when `repo-url` is set, which
   uses `git-clone-v1`) → `supergraph-compose-v1` (Rover 1.0.1, pinned ELv2
   `supergraph@=2.9.3`, `--skip-update`, plugin cached in `.rover-home/`) →
   `supergraph-configmap-v1` (renders and applies `home-graphql-supergraph`,
   then reads back and compares the hash; it has no `create` permission, so the
   gateway chart must already exist).
4. Verify through Hive within about a minute (observed 12–57 s end to end; the
   mounted ConfigMap is polled every 10 s): the new field is queryable, the #16
   cross-service query and #17 auth behaviour still work, and
   `kubectl -n home-graphql get pods` shows an unchanged restart count.
   `/readiness` 200.
5. Commit the refreshed `schemas/*.graphql` and `supergraph.graphql`
   (`developer/examples/home-graphql/scripts/compose.sh`), **then** Sync ArgoCD. The publisher
   writes the ArgoCD-managed ConfigMap, so the Application is OutOfSync after a
   release; Syncing before the commit rolls the live schema back.

Failure behaviour:

| Failure | Result |
|---|---|
| Invalid subgraph SDL, broken `@key`/`@requires` reference | Rover exits non-zero; the Task composes into a temp file and never moves an artifact into the workspace |
| Compose failed | `publish-configmap` has `runAfter: [compose-supergraph]`, so it does not run; the ConfigMap is unchanged and Hive keeps serving the last good supergraph |
| Empty/missing artifact or content mismatch | `supergraph-configmap-v1` refuses to touch the ConfigMap and fails the Task |
| Bad supergraph already accepted (valid but wrong) | Revert the subgraph SDL, re-run the pipeline, or `helm upgrade` the gateway chart with the committed `supergraph.graphql` |

Limits in this iteration: the trigger is manual, nothing diffs the new
supergraph against the previous one (a removed field really disappears), and
the GitOps commit in step 5 is manual. See
`developer/pipelines/charts/supergraph-release/README.md`.


## Rollback

`ingress-nginx` is EOL; this rollback is a temporary safety net, not a target state.

The MetalLB address is exclusive, so the Gateway must be deleted **before**
nginx is reinstalled. Order:

1. Snapshot again (`kubectl get gateway,httproute,tlsroute -A -o yaml`,
   `kubectl get svc -A`, certificates) and confirm no user session matters.
2. Delete the edge release:
   `helmfile -f helmfile-edge.yaml.gotmpl destroy`. This removes the Gateway,
   its generated Deployment/Service, the `internal-tls` Certificate and the
   redirect route, and releases `<lb-ip>` back to MetalLB.
3. Remove the route-admission labels the presync hook added — nginx and the
   reverted routes do not read them, and leaving them behind would silently
   re-admit those five namespaces if a `Gateway` is ever recreated:

   ```sh
   for ns in argo home-sso-auth pipelines-as-code pinniped-system istio-ingress; do
     kubectl label ns "$ns" istio.io/edge-route- 2>/dev/null || true
   done
   ```
4. Revert the cutover commits in the cicd and restricted sso repos and
   re-apply those components. The old Ingresses, the ArgoCD `server.certificate`
   and the nginx annotations come back; the Gateway API routes disappear with
   the revert.
5. Revert the base cutover commit (restores `home-ingress-nginx`), then
   `cd home-ingress-nginx && helmfile apply`. MetalLB auto-assigns the lowest
   free pool address, which is `<lb-ip>` once the Gateway released it. If
   another Service wins the race, set `metallb.io/loadBalancerIPs: <lb-ip>` on
   the controller Service.
6. Confirm the nginx Service owns `<lb-ip>` and every old Ingress reports an
   address, then verify the four hosts exactly as in Phase 1.
7. Only if the Istio path itself is being abandoned: remove the ambient
   labels, `PeerAuthentication` policies and probe policy, uninstall the Istio
   CNI/istiod/base components, and restore Cilium `cni.exclusive: true`.
   Tear down with `cd home-istio && helmfile -l 'stage!=crds' destroy` and
   `helmfile destroy` only if the Gateway API CRDs are also going away — a
   routine `helmfile destroy` deletes them cluster-wide.
8. Restore any URL/Cognito/Pinniped changes if the address changed (it did
   not in this cutover).

### Rollback dry run (2026-11-01)

Not executed against production. Verified in place:

- The pre-cutover ingress-nginx chart still renders a `Service type:
  LoadBalancer` with no hardcoded address (auto-assign from the pool), and the
  pre-cutover cicd values render the ArgoCD Ingress plus the `argocd-server`
  Certificate with the unchanged hostname.
- Address free/reclaim was exercised on a scratch VIP: a temporary Service
  requested `<spare-lb-ip>`, MetalLB assigned it, and deleting the Service
  returned it to the pool. That is the same path that returns `<lb-ip>` after
  step 2.
- MetalLB allocation is exclusive: while `istio-internal` owns `<lb-ip>` a
  recreated nginx Service cannot take it. Hence step 2 before step 4.
- No production Ingress, Gateway or Service was deleted during the dry run.

If Pinniped's mixed 443 listener fails, use a separate address/port or an Istio-native `Gateway` for the `TLSRoute`; record the hostname change and update the kubeconfig and Cognito callbacks. Mixed mode was verified on Istio 1.31.3 (Phase 0b), so this path is only for a regression after an upgrade.

## Deferred / follow-ups

- [ ] **Public access:** domain, Cloudflare Tunnel, DNS-01 issuer, public `Gateway`, public-route allowlist, public TLS. Only the Next.js UI gets a public route; backends stay private. Update `cicd/access/README.md` when it lands.
- [ ] **Observability:** Istio metrics, Hive OpenTelemetry traces, and the Prometheus/Grafana/OTel stack they need.
- [ ] **PDB / second gateway replica:** revisit if downtime becomes unacceptable.
- [ ] **Schema automation:** service CI triggers composition; add a breaking-change gate.
- [ ] **Waypoint / per-caller authorization:** add only when a concrete per-service rule exists.
- [ ] **EKS portability:** out of scope; the AWS edge will use NLB/ALB, VPC CNI and EBS CSI.

## Notes

- Istio 1.31.3 supports Kubernetes 1.36.4; plan the 1.32 upgrade separately.
- **Teardown is CRD-safe:** `cd home-istio && helmfile -l 'stage!=crds' destroy`.
  A plain `helmfile destroy` also uninstalls `gateway-api-crds`, deleting the
  Gateway API CRDs cluster-wide and cascading every Gateway/HTTPRoute/TLSRoute.
  Reserve it for abandoning Istio, and destroy the edge release first.
- **The ambient sandbox is not removed by `-l stage=sandbox destroy`:** both
  releases are declared in `home-istio/helmfile.yaml`, so the next
  `helmfile apply` recreates them. Delete the releases to retire the proof.
- **Edge scheduling risk (accepted):** ingress-nginx was pinned to the general
  workers with a `nodeSelector`; the Gateway API `Gateway` has no node affinity,
  so the single gateway pod can land on the CI node and compete with
  PipelineRuns. The supported fix is a `GatewayClass.spec.parametersRef`
  ConfigMap with `nodeSelector`/`affinity`, which would make this repo own the
  `istio` GatewayClass and re-apply it after every upgrade — deliberately not
  done. Details in `home-istio/README.md`.
- **CA reissue and Go clients (found during the #11 rehearsal, fixed in #12):** the 100-year `home-root-ca` reissue encodes the CA subject as `UTF8String`, while serving certificates issued before the reissue carry a `PrintableString` issuer. OpenSSL, curl and browsers normalise the names and verify; Go-based clients do not match the chain and fail with `x509: certificate signed by unknown authority`. **Fixed 2026-11-01:** `internal-tls` and `supervisor-public-tls` were reissued against the current root, `expose-kubeconfig` was re-rendered so `JWTAuthenticator` embeds the current CA, and both chains now verify in OpenSSL and Go. The impersonation-proxy chain (its own CA) was already consistent. Do not regenerate the root CA.
- Cilium + Istio: `cni.exclusive: false`; avoid Cilium L7 policy (split-brain, and Cilium loses HBONE visibility on 15008).
- One gateway replica: image rollouts have a brief gap. Schema-only changes are picked up by ConfigMap polling and do not restart the pod.
- The GraphQL gateway has no edge route; only the Next.js server and internal services call its Service DNS name.
