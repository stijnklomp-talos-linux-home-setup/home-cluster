# Tips

- Shared Tasks/Pipelines live in `tekton-pipelines`; each tenant's PipelineRun
  runs in its tenant namespace (the Repository CR namespace). Reference shared
  definitions with the cluster resolver (`kind: task` or `pipeline`, namespace
  `tekton-pipelines`); a bare `taskRef`/`pipelineRef` only searches the tenant
  namespace.
- The registry is the only way to enable a repo. PaC auto-configure is off; add
  the repository to `tenant-platform/values/tenants.yaml` and apply. See the
  [tenancy runbook](../tenancy/runbook.md).
- Additional PaC annotations: `pipelinesascode.tekton.dev/on-comment`,
  `/retest`/`/ok-to-test` comments (PaC docs).
- Use dynamic params in `.tekton/pipelinerun.yaml`: `{{ repo_url }}`,
  `{{ revision }}`, `{{ repo_name }}`, `{{ branch_name }}`,
  `{{ git_auth_secret }}`.
- Workspaces: always request `storageClassName: longhorn` for PVC-backed
  workspaces (see usage examples). Use `emptyDir` when the workspace does not
  need to survive the run.
- TaskRun pods land on the CI node (the Dell) by default — TektonConfig
  `default-pod-template` (see `platform/node/designate-node-roles.yaml` for
  node roles).
- NPM installs go through the Verdaccio cache (`cicd-verdaccio`): bind the
  `npmrc` workspace to the tenant namespace's `npm-npmrc` ConfigMap (required by
  `npm-install-v1`, optional for `build-push-v1` Dockerfile builds). Packages
  older than 90 days are evicted automatically; re-fetched packages reset their
  TTL.
- Secrets are namespace-scoped. PaC creates the per-run git auth Secret; a
  tenant's registry credentials are supplied out-of-band in its own namespace;
  deploy credentials exist only in the deploy tenant namespace. Never put
  operator credentials in a `build` namespace.
- `tekton-pipelines` is control-plane only: shared Tasks/Pipelines and Tekton
  configuration. Do not create Repository CRs or PipelineRuns there.
