# Longhorn

Distributed block storage (default StorageClass `longhorn`). Needed by Tekton workspaces, the plans PVC, and any stateful service.

## Deploy

```sh
helmfile apply
```

Requires K8s >= 1.34 (cluster is v1.36.4). Single replica per volume on the worker node; revisit when adding nodes.

## Cleanup

```sh
helmfile destroy
```