/// <reference types="vite/client" />

interface ImportMetaEnv {
  readonly VITE_MIKA_DASHBOARD_TOKEN: string
}

interface ImportMeta {
  readonly env: ImportMetaEnv
}
