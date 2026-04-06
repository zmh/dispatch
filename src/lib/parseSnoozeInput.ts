const DAYS = ["sunday", "monday", "tuesday", "wednesday", "thursday", "friday", "saturday"];
const MONTHS: Record<string, number> = {
  jan: 0, january: 0, feb: 1, february: 1, mar: 2, march: 2,
  apr: 3, april: 3, may: 4, jun: 5, june: 5, jul: 6, july: 6,
  aug: 7, august: 7, sep: 8, september: 8, oct: 9, october: 9,
  nov: 10, november: 10, dec: 11, december: 11,
};

export function formatPreview(date: Date): string {
  const days = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
  const months = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
  const h = date.getHours();
  const m = date.getMinutes();
  const ampm = h >= 12 ? "PM" : "AM";
  const h12 = h % 12 || 12;
  const mStr = m === 0 ? "" : `:${String(m).padStart(2, "0")}`;
  return `${days[date.getDay()]}, ${months[date.getMonth()]} ${date.getDate()} at ${h12}${mStr} ${ampm}`;
}

function resolveTime(hour: number, minute: number, ampm: string | undefined): { h: number; m: number } {
  let h = hour;
  const m = minute;
  if (ampm) {
    const suffix = ampm.toLowerCase();
    if (suffix === "pm" && h < 12) h += 12;
    if (suffix === "am" && h === 12) h = 0;
  } else {
    // No suffix: default PM for 1-6, AM for 7-11
    if (h >= 1 && h <= 6) h += 12;
  }
  return { h, m };
}

function nextOccurrenceOfDay(dayIndex: number): Date {
  const now = new Date();
  const d = new Date(now);
  const diff = ((dayIndex - now.getDay()) + 7) % 7 || 7;
  d.setDate(d.getDate() + diff);
  d.setHours(9, 0, 0, 0);
  return d;
}

export function parseSnoozeInput(input: string): { timestamp: number; label: string } | null {
  const s = input.trim().toLowerCase();
  if (!s) return null;

  const now = new Date();
  let result: Date | null = null;

  // "later today"
  if (s === "later today" || s === "later") {
    result = new Date(now.getTime() + 3 * 60 * 60 * 1000);
  }

  // "tonight"
  if (!result && s === "tonight") {
    result = new Date(now);
    result.setHours(21, 0, 0, 0);
  }

  // "this evening"
  if (!result && s === "this evening") {
    result = new Date(now);
    result.setHours(18, 0, 0, 0);
  }

  // "tomorrow morning/afternoon/evening" or bare "tomorrow"
  if (!result && s.startsWith("tomorrow")) {
    result = new Date(now);
    result.setDate(result.getDate() + 1);
    if (s.includes("afternoon")) {
      result.setHours(13, 0, 0, 0);
    } else if (s.includes("evening")) {
      result.setHours(18, 0, 0, 0);
    } else if (s.includes("morning") || s === "tomorrow") {
      result.setHours(9, 0, 0, 0);
    }

    // "tomorrow at 5pm"
    const tomorrowAt = s.match(/^tomorrow\s+(?:at\s+)?(\d{1,2})(?::(\d{2}))?\s*(am|pm)?$/);
    if (tomorrowAt) {
      const { h, m } = resolveTime(parseInt(tomorrowAt[1]), tomorrowAt[2] ? parseInt(tomorrowAt[2]) : 0, tomorrowAt[3]);
      result.setHours(h, m, 0, 0);
    }
  }

  // Relative: "in 3 hours", "3 days", "in 2 weeks"
  if (!result) {
    const rel = s.match(/^(?:in\s+)?(\d+)\s*(min(?:ute)?s?|hours?|days?|weeks?)$/);
    if (rel) {
      const n = parseInt(rel[1]);
      const unit = rel[2].replace(/s$/, "");
      const ms: Record<string, number> = {
        min: 60 * 1000, minute: 60 * 1000,
        hour: 60 * 60 * 1000,
        day: 24 * 60 * 60 * 1000,
        week: 7 * 24 * 60 * 60 * 1000,
      };
      const mult = ms[unit];
      if (mult) result = new Date(now.getTime() + n * mult);
    }
  }

  // Specific time: "at 5pm", "5pm", "at 5:30pm", "5:30 pm", "at 17:00"
  if (!result) {
    const timeMatch = s.match(/^(?:at\s+)?(\d{1,2})(?::(\d{2}))?\s*(am|pm)?$/);
    if (timeMatch) {
      const { h, m } = resolveTime(parseInt(timeMatch[1]), timeMatch[2] ? parseInt(timeMatch[2]) : 0, timeMatch[3]);
      result = new Date(now);
      result.setHours(h, m, 0, 0);
      // If time already passed today, use tomorrow
      if (result <= now) {
        result.setDate(result.getDate() + 1);
      }
    }
  }

  // Named day: "monday", "next tuesday"
  if (!result) {
    const dayMatch = s.match(/^(?:next\s+)?(\w+)$/);
    if (dayMatch) {
      const dayIdx = DAYS.indexOf(dayMatch[1]);
      if (dayIdx !== -1) {
        result = nextOccurrenceOfDay(dayIdx);
      }
    }
  }

  // Named day with time: "monday at 2pm", "next friday 3pm"
  if (!result) {
    const dayTimeMatch = s.match(/^(?:next\s+)?(\w+)\s+(?:at\s+)?(\d{1,2})(?::(\d{2}))?\s*(am|pm)?$/);
    if (dayTimeMatch) {
      const dayIdx = DAYS.indexOf(dayTimeMatch[1]);
      if (dayIdx !== -1) {
        result = nextOccurrenceOfDay(dayIdx);
        const { h, m } = resolveTime(parseInt(dayTimeMatch[2]), dayTimeMatch[3] ? parseInt(dayTimeMatch[3]) : 0, dayTimeMatch[4]);
        result.setHours(h, m, 0, 0);
      }
    }
  }

  // Date: "april 15", "apr 15"
  if (!result) {
    const dateMatch = s.match(/^(\w+)\s+(\d{1,2})$/);
    if (dateMatch) {
      const monthIdx = MONTHS[dateMatch[1]];
      if (monthIdx !== undefined) {
        const day = parseInt(dateMatch[2]);
        result = new Date(now.getFullYear(), monthIdx, day, 9, 0, 0, 0);
        // If date is in the past, use next year
        if (result <= now) {
          result.setFullYear(result.getFullYear() + 1);
        }
      }
    }
  }

  // Date with time: "april 15 2pm", "apr 15 at 3:30pm"
  if (!result) {
    const dateTimeMatch = s.match(/^(\w+)\s+(\d{1,2})\s+(?:at\s+)?(\d{1,2})(?::(\d{2}))?\s*(am|pm)?$/);
    if (dateTimeMatch) {
      const monthIdx = MONTHS[dateTimeMatch[1]];
      if (monthIdx !== undefined) {
        const day = parseInt(dateTimeMatch[2]);
        const { h, m } = resolveTime(parseInt(dateTimeMatch[3]), dateTimeMatch[4] ? parseInt(dateTimeMatch[4]) : 0, dateTimeMatch[5]);
        result = new Date(now.getFullYear(), monthIdx, day, h, m, 0, 0);
        if (result <= now) {
          result.setFullYear(result.getFullYear() + 1);
        }
      }
    }
  }

  if (!result || result <= now) return null;

  return {
    timestamp: Math.floor(result.getTime() / 1000),
    label: formatPreview(result),
  };
}
