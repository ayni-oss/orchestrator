{{- define "worker-fixtures.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" -}}
{{- end }}

{{- define "worker-fixtures.commonLabels" -}}
helm.sh/chart: {{ include "worker-fixtures.chart" . }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
app.kubernetes.io/part-of: ayni-orchestrator
{{- end }}

{{- define "worker-fixtures.validateSelection" -}}
{{- if and .Values.fixtures.managedCheck.enabled .Values.fixtures.artifactRejection.enabled -}}
{{- fail "fixtures.managedCheck.enabled and fixtures.artifactRejection.enabled cannot both be true" -}}
{{- end -}}
{{- end }}
