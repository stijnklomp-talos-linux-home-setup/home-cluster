locals {
  project        = "home-cluster-1"
  cluster_name   = "home-cluster-1"
  k8s_version    = "v1.36.4"
  talos_version  = "v1.13.9"
  endpoint       = "https://192.168.1.107:6443"
  nodes          = ["192.168.1.107", "192.168.1.108", "192.168.1.106", "192.168.1.109"]
  control_planes = ["192.168.1.107"]
  workers        = ["192.168.1.108", "192.168.1.106", "192.168.1.109"]
  aws_account_id = "220402652377" # home AWS account — state bucket owner
}

# State lives in the shared S3 bucket created by terraform-remote-state
# (S3-native locking via use_lockfile — no DynamoDB table).
remote_state {
  backend = "s3"
  generate = {
    path      = "backend.tf"
    if_exists = "overwrite_terragrunt"
  }
  config = {
    bucket       = "home-terraform-state-${local.aws_account_id}"
    key          = "${path_relative_to_include()}/terraform.tfstate"
    region       = "eu-west-1"
    encrypt      = true
    use_lockfile = true
  }
}

generate "versions" {
  path      = "versions.generated.tf"
  if_exists = "overwrite_terragrunt"
  contents  = <<-EOF
    terraform {
      required_version = ">= 1.15.9"
      required_providers {
        aws = {
          source  = "hashicorp/aws"
          version = "~> 6.64.0"
        }
        talos = {
          source  = "siderolabs/talos"
          version = "~> 0.11.0"
        }
      }
    }
  EOF
}

generate "providers" {
  path      = "providers.generated.tf"
  if_exists = "overwrite_terragrunt"
  contents  = <<-EOF
    provider "talos" {}
  EOF
}