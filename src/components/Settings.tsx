import { useState, useEffect, useRef, useCallback, useMemo } from "react";
import {
  Settings as SettingsType,
  SlackFilter,
  Category,
  CategoryRule,
  getSettings,
  saveSettings,
  searchSlackUsers,
  searchSlackChannels,
} from "../lib/tauri";
import { applyTheme } from "../hooks/useMessages";

const DEFAULT_CATEGORIES: Category[] = [
  { name: "important", builtin: true, position: 0 },
  { name: "other", builtin: true, position: 1 },
];

interface SettingsProps {
  onClose: () => void;
  onCategoriesChanged?: () => void;
}

// Typeahead dropdown item
interface TypeaheadItem {
  id: string;
  label: string;
  sublabel: string;
  type: "user" | "channel" | "to";
}

function TypeaheadInput({
  placeholder,
  onSelect,
}: {
  placeholder: string;
  onSelect: (item: TypeaheadItem) => void;
}) {
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<TypeaheadItem[]>([]);
  const [showDropdown, setShowDropdown] = useState(false);
  const [highlightIndex, setHighlightIndex] = useState(0);
  const [dropdownPos, setDropdownPos] = useState<{ top: number; left: number; width: number } | null>(null);
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  const updateDropdownPos = useCallback(() => {
    if (inputRef.current) {
      const rect = inputRef.current.getBoundingClientRect();
      setDropdownPos({ top: rect.bottom, left: rect.left, width: rect.width });
    }
  }, []);

  const doSearch = useCallback(async (q: string) => {
    if (q.length < 2) {
      setResults([]);
      setShowDropdown(false);
      return;
    }

    // Handle to: prefix — freeform, no API search needed
    if (q.startsWith("to:")) {
      // Don't show dropdown for to: — user submits with Enter
      setResults([]);
      setShowDropdown(false);
      return;
    }

    const prefix = q[0];
    const searchTerm = q.slice(1);
    if (searchTerm.length === 0) return;

    try {
      if (prefix === "@") {
        const users = await searchSlackUsers(searchTerm);
        setResults(
          users.map((u) => ({
            id: u.id,
            label: u.real_name || u.name,
            sublabel: `@${u.name || u.real_name}`,
            type: "user" as const,
          }))
        );
      } else if (prefix === "#") {
        const channels = await searchSlackChannels(searchTerm);
        setResults(
          channels.map((c) => ({
            id: c.id,
            label: c.name,
            sublabel: c.is_private ? "private" : "public",
            type: "channel" as const,
          }))
        );
      } else {
        setResults([]);
        setShowDropdown(false);
        return;
      }
      updateDropdownPos();
      setShowDropdown(true);
      setHighlightIndex(0);
    } catch (e) {
      console.error("Search failed:", e);
    }
  }, [updateDropdownPos]);

  const handleChange = (value: string) => {
    setQuery(value);
    if (debounceRef.current) clearTimeout(debounceRef.current);
    debounceRef.current = setTimeout(() => doSearch(value), 150);
  };

  const selectItem = (item: TypeaheadItem) => {
    onSelect(item);
    setQuery("");
    setResults([]);
    setShowDropdown(false);
    inputRef.current?.focus();
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    // Handle to: prefix — submit on Enter without dropdown
    if (e.key === "Enter" && query.startsWith("to:")) {
      e.preventDefault();
      const value = query.slice(3).trim();
      if (value) {
        selectItem({
          id: `to:${value}`,
          label: value,
          sublabel: "to filter",
          type: "to",
        });
      }
      return;
    }

    if (!showDropdown || results.length === 0) return;

    if (e.key === "ArrowDown") {
      e.preventDefault();
      setHighlightIndex((prev) => Math.min(prev + 1, results.length - 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setHighlightIndex((prev) => Math.max(prev - 1, 0));
    } else if (e.key === "Enter") {
      e.preventDefault();
      selectItem(results[highlightIndex]);
    } else if (e.key === "Escape") {
      setShowDropdown(false);
    }
  };

  return (
    <div className="typeahead-container">
      <input
        ref={inputRef}
        type="text"
        className="settings-input"
        value={query}
        onChange={(e) => handleChange(e.target.value)}
        onKeyDown={handleKeyDown}
        onBlur={() => setTimeout(() => setShowDropdown(false), 200)}
        placeholder={placeholder}
      />
      {showDropdown && results.length > 0 && dropdownPos && (
        <div
          className="typeahead-dropdown"
          style={{
            position: "fixed",
            top: dropdownPos.top,
            left: dropdownPos.left,
            width: dropdownPos.width,
          }}
        >
          {results.map((item, i) => (
            <div
              key={item.id}
              className={`typeahead-item ${i === highlightIndex ? "highlighted" : ""}`}
              onMouseDown={() => selectItem(item)}
              onMouseEnter={() => setHighlightIndex(i)}
            >
              <span className="typeahead-label">{item.label}</span>
              <span className="typeahead-sublabel">{item.sublabel}</span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

export function Settings({ onClose, onCategoriesChanged }: SettingsProps) {
  const [settings, setSettings] = useState<SettingsType>({
    slack_token: null,
    slack_cookie: null,
    claude_api_key: null,
    classification_prompt: null,
    slack_filters: null,
    categories: null,
    category_rules: null,
    theme: null,
    font: null,
    font_size: null,
    open_in_slack_app: null,
  });
  const [loaded, setLoaded] = useState(false);
  const [newCategoryName, setNewCategoryName] = useState("");
  const [ruleInputs, setRuleInputs] = useState<Record<string, string>>({});
  const saveTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const filters = settings.slack_filters ?? [];
  const categories = settings.categories ?? DEFAULT_CATEGORIES;
  const rules = settings.category_rules ?? [];

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

  // Stable serialization for deep comparison
  const settingsKey = useMemo(() => JSON.stringify(settings), [settings]);

  // Auto-save whenever settings change (after initial load)
  useEffect(() => {
    if (!loaded) return;
    if (saveTimeoutRef.current) clearTimeout(saveTimeoutRef.current);
    saveTimeoutRef.current = setTimeout(async () => {
      try {
        await saveSettings(settings);
        onCategoriesChanged?.();
      } catch (e) {
        console.error("Failed to save settings:", e);
      }
    }, 300);
    return () => {
      if (saveTimeoutRef.current) clearTimeout(saveTimeoutRef.current);
    };
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [settingsKey, loaded]);

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

  const sortedCategories = [...categories].sort((a, b) => a.position - b.position);

  // Appearance helpers
  const currentTheme = settings.theme || "dark";
  const currentFont = settings.font || "system";

  const THEMES = [
    { key: "light", label: "Light", bg: "#ffffff", surface: "#f5f5f7", accent: "#007aff" },
    { key: "dark", label: "Dark", bg: "#1c1c1e", surface: "#2c2c2e", accent: "#0a84ff" },
    { key: "black", label: "Black", bg: "#000000", surface: "#1c1c1e", accent: "#0a84ff" },
    { key: "solarized-light", label: "Sol Light", bg: "#fdf6e3", surface: "#eee8d5", accent: "#268bd2" },
    { key: "solarized-dark", label: "Sol Dark", bg: "#002b36", surface: "#073642", accent: "#268bd2" },
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
    <div className="settings-overlay" onClick={onClose}>
      <div
        className="settings-dialog"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="settings-titlebar">
          <button className="settings-close" onClick={onClose} title="Close" />
          <span className="settings-titlebar-text">General</span>
        </div>

        <div className="settings-form settings-scrollable">
          {/* Appearance group */}
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
                      <div className="swatch-preview">
                        <div className="swatch-bg" style={{ background: t.bg }} />
                        <div className="swatch-surface" style={{ background: t.surface }} />
                        <div className="swatch-accent" style={{ background: t.accent }} />
                      </div>
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
              </div>
            </div>
          </div>

          {/* Credentials group */}
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
                  Open <a href="https://app.slack.com" target="_blank" rel="noopener noreferrer">app.slack.com</a> in your browser, press <kbd>F12</kbd> to open DevTools, go to <strong>Console</strong>, and paste:
                  <pre className="settings-code-block" onClick={(e) => {
                    const text = (e.currentTarget.textContent || "").trim();
                    navigator.clipboard.writeText(text);
                    const el = e.currentTarget;
                    el.classList.add("copied");
                    setTimeout(() => el.classList.remove("copied"), 1500);
                  }}>
{`Object.entries(JSON.parse(localStorage.localConfig_v2).teams).forEach(([,t])=>console.log(t.name,t.token))`}
                  </pre>
                  <span className="settings-hint-muted">Click to copy. If you have multiple workspaces, this prints a token for each one — use the one for your workspace.</span>
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
                  Same DevTools console on <a href="https://app.slack.com" target="_blank" rel="noopener noreferrer">app.slack.com</a>, paste:
                  <pre className="settings-code-block" onClick={(e) => {
                    const text = (e.currentTarget.textContent || "").trim();
                    navigator.clipboard.writeText(text);
                    const el = e.currentTarget;
                    el.classList.add("copied");
                    setTimeout(() => el.classList.remove("copied"), 1500);
                  }}>
{`document.cookie.split("; ").find(c=>c.startsWith("d=")).slice(2)`}
                  </pre>
                  <span className="settings-hint-muted">Click to copy. The cookie is shared across all workspaces — you only need one.</span>
                </div>
              </div>
            </div>

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

            <div className="settings-row-ia">
              <span className="settings-row-label">Classifier:</span>
              <div className="settings-row-control">
                <textarea
                  className="settings-textarea"
                  value={settings.classification_prompt || ""}
                  onChange={(e) =>
                    setSettings({
                      ...settings,
                      classification_prompt: e.target.value || null,
                    })
                  }
                  rows={3}
                  placeholder="Instructions for Claude to classify messages..."
                />
              </div>
            </div>
          </div>

          {/* Source Filters group */}
          <div className="settings-group">
            <div className="settings-row-ia">
              <span className="settings-row-label">Source filters:</span>
              <div className="settings-row-control">
                <div className="filter-chips">
                  <span className="filter-chip filter-chip-auto">
                    to:me
                  </span>
                  {filters.map((f) => (
                    <span key={f.id} className="filter-chip">
                      {f.filter_type === "user" ? "@" : f.filter_type === "to" ? "to:" : "#"}
                      {f.display_name}
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
          </div>

          {/* Categories & Rules group */}
          <div className="settings-group">
            <div className="settings-row-ia settings-row-ia-top">
              <span className="settings-row-label">Categories:</span>
              <div className="settings-row-control">
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
                    placeholder="New category name (press Enter)"
                  />
                </div>

                <div className="settings-hint-text">
                  Add categories to organize messages. Use rules to auto-sort by sender, channel, or keyword.
                </div>
              </div>
            </div>
          </div>
        </div>

      </div>
    </div>
  );
}
