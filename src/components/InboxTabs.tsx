import { MessageCounts, Category } from "../lib/tauri";

interface InboxTabsProps {
  tab: string;
  counts: MessageCounts;
  categories: Category[];
  refreshing: boolean;
  onSwitchTab: (tab: string) => void;
  onRefresh: () => void;
  onOpenSettings: () => void;
}

export function InboxTabs({
  tab,
  counts,
  categories,
  refreshing,
  onSwitchTab,
  onRefresh,
  onOpenSettings,
}: InboxTabsProps) {
  return (
    <div className="inbox-tabs" data-tauri-drag-region>
      <div className="tabs-left">
        {categories.map((cat) => {
          const count = counts.counts[cat.name] || 0;
          return (
            <button
              key={cat.name}
              className={`tab ${tab === cat.name ? "active" : ""}`}
              onClick={() => onSwitchTab(cat.name)}
            >
              {cat.name.charAt(0).toUpperCase() + cat.name.slice(1)}
              {count > 0 && <span className="tab-count">{count}</span>}
            </button>
          );
        })}
      </div>
      <div className="tabs-right">
        <button
          className="tab-action"
          onClick={onRefresh}
          disabled={refreshing}
          title="Refresh (r)"
        >
          {refreshing ? "⟳" : "↻"} {refreshing ? "Refreshing..." : "Refresh"}
        </button>
        <button
          className="tab-action"
          onClick={onOpenSettings}
          title="Settings"
        >
          ⚙ Settings
        </button>
      </div>
    </div>
  );
}
