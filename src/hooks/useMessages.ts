import { useState, useCallback, useEffect, useRef } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  Message,
  MessageCounts,
  Category,
  getMessages,
  getMessagesByStatus,
  getStarredMessages,
  getMessageCounts,
  getSettings,
  refreshInbox,
  markDoneMessage,
  snoozeMessage,
  starMessage,
  setUnreadMessage,
  openLink,
  RefreshResult,
  setWindowTheme,
} from "../lib/tauri";

const STARRED_CATEGORY: Category = { name: "starred", builtin: true, position: 0.5 };

const DEFAULT_CATEGORIES: Category[] = [
  { name: "important", builtin: true, position: 0 },
  { name: "other", builtin: true, position: 1 },
];

let systemThemeCleanup: (() => void) | null = null;

function resolveSystemTheme(): string {
  return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

export function applyTheme(theme: string, font: string, fontSize?: string) {
  // Clean up any previous system theme listener
  if (systemThemeCleanup) {
    systemThemeCleanup();
    systemThemeCleanup = null;
  }

  const resolved = theme === "system" ? resolveSystemTheme() : theme;
  document.documentElement.setAttribute("data-theme", resolved);
  document.documentElement.setAttribute("data-font", font);
  document.documentElement.setAttribute("data-font-size", fontSize || "s");
  setWindowTheme(resolved).catch(console.error);

  // Listen for OS appearance changes when using system theme
  if (theme === "system") {
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const handler = (e: MediaQueryListEvent) => {
      const newTheme = e.matches ? "dark" : "light";
      document.documentElement.setAttribute("data-theme", newTheme);
      setWindowTheme(newTheme).catch(console.error);
    };
    mq.addEventListener("change", handler);
    systemThemeCleanup = () => mq.removeEventListener("change", handler);
  }
}

export function useMessages() {
  const [tab, setTab] = useState("important");
  const [messages, setMessages] = useState<Message[]>([]);
  const [counts, setCounts] = useState<MessageCounts>({ counts: {} });
  const [categories, setCategories] = useState<Category[]>(DEFAULT_CATEGORIES);
  const [selectedIndex, setSelectedIndex] = useState(0);
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [selectionAnchor, setSelectionAnchor] = useState<number | null>(null);
  const [loading, setLoading] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  const [lastRefreshResult, setLastRefreshResult] = useState<RefreshResult | null>(null);
  const [refreshProgressPercent, setRefreshProgressPercent] = useState(0);
  const [refreshStatusVisible, setRefreshStatusVisible] = useState(false);
  const refreshInterval = useRef<ReturnType<typeof setInterval> | null>(null);
  const refreshStatusPollInterval = useRef<ReturnType<typeof setInterval> | null>(null);
  const afterArchiveRef = useRef<string>("newer");
  const pendingCursorTarget = useRef<string | null>(null);
  const refreshPromiseRef = useRef<Promise<void> | null>(null);
  const previewedMessageIdRef = useRef<string | null>(null);

  const loadCategories = useCallback(async () => {
    try {
      const settings = await getSettings();
      const cats = settings.categories ?? DEFAULT_CATEGORIES;
      // Inject the built-in Starred category after Important
      const withStarred = [...cats.filter(c => c.name !== "starred"), STARRED_CATEGORY];
      setCategories(withStarred.sort((a, b) => a.position - b.position));
      // Apply theme/font on load
      applyTheme(settings.theme || "dark", settings.font || "system", settings.font_size || "s");
      afterArchiveRef.current = settings.after_archive ?? "newer";
    } catch (e) {
      console.error("Failed to load categories:", e);
    }
  }, []);

  const isStatusTab = (t: string) => t === "snoozed" || t === "archived";

  const fetchMessages = useCallback(async () => {
    setLoading(true);
    try {
      let msgsPromise: Promise<Message[]>;
      if (tab === "starred") {
        msgsPromise = getStarredMessages();
      } else if (isStatusTab(tab)) {
        msgsPromise = getMessagesByStatus(tab);
      } else {
        msgsPromise = getMessages(tab, "inbox");
      }

      const [msgs, inboxCts, snoozedCts, archivedCts] = await Promise.all([
        msgsPromise,
        getMessageCounts("inbox"),
        getMessageCounts("snoozed"),
        getMessageCounts("archived"),
      ]);

      // Merge counts: inbox classification counts + total snoozed/archived
      const mergedCounts = { ...inboxCts.counts };
      const snoozedTotal = Object.values(snoozedCts.counts).reduce((a, b) => a + b, 0) - (snoozedCts.counts["starred"] || 0);
      const archivedTotal = Object.values(archivedCts.counts).reduce((a, b) => a + b, 0) - (archivedCts.counts["starred"] || 0);
      mergedCounts["snoozed"] = snoozedTotal;
      mergedCounts["archived"] = archivedTotal;

      setMessages(msgs);
      setCounts({ counts: mergedCounts });
      // If a cursor target was set (e.g. after archive), find it in the new list
      if (pendingCursorTarget.current) {
        const targetIdx = msgs.findIndex(m => m.id === pendingCursorTarget.current);
        pendingCursorTarget.current = null;
        if (targetIdx >= 0) {
          setSelectedIndex(targetIdx);
        } else if (msgs.length > 0) {
          setSelectedIndex(Math.min(selectedIndex, msgs.length - 1));
        }
      } else if (msgs.length > 0 && selectedIndex >= msgs.length) {
        // Clamp selected index
        setSelectedIndex(msgs.length - 1);
      }
    } catch (e) {
      console.error("Failed to fetch messages:", e);
    } finally {
      setLoading(false);
    }
  }, [tab, selectedIndex]);

  const doRefresh = useCallback(async (showStatus: boolean = false) => {
    if (refreshPromiseRef.current) {
      if (showStatus) {
        setRefreshStatusVisible(true);
      }
      return refreshPromiseRef.current;
    }

    const run = (async () => {
      setRefreshing(true);
      setRefreshProgressPercent(1);
      setRefreshStatusVisible(showStatus);

      try {
        const activeRefresh = refreshInbox(true);
        const pollRefreshStatus = async () => {
          try {
            const status = await refreshInbox(false);
            if (!status.in_progress) {
              return;
            }
            setRefreshProgressPercent((prev) =>
              Math.max(prev, Math.min(99, Math.max(1, status.progress_percent || 0)))
            );
          } catch {
            // Ignore polling failures during an active refresh.
          }
        };

        if (refreshStatusPollInterval.current) {
          clearInterval(refreshStatusPollInterval.current);
        }
        refreshStatusPollInterval.current = setInterval(() => {
          void pollRefreshStatus();
        }, 800);
        void pollRefreshStatus();

        let result = await activeRefresh;
        setLastRefreshResult(result);
        setRefreshProgressPercent((prev) =>
          Math.max(
            prev,
            Math.min(result.in_progress ? 99 : 100, Math.max(1, result.progress_percent || 0))
          )
        );

        while (result.in_progress) {
          await new Promise<void>((resolve) => {
            setTimeout(resolve, 1000);
          });
          result = await refreshInbox(false);
          setLastRefreshResult(result);
          setRefreshProgressPercent((prev) =>
            Math.max(
              prev,
              Math.min(result.in_progress ? 99 : 100, Math.max(1, result.progress_percent || 0))
            )
          );
        }

        await fetchMessages();
      } catch (e) {
        console.error("Failed to refresh:", e);
        const reason = e instanceof Error && e.message ? ` ${e.message}` : "";
        setLastRefreshResult({
          new_messages: 0,
          classified: 0,
          pending_classification: 0,
          in_progress: false,
          progress_percent: 0,
          slack_fetch_ms: 0,
          db_write_ms: 0,
          classify_ms: 0,
          avatar_ms: 0,
          errors: [`Refresh failed.${reason}`],
        });
      } finally {
        if (refreshStatusPollInterval.current) {
          clearInterval(refreshStatusPollInterval.current);
          refreshStatusPollInterval.current = null;
        }
        setRefreshProgressPercent(0);
        setRefreshStatusVisible(false);
        setRefreshing(false);
        refreshPromiseRef.current = null;
      }
    })();

    refreshPromiseRef.current = run;
    return run;
  }, [fetchMessages]);

  const computeCursorTarget = useCallback((ids: string[]) => {
    const setting = afterArchiveRef.current;
    if (setting === "stay" || messages.length === 0) return;

    const idsSet = new Set(ids);
    // Messages are sorted newest-first (index 0 = newest, higher index = older)
    if (setting === "newer") {
      // Newer = lower index (toward top of list)
      for (let i = selectedIndex - 1; i >= 0; i--) {
        if (!idsSet.has(messages[i].id)) {
          pendingCursorTarget.current = messages[i].id;
          return;
        }
      }
      // No newer survivor — fall back to older
      for (let i = selectedIndex + 1; i < messages.length; i++) {
        if (!idsSet.has(messages[i].id)) {
          pendingCursorTarget.current = messages[i].id;
          return;
        }
      }
    } else {
      // "older" = higher index (toward bottom of list)
      for (let i = selectedIndex + 1; i < messages.length; i++) {
        if (!idsSet.has(messages[i].id)) {
          pendingCursorTarget.current = messages[i].id;
          return;
        }
      }
      // No older survivor — fall back to newer
      for (let i = selectedIndex - 1; i >= 0; i--) {
        if (!idsSet.has(messages[i].id)) {
          pendingCursorTarget.current = messages[i].id;
          return;
        }
      }
    }
  }, [messages, selectedIndex]);

  const doMarkDoneMany = useCallback(async (ids: string[]) => {
    computeCursorTarget(ids);
    setSelectedIds(new Set());
    setSelectionAnchor(null);
    for (const id of ids) await markDoneMessage(id);
    await fetchMessages();
  }, [fetchMessages, computeCursorTarget]);

  const doSnoozeMany = useCallback(async (ids: string[], until: number) => {
    computeCursorTarget(ids);
    setSelectedIds(new Set());
    setSelectionAnchor(null);
    for (const id of ids) await snoozeMessage(id, until);
    await fetchMessages();
  }, [fetchMessages, computeCursorTarget]);

  const doStar = useCallback(async (id: string) => {
    await starMessage(id);
    await fetchMessages();
  }, [fetchMessages]);

  const doStarMany = useCallback(async (ids: string[]) => {
    for (const id of ids) await starMessage(id);
    setSelectedIds(new Set());
    await fetchMessages();
  }, [fetchMessages]);

  const setUnreadLocal = useCallback((ids: string[], unread: boolean) => {
    if (ids.length === 0) return;
    const idSet = new Set(ids);
    setMessages(prev => {
      let changed = false;
      const next = prev.map((msg) => {
        if (!idSet.has(msg.id) || msg.unread === unread) return msg;
        changed = true;
        return { ...msg, unread };
      });
      return changed ? next : prev;
    });
  }, []);

  const doSetUnreadMany = useCallback(async (ids: string[], unread: boolean) => {
    if (ids.length === 0) return;
    setUnreadLocal(ids, unread);
    try {
      await Promise.all(ids.map(id => setUnreadMessage(id, unread)));
    } catch (e) {
      console.error("Failed to update unread status:", e);
      await fetchMessages();
    }
  }, [fetchMessages, setUnreadLocal]);

  const doToggleUnreadMany = useCallback(async (ids: string[]) => {
    if (ids.length === 0) return;
    const byId = new Map(messages.map(msg => [msg.id, msg]));
    const allRead = ids.every(id => !byId.get(id)?.unread);
    await doSetUnreadMany(ids, allRead);
  }, [messages, doSetUnreadMany]);

  const doOpenLink = useCallback(async (url: string) => {
    try {
      const settings = await getSettings();
      await openLink(url, settings.open_in_slack_app ?? false);
    } catch {
      await openLink(url, false);
    }
  }, []);

  const toggleSelect = useCallback((id: string) => {
    setSelectedIds(prev => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }, []);

  const addToSelection = useCallback((id: string) => {
    setSelectedIds(prev => {
      if (prev.has(id)) return prev;
      const next = new Set(prev);
      next.add(id);
      return next;
    });
  }, []);

  const selectRange = useCallback((anchor: number, current: number) => {
    const start = Math.min(anchor, current);
    const end = Math.max(anchor, current);
    const ids = new Set<string>();
    for (let i = start; i <= end; i++) {
      if (messages[i]) ids.add(messages[i].id);
    }
    setSelectedIds(ids);
  }, [messages]);

  const clearSelection = useCallback(() => {
    setSelectedIds(new Set());
    setSelectionAnchor(null);
  }, []);

  const selectAll = useCallback(() => {
    setSelectedIds(new Set(messages.map(m => m.id)));
    setSelectionAnchor(0);
  }, [messages]);

  const switchTab = useCallback((newTab: string) => {
    // Prevent a transient tab-switch render from auto-reading the previous tab's first row.
    previewedMessageIdRef.current = messages[0]?.id ?? null;
    setTab(newTab);
    setSelectedIndex(0);
    setSelectedIds(new Set());
    setSelectionAnchor(null);
  }, [messages]);

  const cycleTab = useCallback(() => {
    // Prevent a transient tab-switch render from auto-reading the previous tab's first row.
    previewedMessageIdRef.current = messages[0]?.id ?? null;
    setTab(prev => {
      const idx = categories.findIndex(c => c.name === prev);
      if (idx < 0) return categories[0].name;
      const next = (idx + 1) % categories.length;
      return categories[next].name;
    });
    setSelectedIndex(0);
    setSelectedIds(new Set());
    setSelectionAnchor(null);
  }, [categories, messages]);

  const cyclePrevTab = useCallback(() => {
    // Prevent a transient tab-switch render from auto-reading the previous tab's first row.
    previewedMessageIdRef.current = messages[0]?.id ?? null;
    setTab(prev => {
      const idx = categories.findIndex(c => c.name === prev);
      if (idx < 0) return categories[0].name;
      const next = (idx - 1 + categories.length) % categories.length;
      return categories[next].name;
    });
    setSelectedIndex(0);
    setSelectedIds(new Set());
    setSelectionAnchor(null);
  }, [categories, messages]);

  const moveSelection = useCallback((delta: number) => {
    setSelectedIndex(prev => {
      const next = prev + delta;
      if (next < 0) return 0;
      if (next >= messages.length) return messages.length - 1;
      return next;
    });
  }, [messages.length]);

  // Load categories on mount
  useEffect(() => {
    loadCategories();
  }, [loadCategories]);

  // Fetch messages when tab changes
  useEffect(() => {
    fetchMessages();
  }, [tab]);

  // Auto-refresh every 5 minutes
  useEffect(() => {
    refreshInterval.current = setInterval(() => {
      doRefresh();
    }, 5 * 60 * 1000);
    return () => {
      if (refreshInterval.current) clearInterval(refreshInterval.current);
      if (refreshStatusPollInterval.current) {
        clearInterval(refreshStatusPollInterval.current);
        refreshStatusPollInterval.current = null;
      }
    };
  }, [doRefresh]);

  // Refresh when snoozed messages return (from background checker)
  useEffect(() => {
    const unlisten = listen("snooze-returned", () => {
      fetchMessages();
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, [fetchMessages]);

  // Refresh when background classification updates message categories.
  useEffect(() => {
    const unlisten = listen("messages-classified", () => {
      fetchMessages();
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, [fetchMessages]);

  // Mark newly previewed messages as read when the cursor/hovered row changes.
  useEffect(() => {
    const current = messages[selectedIndex];
    const currentId = current?.id ?? null;

    if (currentId === previewedMessageIdRef.current) {
      return;
    }
    previewedMessageIdRef.current = currentId;

    if (current && current.unread) {
      doSetUnreadMany([current.id], false);
    }
  }, [messages, selectedIndex, doSetUnreadMany]);

  return {
    tab,
    messages,
    counts,
    categories,
    selectedIndex,
    selectedIds,
    selectionAnchor,
    loading,
    refreshing,
    refreshProgressPercent,
    refreshStatusVisible,
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
    doToggleUnreadMany,
    doOpenLink,
    fetchMessages,
    loadCategories,
  };
}
