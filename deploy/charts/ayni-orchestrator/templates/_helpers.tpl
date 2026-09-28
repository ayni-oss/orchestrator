{{- define "ayni-orchestrator.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" -}}
{{- end }}

{{- define "ayni-orchestrator.commonLabels" -}}
helm.sh/chart: {{ include "ayni-orchestrator.chart" . }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
app.kubernetes.io/part-of: ayni-orchestrator
{{- end }}
