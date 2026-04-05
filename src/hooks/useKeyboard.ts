import { useEffect, useCallback } from "react";
import { Message, Category } from "../lib/tauri";

interface UseKeyboardProps {
  messages: Message[];
  selectedIndex: number;
  selectedIds: Set<string>;
  selectionAnchor: number | null;
  categories: Category[];
  showSettings: boolean;
  showSnooze: boolean;
  setShowSettings: (v: boolean) => void;
  setShowSnooze: (v: boolean) => void;
  setShowShortcuts: (v: boolean | ((prev: boolean) => boolean)) => void;
  moveSelection: (delta: number) => void;
  toggleSelect: (id: string) => void;
  addToSelection: (id: string) => void;
  selectRange: (anchor: number, current: number) => void;
  setSelectionAnchor: (anchor: number | null) => void;
  clearSelection: () => void;
  selectAll: () => void;
  switchTab: (tab: string) => void;
  cycleTab: () => void;
  doMarkDone: (id: string) => Promise<void>;
  doMarkDoneMany: (ids: string[]) => Promise<void>;
  doStar: (id: string) => Promise<void>;
  doStarMany: (ids: string[]) => Promise<void>;
  doOpenLink: (url: string) => Promise<void>;
  doRefresh: () => Promise<void>;
  setShowSnoozeForSelected: () => void;
}

export function useKeyboard({
  messages,
  selectedIndex,
  selectedIds,
  selectionAnchor,
  categories,
  showSettings,
  showSnooze,
  setShowSettings,
  setShowSnooze,
  setShowShortcuts,
  moveSelection,
  toggleSelect,
  addToSelection,
  selectRange,
  setSelectionAnchor,
  clearSelection,
  selectAll,
  switchTab,
  cycleTab,
  doMarkDone,
  doMarkDoneMany,
  doStar,
  doStarMany,
  doOpenLink,
  doRefresh,
  setShowSnoozeForSelected,
}: UseKeyboardProps) {
  const handleKeyDown = useCallback(
    (e: KeyboardEvent) => {
      // Don't handle keys when in input fields
      const target = e.target as HTMLElement;
      if (target.tagName === "INPUT" || target.tagName === "TEXTAREA") {
        if (e.key === "Escape") {
          setShowSettings(false);
          setShowSnooze(false);
          target.blur();
        }
        return;
      }

      // Close dialogs on Escape
      if (e.key === "Escape") {
        setShowSettings(false);
        setShowSnooze(false);
        setShowShortcuts(false);
        if (selectedIds.size > 0) clearSelection();
        return;
      }

      // Cmd+A: select all messages in current view
      if (e.key === "a" && e.metaKey) {
        e.preventDefault();
        selectAll();
        return;
      }

      // Don't handle shortcuts when settings or snooze dialog is open
      if (showSettings || showSnooze) return;

      const selected = messages[selectedIndex];

      // Helper: get action targets (selected IDs if multi-selected, else cursor message)
      const getActionIds = (): string[] => {
        if (selectedIds.size > 0) return Array.from(selectedIds);
        if (selected) return [selected.id];
        return [];
      };

      switch (e.key) {
        case "j":
        case "ArrowDown":
          e.preventDefault();
          if (e.shiftKey) {
            const anchor = selectionAnchor ?? selectedIndex;
            if (selectionAnchor === null) setSelectionAnchor(selectedIndex);
            const nextIdx = Math.min(selectedIndex + 1, messages.length - 1);
            moveSelection(1);
            selectRange(anchor, nextIdx);
          } else {
            moveSelection(1);
          }
          break;
        case "k":
        case "ArrowUp":
          e.preventDefault();
          if (e.shiftKey) {
            const anchor = selectionAnchor ?? selectedIndex;
            if (selectionAnchor === null) setSelectionAnchor(selectedIndex);
            const prevIdx = Math.max(selectedIndex - 1, 0);
            moveSelection(-1);
            selectRange(anchor, prevIdx);
          } else {
            moveSelection(-1);
          }
          break;
        case "x":
          if (selected) {
            e.preventDefault();
            toggleSelect(selected.id);
          }
          break;
        case "e": {
          const ids = getActionIds();
          if (ids.length > 0) {
            e.preventDefault();
            doMarkDoneMany(ids);
          }
          break;
        }
        case "s": {
          const ids = getActionIds();
          if (ids.length > 0) {
            e.preventDefault();
            doStarMany(ids);
          }
          break;
        }
        case "h": {
          const ids = getActionIds();
          if (ids.length > 0) {
            e.preventDefault();
            setShowSnoozeForSelected();
          }
          break;
        }
        case "Enter":
          if (selectedIds.size > 0) {
            e.preventDefault();
            for (const id of selectedIds) {
              const msg = messages.find(m => m.id === id);
              if (msg?.permalink) doOpenLink(msg.permalink);
            }
          } else if (selected?.permalink) {
            e.preventDefault();
            doOpenLink(selected.permalink);
          }
          break;
        case "r":
          e.preventDefault();
          doRefresh();
          break;
        case "Tab":
          e.preventDefault();
          cycleTab();
          break;
        case "?":
          e.preventDefault();
          setShowShortcuts((prev: boolean) => !prev);
          break;
        default:
          // Number keys 1-9 switch to categories by position
          if (e.key >= "1" && e.key <= "9") {
            const idx = parseInt(e.key, 10) - 1;
            if (idx < categories.length) {
              e.preventDefault();
              switchTab(categories[idx].name);
            }
          }
          break;
      }
    },
    [
      messages,
      selectedIndex,
      selectedIds,
      selectionAnchor,
      categories,
      showSettings,
      showSnooze,
      moveSelection,
      toggleSelect,
      addToSelection,
      selectRange,
      setSelectionAnchor,
      clearSelection,
      selectAll,
      switchTab,
      cycleTab,
      doMarkDone,
      doMarkDoneMany,
      doStar,
      doStarMany,
      doOpenLink,
      doRefresh,
      setShowSettings,
      setShowSnooze,
      setShowShortcuts,
      setShowSnoozeForSelected,
    ]
  );

  useEffect(() => {
    document.addEventListener("keydown", handleKeyDown);
    return () => document.removeEventListener("keydown", handleKeyDown);
  }, [handleKeyDown]);
}
