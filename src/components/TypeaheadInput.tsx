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
      let newResults: TypeaheadItem[] = [];
      if (prefix === "@") {
        const users = await searchSlackUsers(searchTerm);
        newResults = users.map((u) => ({
          id: u.id,
          label: u.real_name || u.name,
          sublabel: `@${u.name || u.real_name}`,
          type: "user" as const,
        }));
      } else if (prefix === "#") {
        const channels = await searchSlackChannels(searchTerm);
        newResults = channels.map((c) => ({
          id: c.id,
          label: c.name,
          sublabel: c.is_private ? "private" : "public",
          type: "channel" as const,
        }));
      } else {
        setResults([]);
        setShowDropdown(false);
        return;
      }
      setResults(newResults);
      updateDropdownPos();
      setShowDropdown(newResults.length > 0);
      setHighlightIndex(0);
      return newResults;
    } catch (e) {
      console.error("Search failed:", e);
      return [];
    }
  }, [updateDropdownPos]);

  const handleChange = (value: string) => {
    setQuery(value);
    doSearch(value);
  };

  const selectItem = (item: TypeaheadItem) => {
    onSelect(item);
    setQuery("");
    setResults([]);
    setShowDropdown(false);
    inputRef.current?.focus();
  };

  const handleKeyDown = async (e: React.KeyboardEvent) => {
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

    // Enter-to-add: when dropdown is not visible, try to add typed @/# query
    if (e.key === "Enter" && (!showDropdown || results.length === 0)) {
      e.preventDefault();
      const prefix = query[0];
      const searchTerm = query.slice(1).trim();
      if ((prefix === "@" || prefix === "#") && searchTerm.length > 0) {
        // Try an immediate search to find a match
        const searchResults = await doSearch(query);
        if (searchResults && searchResults.length > 0) {
          selectItem(searchResults[0]);
        } else {
          // No match found — add as-is using typed name
          selectItem({
            id: searchTerm,
            label: searchTerm,
            sublabel: prefix === "@" ? `@${searchTerm}` : `#${searchTerm}`,
            type: prefix === "@" ? "user" : "channel",
          });
        }
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
