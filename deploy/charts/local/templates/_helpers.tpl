{{- define "local.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" -}}
{{- end }}

{{- define "local.commonLabels" -}}
helm.sh/chart: {{ include "local.chart" . }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
app.kubernetes.io/part-of: ayni-orchestrator
{{- end }}

{{- define "local.observabilityLabels" -}}
helm.sh/chart: {{ include "local.chart" . }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
app.kubernetes.io/part-of: ayni-orchestrator-observability
{{- end }}

{{- define "local.prometheusClusterRoleName" -}}
{{- printf "%s-prometheus" .Release.Namespace | trunc 63 | trimSuffix "-" -}}
{{- end }}

{{- define "local.validateFixtureSelection" -}}
{{- if and .Values.fixtures.managedCheck.enabled .Values.fixtures.artifactRejection.enabled -}}
{{- fail "fixtures.managedCheck.enabled and fixtures.artifactRejection.enabled cannot both be true" -}}
{{- end -}}
{{- $application := index .Values "ayni-orchestrator" -}}
{{- if and $application.enabled .Values.fixtures.managedCheck.enabled .Values.fixtures.managedCheck.createEvidenceClaim -}}
{{- fail "fixtures.managedCheck.createEvidenceClaim requires ayni-orchestrator.enabled=false" -}}
{{- end -}}
{{- end }}
