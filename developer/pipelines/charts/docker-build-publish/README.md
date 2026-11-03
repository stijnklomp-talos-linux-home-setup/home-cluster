# docker-build-publish

Container image build/push tasks for the shared Tekton catalog. Both run under
Pod Security `restricted`:

| Task | Status | Implementation |
|------|--------|----------------|
| `build-push-v1` | canonical | rootless Buildah, chroot isolation, vfs storage |
| `kaniko-v1` | deprecated **alias** | exactly the same spec as `build-push-v1` |

`kaniko-v1` is kept so existing `.tekton/` pipelines that reference it keep
working. It no longer runs kaniko and no shared Task requires UID 0 anymore.
Update new pipelines to `build-push-v1`; the alias can be removed once no caller
references it (`grep -r kaniko-v1 <repos>/.tekton`).

## Why rootless Buildah

Kaniko unpacks the base image over its own root filesystem and `chown`s /
`mknod`s the result, so it must run as UID 0. Pod Security `restricted` requires
`runAsNonRoot: true`, which makes kaniko impossible in a tenant namespace.
Rootless Buildah with `--isolation=chroot` and the `vfs` storage driver needs no
`/dev/fuse`, no overlay mounts, no privileged mode and no added capabilities.

BuildKit rootless and `img` were also evaluated; both need rootlesskit (a user
namespace) **and** a seccomp exception, and BuildKit additionally needs
`--oci-worker-no-process-sandbox`/Unconfined in its documented Kubernetes
setup. Buildah with chroot isolation is the smallest working surface.

## Platform requirements (live on home-cluster-1)

1. **Unprivileged user namespaces on every node.**
   `user.max_user_namespaces=15000` (Talos `machine.sysctls`; repo patch
   `platform/node/patches/user-namespaces.yaml`). With the Talos default
   `0`, Buildah fails with `unshare(CLONE_NEWUSER): no space left on device`
   and BuildKit rootless says `max_user_namespaces needs to be set to non-zero`.

2. **The `buildah-userns` Localhost seccomp profile on every node** at
   `/var/lib/kubelet/seccomp/profiles/buildah-userns.json`.
   `RuntimeDefault` blocks `unshare(CLONE_NEWUSER)` with `EPERM` — verified by
   running the same pod with `Unconfined` (works) versus `RuntimeDefault`
   (EPERM), independent of `allowPrivilegeEscalation`. `restricted` PSA allows
   `Localhost`, so this is the supported way to grant the build its namespace
   syscalls.

   The canonical profile and the Talos `machine.files` patch live outside this
   chart, in the cluster-config repo:
   `platform/node/patches/buildah-seccomp.yaml` (embedded JSON, sha256
   `7a4951e7b1bb28bf34421d335905bef3ce3d3e5798c9dd1e44f7b2409d004bfe`). The
   profile is the default Docker/containerd allowlist with the namespace, mount
   and `chroot` syscalls (which the stock profile gates on
   `CAP_SYS_ADMIN`/`CAP_SYS_CHROOT`) added unconditionally. `machine.files`
   cannot be applied in immediate mode: stage it
   (`talosctl patch machineconfig --mode=staged`) and reboot at the next
   maintenance window. A node that has not rebooted still needs the file
   installed out-of-band; a node that never got the file fails the build step
   at `unshare(CLONE_NEWUSER)`.

   **Trade-off:** build pods may create user/mount namespaces and call mount /
   `chroot`. They still have no capabilities, cannot become root on the host,
   and every other syscall stays default-deny. Only the Buildah step uses this
   profile; all other steps use `RuntimeDefault`. This is narrower than
   `Unconfined` and it is the only way to run a rootless builder under
   `restricted` on containerd.

3. **Tekton `set-security-context: "true"`** in the `TektonConfig` pipeline
   section (live; keep it in the operator repo). Tekton's injected `prepare` /
   `place-scripts` / `working-dir-initializer` containers otherwise have no
   securityContext and PSA rejects the pod before any step runs.

The task reads the profile settings from `values.yaml`:

```yaml
buildah:
  seccompProfile:
    type: Localhost
    localhostProfile: profiles/buildah-userns.json
```

Set `type: RuntimeDefault` to fall back; rootless builds then fail at `unshare`
on this cluster.

## Test evidence (2026-10-24, `t3-verify`, `enforce=restricted:latest`)

