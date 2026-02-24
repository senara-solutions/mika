{{- define "mika-gateway.name" -}}
mika-gateway
{{- end }}

{{- define "mika-gateway.labels" -}}
app.kubernetes.io/name: mika-gateway
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{- define "mika-gateway.selectorLabels" -}}
app.kubernetes.io/name: mika-gateway
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}
