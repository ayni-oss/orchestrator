{{- define "ayni-orchestrator.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" -}}
{{- end }}

{{- define "ayni-orchestrator.commonLabels" -}}
helm.sh/chart: {{ include "ayni-orchestrator.chart" . }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
app.kubernetes.io/part-of: ayni-orchestrator
{{- end }}

{{- define "ayni-orchestrator.traceEndpoint" -}}
{{- if .Values.observability.tracesEndpoint -}}
{{- .Values.observability.tracesEndpoint -}}
{{- else if .Values.observability.collector.enabled -}}
{{- printf "http://workflow-traces.%s.svc:4318/v1/traces" .Release.Namespace -}}
{{- end -}}
{{- end -}}
