import { useState, useRef, useEffect, useCallback } from "react";
import { parseSnoozeInput } from "../lib/parseSnoozeInput";

interface SnoozeDialogProps {
  onSnooze: (until: number) => void;
  onClose: () => void;
}

function formatTime(date: Date): string {
  const h = date.getHours();
  const m = date.getMinutes();
  const ampm = h >= 12 ? "pm" : "am";
  const h12 = h % 12 || 12;
  return m === 0 ? `${h12}${ampm}` : `${h12}:${String(m).padStart(2, "0")}${ampm}`;
}

function getSnoozeOptions(): { label: string; detail: string; until: number }[] {
  const now = new Date();
  const hour = now.getHours();
  const day = now.getDay(); // 0=Sun, 6=Sat
  const options: { label: string; detail: string; until: number }[] = [];

  // Later today (3 hours) — show before 9pm
  if (hour < 21) {
    const later = new Date(now.getTime() + 3 * 60 * 60 * 1000);
    options.push({
      label: "Later today",
      detail: formatTime(later),
      until: Math.floor(later.getTime() / 1000),
    });
  }

  // This evening (6pm) — show before 5pm
  if (hour < 17) {
    const evening = new Date(now);
    evening.setHours(18, 0, 0, 0);
    options.push({
      label: "This evening",
      detail: "6pm",
      until: Math.floor(evening.getTime() / 1000),
    });
  }

  // Tomorrow morning (9am)
  const tomorrowMorning = new Date(now);
  tomorrowMorning.setDate(tomorrowMorning.getDate() + 1);
  tomorrowMorning.setHours(9, 0, 0, 0);
  options.push({
    label: "Tomorrow morning",
    detail: "9am",
    until: Math.floor(tomorrowMorning.getTime() / 1000),
  });

  // Tomorrow afternoon (1pm)
  const tomorrowAfternoon = new Date(now);
  tomorrowAfternoon.setDate(tomorrowAfternoon.getDate() + 1);
  tomorrowAfternoon.setHours(13, 0, 0, 0);
  options.push({
    label: "Tomorrow afternoon",
    detail: "1pm",
    until: Math.floor(tomorrowAfternoon.getTime() / 1000),
  });

  // This weekend (Saturday 9am) — Mon-Fri only
  if (day >= 1 && day <= 5) {
    const saturday = new Date(now);
    saturday.setDate(saturday.getDate() + (6 - day));
    saturday.setHours(9, 0, 0, 0);
    options.push({
      label: "This weekend",
      detail: "Sat 9am",
      until: Math.floor(saturday.getTime() / 1000),
    });
  }

  // Next week (Monday 9am)
  const nextMonday = new Date(now);
  const daysUntilMonday = ((1 - day + 7) % 7) || 7;
  nextMonday.setDate(nextMonday.getDate() + daysUntilMonday);
  nextMonday.setHours(9, 0, 0, 0);
  options.push({
    label: "Next week",
    detail: "Mon 9am",
    until: Math.floor(nextMonday.getTime() / 1000),
  });

  return options;
}

export function SnoozeDialog({ onSnooze, onClose }: SnoozeDialogProps) {
  const options = getSnoozeOptions();
  const [customInput, setCustomInput] = useState("");
  const [parsedResult, setParsedResult] = useState<{ timestamp: number; label: string } | null>(null);
  const [selectedIndex, setSelectedIndex] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (customInput.trim()) {
      setParsedResult(parseSnoozeInput(customInput));
    } else {
      setParsedResult(null);
    }
  }, [customInput]);

  const handleKeyDown = useCallback(
    (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
        return;
      }

      const isInputFocused = document.activeElement === inputRef.current;

      if (e.key === "Enter") {
        e.preventDefault();
        if (isInputFocused && parsedResult) {
          onSnooze(parsedResult.timestamp);
        } else if (!isInputFocused && selectedIndex >= 0 && selectedIndex < options.length) {
          onSnooze(options[selectedIndex].until);
        }
        return;
      }

      if (e.key === "ArrowDown") {
        e.preventDefault();
        if (isInputFocused) return;
        setSelectedIndex((prev) => Math.min(prev + 1, options.length - 1));
        return;
      }

      if (e.key === "ArrowUp") {
        e.preventDefault();
        if (isInputFocused) return;
        setSelectedIndex((prev) => Math.max(prev - 1, 0));
        return;
      }

      // Any printable character focuses the input
      if (
        !isInputFocused &&
        e.key.length === 1 &&
        !e.metaKey &&
        !e.ctrlKey &&
        !e.altKey
      ) {
        inputRef.current?.focus();
      }
    },
    [onClose, onSnooze, parsedResult, customInput, options, selectedIndex]
  );

  useEffect(() => {
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [handleKeyDown]);

  return (
    <div className="dialog-overlay" onClick={onClose}>
      <div className="dialog" onClick={(e) => e.stopPropagation()}>
        <div className="dialog-title">Snooze until...</div>
        <div className="snooze-options">
          {options.map((opt, i) => (
            <button
              key={opt.label}
              className={`snooze-option${i === selectedIndex ? " snooze-option--selected" : ""}`}
              onClick={() => onSnooze(opt.until)}
              onMouseEnter={() => setSelectedIndex(i)}
            >
              <span>{opt.label}</span>
              <span className="snooze-option-detail">{opt.detail}</span>
            </button>
          ))}
        </div>
        <div className="snooze-divider" />
        <div className="snooze-custom">
          <input
            ref={inputRef}
            type="text"
            className="snooze-custom-input"
            placeholder="Type a time... (e.g. 'in 3 hours', 'tomorrow 2pm')"
            value={customInput}
            onChange={(e) => setCustomInput(e.target.value)}
            autoFocus={false}
          />
          {customInput.trim() && (
            <div
              className={`snooze-preview ${parsedResult ? "snooze-preview--valid" : "snooze-preview--invalid"}`}
            >
              {parsedResult
                ? `Snooze until ${parsedResult.label}`
                : "Couldn't understand that"}
            </div>
          )}
        </div>
        <div className="dialog-footer">
          {parsedResult && (
            <button
              className="snooze-confirm"
              onClick={() => onSnooze(parsedResult.timestamp)}
            >
              Snooze (Enter)
            </button>
          )}
          <button className="dialog-cancel" onClick={onClose}>
            Cancel (Esc)
          </button>
        </div>
      </div>
    </div>
  );
}
