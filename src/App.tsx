import { useState, useCallback, useRef, useEffect } from "react";
import { useMessages } from "./hooks/useMessages";
import { useKeyboard } from "./hooks/useKeyboard";
import { InboxTabs } from "./components/InboxTabs";
import { MessageList } from "./components/MessageList";
import { MessagePreview } from "./components/MessagePreview";
import { SnoozeDialog } from "./components/SnoozeDialog";
import { Settings } from "./components/Settings";
import { AboutDialog } from "./components/AboutDialog";
import { UpdateDialog, UpdateStatus } from "./components/UpdateDialog";
import { OnboardingWizard } from "./components/OnboardingWizard";
import { listen } from "@tauri-apps/api/event";
import { getVersion } from "@tauri-apps/api/app";
import { openLink, getSettings, Settings as SettingsType } from "./lib/tauri";

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
    cyclePrevTab,
    moveSelection,
    toggleSelect,
    addToSelection,
    selectRange,
    clearSelection,
    selectAll,
    doRefresh,
    doMarkDoneMany,
    doSnoozeMany,
    doStar,
    doStarMany,
    doOpenLink,
    fetchMessages,
    loadCategories,
  } = useMessages();

  const [showSettings, setShowSettings] = useState(false);
  const [showSnooze, setShowSnooze] = useState(false);
  const [showShortcuts, setShowShortcuts] = useState(false);
  const [showAbout, setShowAbout] = useState(false);
  const [updateStatus, setUpdateStatus] = useState<UpdateStatus | null>(null);
  const [showOnboarding, setShowOnboarding] = useState(false);
  const [settingsChecked, setSettingsChecked] = useState(false);
  const [onboardingSettings, setOnboardingSettings] = useState<SettingsType | undefined>(undefined);
  const [panelWidth, setPanelWidth] = useState(400);
  const isResizing = useRef(false);

  // First-run detection
  useEffect(() => {
    (async () => {
      try {
        const settings = await getSettings();
        if (!settings.slack_token && !settings.slack_cookie) {
          setShowOnboarding(true);
        }
      } catch (e) {
        console.error("Failed to check settings:", e);
      } finally {
        setSettingsChecked(true);
      }
    })();
  }, []);

  // Intercept all <a> clicks and open them in the default browser
  useEffect(() => {
    const handler = async (e: MouseEvent) => {
      const anchor = (e.target as HTMLElement).closest("a");
      if (anchor?.href) {
        e.preventDefault();
        try {
          const settings = await getSettings();
          await openLink(anchor.href, settings.open_in_slack_app ?? false);
        } catch {
          await openLink(anchor.href, false);
        }
      }
    };
    document.addEventListener("click", handler);
    return () => document.removeEventListener("click", handler);
  }, []);

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

  // Listen for update lifecycle events
  useEffect(() => {
    const unlisteners = [
      listen("update-checking", () => {
        setUpdateStatus({ state: "checking" });
      }),
      listen("no-update", () => {
        getVersion().then((v) => {
          setUpdateStatus({ state: "up-to-date", version: v });
        });
      }),
      listen<string>("update-available", (event) => {
        setUpdateStatus((prev) => {
          // Only show downloading state if user triggered (checking was shown)
          if (prev?.state === "checking") {
            return { state: "downloading", version: event.payload };
          }
          return prev;
        });
      }),
      listen<string>("update-installed", (event) => {
        setUpdateStatus({ state: "installed", version: event.payload });
      }),
      listen<string>("update-error", (event) => {
        setUpdateStatus({ state: "error", message: event.payload });
      }),
    ];
    return () => {
      unlisteners.forEach((p) => p.then((f) => f()));
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
    cyclePrevTab,
    doMarkDoneMany,
    doStar,
    doStarMany,
    doOpenLink,
    doRefresh,
    setShowSnoozeForSelected,
  });

  const selectedMessage = messages[selectedIndex];

  const handleSnooze = async (until: number) => {
    const ids = selectedIds.size > 0
      ? Array.from(selectedIds)
      : selectedMessage ? [selectedMessage.id] : [];
    if (ids.length > 0) {
      await doSnoozeMany(ids, until);
      setShowSnooze(false);
    }
  };

  const handleOnboardingComplete = useCallback(() => {
    setShowOnboarding(false);
    setOnboardingSettings(undefined);
    doRefresh();
  }, [doRefresh]);

  const handleRunSetup = useCallback(async () => {
    setShowSettings(false);
    try {
      const settings = await getSettings();
      setOnboardingSettings(settings);
    } catch {}
    setShowOnboarding(true);
  }, []);

  if (!settingsChecked) {
    return <div className="app" />;
  }

  if (showOnboarding) {
    return (
      <OnboardingWizard
        onComplete={handleOnboardingComplete}
        initialSettings={onboardingSettings}
      />
    );
  }

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
          onMessagesChanged={fetchMessages}
          onRequestRefresh={doRefresh}
          onRunSetup={handleRunSetup}
        />
      )}

      {showAbout && (
        <AboutDialog onClose={() => setShowAbout(false)} />
      )}

      {updateStatus && (
        <UpdateDialog
          status={updateStatus}
          onClose={() => setUpdateStatus(null)}
        />
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
