# S3 state store for ALL Terraform applies in the home AWS account.
#
# Bootstrap unit: its own state starts LOCAL (no remote_state in root.hcl yet),
# this unit is applied once, then root.hcl gains a remote_state block and the
# unit self-migrates (terragrunt init -migrate-state). See README.md.
#
# No DynamoDB lock table: S3-native locking via the backend's use_lockfile flag.

data "aws_caller_identity" "current" {}

locals {
  bucket_name = "home-terraform-state-${data.aws_caller_identity.current.account_id}"
}

resource "aws_s3_bucket" "state" {
  bucket = local.bucket_name

  tags = {
    "managed-by" = "terragrunt"
    "purpose"    = "terraform-state"
  }
}

resource "aws_s3_bucket_versioning" "state" {
  bucket = aws_s3_bucket.state.id

  versioning_configuration {
    status = "Enabled"
  }
}

resource "aws_s3_bucket_server_side_encryption_configuration" "state" {
  bucket = aws_s3_bucket.state.id

  rule {
    apply_server_side_encryption_by_default {
      sse_algorithm = "AES256"
    }
  }
}

resource "aws_s3_bucket_public_access_block" "state" {
  bucket = aws_s3_bucket.state.id

  block_public_acls       = true
  block_public_policy     = true
  ignore_public_acls      = true
  restrict_public_buckets = true
}