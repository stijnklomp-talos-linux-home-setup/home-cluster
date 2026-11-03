{{/*
Helpers for the tenant-platform chart.

The registry is the only source of tenant names. Nothing here derives a
namespace from an owner: the registry always carries the explicit namespace
alias (e.g. stijnklomp-LAVE -> stijnklomp-lave).
*/}}

{{/*
Turn a developer email into a DNS-1123 subdomain fragment for object names:
lowercase, runs of non-alphanumerics collapsed to '-', at most 200 characters.
Called with the email as the template argument.
*/}}
{{- define "tenant-platform.developerSlug" -}}
{{- $slug := regexReplaceAll "[^a-z0-9]+" (lower .) "-" -}}
{{- $slug | trimAll "-" | trunc 200 | trimSuffix "-" -}}
{{- end -}}

{{/*
Comma-separated, alphabetically sorted, de-duplicated developer emails across
all tenants. Empty when no tenant lists a developer.
*/}}
{{- define "tenant-platform.uniqueDevelopers" -}}
{{- $unique := dict -}}
{{- range $tenant := (.Values.tenants | default list) -}}
{{- range $developer := ($tenant.developers | default list) -}}
{{- $_ := set $unique (toString $developer) (toString $developer) -}}
{{- end -}}
{{- end -}}
{{- $emails := list -}}
{{- range $email := (keys $unique | sortAlpha) -}}
{{- $emails = append $emails $email -}}
{{- end -}}
{{- join "," $emails -}}
{{- end -}}

{{/*
Validate the whole registry. Every problem is reported in one failure so a
bad registry does not need one render per typo. Called from validate.yaml.
*/}}
{{- define "tenant-platform.validateRegistry" -}}
{{- $errors := list -}}
{{- $tenants := .Values.tenants | default list -}}
{{- if not (kindIs "slice" $tenants) -}}
{{- $errors = append $errors "tenants must be a list" -}}
{{- else -}}
{{- $owners := dict -}}
{{- $namespaces := dict -}}
{{- $repoNames := dict -}}
{{- $repoURLs := dict -}}
{{- $developerSlugs := dict -}}
{{- $dns1123Label := "^[a-z0-9]([-a-z0-9]*[a-z0-9])?$" -}}
{{- $dnsSubdomain := "^[a-z0-9]([-a-z0-9]*[a-z0-9])?(\\.[a-z0-9]([-a-z0-9]*[a-z0-9])?)*$" -}}

{{- range $i, $tenant := $tenants -}}
{{- $where := printf "tenants[%d]" $i -}}
{{- if not (kindIs "map" $tenant) -}}
{{- $errors = append $errors (printf "%s: must be a map" $where) -}}
{{- else -}}
{{- $owner := $tenant.owner | default "" | toString -}}
{{- $namespace := $tenant.namespace | default "" | toString -}}
{{- $tier := $tenant.tier | default "" | toString -}}

{{- if not $owner -}}
{{- $errors = append $errors (printf "%s: owner is required" $where) -}}
{{- else -}}
{{- if gt (len $owner) 63 -}}
{{- $errors = append $errors (printf "%s: owner %q is longer than 63 characters (home.cluster/tenant label value)" $where $owner) -}}
{{- end -}}
{{- $ownerKey := lower $owner -}}
{{- if hasKey $owners $ownerKey -}}
{{- $errors = append $errors (printf "%s: duplicate owner %q (also declared at %s)" $where $owner (get $owners $ownerKey)) -}}
{{- else -}}
{{- $_ := set $owners $ownerKey $where -}}
{{- end -}}
{{- end -}}

{{- if not $namespace -}}
{{- $errors = append $errors (printf "%s: namespace is required" $where) -}}
{{- else -}}
{{- if or (gt (len $namespace) 63) (not (regexMatch $dns1123Label $namespace)) -}}
{{- $errors = append $errors (printf "%s: namespace %q is not a valid DNS-1123 label (lowercase alphanumerics and '-', at most 63 characters)" $where $namespace) -}}
{{- end -}}
{{- if hasKey $namespaces $namespace -}}
{{- $errors = append $errors (printf "%s: duplicate namespace %q (also declared at %s)" $where $namespace (get $namespaces $namespace)) -}}
{{- else -}}
{{- $_ := set $namespaces $namespace $where -}}
{{- end -}}
{{- end -}}

{{- if not (has $tier (list "build" "deploy")) -}}
{{- $errors = append $errors (printf "%s: tier must be one of build or deploy (got %q)" $where $tier) -}}
{{- end -}}

