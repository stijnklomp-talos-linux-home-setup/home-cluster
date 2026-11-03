# Troubleshoot

- PaC tenancy (tenant namespaces, Repository admission, Cilium policy): see the [tenancy runbook](../tenancy/runbook.md#troubleshooting).
- PipelineRun stuck in `ResolvingTaskRef`: resolver blocked namespace — check TektonConfig `cluster-resolver-config` allow-list.
- TaskRun pods pending: `kubectl describe pod -n <ns> -l tekton.dev/task=<task>`; check PVC/PV binding (`kubectl get pvc -A`), the `longhorn` StorageClass, and node taints.
- oauth2-proxy login loop: check `oauth2-proxy-secret` values (client-id/secret, cookie-secret) and that the Cognito app client's callback URLs match the ingress host.
- Pinniped `helm upgrade` re-run needed after first install (CA secret lookup — see `home-sso-auth-rbac` README).
- Plan apply fails with "Plan file not found": plan and apply must reference the same `plan-key` result.