import React, { useState, useEffect, useRef, useMemo, useCallback } from "react";
import {
  Settings as SettingsType,
  CodexStatus,
  SlackFilter,
  Category,
  CategoryRule,
  getSettings,
  getCodexStatus,
  saveSettings,
  populateSlackCache,
} from "../lib/tauri";
import { applyTheme } from "../hooks/useMessages";
import { TypeaheadInput, TypeaheadItem } from "./TypeaheadInput";

const DEFAULT_DESCRIPTIONS: Record<string, string> = {
  important: "Messages that require direct attention or action — decisions needed, urgent requests, escalations, and messages that need a response.",
};

const DEFAULT_CATEGORIES: Category[] = [
  { name: "important", builtin: true, position: 0, description: DEFAULT_DESCRIPTIONS.important },
  { name: "other", builtin: true, position: 1 },
];

type SettingsTab = "general" | "accounts" | "watchlist" | "inboxes";

const TAB_LABELS: Record<SettingsTab, string> = {
  general: "General",
  accounts: "Accounts",
  watchlist: "Watch List",
  inboxes: "Inboxes",
};

const SETTINGS_TABS: SettingsTab[] = ["general", "accounts", "watchlist", "inboxes"];

const TAB_ICONS: Record<SettingsTab, React.ReactElement> = {
  general: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
      <circle cx="12" cy="12" r="3" />
      <path d="M19.4 15a1.65 1.65 0 00.33 1.82l.06.06a2 2 0 01-2.83 2.83l-.06-.06a1.65 1.65 0 00-1.82-.33 1.65 1.65 0 00-1 1.51V21a2 2 0 01-4 0v-.09A1.65 1.65 0 009 19.4a1.65 1.65 0 00-1.82.33l-.06.06a2 2 0 01-2.83-2.83l.06-.06A1.65 1.65 0 004.68 15a1.65 1.65 0 00-1.51-1H3a2 2 0 010-4h.09A1.65 1.65 0 004.6 9a1.65 1.65 0 00-.33-1.82l-.06-.06a2 2 0 012.83-2.83l.06.06A1.65 1.65 0 009 4.68a1.65 1.65 0 001-1.51V3a2 2 0 014 0v.09a1.65 1.65 0 001 1.51 1.65 1.65 0 001.82-.33l.06-.06a2 2 0 012.83 2.83l-.06.06A1.65 1.65 0 0019.4 9a1.65 1.65 0 001.51 1H21a2 2 0 010 4h-.09a1.65 1.65 0 00-1.51 1z" />
    </svg>
  ),
  accounts: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
      <path d="M21 2l-2 2m-7.61 7.61a5.5 5.5 0 11-7.778 7.778 5.5 5.5 0 017.777-7.777zm0 0L15.5 7.5m0 0l3 3L22 7l-3-3m-3.5 3.5L19 4" />
    </svg>
  ),
  watchlist: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
      <path d="M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z" />
      <circle cx="12" cy="12" r="3" />
    </svg>
  ),
  inboxes: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
      <polyline points="22 12 16 12 14 15 10 15 8 12 2 12" />
      <path d="M5.45 5.11L2 12v6a2 2 0 002 2h16a2 2 0 002-2v-6l-3.45-6.89A2 2 0 0016.76 4H7.24a2 2 0 00-1.79 1.11z" />
    </svg>
  ),
};

interface SettingsProps {
  onClose: () => void;
  onCategoriesChanged?: () => void;
  onMessagesChanged?: () => void;
  onRequestRefresh?: () => Promise<void> | void;
  onRunSetup?: () => void;
}

