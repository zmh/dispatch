interface SnoozeDialogProps {
  onSnooze: (until: number) => void;
  onClose: () => void;
}

function getSnoozeOptions(): { label: string; until: number }[] {
  const now = new Date();
  const threeHours = new Date(now.getTime() + 3 * 60 * 60 * 1000);

  const tomorrow9am = new Date(now);
  tomorrow9am.setDate(tomorrow9am.getDate() + 1);
  tomorrow9am.setHours(9, 0, 0, 0);

  const nextMonday = new Date(now);
  const daysUntilMonday = ((1 - now.getDay() + 7) % 7) || 7;
  nextMonday.setDate(nextMonday.getDate() + daysUntilMonday);
  nextMonday.setHours(9, 0, 0, 0);

  return [
    { label: "Later today (3 hours)", until: Math.floor(threeHours.getTime() / 1000) },
    { label: "Tomorrow morning (9am)", until: Math.floor(tomorrow9am.getTime() / 1000) },
    { label: "Next week (Monday 9am)", until: Math.floor(nextMonday.getTime() / 1000) },
  ];
}

export function SnoozeDialog({ onSnooze, onClose }: SnoozeDialogProps) {
  const options = getSnoozeOptions();

  return (
    <div className="dialog-overlay" onClick={onClose}>
      <div className="dialog" onClick={(e) => e.stopPropagation()}>
        <div className="dialog-title">Snooze until...</div>
        <div className="snooze-options">
          {options.map((opt) => (
            <button
              key={opt.label}
              className="snooze-option"
              onClick={() => onSnooze(opt.until)}
            >
              {opt.label}
            </button>
          ))}
        </div>
        <div className="dialog-footer">
          <button className="dialog-cancel" onClick={onClose}>
            Cancel (Esc)
          </button>
        </div>
      </div>
    </div>
  );
}
