import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { BrowserRouter } from 'react-router'
import App from './App.tsx'
import './index.css'

// Resolve base path: server-injected config (embedded mode) or default '/'
const basePath = window.__MIKA_CONFIG__?.basePath || '/'

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      staleTime: 5000,
      retry: 1,
    },
  },
})

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <BrowserRouter basename={basePath}>
        <App />
      </BrowserRouter>
    </QueryClientProvider>
  </StrictMode>,
)
