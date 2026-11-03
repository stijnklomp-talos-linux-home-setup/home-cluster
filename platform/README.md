# Platform

How `home-cluster-1` is built. Everything here is DevOps-owned: it exists so
that apps can be deployed, not to serve a particular app.

Four layers, bottom up. Each is a self-contained helmfile/terragrunt project
with its own `README.md` and `AGENTS.md`.

| Layer | Directory | What it provides |
|-------|-----------|------------------|
| Machine | [`node/`](node/README.md) | Talos install, upgrades, node patches, schematics |
| Cloud | [`infra/`](infra/) | Terraform: remote state (S3), Cognito federation, Talos cluster definition |
| Cluster | [`cluster/`](cluster/) | CNI, load balancer, TLS, storage, service mesh + edge gateway |
| Delivery | [`cicd/`](cicd/) | Tekton, Pipelines-as-Code, npm cache, tenant platform, ArgoCD |
| GitOps | [`gitops/`](gitops/README.md) | ApplicationSets that turn declarations into running apps |

## Deploy order

Order is not optional — several components provide what the next one needs.

**Day 0, the base layer** (one `helmfile apply` per directory):

1. **Talos** with `patches/cni-cilium.yaml` (`cni: none`) on every node — the cluster must come up without a CNI.
2. **Cilium** — nothing has networking before it.
3. **MetalLB** — nothing gets a LoadBalancer IP before it.
4. **cert-manager** — issues `home-root-ca` and the serving certs.
5. **Longhorn** — persistent storage, including the Verdaccio and Tekton PVCs.
6. **Istio** (ambient) — then the edge Gateway, which is a second helmfile: `helmfile -f helmfile-edge.yaml.gotmpl`.

**Then the delivery layer**, in this order:

7. `cicd-tekton-operator` → `cicd-tekton-pipelines-as-code` → `cicd-verdaccio` → `tenant-platform` → `cicd-argocd`.
8. **GitOps bootstrap**, once: `kubectl apply -f gitops/bootstrap.yaml`. From then on ArgoCD owns the apps.

## Teardown order

The reverse, with one trap:

- Destroy the edge Gateway **first**: `cd cluster/home-istio && helmfile -f helmfile-edge.yaml.gotmpl destroy`.
- Then `helmfile -l 'stage!=crds' destroy` per directory, in reverse order.
- Do **not** run a plain `helmfile destroy` in `home-istio`: it also uninstalls `gateway-api-crds`, which deletes the Gateway API CRDs cluster-wide and takes every Gateway/HTTPRoute/TLSRoute with it. Reserve that for abandoning Istio entirely.
- Rollback procedure for the mesh: [`../developer/docs/operations/istio-ambient-cutover.md`](../developer/docs/operations/istio-ambient-cutover.md).

## The tenant registry

[`cicd/tenant-platform/values/tenants.yaml`](cicd/tenant-platform/values/tenants.yaml)
is **the only** list of tenants and namespaces. Every per-tenant object —
namespace, quota, network policy, service account, PaC `Repository` — is
rendered from it. There is no second list to keep in sync.

Removing an entry prunes the namespace **and its Longhorn PVC data**. Read the
warning in [`cicd/tenant-platform/README.md`](cicd/tenant-platform/README.md)
before editing it.

## Conventions

- **Docker-first.** Nothing is installed on the host; every CLI runs in a container. See the command table in [`../AGENTS.md`](../AGENTS.md).
- **Secrets never enter git.** Each component has a `secrets/` directory that is gitignored; only `*.example.yaml` is committed. Live credentials live in `../home-cluster-1-config/` (sibling directory, outside this repo).
- **Vendored charts are immutable.** `charts/upstream/*.tgz` and `kustomizations/upstream/*.yaml` are refreshed with `helm pull` / `curl`, never edited in place.
- **No parallel install paths.** helmfile is day 0; ArgoCD manages everything after bootstrap.
