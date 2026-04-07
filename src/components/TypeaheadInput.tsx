import { useState, useRef, useCallback } from "react";
import { searchSlackUsers, searchSlackChannels } from "../lib/tauri";

// Typeahead dropdown item
export interface TypeaheadItem {
  id: string;
  label: string;
  sublabel: string;
  type: "user" | "channel" | "to";
}

export function TypeaheadInput({
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
