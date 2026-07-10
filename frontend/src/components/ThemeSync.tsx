'use client'

import { useEffect } from 'react'
import { useTheme } from 'next-themes'
import { Toaster } from 'sonner'

function isTauri(): boolean {
  return typeof (window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ !== 'undefined'
}

/**
 * Bridges the app theme (next-themes) to the Tauri window so native chrome —
 * menus, macOS traffic lights, system dialogs — follows the in-app setting.
 * Renders nothing; must be mounted inside ThemeProvider.
 */
export function ThemeSync() {
  const { theme, resolvedTheme } = useTheme()

  useEffect(() => {
    if (!isTauri()) return
    const apply = async () => {
      try {
        const { getCurrentWindow } = await import('@tauri-apps/api/window')
        // null = follow the OS; otherwise pin the resolved theme
        await getCurrentWindow().setTheme(
          theme === 'system' ? null : resolvedTheme === 'light' ? 'light' : 'dark'
        )
      } catch (error) {
        console.warn('[ThemeSync] Failed to set window theme:', error)
      }
    }
    apply()
  }, [theme, resolvedTheme])

  return null
}

/** sonner Toaster that follows the app theme. Must be mounted inside ThemeProvider. */
export function ThemedToaster() {
  const { resolvedTheme } = useTheme()
  return (
    <Toaster
      position="bottom-center"
      richColors
      closeButton
      theme={resolvedTheme === 'light' ? 'light' : 'dark'}
    />
  )
}
