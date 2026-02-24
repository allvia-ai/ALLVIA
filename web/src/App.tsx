import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { Suspense, lazy, useEffect, useMemo, useState } from 'react'

const Launcher = lazy(() => import('@/features/launcher/Launcher'))
const WidgetLayer = lazy(() => import('@/features/widget/WidgetLayer'))
const Sidebar = lazy(() => import('@/components/Sidebar').then((m) => ({ default: m.Sidebar })))
const Dashboard = lazy(() => import('@/features/dashboard/Dashboard'))
const Routines = lazy(() => import('@/features/routines/Routines'))
const Workflows = lazy(() => import('@/features/workflows/Workflows'))
const Settings = lazy(() => import('@/features/settings/Settings'))
const ChatPanel = lazy(() => import('@/features/chat/ChatPanel'))

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      retry: 1,
      refetchOnWindowFocus: false,
    },
  },
})

type ShellMode = 'launcher' | 'legacy'
type LegacyView = 'dashboard' | 'routines' | 'workflows' | 'settings' | 'chat'

const DEFAULT_LEGACY_VIEW: LegacyView = 'dashboard'
const LEGACY_VIEW_SET = new Set<LegacyView>([
  'dashboard',
  'routines',
  'workflows',
  'settings',
  'chat',
])

const normalizeLegacyView = (raw: string | null | undefined): LegacyView => {
  if (!raw) return DEFAULT_LEGACY_VIEW
  const candidate = raw.trim().toLowerCase()
  if (LEGACY_VIEW_SET.has(candidate as LegacyView)) {
    return candidate as LegacyView
  }
  return DEFAULT_LEGACY_VIEW
}

const resolveShellFromHash = (): { mode: ShellMode; view: LegacyView } => {
  if (typeof window === 'undefined') {
    return { mode: 'launcher', view: DEFAULT_LEGACY_VIEW }
  }
  const hash = window.location.hash.replace(/^#/, '').trim()
  if (!hash) {
    return { mode: 'launcher', view: DEFAULT_LEGACY_VIEW }
  }
  if (hash === 'legacy') {
    return { mode: 'legacy', view: DEFAULT_LEGACY_VIEW }
  }
  if (hash.startsWith('legacy/')) {
    return { mode: 'legacy', view: normalizeLegacyView(hash.split('/')[1]) }
  }
  return { mode: 'launcher', view: DEFAULT_LEGACY_VIEW }
}

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
  const initialShell = resolveShellFromHash()
  const [shellMode, setShellMode] = useState<ShellMode>(initialShell.mode)
  const [legacyView, setLegacyView] = useState<LegacyView>(initialShell.view)

  useEffect(() => {
    if (typeof window === 'undefined') return
    const onHashChange = () => {
      const next = resolveShellFromHash()
      setShellMode(next.mode)
      setLegacyView(next.view)
    }
    window.addEventListener('hashchange', onHashChange)
    return () => window.removeEventListener('hashchange', onHashChange)
  }, [])

  useEffect(() => {
    const transparent = isWidget || shellMode === 'launcher'
    const bg = transparent ? 'transparent' : '#060b14'
    document.documentElement.style.backgroundColor = bg
    document.documentElement.style.setProperty('background', bg, 'important')
    document.body.style.backgroundColor = bg
    document.body.style.setProperty('background', bg, 'important')
    const root = document.getElementById('root')
    if (root) {
      root.style.backgroundColor = bg
      root.style.setProperty('background', bg, 'important')
    }
  }, [isWidget, shellMode]);

  useEffect(() => {
    if (typeof window === 'undefined' || isWidget) return
    const desiredHash = shellMode === 'legacy' ? `legacy/${legacyView}` : 'launcher'
    const currentHash = window.location.hash.replace(/^#/, '')
    if (currentHash !== desiredHash) {
      window.location.hash = desiredHash
    }
  }, [shellMode, legacyView, isWidget])

  const legacyPanel = useMemo(() => {
    switch (legacyView) {
      case 'dashboard':
        return <Dashboard />
      case 'routines':
        return <Routines />
      case 'workflows':
        return <Workflows />
      case 'settings':
        return <Settings />
      case 'chat':
        return (
          <div className="h-[calc(100vh-3rem)]">
            <ChatPanel />
          </div>
        )
      default:
        return <Dashboard />
    }
  }, [legacyView])

  return (
    <QueryClientProvider client={queryClient}>
      <Suspense fallback={<div className="h-screen w-screen bg-transparent" />}>
        {isWidget ? (
          <WidgetLayer />
        ) : (
          <>
            <div className="fixed left-4 top-4 z-50 flex items-center gap-2 rounded-xl border border-white/10 bg-black/35 px-2 py-1.5 backdrop-blur-md">
              <button
                onClick={() => setShellMode('launcher')}
                className={`rounded-md px-3 py-1.5 text-xs font-medium transition-colors ${
                  shellMode === 'launcher'
                    ? 'bg-white/20 text-white'
                    : 'text-white/70 hover:bg-white/10'
                }`}
              >
                Launcher
              </button>
              <button
                onClick={() => setShellMode('legacy')}
                className={`rounded-md px-3 py-1.5 text-xs font-medium transition-colors ${
                  shellMode === 'legacy'
                    ? 'bg-white/20 text-white'
                    : 'text-white/70 hover:bg-white/10'
                }`}
              >
                Legacy
              </button>
            </div>
            {shellMode === 'launcher' ? (
              <div className="h-screen w-screen overflow-hidden relative flex items-end justify-center bg-transparent">
                <Launcher />
              </div>
            ) : (
              <div className="h-screen w-screen overflow-hidden bg-[#060b14] text-white flex">
                <Sidebar
                  active={legacyView}
                  onNavigate={(id) => setLegacyView(normalizeLegacyView(id))}
                />
                <main className="flex-1 overflow-y-auto p-6">{legacyPanel}</main>
              </div>
            )}
          </>
        )}
      </Suspense>
    </QueryClientProvider>
  )
}

export default App
