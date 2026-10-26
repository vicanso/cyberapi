{{/* The release's name, kept to what a resource name may be. */}}
{{- define "cyberapi.fullname" -}}
{{- if contains .Chart.Name .Release.Name -}}
{{- .Release.Name | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- printf "%s-%s" .Release.Name .Chart.Name | trunc 63 | trimSuffix "-" -}}
{{- end -}}
{{- end -}}

{{- define "cyberapi.labels" -}}
helm.sh/chart: {{ printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" }}
{{ include "cyberapi.selectorLabels" . }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end -}}

{{- define "cyberapi.selectorLabels" -}}
app.kubernetes.io/name: {{ .Chart.Name }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end -}}

{{/* The Secret this chart writes, for what was given as a value. */}}
{{- define "cyberapi.secretName" -}}
{{- include "cyberapi.fullname" . -}}
{{- end -}}

{{/* The path the bridge answers under: the base path, or the root. */}}
{{- define "cyberapi.path" -}}
{{- with .Values.bridge.basePath -}}{{ trimSuffix "/" . }}{{- end -}}
{{- end -}}
