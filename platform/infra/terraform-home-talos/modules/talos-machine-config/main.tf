# Provider/version constraints live in the terragrunt-generated
# versions.generated.tf (root.hcl) — do NOT declare terraform{} here.

variable "cluster_name" {
  type = string
}

variable "k8s_version" {
  type = string
}

variable "talos_version" {
  type = string
}

variable "control_plane_endpoint" {
  type = string
}

variable "control_plane_nodes" {
  type = list(string)
}

variable "worker_nodes" {
  type = list(string)
}

# Cluster secrets (CA material, tokens, cluster id/secret).
#
# This resource was imported from the live cluster's secrets bundle — see the
# migration runbook in README.md. It must NEVER be created from scratch for the
# existing cluster: `terraform apply` on an unimported resource would generate a
# brand-new CA/tokens and break the cluster.
resource "talos_machine_secrets" "this" {
  # The provider sets this to "v1.3" when importing a v1.3+ secrets bundle
  # (one without an AES-CBC key). It only affects secret generation on create,
  # which never runs because this resource is imported; pinning it to the
  # imported value keeps `plan` clean.
  talos_version = "v1.3"
}

locals {
  # Image Factory schematics bake in the low-power kernel args
  # (pcie_aspm=force pcie_aspm.policy=power) and, for workers, the iscsi-tools
  # extension. They are part of the UKI and cannot be changed by a config patch;
  # re-minting a schematic means upgrading the nodes to the new image.
  # See platform/node/schematics/{controlplane,worker}-schematic.yaml.
  installer_image_controlplane = "factory.talos.dev/metal-installer/177b1d1bf5738656688be6a31c92bbd1596d50d14c1afcc805a0173b4076a6e3:v1.13.9"
  installer_image_worker       = "factory.talos.dev/metal-installer/8e594bc9dbab2dce4bfcf4a7248c6c2462dca385c77a4ba3812878373179ba27:v1.13.9"

  # The Talos v1.13 contract already emits everything else the live nodes run:
  # UKI cmdline flag, disk quotas, kubePrism, hostDNS, kubelet seccomp, the
  # control-plane load-balancer label, PodSecurity admission, audit policy,
  # discovery registries, pod/service subnets and the HostnameConfig document.
  # On top of the contract defaults we only need the installer image and, for
  # the control plane, the NVMe install disk (the provider defaults to /dev/sda).
  controlplane_patches = [
    <<-EOT
    machine:
      install:
        disk: /dev/nvme0n1
        image: ${local.installer_image_controlplane}
    EOT
  ]

  worker_patches = [
    <<-EOT
    machine:
      install:
        image: ${local.installer_image_worker}
    EOT
  ]
}

# Talos provider 0.11: machine configs are DATA sources (talos_machine_configuration
# with machine_type); talos_machine_configuration_apply applies them to nodes.
data "talos_machine_configuration" "controlplane" {
  cluster_name       = var.cluster_name
  cluster_endpoint   = var.control_plane_endpoint
  kubernetes_version = var.k8s_version
  talos_version      = var.talos_version
  machine_type       = "controlplane"
  machine_secrets    = talos_machine_secrets.this.machine_secrets
  config_patches     = local.controlplane_patches

  # TODO: add cert SANs if the control plane is ever reached by hostname.
}

data "talos_machine_configuration" "worker" {
  for_each           = toset(var.worker_nodes)
  cluster_name       = var.cluster_name
  cluster_endpoint   = var.control_plane_endpoint
  kubernetes_version = var.k8s_version
  talos_version      = var.talos_version
  machine_type       = "worker"
  machine_secrets    = talos_machine_secrets.this.machine_secrets
  config_patches     = local.worker_patches
}

# Client configs are derived from the same secrets bundle, so they carry the
# live cluster CA. The admin client certificate is minted by the provider and
# stored in state; it is renewed automatically when it nears expiry.
data "talos_client_configuration" "this" {
  cluster_name         = var.cluster_name
  client_configuration = talos_machine_secrets.this.client_configuration
  endpoints            = var.control_plane_nodes
  nodes                = concat(var.control_plane_nodes, var.worker_nodes)
}

# Managed as a resource (not a data source) so the provider renews the admin
# certificate stored in state before it expires.
resource "talos_cluster_kubeconfig" "this" {
  client_configuration = talos_machine_secrets.this.client_configuration
  node                 = var.control_plane_nodes[0]
}

output "machine_config_controlplane" {
  value     = data.talos_machine_configuration.controlplane.machine_configuration
  sensitive = true
}

output "machine_config_workers" {
  value     = { for k, v in data.talos_machine_configuration.worker : k => v.machine_configuration }
  sensitive = true
}

output "talosconfig" {
  description = "Talos client config for the cluster (treat as a secret)."
  value       = data.talos_client_configuration.this.talos_config
  sensitive   = true
}

output "kubeconfig" {
  description = "Kubernetes admin kubeconfig for the cluster (treat as a secret)."
  value       = talos_cluster_kubeconfig.this.kubeconfig_raw
  sensitive   = true
}
