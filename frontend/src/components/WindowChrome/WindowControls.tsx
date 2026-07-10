'use client'

import { useEffect, useState } from 'react'
import { Minus, Square, Copy, X } from 'lucide-react'

async function currentWindow() {
  const { getCurrentWindow } = await import('@tauri-apps/api/window')
  return getCurrentWindow()
}

/**
 * Minimize / maximize-restore / close buttons for undecorated windows
 * (Windows and Linux). Close goes through window.close() so the Rust
 * CloseRequested handler keeps hiding to tray instead of quitting.
 */
export function WindowControls() {
  const [isMaximized, setIsMaximized] = useState(false)

  useEffect(() => {
    let unlisten: (() => void) | undefined
    let cancelled = false

    const setup = async () => {
      try {
        const win = await currentWindow()
        setIsMaximized(await win.isMaximized())
        const un = await win.onResized(async () => {
          setIsMaximized(await win.isMaximized())
        })
        if (cancelled) {
          un()
        } else {
          unlisten = un
        }
      } catch (error) {
        console.warn('[WindowControls] Failed to track maximize state:', error)
      }
    }
    setup()

    return () => {
      cancelled = true
      unlisten?.()
    }
  }, [])

  const minimize = async () => (await currentWindow()).minimize()
  const toggleMaximize = async () => (await currentWindow()).toggleMaximize()
  const close = async () => (await currentWindow()).close()

  const buttonClass =
    'flex h-10 w-[46px] items-center justify-center text-muted-foreground transition-colors hover:bg-accent hover:text-foreground'

  return (
    <div className="flex">
      <button onClick={minimize} className={buttonClass} aria-label="Minimize window">
        <Minus className="h-4 w-4" />
      </button>
      <button
        onClick={toggleMaximize}
        className={buttonClass}
        aria-label={isMaximized ? 'Restore window' : 'Maximize window'}
      >
        {isMaximized ? <Copy className="h-3.5 w-3.5 -scale-x-100" /> : <Square className="h-3.5 w-3.5" />}
      </button>
      <button
        onClick={close}
        className="flex h-10 w-[46px] items-center justify-center text-muted-foreground transition-colors hover:bg-destructive hover:text-destructive-foreground"
        aria-label="Close window"
      >
        <X className="h-4 w-4" />
      </button>
    </div>
  )
}
