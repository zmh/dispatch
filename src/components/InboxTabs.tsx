import { useState, useRef, useEffect } from "react";
import { MessageCounts, Category } from "../lib/tauri";

interface InboxTabsProps {
  tab: string;
  counts: MessageCounts;
  categories: Category[];
  refreshing: boolean;
  refreshProgressPercent: number;
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
  refreshProgressPercent,
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
            {" "}
            <svg width="8" height="8" viewBox="0 0 12 12" fill="currentColor" style={{ marginLeft: 2 }}>
              <path d="M2.5 4.5L6 8l3.5-3.5" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
            </svg>
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
          <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round" className={refreshing ? "spin" : ""}>
            <path d="M21 2v6h-6" />
            <path d="M3 12a9 9 0 0 1 15-6.7L21 8" />
            <path d="M3 22v-6h6" />
            <path d="M21 12a9 9 0 0 1-15 6.7L3 16" />
          </svg>
          <span className="loading-fixed-label loading-fixed-label-refresh">
            {refreshing
              ? `Refreshing ${Math.max(1, Math.min(100, refreshProgressPercent))}%`
              : "Refresh"}
          </span>
        </button>
        <button
          className="tab-action"
          onClick={onOpenSettings}
          title="Settings"
        >
          <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round">
            <circle cx="12" cy="12" r="3" />
            <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
          </svg>
          Settings
        </button>
      </div>
    </div>
  );
}