| Test | Result |
|------|--------|
| `build-push-v1` build+push | TaskRun `restricted-buildpush3` **Succeeded**, pushed `ttl.sh/pac-tenancy-rootless-ok2:1h`, digest `sha256:5f5ff65b50ab615e5eeb22e38f92b70ba6b2d9cbf31e44dfba28d7f2f2069657` (registry manifest HTTP 200) |
| `kaniko-v1` alias build+push | TaskRun `restricted-kaniko-alias` **Succeeded**, digest `sha256:d3c78061ca625cdaa87e348a0f54164306c483cbd1e9789ab620f40114626cee` |
| `app-release-v1` end-to-end (canonical `build-push-v1`) | PipelineRun `restricted-app-release-full2` **Succeeded** (3/3 tasks: clone, build+push, GitOps bump); image `ttl.sh/pac-tenancy-app-full2:1h`, digest `sha256:ed26980bb63096adee9b7efc554a5519d986d621cbc26b8ffd755cfebb6f1c81` |
| missing profile / missing sysctl | fails at `unshare(CLONE_NEWUSER)` as documented above |

## `build-push-v1` contract

Params:

| Param | Default | Meaning |
|-------|---------|---------|
| `IMAGE` | — | full target reference, e.g. `ttl.sh/app:1h` |
| `DOCKERFILE` | `./Dockerfile` | relative to the source workspace |
| `CONTEXT` | `./` | relative to the source workspace |
| `EXTRA_ARGS` | `[]` | extra `buildah bud` arguments |
| `BUILDER_IMAGE` | `quay.io/buildah/stable:v1.41.5` | rootless Buildah image |
| `STORAGE_DRIVER` | `vfs` | no fuse/overlay privileges needed |
| `FORMAT` | `oci` | `oci` or `docker` |

Workspaces:

- `source` (required) — build context. The `setup-npmrc` step runs as UID
  65532 and needs it writable by that identity (fsGroup 65532); the Buildah
  step runs as UID 1000 and only reads it.
- `dockerconfig` (optional, mounted at `/tmp/.docker`) — docker `config.json`
  for the push target. Leave it unbound for anonymous registries such as
  `ttl.sh`; Buildah only reads credentials when the file exists.
- `npmrc` (optional) — copied into the context as `.npmrc` before the build.

Results: `IMAGE_DIGEST`, `IMAGE_URL`.

`setup-npmrc` runs as UID/GID 65532 (the catalog-wide workspace identity). The
Buildah step runs as UID/GID 1000 because the image's `build` user is 1000 —
Buildah calls `user.Current()` and refuses an unknown UID. It uses
`HOME=/home/build`, `TMPDIR=/tmp`, drop ALL capabilities,
`allowPrivilegeEscalation: false` and the Localhost seccomp profile above.
Buildah's vfs storage lives in the container's writable layer, so a build
consumes ephemeral storage (roughly two copies of the base image); size tenant
ephemeral-storage quotas accordingly.

## Base image ownership limitation

`restricted` Pod Security requires `allowPrivilegeEscalation: false`, which sets
`no_new_privs`; that blocks `newuidmap`/`newgidmap`, so Buildah gets a
**single-mapping** user namespace (container UID 0 ⇄ container UID 1000 on the
node). A layer file owned by any other UID/GID cannot be represented:

- `FROM scratch` and all-`0:0` base images build and push (verified live in the
  build tenant).
- `alpine`, `nginx:*` and most distribution images fail at layer extraction:
  `potentially insufficient UIDs or GIDs available in user namespace
  (requested 0:42 for /etc/shadow)`. `/etc/shadow` is `root:shadow` (`0:42`).

Rootless BuildKit has the same limit (it also needs `newuidmap`). The fix is
either an all-`0:0`/flattened base image, or a platform change to run build pods
with `hostUsers: false` as UID 0 inside a full 65536-ID user namespace and
replace the `restricted` namespace label with an equivalent user-namespace-aware
policy. Do not relax the tenant namespace's PSA or add capabilities for this.

## Common failures

| Symptom | Cause |
|---|---|
| `Error during unshare(CLONE_NEWUSER): Operation not permitted` | profile not installed, or `values.yaml` still `RuntimeDefault` |
| `... no space left on device` | node sysctl `user.max_user_namespaces` is 0 |
| `potentially insufficient UIDs or GIDs ... requested 0:42` | base image has files owned by a UID/GID outside the single mapping; see **Base image ownership limitation** |
| `credential file is not accessible: .../config.json` | `DOCKER_CONFIG` set without the optional `dockerconfig` workspace (fixed in the task) |
| `user: unknown userid 65532` from Buildah | Buildah must run as 1000, not 65532 |
