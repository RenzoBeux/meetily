'use client';

import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { Database, Download, FolderOpen, RefreshCw } from 'lucide-react';

interface BackupEntry {
  name: string;
  path: string;
  size_bytes: number;
  modified_rfc3339: string | null;
}

function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(0)} KB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}

/**
 * Settings → Data. Create/list rotating DB backups and bulk-export all meetings
 * to Markdown. Restore is intentionally not in-app yet (see the note below) — it
 * needs a destructive DB-replace + relaunch that must be verified on a real app.
 */
export function DataSettings() {
  const [backups, setBackups] = useState<BackupEntry[]>([]);
  const [busy, setBusy] = useState(false);

  const refresh = async () => {
    try {
      setBackups(await invoke<BackupEntry[]>('db_list_backups'));
    } catch (e) {
      console.error('Failed to list backups:', e);
    }
  };

  useEffect(() => {
    refresh();
  }, []);

  const handleBackup = async () => {
    setBusy(true);
    try {
      const entry = await invoke<BackupEntry>('db_backup_now');
      toast.success('Database backed up', { description: entry.name });
      await refresh();
    } catch (e) {
      toast.error('Backup failed', { description: e instanceof Error ? e.message : String(e) });
    } finally {
      setBusy(false);
    }
  };

  const handleExportAll = async () => {
    setBusy(true);
    try {
      const res = await invoke<{ folder: string | null; exported: number }>('export_all_markdown');
      if (res.folder) {
        toast.success(`Exported ${res.exported} meeting${res.exported === 1 ? '' : 's'}`, {
          description: res.folder,
        });
      }
    } catch (e) {
      toast.error('Export failed', { description: e instanceof Error ? e.message : String(e) });
    } finally {
      setBusy(false);
    }
  };

  const handleOpenFolder = async () => {
    try {
      await invoke('open_database_folder');
    } catch (e) {
      console.error('Failed to open data folder:', e);
      toast.error('Could not open the data folder');
    }
  };

  return (
    <div className="space-y-6 py-4">
      <section className="space-y-2">
        <h3 className="flex items-center gap-2 text-sm font-semibold">
          <Database className="h-4 w-4" /> Backups
        </h3>
        <p className="text-xs text-muted-foreground">
          Create a point-in-time snapshot of your local database. Backups live in the app data folder
          (the newest 20 are kept).
        </p>
        <div className="flex flex-wrap gap-2">
          <Button size="sm" onClick={handleBackup} disabled={busy}>
            <Database className="mr-1.5 h-4 w-4" /> Back up now
          </Button>
          <Button size="sm" variant="outline" onClick={refresh} disabled={busy}>
            <RefreshCw className="mr-1.5 h-4 w-4" /> Refresh
          </Button>
          <Button size="sm" variant="outline" onClick={handleOpenFolder}>
            <FolderOpen className="mr-1.5 h-4 w-4" /> Open folder
          </Button>
        </div>
        {backups.length === 0 ? (
          <p className="text-xs text-muted-foreground/70">No backups yet.</p>
        ) : (
          <ul className="mt-1 divide-y divide-border rounded-md border border-border">
            {backups.map((b) => (
              <li key={b.path} className="flex items-center justify-between px-3 py-1.5 text-xs">
                <span className="truncate font-mono">{b.name}</span>
                <span className="ml-2 whitespace-nowrap text-muted-foreground/70">
                  {formatBytes(b.size_bytes)}
                </span>
              </li>
            ))}
          </ul>
        )}
        <p className="text-[11px] text-muted-foreground/60">
          Restoring from a backup isn&apos;t available in-app yet — with the app closed, copy a snapshot
          over <code>meeting_minutes.sqlite</code> in the data folder.
        </p>
      </section>

      <section className="space-y-2">
        <h3 className="flex items-center gap-2 text-sm font-semibold">
          <Download className="h-4 w-4" /> Export
        </h3>
        <p className="text-xs text-muted-foreground">
          Export every meeting to a folder as individual Markdown files (with YAML frontmatter) — ready
          for Obsidian or any notes app.
        </p>
        <Button size="sm" variant="outline" onClick={handleExportAll} disabled={busy}>
          <Download className="mr-1.5 h-4 w-4" /> Export all meetings…
        </Button>
      </section>
    </div>
  );
}
