include "root" {
  path = find_in_parent_folders("root.hcl")
}

locals {
  root_vars = read_terragrunt_config(find_in_parent_folders("root.hcl"))
}

terraform {
  source = "../modules/talos-machine-config"
}

inputs = {
  cluster_name           = local.root_vars.locals.cluster_name
  k8s_version            = local.root_vars.locals.k8s_version
  talos_version          = local.root_vars.locals.talos_version
  control_plane_endpoint = local.root_vars.locals.endpoint
  control_plane_nodes    = local.root_vars.locals.control_planes
  worker_nodes           = local.root_vars.locals.workers
}