import { describe, expect, test } from "bun:test";
import { groupMeetingsByDate } from "../../src/lib/meetingGrouping";

describe("groupMeetingsByDate", () => {
  // Local noon so day-arithmetic never crosses a date boundary under any timezone.
  const now = new Date(2026, 6, 13, 12, 0, 0); // 13 Jul 2026
  const daysAgo = (n: number): string => {
    const d = new Date(now);
    d.setDate(d.getDate() - n);
    return d.toISOString();
  };

  test("buckets by day/week/month in newest-first order", () => {
    const items = [
      { id: "a", createdAt: daysAgo(0) }, // today
      { id: "b", createdAt: daysAgo(1) }, // yesterday
      { id: "c", createdAt: daysAgo(4) }, // this week
      { id: "d", createdAt: daysAgo(30) }, // June 2026
      { id: "e", createdAt: daysAgo(40) }, // June 2026
      { id: "f" }, // undated
    ];
    const groups = groupMeetingsByDate(items, now);
    expect(groups.map((g) => g.key)).toEqual([
      "today",
      "yesterday",
      "this-week",
      "2026-5",
      "undated",
    ]);
    expect(groups[0].label).toBe("Today");
    // Same-month items share a group, preserving input order.
    const june = groups.find((g) => g.key === "2026-5")!;
    expect(june.items.map((i) => i.id)).toEqual(["d", "e"]);
  });

  test("missing or invalid createdAt goes to Undated", () => {
    const groups = groupMeetingsByDate(
      [{ id: "x", createdAt: "not-a-date" }, { id: "y" }],
      now
    );
    expect(groups).toHaveLength(1);
    expect(groups[0].key).toBe("undated");
    expect(groups[0].items.map((i) => i.id)).toEqual(["x", "y"]);
  });
});
