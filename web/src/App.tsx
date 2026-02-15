import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { Suspense, lazy, useEffect } from 'react'

const Launcher = lazy(() => import('@/features/launcher/Launcher'))
const WidgetLayer = lazy(() => import('@/features/widget/WidgetLayer'))

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      retry: 1,
      refetchOnWindowFocus: false,
    },
  },
})

function App() {
  type WindowWithTauriMeta = Window & {
    __TAURI_METADATA__?: unknown
    __TAURI__?: { metadata?: unknown }
    __TAURI_INTERNALS__?: { metadata?: unknown }
  }

  const isWidget = (() => {
    if (typeof window === 'undefined') {
      return false
    }

    const tauriMeta =
      (window as WindowWithTauriMeta).__TAURI_METADATA__ ||
      (window as WindowWithTauriMeta).__TAURI__?.metadata ||
      (window as WindowWithTauriMeta).__TAURI_INTERNALS__?.metadata

    if (!tauriMeta) {
      return false
    }

    try {
      return getCurrentWindow().label === 'widget'
    } catch {
      return false
    }
  })()

  useEffect(() => {
    document.documentElement.style.backgroundColor = 'transparent'
    document.body.style.backgroundColor = 'transparent'
  }, [isWidget]);

  return (
    <QueryClientProvider client={queryClient}>
      <Suspense fallback={<div className="h-screen w-screen bg-transparent" />}>
        {isWidget ? (
          <WidgetLayer />
        ) : (
          <div className="h-screen w-screen overflow-hidden relative flex items-end justify-center bg-transparent">
            <Launcher />
          </div>
        )}
      </Suspense>
    </QueryClientProvider>
  )
}

export default App
