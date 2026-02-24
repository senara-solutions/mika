{{- define "mika-customer.name" -}}
mika-{{ .Values.customer.id }}
{{- end }}

{{- define "mika-customer.labels" -}}
app.kubernetes.io/name: mika-agent
app.kubernetes.io/instance: {{ include "mika-customer.name" . }}
mika.io/customer-id: {{ .Values.customer.id | quote }}
{{- end }}

{{- define "mika-customer.selectorLabels" -}}
app.kubernetes.io/name: mika-agent
app.kubernetes.io/instance: {{ include "mika-customer.name" . }}
{{- end }}
