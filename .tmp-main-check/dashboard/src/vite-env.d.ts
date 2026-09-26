/// <reference types="vite/client" />

interface MikaConfig {
  token: string
  basePath: string
}

interface Window {
  __MIKA_CONFIG__?: MikaConfig
}

interface ImportMetaEnv {
  readonly VITE_MIKA_DASHBOARD_TOKEN: string
}

interface ImportMeta {
  readonly env: ImportMetaEnv
}
