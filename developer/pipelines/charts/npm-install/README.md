# npm-install

`npm-install-v1` installs Node dependencies with pnpm (falling back to `npm ci`).

## Required workspace: `npmrc`

The task copies `.npmrc` from the `npmrc` workspace before installing. The
`npm-npmrc` ConfigMap is owned by `tenant-platform` and exists in every tenant
namespace (the run namespace):

```yaml
workspaces:
  - name: npmrc
    configMap:
      name: npm-npmrc
```

The chart in this repo renders only the Task — no ConfigMap and no
control-plane copy. A tenant namespace without `npm-npmrc` fails on the missing
`/workspace/npmrc/.npmrc`; add the tenant to the registry instead of rendering
the ConfigMap here.

## Pod Security

The step runs as UID/GID 65532 with `HOME=/tmp`, `NPM_CONFIG_CACHE=/tmp/.npm` and
`COREPACK_HOME=/tmp/corepack`; `corepack enable` installs its shims into
`/tmp/corepack-bin` because `/usr/local/bin` is root-owned in `node:22`.

The `source` workspace must be writable by UID 65532. Bind it as a PVC with
`fsGroup: 65532` on the PipelineRun's pod template:

```yaml
spec:
  taskRunTemplate:
    podTemplate:
      securityContext:
        fsGroup: 65532
        fsGroupChangePolicy: OnRootMismatch
```

A PVC written by an older run with different ownership needs a one-time
`chown -R 65532:65532` (or a fresh PVC).

## Parameters

None. `NPM_TOKEN` is read from the optional `npm-secret` (`npm-install-secrets`)
for private registries; public installs through Verdaccio need no token.
