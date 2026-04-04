import { useState, useCallback, useEffect, useRef } from "react";
import {
  Message,
  MessageCounts,
  Category,
  getMessages,
  getStarredMessages,
  getMessageCounts,
  getSettings,
  refreshInbox,
  archiveMessage,
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

export function applyTheme(theme: string, font: string) {
  document.documentElement.setAttribute("data-theme", theme);
  document.documentElement.setAttribute("data-font", font);
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
      applyTheme(settings.theme || "dark", settings.font || "system");
    } catch (e) {
      console.error("Failed to load categories:", e);
    }
  }, []);

  const fetchMessages = useCallback(async () => {
    setLoading(true);
    try {
      const [msgs, cts] = await Promise.all([
        tab === "starred" ? getStarredMessages() : getMessages(tab, "inbox"),
        getMessageCounts("inbox"),
      ]);
      setMessages(msgs);
      setCounts(cts);
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

  const doArchive = useCallback(async (id: string) => {
    await archiveMessage(id);
    await fetchMessages();
  }, [fetchMessages]);

  const doArchiveMany = useCallback(async (ids: string[]) => {
    for (const id of ids) await archiveMessage(id);
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
      const next = (idx + 1) % categories.length;
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
    moveSelection,
    toggleSelect,
    addToSelection,
    selectRange,
    clearSelection,
    selectAll,
    doRefresh,
    doArchive,
    doArchiveMany,
    doSnooze,
    doSnoozeMany,
    doStar,
    doStarMany,
    doOpenLink,
    fetchMessages,
    loadCategories,
  };
}
