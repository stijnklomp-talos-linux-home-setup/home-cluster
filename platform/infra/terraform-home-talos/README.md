# Terraform: Talos machine configs (home-cluster-1)

Manages the Talos machine configurations (controlplane + worker) as code with the `siderolabs/talos` provider (v0.11.0), so that OS/cluster-level settings stop living only in `home-cluster-1-config/` YAML files.

## Flow

```sh
cd prod
terragrunt init
terragrunt plan
terragrunt apply   # writes machine_config_* / talos_config / kubeconfig outputs
```

Apply configs to nodes (diff against what's running):

```sh
talosctl --talosconfig <talos_config> -n 192.168.1.107 apply-config --file <machine_config_controlplane> --mode try
talosctl --talosconfig <talos_config> -n 192.168.1.108 apply-config --file <machine_config_worker> --mode try
```

## TODO

- The machine secrets (cluster CA material) must be moved from `home-cluster-1-config/` into a `machine_secrets` input (gitignored / secrets-managed) — currently only placeholder.
- Port the existing manual machine-config tweaks (kubePrism, hostDNS, UKI, quotas, PodSecurity, audit policy) into `config_patches` in the module.
- Add cert SANs for the control-plane endpoint hostname(s) used on the LAN.
- Decide replacement of the plaintext `home-cluster-1-config` files by these managed outputs.