# Retention

Cheat sheet: how long pipeline runs and their data stick around.

| Item | Retained | Notes |
|------|----------|-------|
| PipelineRun / TaskRun | ~24h after it finishes | visible in the Tekton Dashboard / `kubectl` until pruned |
| Logs | ~24h | read from the run's pods; nothing is archived, so copy a log out if you need it longer |
| Per-run workspace PVC | ~24h | e.g. the cloned source; deleted together with its run |
| Deploy tenant PVC (`home-terraform-plans`) | volume: indefinite · plan files: cleaned automatically | lives in the deploy tenant namespace; one plan+apply pair at a time (RWO); removed after a successful apply, never-applied plans swept after ~30 days |

**Rules of thumb**

- Older than a day? The run and its logs are gone — export anything you need before then.
- Terraform plan → apply: the plan file outlives the plan run (it's on the shared PVC), so the apply can run later; a failed apply keeps its plan for debugging.

## Cleanup mechanism

- A Tekton pruner deletes finished PipelineRuns/TaskRuns older than 24h, hourly (TektonConfig `pruner`).
- Per-run PVCs are owned by their run, so Kubernetes deletes them with it; Longhorn reclaims the storage.
- Terraform plans: the apply task deletes the plan after a successful apply, and a nightly CronJob sweeps plan files older than 30 days that were never applied. The deploy tenant's volume itself is chart-managed and never pruned.
- Logs only live with the run's pods — Tekton Results is disabled, so there is no log archive.
