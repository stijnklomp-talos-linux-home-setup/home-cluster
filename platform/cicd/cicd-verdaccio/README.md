# cicd-verdaccio

Cache-only NPM registry for Tekton TaskRuns. Proxies `registry.npmjs.org` and stores packages in a Longhorn PVC so repeated `pnpm install` runs reuse cached dependencies.

## Deploy

```sh
helmfile apply
```

## How TaskRuns use it

The `npm-install-v1` task (`developer/pipelines`) sets each team's `.npmrc` registry to `http://verdaccio.verdaccio.svc.cluster.local:4873`. First fetch of a package populates the cache; later runs hit Verdaccio instead of the internet.

## Access

Cluster-internal only (ClusterIP): no ingress, no auth. Anonymous read; publishing is disabled (`max_users: -1`).

## Cleanup

```sh
helmfile destroy
```

(Removes the PVC too — cache is not durable by design.)