{{- if hasKey $tenant "concurrencyLimit" -}}
{{- $limit := $tenant.concurrencyLimit -}}
{{- if or (not (kindIs "float64" $limit)) (lt (int $limit) 1) (ne (toString (int $limit)) (toString $limit)) -}}
{{- $errors = append $errors (printf "%s: concurrencyLimit must be a positive integer (got %v)" $where $limit) -}}
{{- end -}}
{{- end -}}

{{- if and (hasKey $tenant "quota") (not (kindIs "map" $tenant.quota)) -}}
{{- $errors = append $errors (printf "%s: quota must be a map of ResourceQuota hard limits" $where) -}}
{{- end -}}
{{- if and (hasKey $tenant "limitRange") (not (kindIs "map" $tenant.limitRange)) -}}
{{- $errors = append $errors (printf "%s: limitRange must be a map with default/defaultRequest" $where) -}}
{{- end -}}

{{- if hasKey $tenant "developers" -}}
{{- if not (kindIs "slice" $tenant.developers) -}}
{{- $errors = append $errors (printf "%s: developers must be a list of email addresses" $where) -}}
{{- else -}}
{{- $tenantSlugs := dict -}}
{{- range $j, $developer := $tenant.developers -}}
{{- $devWhere := printf "%s.developers[%d]" $where $j -}}
{{- $email := toString $developer -}}
{{- if not $email -}}
{{- $errors = append $errors (printf "%s: developer email must not be empty" $devWhere) -}}
{{- else -}}
{{- $slug := include "tenant-platform.developerSlug" $email -}}
{{- if not $slug -}}
{{- $errors = append $errors (printf "%s: developer email %q does not produce a valid object-name slug" $devWhere $email) -}}
{{- else -}}
{{- if hasKey $tenantSlugs $slug -}}
{{- $errors = append $errors (printf "%s: developer %q collides with %q after slugging to %q" $devWhere $email (get $tenantSlugs $slug) $slug) -}}
{{- else -}}
{{- $_ := set $tenantSlugs $slug $email -}}
{{- end -}}
{{- if hasKey $developerSlugs $slug -}}
{{- if ne (get $developerSlugs $slug) $email -}}
{{- $errors = append $errors (printf "%s: developer %q collides with %q after slugging to %q" $devWhere $email (get $developerSlugs $slug) $slug) -}}
{{- end -}}
{{- else -}}
{{- $_ := set $developerSlugs $slug $email -}}
{{- end -}}
{{- end -}}
{{- end -}}
{{- end -}}
{{- end -}}
{{- end -}}

{{- $repos := $tenant.repos | default list -}}
{{- if not (kindIs "slice" $repos) -}}
{{- $errors = append $errors (printf "%s: repos must be a list (it may be empty)" $where) -}}
{{- else -}}
{{- range $j, $repo := $repos -}}
{{- $repoWhere := printf "%s.repos[%d]" $where $j -}}
{{- if not (kindIs "map" $repo) -}}
{{- $errors = append $errors (printf "%s: must be a map with name and url" $repoWhere) -}}
{{- else -}}
{{- $name := $repo.name | default "" | toString -}}
{{- $url := $repo.url | default "" | toString -}}
{{- if not $name -}}
{{- $errors = append $errors (printf "%s: repo name is required" $repoWhere) -}}
{{- else -}}
{{- if or (gt (len $name) 253) (not (regexMatch $dnsSubdomain $name)) -}}
{{- $errors = append $errors (printf "%s: repo name %q is not a valid DNS subdomain (lowercase alphanumerics, '-' and '.')" $repoWhere $name) -}}
{{- end -}}
{{- $nameKey := printf "%s/%s" $namespace $name -}}
{{- if hasKey $repoNames $nameKey -}}
{{- $errors = append $errors (printf "%s: duplicate repo name %q in namespace %q" $repoWhere $name $namespace) -}}
{{- else -}}
{{- $_ := set $repoNames $nameKey $repoWhere -}}
{{- end -}}
{{- end -}}
{{- if not $url -}}
{{- $errors = append $errors (printf "%s: repo url is required" $repoWhere) -}}
{{- else -}}
{{- $urlKey := printf "%s/%s" $namespace $url -}}
{{- if hasKey $repoURLs $urlKey -}}
{{- $errors = append $errors (printf "%s: duplicate repo url %q in namespace %q" $repoWhere $url $namespace) -}}
{{- else -}}
{{- $_ := set $repoURLs $urlKey $repoWhere -}}
{{- end -}}
{{- end -}}
{{- end -}}
{{- end -}}
{{- end -}}
{{- end -}}
{{- end -}}
{{- end -}}

{{- if $errors -}}
{{- fail (printf "tenant-platform registry validation failed:\n  - %s" (join "\n  - " $errors)) -}}
{{- end -}}
{{- end -}}
