// Groups meetings into dated buckets (Today / Yesterday / This Week / by month)
// for the sidebar. Generic over any item carrying an ISO `createdAt`, and takes
// an injectable `now` so the bucketing is deterministic and unit-testable.
//
// Input is expected newest-first (the meetings API returns ORDER BY created_at
// DESC); groups are emitted in first-appearance order, so that yields
// Today → Yesterday → This Week → months (newest first) → Undated.

export interface DateGroup<T> {
  key: string;
  label: string;
  items: T[];
}

function startOfDay(d: Date): Date {
  return new Date(d.getFullYear(), d.getMonth(), d.getDate());
}

export function groupMeetingsByDate<T extends { createdAt?: string | null }>(
  items: T[],
  now: Date = new Date()
): DateGroup<T>[] {
  const today = startOfDay(now);
  const msPerDay = 86_400_000;
  const groups: DateGroup<T>[] = [];
  const byKey = new Map<string, DateGroup<T>>();

  const push = (key: string, label: string, item: T) => {
    let group = byKey.get(key);
    if (!group) {
      group = { key, label, items: [] };
      byKey.set(key, group);
      groups.push(group);
    }
    group.items.push(item);
  };

  for (const item of items) {
    const raw = item.createdAt;
    const date = raw ? new Date(raw) : null;
    if (!date || isNaN(date.getTime())) {
      push('undated', 'Undated', item);
      continue;
    }

    const day = startOfDay(date);
    const diffDays = Math.round((today.getTime() - day.getTime()) / msPerDay);

    if (diffDays <= 0) {
      push('today', 'Today', item);
    } else if (diffDays === 1) {
      push('yesterday', 'Yesterday', item);
    } else if (diffDays < 7) {
      push('this-week', 'This Week', item);
    } else {
      const key = `${date.getFullYear()}-${date.getMonth()}`;
      const label = date.toLocaleDateString(undefined, { month: 'long', year: 'numeric' });
      push(key, label, item);
    }
  }

  return groups;
}
