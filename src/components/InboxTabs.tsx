import { useState, useRef, useEffect } from "react";
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

const STATUS_TABS = ["snoozed", "archived"] as const;

export function InboxTabs({
  tab,
  counts,
  categories,
  refreshing,
  onSwitchTab,
  onRefresh,
  onOpenSettings,
}: InboxTabsProps) {
  const [dropdownOpen, setDropdownOpen] = useState(false);
  const dropdownRef = useRef<HTMLDivElement>(null);

  const isStatusTab = STATUS_TABS.includes(tab as typeof STATUS_TABS[number]);

  // Close dropdown when clicking outside
  useEffect(() => {
    if (!dropdownOpen) return;
    const handleClick = (e: MouseEvent) => {
      if (dropdownRef.current && !dropdownRef.current.contains(e.target as Node)) {
        setDropdownOpen(false);
      }
    };
    document.addEventListener("mousedown", handleClick);
    return () => document.removeEventListener("mousedown", handleClick);
  }, [dropdownOpen]);

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
        <div className="tab-dropdown" ref={dropdownRef}>
          <button
            className={`tab-action ${isStatusTab ? "active" : ""}`}
            onClick={() => setDropdownOpen(prev => !prev)}
          >
            {isStatusTab
              ? tab.charAt(0).toUpperCase() + tab.slice(1)
              : "More"}
            {" ▾"}
          </button>
          {dropdownOpen && (
            <div className="tab-dropdown-menu">
              {STATUS_TABS.map((name) => {
                const count = counts.counts[name] || 0;
                return (
                  <button
                    key={name}
                    className={`tab-dropdown-item ${tab === name ? "active" : ""}`}
                    onClick={() => {
                      onSwitchTab(name);
                      setDropdownOpen(false);
                    }}
                  >
                    <span>{name.charAt(0).toUpperCase() + name.slice(1)}</span>
                    {count > 0 && <span className="tab-count">{count}</span>}
                  </button>
                );
              })}
            </div>
          )}
        </div>
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
