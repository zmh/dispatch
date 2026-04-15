import { useState, useRef, useCallback } from "react";
import { searchSlackUsers, searchSlackChannels } from "../lib/tauri";
import { useLoadingTimer } from "../hooks/useLoadingTimer";

// Typeahead dropdown item
export interface TypeaheadItem {
  id: string;
  label: string;
  sublabel: string;
  type: "user" | "channel" | "to";
}

type SearchState = "idle" | "loading" | "error";

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
  const [searchState, setSearchState] = useState<SearchState>("idle");
  const [searchError, setSearchError] = useState<string | null>(null);
  const [retryQuery, setRetryQuery] = useState<string | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const searchCounterRef = useRef(0);
  const { isSlow } = useLoadingTimer(searchState === "loading");

  const clearSearchFeedback = useCallback(() => {
    setSearchState("idle");
    setSearchError(null);
    setRetryQuery(null);
  }, []);

  const cancelPendingSearch = useCallback(() => {
    searchCounterRef.current += 1;
    clearSearchFeedback();
  }, [clearSearchFeedback]);

  const updateDropdownPos = useCallback(() => {
    if (inputRef.current) {
      const rect = inputRef.current.getBoundingClientRect();
      setDropdownPos({ top: rect.bottom, left: rect.left, width: rect.width });
    }
  }, []);

  const doSearch = useCallback(async (q: string): Promise<TypeaheadItem[]> => {
    if (q.length < 2) {
      setResults([]);
      setShowDropdown(false);
      cancelPendingSearch();
      return [];
    }

    // Handle to: prefix — freeform, no API search needed
    if (q.startsWith("to:")) {
      // Don't show dropdown for to: — user submits with Enter
      setResults([]);
      setShowDropdown(false);
      cancelPendingSearch();
      return [];
    }

    const prefix = q[0];
    const searchTerm = q.slice(1);
    if (searchTerm.trim().length === 0) {
      setResults([]);
      setShowDropdown(false);
      cancelPendingSearch();
      return [];
    }
    if (prefix !== "@" && prefix !== "#") {
      setResults([]);
      setShowDropdown(false);
      cancelPendingSearch();
      return [];
    }

    const thisSearch = ++searchCounterRef.current;
    setSearchState("loading");
    setSearchError(null);
    setRetryQuery(q);

    try {
      let newResults: TypeaheadItem[] = [];
      if (prefix === "@") {
        const users = await searchSlackUsers(searchTerm);
        if (searchCounterRef.current !== thisSearch) return [];
        newResults = users.map((u) => ({
          id: u.id,
          label: u.real_name || u.name,
          sublabel: `@${u.name || u.real_name}`,
          type: "user" as const,
        }));
      } else if (prefix === "#") {
        const channels = await searchSlackChannels(searchTerm);
        if (searchCounterRef.current !== thisSearch) return [];
        newResults = channels.map((c) => ({
          id: c.id,
          label: c.name,
          sublabel: c.is_private ? "private" : "public",
          type: "channel" as const,
        }));
      }
      setResults(newResults);
      updateDropdownPos();
      setShowDropdown(newResults.length > 0);
      setHighlightIndex(0);
      setSearchState("idle");
      setSearchError(null);
      return newResults;
    } catch (e) {
      if (searchCounterRef.current !== thisSearch) return [];
      console.error("Search failed:", e);
      setResults([]);
      setShowDropdown(false);
      setSearchState("error");
      setSearchError("Search failed. Check your connection and retry.");
      setRetryQuery(q);
      return [];
    }
  }, [cancelPendingSearch, updateDropdownPos]);

  const handleChange = (value: string) => {
    setQuery(value);
    doSearch(value);
  };

  const selectItem = (item: TypeaheadItem) => {
    onSelect(item);
    setQuery("");
    setResults([]);
    setShowDropdown(false);
    cancelPendingSearch();
    inputRef.current?.focus();
  };

  const retrySearch = useCallback(() => {
    const queryToRetry = query.trim().length > 0 ? query : retryQuery;
    if (!queryToRetry) return;
    void doSearch(queryToRetry);
  }, [doSearch, query, retryQuery]);

  const handleBlur = () => {
    setTimeout(() => {
      setShowDropdown(false);
      cancelPendingSearch();
    }, 200);
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
      <div className="typeahead-input-shell">
        <input
          ref={inputRef}
          type="text"
          className="settings-input typeahead-input"
          value={query}
          onChange={(e) => handleChange(e.target.value)}
          onKeyDown={handleKeyDown}
          onBlur={handleBlur}
          placeholder={placeholder}
        />
        <span className="typeahead-indicator-slot" aria-hidden="true">
          {searchState === "loading" && <span className="typeahead-spinner" />}
        </span>
      </div>
      <div className="typeahead-status-row" aria-live="polite">
        {searchState === "error" ? (
          <>
            <span className="typeahead-status-error">
              {searchError}
            </span>
            <button
              type="button"
              className="typeahead-retry-btn"
              onMouseDown={(e) => e.preventDefault()}
              onClick={retrySearch}
            >
              Retry
            </button>
          </>
        ) : searchState === "loading" && isSlow ? (
          <>
            <span className="typeahead-status-slow">Still searching...</span>
            <button
              type="button"
              className="typeahead-retry-btn"
              onMouseDown={(e) => e.preventDefault()}
              onClick={retrySearch}
            >
              Retry
            </button>
          </>
        ) : (
          <span className="typeahead-status-placeholder">&nbsp;</span>
        )}
      </div>
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