export function Settings({ onClose, onCategoriesChanged, onMessagesChanged, onRequestRefresh, onRunSetup }: SettingsProps) {
  const [settings, setSettings] = useState<SettingsType>({
    slack_token: null,
    slack_cookie: null,
    ai_provider: null,
    claude_api_key: null,
    openai_api_key: null,
    slack_filters: null,
    categories: null,
    category_rules: null,
    theme: null,
    font: null,
    font_size: null,
    open_in_slack_app: null,
    notifications_enabled: null,
    beta_release_channel: null,
    after_archive: null,
  });
  const [activeTab, setActiveTab] = useState<SettingsTab>("general");
  const [loaded, setLoaded] = useState(false);
  const [newCategoryName, setNewCategoryName] = useState("");
  const [ruleInputs, setRuleInputs] = useState<Record<string, string>>({});
  const [refreshingCache, setRefreshingCache] = useState(false);
  const [loadingCodexStatus, setLoadingCodexStatus] = useState(false);
  const [codexStatus, setCodexStatus] = useState<CodexStatus | null>(null);
  const saveTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const reclassifyTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const reclassifyQueuedRef = useRef(false);
  const pendingSaveRef = useRef(false);
  const closingRef = useRef(false);
  const saveInFlightRef = useRef<Promise<void> | null>(null);
  const settingsRef = useRef(settings);
  const loadedRef = useRef(loaded);

  const filters = settings.slack_filters ?? [];
  const categories = settings.categories ?? DEFAULT_CATEGORIES;
  const rules = settings.category_rules ?? [];
  const selectedAiProvider = (settings.ai_provider || "").trim().toLowerCase();
  const aiConfigured =
    selectedAiProvider === "claude"
      ? !!settings.claude_api_key
      : selectedAiProvider === "openai"
        ? !!settings.openai_api_key
        : selectedAiProvider === "codex"
          ? !!codexStatus?.authenticated
          : false;

  useEffect(() => {
    (async () => {
      try {
        const loadedSettings = await getSettings();
        setSettings(loadedSettings);
        setLoaded(true);
      } catch (e) {
        console.error("Failed to load settings:", e);
      }
    })();
  }, []);

  const refreshCodexStatus = useCallback(async () => {
    setLoadingCodexStatus(true);
    try {
      const status = await getCodexStatus();
      setCodexStatus(status);
    } catch (e) {
      console.error("Failed to load Codex status:", e);
      setCodexStatus({
        installed: false,
        authenticated: false,
        auth_mode: null,
        has_codex_subscription: false,
        message: "Could not read Codex status",
      });
    } finally {
      setLoadingCodexStatus(false);
    }
  }, []);

  useEffect(() => {
    if (activeTab !== "accounts") return;
    void refreshCodexStatus();
  }, [activeTab, refreshCodexStatus]);

  useEffect(() => {
    settingsRef.current = settings;
  }, [settings]);

  useEffect(() => {
    loadedRef.current = loaded;
  }, [loaded]);

  // Stable serialization for deep comparison
  const settingsKey = useMemo(() => JSON.stringify(settings), [settings]);

  const runReclassifyRefresh = useCallback(async () => {
    reclassifyQueuedRef.current = false;
    if (!onRequestRefresh) return;
    await Promise.resolve(onRequestRefresh());
  }, [onRequestRefresh]);

  const scheduleReclassifyRefresh = useCallback(() => {
    reclassifyQueuedRef.current = true;
    if (reclassifyTimeoutRef.current) clearTimeout(reclassifyTimeoutRef.current);
    reclassifyTimeoutRef.current = setTimeout(() => {
      runReclassifyRefresh().catch(console.error);
    }, 3500);
  }, [runReclassifyRefresh]);

  const persistSettings = useCallback(async (nextSettings: SettingsType, queueRefreshOnly: boolean) => {
    if (!loadedRef.current) return;
    if (saveInFlightRef.current) {
      await saveInFlightRef.current;
    }
    const run = (async () => {
      try {
        const result = await saveSettings(nextSettings);
        onCategoriesChanged?.();
        if (result.filters_cleaned) {
          onMessagesChanged?.();
        }
        if (result.classifications_reset) {
          if (queueRefreshOnly) {
            reclassifyQueuedRef.current = true;
          } else {
            scheduleReclassifyRefresh();
          }
        }
      } catch (e) {
        console.error("Failed to save settings:", e);
      }
    })();
    saveInFlightRef.current = run;
    try {
      await run;
    } finally {
      if (saveInFlightRef.current === run) {
        saveInFlightRef.current = null;
      }
    }
  }, [onCategoriesChanged, onMessagesChanged, scheduleReclassifyRefresh]);

  const flushPendingSave = useCallback(async () => {
    if (saveTimeoutRef.current) {
      clearTimeout(saveTimeoutRef.current);
      saveTimeoutRef.current = null;
    }
    if (pendingSaveRef.current) {
      pendingSaveRef.current = false;
      await persistSettings(settingsRef.current, true);
    } else if (saveInFlightRef.current) {
      await saveInFlightRef.current;
    }
  }, [persistSettings]);

  const performClose = useCallback(async (afterClose?: () => void) => {
    if (closingRef.current) return;
    closingRef.current = true;
    try {
      await flushPendingSave();
      if (reclassifyTimeoutRef.current) {
        clearTimeout(reclassifyTimeoutRef.current);
        reclassifyTimeoutRef.current = null;
      }
      if (reclassifyQueuedRef.current) {
        await runReclassifyRefresh();
      }
    } catch (e) {
      console.error("Failed to flush settings before close:", e);
    } finally {
      onClose();
      afterClose?.();
    }
  }, [flushPendingSave, onClose, runReclassifyRefresh]);

  const handleClose = useCallback(() => {
    void performClose();
  }, [performClose]);

  const handleRunSetupClick = useCallback(() => {
    if (!onRunSetup) return;
    void performClose(onRunSetup);
  }, [onRunSetup, performClose]);

  useEffect(() => {
    const closeEvent = "dispatch:close-settings";
    const onCloseRequest = () => {
      handleClose();
    };
    window.addEventListener(closeEvent, onCloseRequest);
    return () => {
      window.removeEventListener(closeEvent, onCloseRequest);
    };
  }, [handleClose]);

  useEffect(() => {
    return () => {
      if (saveTimeoutRef.current) clearTimeout(saveTimeoutRef.current);
      if (reclassifyTimeoutRef.current) clearTimeout(reclassifyTimeoutRef.current);
    };
  }, []);

  // Auto-save whenever settings change (after initial load)
  useEffect(() => {
    if (!loaded) return;
    pendingSaveRef.current = true;
    if (saveTimeoutRef.current) clearTimeout(saveTimeoutRef.current);
    const nextSettings = settings;
    saveTimeoutRef.current = setTimeout(() => {
      pendingSaveRef.current = false;
      void persistSettings(nextSettings, false);
    }, 300);
    return () => {
      if (saveTimeoutRef.current) clearTimeout(saveTimeoutRef.current);
    };
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [settingsKey, loaded, persistSettings]);

  // --- Filter management ---
  const addFilter = (item: TypeaheadItem) => {
    if (filters.some((f) => f.id === item.id)) return;
    const newFilter: SlackFilter = {
      filter_type: item.type,
      id: item.id,
      display_name: item.label,
    };
    setSettings({
      ...settings,
      slack_filters: [...filters, newFilter],
    });
  };

  const removeFilter = (id: string) => {
    setSettings({
      ...settings,
      slack_filters: filters.filter((f) => f.id !== id),
    });
  };

  // --- Category management ---
  const addCategory = () => {
    const name = newCategoryName.trim().toLowerCase();
    if (!name || categories.some((c) => c.name === name)) return;
    // Insert before "other" (which is always last)
    const otherIdx = categories.findIndex((c) => c.name === "other");
    const position = otherIdx >= 0 ? otherIdx : categories.length;
    const newCat: Category = { name, builtin: false, position };
    // Bump "other" position
    const updated = categories.map((c) =>
      c.name === "other" ? { ...c, position: position + 1 } : c
    );
    setSettings({
      ...settings,
      categories: [...updated, newCat].sort((a, b) => a.position - b.position),
    });
    setNewCategoryName("");
  };

  const removeCategory = (name: string) => {
    const updated = categories
      .filter((c) => c.name !== name)
      .map((c, i) => ({ ...c, position: i }));
    const updatedRules = rules.filter((r) => r.category !== name);
    setSettings({
      ...settings,
      categories: updated,
      category_rules: updatedRules,
    });
  };

  // --- Rule management ---
  const addRule = (category: string) => {
    const input = (ruleInputs[category] || "").trim();
    if (!input) return;

    // Will be overridden if typeahead was used — this handles plain keyword input
    const newRule: CategoryRule = {
      category,
      rule_type: "keyword",
      value: input,
      id: null,
    };

    setSettings({
      ...settings,
      category_rules: [...rules, newRule],
    });
    setRuleInputs({ ...ruleInputs, [category]: "" });
  };

  const addRuleFromTypeahead = (category: string, item: TypeaheadItem) => {
    const newRule: CategoryRule = {
      category,
      rule_type: item.type === "user" ? "sender" : "channel",
      value: item.label,
      id: item.id,
    };
    setSettings({
      ...settings,
      category_rules: [...rules, newRule],
    });
  };

  const removeRule = (idx: number) => {
    setSettings({
      ...settings,
      category_rules: rules.filter((_, i) => i !== idx),
    });
  };

  const updateCategoryDescription = (name: string, description: string) => {
    setSettings({
      ...settings,
      categories: categories.map((c) =>
        c.name === name ? { ...c, description: description || DEFAULT_DESCRIPTIONS[name] || undefined } : c
      ),
    });
  };

  const sortedCategories = [...categories].sort((a, b) => a.position - b.position);

  // Appearance helpers
  const currentTheme = settings.theme || "dark";
  const currentFont = settings.font || "system";

  const THEMES = [
    { key: "system", label: "Auto" },
    { key: "light", label: "Light", bg: "#ffffff", surface: "#f5f5f7", accent: "#007aff" },
    { key: "dark", label: "Dark", bg: "#1c1c1e", surface: "#2c2c2e", accent: "#0a84ff" },
    { key: "black", label: "Black", bg: "#000000", surface: "#1c1c1e", accent: "#0a84ff" },
    { key: "solarized-light", label: "Sol Light", bg: "#fdf6e3", surface: "#eee8d5", accent: "#268bd2" },
    { key: "solarized-dark", label: "Sol Dark", bg: "#002b36", surface: "#073642", accent: "#268bd2" },
    { key: "nord", label: "Nord", bg: "#2e3440", surface: "#3b4252", accent: "#88c0d0" },
    { key: "catppuccin", label: "Catppuccin", bg: "#1e1e2e", surface: "#2a2a3c", accent: "#cba6f7" },
    { key: "monokai", label: "Monokai", bg: "#272822", surface: "#333428", accent: "#a6e22e" },
    { key: "cyberpunk", label: "Cyberpunk", bg: "#0a0a1a", surface: "#141428", accent: "#ff2d95" },
    { key: "retro", label: "Retro", bg: "#0c0c0c", surface: "#1a1a1a", accent: "#33ff33" },
    { key: "sunset", label: "Sunset", bg: "#1a1220", surface: "#261a2e", accent: "#ff7849" },
  ] as const;

  const FONTS = [
    { key: "system", label: "System", className: "font-btn-system" },
    { key: "mono", label: "Mono", className: "font-btn-mono" },
  ] as const;

  const FONT_SIZES = [
    { key: "xs", label: "XS" },
    { key: "s", label: "S" },
    { key: "m", label: "M" },
    { key: "l", label: "L" },
    { key: "xl", label: "XL" },
  ] as const;

  const currentFontSize = settings.font_size || "s";

  const setTheme = (theme: string) => {
    setSettings({ ...settings, theme });
    applyTheme(theme, currentFont, currentFontSize);
  };

  const setFont = (font: string) => {
    setSettings({ ...settings, font });
    applyTheme(currentTheme, font, currentFontSize);
  };

  const setFontSize = (font_size: string) => {
    setSettings({ ...settings, font_size });
    applyTheme(currentTheme, currentFont, font_size);
  };

  return (
    <div className="settings-overlay" onClick={handleClose}>
      <div
        className="settings-dialog"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="settings-titlebar" data-tauri-drag-region>
          <button className="settings-close" onClick={handleClose} title="Close" />
          <span className="settings-titlebar-text">{TAB_LABELS[activeTab]}</span>
        </div>

        <div className="settings-tabbar">
          {SETTINGS_TABS.map((tab) => (
            <button
              key={tab}
              className={`settings-tab ${activeTab === tab ? "active" : ""}`}
              onClick={() => setActiveTab(tab)}
            >
              <span className="settings-tab-icon">{TAB_ICONS[tab]}</span>
              <span className="settings-tab-label">{TAB_LABELS[tab]}</span>
            </button>
          ))}
        </div>

        <div className="settings-form settings-scrollable">
          {/* General tab — Appearance */}
          {activeTab === "general" && (
          <div className="settings-group">
            <div className="settings-row-ia">
              <span className="settings-row-label">Theme:</span>
              <div className="settings-row-control">
                <div className="theme-picker">
                  {THEMES.map((t) => (
                    <button
                      key={t.key}
                      className={`theme-swatch ${currentTheme === t.key ? "active" : ""}`}
                      onClick={() => setTheme(t.key)}
                      title={t.label}
                    >
                      {t.key === "system" ? (
                        <div className="swatch-preview swatch-preview-split">
                          <div className="swatch-half-light" />
                          <div className="swatch-half-dark" />
                        </div>
                      ) : (
                        <div className="swatch-preview">
                          <div className="swatch-bg" style={{ background: ("bg" in t) ? t.bg : undefined }} />
                          <div className="swatch-surface" style={{ background: ("surface" in t) ? t.surface : undefined }} />
                          <div className="swatch-accent" style={{ background: ("accent" in t) ? t.accent : undefined }} />
                        </div>
                      )}
                      <span className="swatch-label">{t.label}</span>
                    </button>
                  ))}
                </div>
              </div>
            </div>

            <div className="settings-row-ia">
              <span className="settings-row-label">Font:</span>
              <div className="settings-row-control">
                <div className="font-picker">
                  {FONTS.map((f) => (
                    <button
                      key={f.key}
                      className={`font-btn ${f.className} ${currentFont === f.key ? "active" : ""}`}
                      onClick={() => setFont(f.key)}
                    >
                      {f.label}
                    </button>
                  ))}
                </div>
              </div>
            </div>

            <div className="settings-row-ia">
              <span className="settings-row-label">Size:</span>
              <div className="settings-row-control">
                <div className="font-picker">
                  {FONT_SIZES.map((s) => (
                    <button
                      key={s.key}
                      className={`font-btn ${currentFontSize === s.key ? "active" : ""}`}
                      onClick={() => setFontSize(s.key)}
                    >
                      {s.label}
                    </button>
                  ))}
                </div>
              </div>
            </div>

            <div className="settings-row-ia">
              <span className="settings-row-label">Behavior:</span>
              <div className="settings-row-control">
                <label className="settings-checkbox-label">
                  <input
                    type="checkbox"
                    checked={settings.open_in_slack_app ?? false}
                    onChange={(e) =>
                      setSettings({ ...settings, open_in_slack_app: e.target.checked })
                    }
                  />
                  Open links directly in Slack app
                </label>
                <label className="settings-checkbox-label">
                  <input
                    type="checkbox"
                    checked={settings.notifications_enabled ?? true}
                    onChange={(e) =>
                      setSettings({ ...settings, notifications_enabled: e.target.checked })
                    }
                  />
                  Desktop notifications for new &amp; snoozed messages
                </label>
                <label className="settings-checkbox-label">
                  <input
                    type="checkbox"
                    checked={settings.beta_release_channel ?? false}
                    onChange={(e) =>
                      setSettings({ ...settings, beta_release_channel: e.target.checked })
                    }
                  />
                  Get beta releases before production
                </label>
              </div>
            </div>

            <div className="settings-row-ia">
              <span className="settings-row-label">After archive:</span>
              <div className="settings-row-control">
                {([
                  { key: "newer", label: "Go to newer conversation" },
                  { key: "older", label: "Go to older conversation" },
                  { key: "stay", label: "Return to conversation list" },
                ] as const).map((opt) => (
                  <label key={opt.key} className="settings-checkbox-label">
                    <input
                      type="radio"
                      name="after_archive"
                      checked={(settings.after_archive ?? "newer") === opt.key}
                      onChange={() => setSettings({ ...settings, after_archive: opt.key })}
                    />
                    {opt.label}
                  </label>
                ))}
              </div>
            </div>
          </div>
          )}

          {/* Accounts tab — Credentials */}
          {activeTab === "accounts" && (
          <div className="settings-group">
            <div className="settings-row-ia">
              <span className="settings-row-label">Slack token:</span>
              <div className="settings-row-control">
                <input
                  type="password"
                  className="settings-input"
                  value={settings.slack_token || ""}
                  onChange={(e) =>
                    setSettings({ ...settings, slack_token: e.target.value || null })
                  }
                  placeholder="xoxc-..."
                />
                <div className="settings-hint-text">
                  Open <a href="https://app.slack.com" target="_blank" rel="noopener noreferrer">app.slack.com</a> &rarr; DevTools (<kbd>F12</kbd>) &rarr; Console &rarr; <span className="settings-copy-link" onClick={(e) => {
                    navigator.clipboard.writeText(`Object.entries(JSON.parse(localStorage.localConfig_v2).teams).forEach(([,t])=>console.log(t.name,t.token))`);
                    const el = e.currentTarget;
                    const orig = el.textContent;
                    el.textContent = "Copied!";
                    el.classList.add("copied");
                    setTimeout(() => { el.textContent = orig; el.classList.remove("copied"); }, 1500);
                  }}>copy command</span> and paste.
                </div>
              </div>
            </div>

            <div className="settings-row-ia">
              <span className="settings-row-label">Slack cookie:</span>
              <div className="settings-row-control">
                <input
                  type="password"
                  className="settings-input"
                  value={settings.slack_cookie || ""}
                  onChange={(e) =>
                    setSettings({ ...settings, slack_cookie: e.target.value || null })
                  }
                  placeholder="xoxd-..."
                />
                <div className="settings-hint-text">
                  Same console &rarr; <span className="settings-copy-link" onClick={(e) => {
                    navigator.clipboard.writeText(`document.cookie.split("; ").find(c=>c.startsWith("d=")).slice(2)`);
                    const el = e.currentTarget;
                    const orig = el.textContent;
                    el.textContent = "Copied!";
                    el.classList.add("copied");
                    setTimeout(() => { el.textContent = orig; el.classList.remove("copied"); }, 1500);
                  }}>copy command</span> and paste.
                </div>
              </div>
            </div>

            <div className="settings-row-ia">
              <span className="settings-row-label">AI provider:</span>
              <div className="settings-row-control">
                {([
                  { key: "codex", label: "Codex (ChatGPT/Codex subscription)" },
                  { key: "openai", label: "OpenAI API key" },
                  { key: "claude", label: "Claude API key" },
                  { key: "", label: "Rules only (no AI provider)" },
                ] as const).map((opt) => (
                  <label key={opt.key || "none"} className="settings-checkbox-label">
                    <input
                      type="radio"
                      name="ai_provider"
                      checked={selectedAiProvider === opt.key}
                      onChange={() =>
                        setSettings({ ...settings, ai_provider: opt.key || null })
                      }
                    />
                    {opt.label}
                  </label>
                ))}
              </div>
            </div>

            {selectedAiProvider === "claude" && (
              <div className="settings-row-ia">
                <span className="settings-row-label">Claude API key:</span>
                <div className="settings-row-control">
                  <input
                    type="password"
                    className="settings-input"
                    value={settings.claude_api_key || ""}
                    onChange={(e) =>
                      setSettings({ ...settings, claude_api_key: e.target.value || null })
                    }
                    placeholder="sk-ant-..."
                  />
                </div>
              </div>
            )}

            {selectedAiProvider === "openai" && (
              <div className="settings-row-ia">
                <span className="settings-row-label">OpenAI API key:</span>
                <div className="settings-row-control">
                  <input
                    type="password"
                    className="settings-input"
                    value={settings.openai_api_key || ""}
                    onChange={(e) =>
                      setSettings({ ...settings, openai_api_key: e.target.value || null })
                    }
                    placeholder="sk-..."
                  />
                </div>
              </div>
            )}

            {selectedAiProvider === "codex" && (
              <div className="settings-row-ia">
                <span className="settings-row-label">Codex status:</span>
                <div className="settings-row-control">
                  <div className="settings-hint-text" style={{ marginBottom: 8 }}>
                    {loadingCodexStatus
                      ? "Checking Codex login..."
                      : codexStatus
                        ? codexStatus.message
                        : "Status unavailable"}
                  </div>
                  {codexStatus && (
                    <div className="settings-hint-text" style={{ marginBottom: 8 }}>
                      Installed: {codexStatus.installed ? "yes" : "no"} · Authenticated:{" "}
                      {codexStatus.authenticated ? "yes" : "no"} · Mode:{" "}
                      {codexStatus.auth_mode || "unknown"} · Subscription:{" "}
                      {codexStatus.has_codex_subscription ? "yes" : "no"}
                    </div>
                  )}
                  <button
                    className="setup-wizard-btn"
                    onClick={() => {
                      void refreshCodexStatus();
                    }}
                    disabled={loadingCodexStatus}
                  >
                    {loadingCodexStatus ? "Checking..." : "Refresh Codex Status"}
                  </button>
                </div>
              </div>
            )}

            {onRunSetup && (
              <div className="settings-row-ia">
                <span className="settings-row-label"></span>
                <div className="settings-row-control" style={{ display: "flex", gap: 8 }}>
                  <button className="setup-wizard-btn" onClick={handleRunSetupClick}>
                    Run Setup Wizard...
                  </button>
                  <button
                    className="setup-wizard-btn"
                    disabled={refreshingCache}
                    onClick={async () => {
                      setRefreshingCache(true);
                      try {
                        await populateSlackCache();
                      } catch (e) {
                        console.error("Cache refresh failed:", e);
                      } finally {
                        setRefreshingCache(false);
                      }
                    }}
                  >
                    {refreshingCache ? "Refreshing..." : "Refresh Workspace Cache"}
                  </button>
                </div>
              </div>
            )}

          </div>
          )}

          {/* Watch List tab */}
          {activeTab === "watchlist" && (
          <div className="settings-section">
            <div className="settings-section-desc">
              People, channels, and DMs to monitor for new messages.
            </div>
            <div className="settings-section-body">
              <div className="filter-chips">
                <span className="filter-chip filter-chip-auto">
                  to:me
                </span>
                {filters.map((f) => (
                  <span key={f.id} className="filter-chip">
                    {f.filter_type === "user" ? "@" : f.filter_type === "to" ? "to:" : "#"}
                    {f.display_name.replace(/^#/, "")}
                    <button
                      className="chip-remove"
                      onClick={() => removeFilter(f.id)}
                    >
                      ×
                    </button>
                  </span>
                ))}
              </div>

              <TypeaheadInput
                placeholder="@person, #channel, or to:someone..."
                onSelect={addFilter}
              />

              <div className="settings-hint-text">
                Type <kbd>@</kbd> for people, <kbd>#</kbd> for channels, or <kbd>to:</kbd> for directed messages. <code>to:me</code> is included automatically.
              </div>
            </div>
          </div>
          )}

          {/* Inboxes tab */}
          {activeTab === "inboxes" && (
          <div className="settings-section">
            <div className="settings-section-desc">
              Split your messages into separate inboxes with rules and AI classification.
            </div>
            <div className="settings-section-body">
              <div className="category-list">
                {sortedCategories.map((cat) => {
                  const catRules = rules
                    .map((r, i) => ({ ...r, _idx: i }))
                    .filter((r) => r.category === cat.name);
                  const isOther = cat.name === "other";

                  return (
                    <div key={cat.name} className="category-item">
                      <div className="category-header">
                        <span className="category-name">
                          {cat.name.charAt(0).toUpperCase() + cat.name.slice(1)}
                          {cat.builtin && (
                            <span className="category-builtin"> (built-in)</span>
                          )}
                        </span>
                        {!cat.builtin && (
                          <button
                            className="category-delete"
                            onClick={() => removeCategory(cat.name)}
                          >
                            ×
                          </button>
                        )}
                      </div>

                      {isOther ? (
                        <div className="category-catchall">Catch-all for unmatched messages</div>
                      ) : (
                        <>
                          <textarea
                            className="category-description"
                            value={cat.description || ""}
                            onChange={(e) => updateCategoryDescription(cat.name, e.target.value)}
                            rows={2}
                            placeholder="Describe what messages belong here (used by AI classifier)..."
                          />
                          {!aiConfigured && (
                            <div className="category-ai-hint">
                              {selectedAiProvider === "claude" &&
                                "Add a Claude API key to enable AI classification"}
                              {selectedAiProvider === "openai" &&
                                "Add an OpenAI API key to enable AI classification"}
                              {selectedAiProvider === "codex" &&
                                "Sign in to Codex to enable AI classification"}
                              {!selectedAiProvider &&
                                "Choose an AI provider in Accounts to enable AI classification"}
                            </div>
                          )}

                          {catRules.length > 0 && (
                            <div className="rule-list">
                              {catRules.map((rule) => (
                                <div key={rule._idx} className="rule-item">
                                  <span className="rule-type">{rule.rule_type}</span>
                                  <span className="rule-value">{rule.value}</span>
                                  <button
                                    className="rule-remove"
                                    onClick={() => removeRule(rule._idx)}
                                  >
                                    ×
                                  </button>
                                </div>
                              ))}
                            </div>
                          )}

                          <div className="rule-add">
                            <TypeaheadInput
                              placeholder="@sender, #channel, or keyword..."
                              onSelect={(item) => addRuleFromTypeahead(cat.name, item)}
                            />
                            <div className="rule-keyword-add">
                              <input
                                type="text"
                                className="settings-input rule-keyword-input"
                                value={ruleInputs[cat.name] || ""}
                                onChange={(e) =>
                                  setRuleInputs({
                                    ...ruleInputs,
                                    [cat.name]: e.target.value,
                                  })
                                }
                                onKeyDown={(e) => {
                                  if (e.key === "Enter") {
                                    e.preventDefault();
                                    addRule(cat.name);
                                  }
                                }}
                                placeholder="keyword (press Enter)"
                              />
                            </div>
                          </div>
                        </>
                      )}
                    </div>
                  );
                })}
              </div>

              <div className="category-add">
                <input
                  type="text"
                  className="settings-input"
                  value={newCategoryName}
                  onChange={(e) => setNewCategoryName(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") {
                      e.preventDefault();
                      addCategory();
                    }
                  }}
                  placeholder="New inbox name (press Enter)"
                />
              </div>

              <div className="settings-hint-text">
                Rules auto-sort messages by keyword, sender, or channel. AI uses descriptions for everything else.
              </div>
            </div>
          </div>
          )}
        </div>

      </div>
    </div>
  );
}
