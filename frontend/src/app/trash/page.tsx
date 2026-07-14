'use client';

import React, { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Trash2, RotateCcw, AlertTriangle } from 'lucide-react';
import { toast } from 'sonner';

interface TrashedMeeting {
  id: string;
  title: string;
  createdAt: string;
  deletedAt: string;
}

const RETENTION_DAYS = 30;

// created_at is stored with a "+00:00" offset; deleted_at (naive UTC) has none.
// Normalize both to a parseable ISO string, assuming UTC when no zone is present.
function parseDbDate(s: string): Date {
  let iso = s.trim().replace(' ', 'T');
  if (!/[zZ]|[+-]\d\d:?\d\d$/.test(iso)) iso += 'Z';
  return new Date(iso);
}

function daysLeft(deletedAt: string): number {
  const d = parseDbDate(deletedAt);
  if (isNaN(d.getTime())) return RETENTION_DAYS;
  const elapsedDays = (Date.now() - d.getTime()) / 86_400_000;
  return Math.max(0, Math.ceil(RETENTION_DAYS - elapsedDays));
}

function formatDate(s: string): string {
  const d = parseDbDate(s);
  return isNaN(d.getTime())
    ? s
    : d.toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' });
}

export default function TrashPage() {
  const [items, setItems] = useState<TrashedMeeting[] | null>(null);
  const [confirmId, setConfirmId] = useState<string | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      const rows = await invoke<TrashedMeeting[]>('api_list_trashed_meetings');
      setItems(rows);
    } catch (e) {
      console.error('Failed to load trash:', e);
      toast.error('Failed to load trash', {
        description: e instanceof Error ? e.message : String(e),
      });
      setItems([]);
    }
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  const restore = async (m: TrashedMeeting) => {
    setBusyId(m.id);
    try {
      await invoke('api_restore_meeting', { meetingId: m.id });
      setItems((prev) => (prev ?? []).filter((x) => x.id !== m.id));
      // Let the sidebar's meeting list pick the restored meeting back up.
      window.dispatchEvent(new CustomEvent('meetings-changed'));
      toast.success('Meeting restored', { description: `"${m.title}" is back in your meetings.` });
    } catch (e) {
      toast.error('Failed to restore', { description: e instanceof Error ? e.message : String(e) });
    } finally {
      setBusyId(null);
    }
  };

  const purge = async (m: TrashedMeeting) => {
    setBusyId(m.id);
    try {
      await invoke('api_purge_meeting', { meetingId: m.id });
      setItems((prev) => (prev ?? []).filter((x) => x.id !== m.id));
      toast.success('Meeting permanently deleted');
    } catch (e) {
      toast.error('Failed to delete', { description: e instanceof Error ? e.message : String(e) });
    } finally {
      setBusyId(null);
      setConfirmId(null);
    }
  };

  return (
    <div className="h-[calc(100vh-var(--titlebar-height))] bg-background flex flex-col">
      <div className="sticky top-0 z-10 bg-background/80 backdrop-blur border-b border-border">
        <div className="max-w-4xl mx-auto px-4 md:px-8 py-6 flex flex-wrap items-center gap-x-3 gap-y-1">
          <Trash2 className="w-6 h-6 text-muted-foreground" />
          <h1 className="text-3xl font-bold">Trash</h1>
          <span className="text-sm text-muted-foreground ml-1">
            Deleted meetings are removed automatically after {RETENTION_DAYS} days.
          </span>
        </div>
      </div>

      <div className="flex-1 overflow-y-auto">
        <div className="max-w-4xl mx-auto p-4 md:p-8">
          {items === null ? (
            <div className="text-muted-foreground">Loading…</div>
          ) : items.length === 0 ? (
            <div className="flex flex-col items-center justify-center py-20 text-center text-muted-foreground">
              <Trash2 className="w-10 h-10 mb-3 opacity-40" />
              <p className="text-lg">Trash is empty</p>
              <p className="text-sm">Meetings you delete show up here for {RETENTION_DAYS} days.</p>
            </div>
          ) : (
            <ul className="space-y-2">
              {items.map((m) => {
                const left = daysLeft(m.deletedAt);
                const confirming = confirmId === m.id;
                const busy = busyId === m.id;
                return (
                  <li
                    key={m.id}
                    className="rounded-lg border border-border bg-card p-4 flex items-center gap-4"
                  >
                    <div className="flex-1 min-w-0">
                      <p className="font-medium truncate">{m.title}</p>
                      <p className="text-xs text-muted-foreground">
                        Created {formatDate(m.createdAt)} · deleted {formatDate(m.deletedAt)} ·{' '}
                        <span className={left <= 3 ? 'text-warning' : ''}>
                          {left} day{left === 1 ? '' : 's'} left
                        </span>
                      </p>
                    </div>
                    {confirming ? (
                      <div className="flex items-center gap-2 shrink-0">
                        <span className="text-xs text-destructive hidden sm:flex items-center gap-1">
                          <AlertTriangle className="w-4 h-4" /> Permanently delete?
                        </span>
                        <button
                          disabled={busy}
                          onClick={() => purge(m)}
                          className="px-3 py-1.5 rounded-md text-sm bg-destructive text-destructive-foreground hover:bg-destructive/90 disabled:opacity-50"
                        >
                          Delete
                        </button>
                        <button
                          disabled={busy}
                          onClick={() => setConfirmId(null)}
                          className="px-3 py-1.5 rounded-md text-sm bg-muted text-muted-foreground hover:bg-accent"
                        >
                          Cancel
                        </button>
                      </div>
                    ) : (
                      <div className="flex items-center gap-2 shrink-0">
                        <button
                          disabled={busy}
                          onClick={() => restore(m)}
                          className="px-3 py-1.5 rounded-md text-sm bg-primary text-primary-foreground hover:bg-brand-hover flex items-center gap-1.5 disabled:opacity-50"
                        >
                          <RotateCcw className="w-4 h-4" /> Restore
                        </button>
                        <button
                          disabled={busy}
                          onClick={() => setConfirmId(m.id)}
                          className="px-3 py-1.5 rounded-md text-sm text-muted-foreground hover:text-destructive hover:bg-destructive/10 flex items-center gap-1.5"
                        >
                          <Trash2 className="w-4 h-4" /> Delete forever
                        </button>
                      </div>
                    )}
                  </li>
                );
              })}
            </ul>
          )}
        </div>
      </div>
    </div>
  );
}
