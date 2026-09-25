const HOUR = 3_600_000;

function sameDay(a: Date, b: Date) {
  return (
    a.getFullYear() === b.getFullYear() &&
    a.getMonth() === b.getMonth() &&
    a.getDate() === b.getDate()
  );
}

function day(time: number, now: number) {
  const date = new Date(time);
  return date.toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
    ...(date.getFullYear() === new Date(now).getFullYear()
      ? {}
      : { year: "numeric" }),
  });
}

/** "just now", "5 minutes ago", "2 hours ago", then "Sep 22". */
export function formatAgo(time: number, now: number) {
  const elapsed = now - time;
  if (elapsed < 60_000) return "just now";
  if (elapsed < HOUR) {
    const minutes = Math.floor(elapsed / 60_000);
    return `${minutes} minute${minutes === 1 ? "" : "s"} ago`;
  }
  if (elapsed < 24 * HOUR && sameDay(new Date(time), new Date(now))) {
    const hours = Math.floor(elapsed / HOUR);
    return `${hours} hour${hours === 1 ? "" : "s"} ago`;
  }
  return day(time, now);
}

/** "today, 14:20", "yesterday, 21:10" or "Sep 20, 09:05". */
export function formatMoment(time: number, now: number) {
  const date = new Date(time);
  const yesterday = new Date(now);
  yesterday.setDate(yesterday.getDate() - 1);
  const clock = date.toLocaleTimeString(undefined, {
    hour: "2-digit",
    minute: "2-digit",
  });
  const prefix = sameDay(date, new Date(now))
    ? "today"
    : sameDay(date, yesterday)
      ? "yesterday"
      : day(time, now);
  return `${prefix}, ${clock}`;
}
