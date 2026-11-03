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

variable "machine_secrets" {
  type        = any
  default     = null
  description = "TODO: real cluster CA material from secrets management (never commit). Required at apply — until wired, this module validates/plans only."
}

# Machine configs (one per node; differentiated by role)
locals {
  # TODO: port the remaining patches currently applied manually in
  # home-cluster-1-config/ (kubePrism, hostDNS, UKI boot, disk quota, pod
  # security, audit policy...) into config patches below so the cluster config
  # is fully code-managed.
  #
  # Low-power kernel args (pcie_aspm=force pcie_aspm.policy=power) are baked into
  # the UKI at image build time — an Image Factory schematic (ID 177b1d1b...);
  # see talos-linux-setup/schematics/controlplane-schematic.yaml. Changing them
  # means re-minting the schematic and upgrading the nodes to the new image.
  patches = [
    <<-EOT
    machine:
      install:
        image: factory.talos.dev/metal-installer/177b1d1bf5738656688be6a31c92bbd1596d50d14c1afcc805a0173b4076a6e3:v1.13.9
    EOT
  ]
  #
  # Common worker patches: Longhorn requires the iSCSI initiator on the host
  # (replicas attach via iSCSI). Talos v1.13 deprecated machine.install.extensions —
  # the iscsi-tools extension (plus the low-power kernel args) is baked into a
  # custom installer via an Image Factory schematic (ID 8e594bc9..., siderolabs/iscsi-tools
  # + pcie_aspm args); see talos-linux-setup/schematics/worker-schematic.yaml.
  worker_patches = [
    <<-EOT
    machine:
      install:
        image: factory.talos.dev/metal-installer/8e594bc9dbab2dce4bfcf4a7248c6c2462dca385c77a4ba3812878373179ba27:v1.13.9
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
  machine_secrets    = var.machine_secrets
  config_patches     = local.patches

  # TODO: cert SANs for LAN access (control plane endpoint IP/DNS)
}

data "talos_machine_configuration" "worker" {
  for_each           = toset(var.worker_nodes)
  cluster_name       = var.cluster_name
  cluster_endpoint   = var.control_plane_endpoint
  kubernetes_version = var.k8s_version
  talos_version      = var.talos_version
  machine_type       = "worker"
  machine_secrets    = var.machine_secrets
  config_patches     = local.worker_patches
}

# talosconfig + kubeconfig outputs are DEFERRED: they need
# client_configuration from talos_machine_secrets (see machine_secrets TODO),
# which requires importing the live cluster's CA material — separate follow-up.

output "machine_config_controlplane" {
  value     = data.talos_machine_configuration.controlplane.machine_configuration
  sensitive = true
}

output "machine_config_workers" {
  value     = { for k, v in data.talos_machine_configuration.worker : k => v.machine_configuration }
  sensitive = true
}