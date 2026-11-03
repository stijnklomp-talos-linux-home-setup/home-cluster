# <APP> — one declaration per (service, target cluster). The apps-<CLUSTER>
# ApplicationSet generates <APP>-dev / -staging / -prod from this file.
#
# In the monorepo, repoURL is always the home-cluster repo and chartPath points
# into it, e.g. `developer/examples/<app>/deploy/<app>`. An app living in its
# own repo keeps repoURL = its repo and chartPath = `deploy/<app>`.
#
# Keys are flat on purpose: the ApplicationSet git file generator flattens
# nested YAML and the gitops-yaml-bump task edits these keys by name. CI owns
# targetRevision<Env>/imageTag<Env> — merge to main bumps Dev, a v* tag bumps
# Staging and Prod. Do not rename keys without updating the pipelines.
app: <APP>
repoURL: <REPO_URL>
chartPath: <CHART_PATH>
# Chart-relative overlay; "" means the chart defaults. The ApplicationSet
# renders `helm.valueFiles` only when this is non-empty.
valuesFile: ""
imageRepository: <IMAGE_REPOSITORY>
targetRevisionDev: main
targetRevisionStaging: main
targetRevisionProd: main
imageTagDev: latest
imageTagStaging: latest
imageTagProd: latest
