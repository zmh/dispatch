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
  openLink,
  RefreshResult,
  setWindowTheme,
} from "../lib/tauri";

const STARRED_CATEGORY: Category = { name: "starred", builtin: true, position: 0.5 };

const DEFAULT_CATEGORIES: Category[] = [
  { name: "important", builtin: true, position: 0 },
  { name: "other", builtin: true, position: 1 },
];

export function applyTheme(theme: string, font: string, fontSize?: string) {
  document.documentElement.setAttribute("data-theme", theme);
  document.documentElement.setAttribute("data-font", font);
  document.documentElement.setAttribute("data-font-size", fontSize || "s");
  setWindowTheme(theme).catch(console.error);
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
  const refreshInterval = useRef<ReturnType<typeof setInterval> | null>(null);

  const loadCategories = useCallback(async () => {
    try {
      const settings = await getSettings();
      const cats = settings.categories ?? DEFAULT_CATEGORIES;
      // Inject the built-in Starred category after Important
      const withStarred = [...cats.filter(c => c.name !== "starred"), STARRED_CATEGORY];
      setCategories(withStarred.sort((a, b) => a.position - b.position));
      // Apply theme/font on load
      applyTheme(settings.theme || "dark", settings.font || "system", settings.font_size || "s");
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
      // Clamp selected index
      if (msgs.length > 0 && selectedIndex >= msgs.length) {
        setSelectedIndex(msgs.length - 1);
      }
    } catch (e) {
      console.error("Failed to fetch messages:", e);
    } finally {
      setLoading(false);
    }
  }, [tab, selectedIndex]);

  const doRefresh = useCallback(async () => {
    setRefreshing(true);
    try {
      const result = await refreshInbox();
      setLastRefreshResult(result);
      await fetchMessages();
    } catch (e) {
      console.error("Failed to refresh:", e);
    } finally {
      setRefreshing(false);
    }
  }, [fetchMessages]);

  const doMarkDone = useCallback(async (id: string) => {
    await markDoneMessage(id);
    await fetchMessages();
  }, [fetchMessages]);

  const doMarkDoneMany = useCallback(async (ids: string[]) => {
    for (const id of ids) await markDoneMessage(id);
    setSelectedIds(new Set());
    await fetchMessages();
  }, [fetchMessages]);

  const doSnooze = useCallback(async (id: string, until: number) => {
    await snoozeMessage(id, until);
    await fetchMessages();
  }, [fetchMessages]);

  const doSnoozeMany = useCallback(async (ids: string[], until: number) => {
    for (const id of ids) await snoozeMessage(id, until);
    setSelectedIds(new Set());
    await fetchMessages();
  }, [fetchMessages]);

  const doStar = useCallback(async (id: string) => {
    await starMessage(id);
    await fetchMessages();
  }, [fetchMessages]);

  const doStarMany = useCallback(async (ids: string[]) => {
    for (const id of ids) await starMessage(id);
    setSelectedIds(new Set());
    await fetchMessages();
  }, [fetchMessages]);

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
  }, [messages]);

  const switchTab = useCallback((newTab: string) => {
    setTab(newTab);
    setSelectedIndex(0);
    setSelectedIds(new Set());
  }, []);

  const cycleTab = useCallback(() => {
    setTab(prev => {
      const idx = categories.findIndex(c => c.name === prev);
      if (idx < 0) return categories[0].name;
      const next = (idx + 1) % categories.length;
      return categories[next].name;
    });
    setSelectedIndex(0);
    setSelectedIds(new Set());
  }, [categories]);

  const cyclePrevTab = useCallback(() => {
    setTab(prev => {
      const idx = categories.findIndex(c => c.name === prev);
      if (idx < 0) return categories[0].name;
      const next = (idx - 1 + categories.length) % categories.length;
      return categories[next].name;
    });
    setSelectedIndex(0);
    setSelectedIds(new Set());
  }, [categories]);

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
    doMarkDone,
    doMarkDoneMany,
    doSnooze,
    doSnoozeMany,
    doStar,
    doStarMany,
    doOpenLink,
    fetchMessages,
    loadCategories,
  };
}
