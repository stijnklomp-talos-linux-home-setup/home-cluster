locals {
  project        = "home-cognito"
  region         = "eu-west-1"
  aws_account_id = "220402652377" # home AWS account (SSO profile `testing`)
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