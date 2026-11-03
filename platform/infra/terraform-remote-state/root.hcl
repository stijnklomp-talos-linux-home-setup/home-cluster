locals {
  project        = "home-terraform-state"
  region         = "eu-west-1"
  aws_account_id = "220402652377" # home AWS account (SSO profile `testing`)
}

# Bootstrap complete: the bucket exists, this repo now self-manages its own
# state in S3 like every other home Terraform repo.
remote_state {
  backend = "s3"
  generate = {
    path      = "backend.tf"
    if_exists = "overwrite_terragrunt"
  }
  config = {
    bucket       = "home-terraform-state-${local.aws_account_id}"
    key          = "${path_relative_to_include()}/terraform.tfstate"
    region       = local.region
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
      }
    }
  EOF
}

generate "providers" {
  path      = "providers.generated.tf"
  if_exists = "overwrite_terragrunt"
  contents  = <<-EOF
    provider "aws" {
      region = var.region
    }
  EOF
}