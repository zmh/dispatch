import { useEffect, useCallback } from "react";
import { Message, Category } from "../lib/tauri";

interface UseKeyboardProps {
  messages: Message[];
  selectedIndex: number;
  categories: Category[];
  showSettings: boolean;
  showSnooze: boolean;
  setShowSettings: (v: boolean) => void;
  setShowSnooze: (v: boolean) => void;
  setShowShortcuts: (v: boolean | ((prev: boolean) => boolean)) => void;
  moveSelection: (delta: number) => void;
  switchTab: (tab: string) => void;
  cycleTab: () => void;
  doArchive: (id: string) => Promise<void>;
  doStar: (id: string) => Promise<void>;
  doOpenLink: (url: string) => Promise<void>;
  doRefresh: () => Promise<void>;
}

export function useKeyboard({
  messages,
  selectedIndex,
  categories,
  showSettings,
  showSnooze,
  setShowSettings,
  setShowSnooze,
  setShowShortcuts,
  moveSelection,
  switchTab,
  cycleTab,
  doArchive,
  doStar,
  doOpenLink,
  doRefresh,
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
        return;
      }

      // Don't handle shortcuts when settings or snooze dialog is open
      if (showSettings || showSnooze) return;

      const selected = messages[selectedIndex];

      switch (e.key) {
        case "j":
        case "ArrowDown":
          e.preventDefault();
          moveSelection(1);
          break;
        case "k":
        case "ArrowUp":
          e.preventDefault();
          moveSelection(-1);
          break;
        case "e":
          if (selected) {
            e.preventDefault();
            doArchive(selected.id);
          }
          break;
        case "s":
          if (selected) {
            e.preventDefault();
            doStar(selected.id);
          }
          break;
        case "h":
          if (selected) {
            e.preventDefault();
            setShowSnooze(true);
          }
          break;
        case "Enter":
          if (selected?.permalink) {
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
      categories,
      showSettings,
      showSnooze,
      moveSelection,
      switchTab,
      cycleTab,
      doArchive,
      doStar,
      doOpenLink,
      doRefresh,
      setShowSettings,
      setShowSnooze,
      setShowShortcuts,
    ]
  );

  useEffect(() => {
    document.addEventListener("keydown", handleKeyDown);
    return () => document.removeEventListener("keydown", handleKeyDown);
  }, [handleKeyDown]);
}
