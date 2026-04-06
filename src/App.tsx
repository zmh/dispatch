import { useState, useCallback, useRef, useEffect } from "react";
import { useMessages } from "./hooks/useMessages";
import { useKeyboard } from "./hooks/useKeyboard";
import { InboxTabs } from "./components/InboxTabs";
import { MessageList } from "./components/MessageList";
import { MessagePreview } from "./components/MessagePreview";
import { SnoozeDialog } from "./components/SnoozeDialog";
import { Settings } from "./components/Settings";
import { AboutDialog } from "./components/AboutDialog";
import { listen } from "@tauri-apps/api/event";

function App() {
  const {
    tab,
    messages,
    counts,
    categories,
    selectedIndex,
    selectedIds,
    selectionAnchor,
    loading,
    refreshing,
    lastRefreshResult,
    setSelectedIndex,
    setSelectionAnchor,
    switchTab,
    cycleTab,
    moveSelection,
    toggleSelect,
    addToSelection,
    selectRange,
    clearSelection,
    selectAll,
    doRefresh,
    doMarkDone,
    doMarkDoneMany,
    doSnooze,
    doSnoozeMany,
    doStar,
    doStarMany,
    doOpenLink,
    loadCategories,
  } = useMessages();

  const [showSettings, setShowSettings] = useState(false);
  const [showSnooze, setShowSnooze] = useState(false);
  const [showShortcuts, setShowShortcuts] = useState(false);
  const [showAbout, setShowAbout] = useState(false);
  const [panelWidth, setPanelWidth] = useState(400);
  const isResizing = useRef(false);

  // Listen for native menu events
  useEffect(() => {
    const unlistenSettings = listen("open-settings", () => {
      setShowSettings(true);
    });
    const unlistenAbout = listen("open-about", () => {
      setShowAbout(true);
    });
    return () => {
      unlistenSettings.then((f) => f());
      unlistenAbout.then((f) => f());
    };
  }, []);

  const handleResizeStart = useCallback((e: React.MouseEvent) => {
    e.preventDefault();
    isResizing.current = true;
    const startX = e.clientX;
    const startWidth = panelWidth;

    const onMouseMove = (e: MouseEvent) => {
      if (!isResizing.current) return;
      const newWidth = Math.min(600, Math.max(250, startWidth + (e.clientX - startX)));
      setPanelWidth(newWidth);
    };

    const onMouseUp = () => {
      isResizing.current = false;
      document.removeEventListener("mousemove", onMouseMove);
      document.removeEventListener("mouseup", onMouseUp);
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
    };

    document.addEventListener("mousemove", onMouseMove);
    document.addEventListener("mouseup", onMouseUp);
    document.body.style.cursor = "col-resize";
    document.body.style.userSelect = "none";
  }, [panelWidth]);

  const setShowSnoozeForSelected = useCallback(() => {
    const hasTargets = selectedIds.size > 0 || messages[selectedIndex];
    if (hasTargets) setShowSnooze(true);
  }, [selectedIds, messages, selectedIndex]);

  useKeyboard({
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
  });

  const selectedMessage = messages[selectedIndex];

  const handleSnooze = async (until: number) => {
    if (selectedIds.size > 0) {
      await doSnoozeMany(Array.from(selectedIds), until);
      setShowSnooze(false);
    } else if (selectedMessage) {
      await doSnooze(selectedMessage.id, until);
      setShowSnooze(false);
    }
  };

  return (
    <div className="app">
      <InboxTabs
        tab={tab}
        counts={counts}
        categories={categories}
        refreshing={refreshing}
        onSwitchTab={switchTab}
        onRefresh={doRefresh}
        onOpenSettings={() => setShowSettings(true)}
      />

      <div className="app-main">
        <MessageList
          messages={messages}
          selectedIndex={selectedIndex}
          selectedIds={selectedIds}
          loading={loading}
          onSelect={setSelectedIndex}
          onOpen={doOpenLink}
          style={{ width: panelWidth }}
        />
        <div className="resize-handle" onMouseDown={handleResizeStart} />
        <MessagePreview
          message={selectedMessage}
          onOpenLink={doOpenLink}
        />
      </div>

      <div className="shortcut-bar">
        {lastRefreshResult && lastRefreshResult.errors.length > 0 && (
          <span className="status-errors">
            ⚠ {lastRefreshResult.errors[0]}
          </span>
        )}
        {selectedIds.size > 0 ? (
          <>
            <span className="selection-count">
              {selectedIds.size} selected
            </span>
            <span className="shortcut-bar-spacer" />
            <span className="shortcut"><kbd>e</kbd> done</span>
            <span className="shortcut"><kbd>h</kbd> snooze</span>
            <span className="shortcut"><kbd>s</kbd> star</span>
            <span className="shortcut"><kbd>↵</kbd> open</span>
            <span className="shortcut"><kbd>Esc</kbd> clear</span>
          </>
        ) : (
          <>
            <span className="shortcut-bar-spacer" />
            <span className="shortcut"><kbd>x</kbd> select</span>
            <span className="shortcut"><kbd>e</kbd> done</span>
            <span className="shortcut"><kbd>h</kbd> snooze</span>
            <span className="shortcut"><kbd>s</kbd> star</span>
            <span className="shortcut"><kbd>j</kbd>/<kbd>k</kbd> navigate</span>
            <span className="shortcut"><kbd>↵</kbd> open</span>
            <span className="shortcut"><kbd>r</kbd> refresh</span>
            <span className="shortcut"><kbd>?</kbd> help</span>
          </>
        )}
      </div>

      {showSnooze && (
        <SnoozeDialog
          onSnooze={handleSnooze}
          onClose={() => setShowSnooze(false)}
        />
      )}

      {showSettings && (
        <Settings
          onClose={() => setShowSettings(false)}
          onCategoriesChanged={loadCategories}
        />
      )}

      {showAbout && (
        <AboutDialog onClose={() => setShowAbout(false)} />
      )}

      {showShortcuts && (
        <div className="dialog-overlay" onClick={() => setShowShortcuts(false)}>
          <div className="dialog shortcuts-dialog" onClick={(e) => e.stopPropagation()}>
            <div className="dialog-title">Keyboard Shortcuts</div>
            <div className="shortcuts-list">
              <div className="shortcut-row"><kbd>j</kbd> / <kbd>↓</kbd><span>Move down</span></div>
              <div className="shortcut-row"><kbd>k</kbd> / <kbd>↑</kbd><span>Move up</span></div>
              <div className="shortcut-row"><kbd>x</kbd><span>Toggle select</span></div>
              <div className="shortcut-row"><kbd>⇧↓</kbd> / <kbd>⇧↑</kbd><span>Extend selection</span></div>
              <div className="shortcut-row"><kbd>e</kbd><span>Mark done</span></div>
              <div className="shortcut-row"><kbd>h</kbd><span>Snooze message</span></div>
              <div className="shortcut-row"><kbd>s</kbd><span>Toggle star</span></div>
              <div className="shortcut-row"><kbd>Enter</kbd><span>Open in browser</span></div>
              <div className="shortcut-row"><kbd>r</kbd><span>Refresh inbox</span></div>
              <div className="shortcut-row"><kbd>Tab</kbd><span>Cycle tabs</span></div>
              {categories.map((cat, i) => (
                <div key={cat.name} className="shortcut-row">
                  <kbd>{i + 1}</kbd>
                  <span>{cat.name.charAt(0).toUpperCase() + cat.name.slice(1)} tab</span>
                </div>
              ))}
              <div className="shortcut-row"><kbd>?</kbd><span>Toggle this help</span></div>
              <div className="shortcut-row"><kbd>Esc</kbd><span>Close dialog</span></div>
            </div>
            <div className="dialog-footer">
              <button className="dialog-cancel" onClick={() => setShowShortcuts(false)}>
                Close
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

export default App;
