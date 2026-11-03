{{/* Common name and labels. */}}
{{- define "home-orders.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "home-orders.fullname" -}}
{{- if .Values.fullnameOverride -}}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- $name := default .Chart.Name .Values.nameOverride -}}
{{- if contains $name .Release.Name -}}
{{- .Release.Name | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" -}}
{{- end -}}
{{- end -}}
{{- end -}}

{{- define "home-orders.labels" -}}
app.kubernetes.io/name: {{ include "home-orders.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
helm.sh/chart: {{ printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" }}
{{- end -}}

{{- define "home-orders.selectorLabels" -}}
app.kubernetes.io/name: {{ include "home-orders.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end -}}

{{/* GraphQL settings: the test JWKS server overrides the placeholders. */}}
{{- define "home-orders.graphqlJwksUrl" -}}
{{- if .Values.testJwks.enabled -}}
{{- printf "http://%s-jwks.%s.svc.cluster.local/jwks.json" (include "home-orders.fullname" .) .Release.Namespace -}}
{{- else -}}
{{- .Values.config.graphqlJwksUrl -}}
{{- end -}}
{{- end -}}

{{- define "home-orders.graphqlJwtIssuer" -}}
{{- if .Values.testJwks.enabled -}}
{{- .Values.testJwks.issuer -}}
{{- else -}}
{{- .Values.config.graphqlJwtIssuer -}}
{{- end -}}
{{- end -}}

{{- define "home-orders.graphqlJwtAudience" -}}
{{- if .Values.testJwks.enabled -}}
{{- .Values.testJwks.audience -}}
{{- else -}}
{{- .Values.config.graphqlJwtAudience -}}
{{- end -}}
{{- end -}}

{{- define "home-orders.graphqlJwtClientId" -}}
{{- if .Values.testJwks.enabled -}}
{{- .Values.testJwks.clientId | default "" -}}
{{- else -}}
{{- .Values.config.graphqlJwtClientId -}}
{{- end -}}
{{- end -}}

{{- define "home-orders.postgresSecretName" -}}
{{- printf "%s-postgres" (include "home-orders.fullname" .) -}}
{{- end -}}

{{/* The Postgres password: explicit value, existing Secret, or generated. */}}
{{- define "home-orders.postgresPassword" -}}
{{- if .Values.postgres.password -}}
{{- .Values.postgres.password -}}
{{- else -}}
{{- $secret := lookup "v1" "Secret" .Release.Namespace (include "home-orders.postgresSecretName" .) -}}
{{- if and $secret $secret.data (index $secret.data "POSTGRES_PASSWORD") -}}
{{- index $secret.data "POSTGRES_PASSWORD" | b64dec -}}
{{- else -}}
{{- randAlphaNum 32 -}}
{{- end -}}
{{- end -}}
{{- end -}}
