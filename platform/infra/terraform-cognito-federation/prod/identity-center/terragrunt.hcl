include "root" {
  path = find_in_parent_folders("root.hcl")
}

locals {
  root_vars = read_terragrunt_config(find_in_parent_folders("root.hcl"))
}

terraform {
  source = "../../modules/identity-center"
}

inputs = {
  region = local.root_vars.locals.region
}