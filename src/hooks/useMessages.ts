import { useState, useCallback, useEffect, useRef } from "react";
import {
  Message,
  MessageCounts,
  Category,
  getMessages,
  getMessageCounts,
  getSettings,
  refreshInbox,
  archiveMessage,
  snoozeMessage,
  starMessage,
  openLink,
  RefreshResult,
} from "../lib/tauri";

const DEFAULT_CATEGORIES: Category[] = [
  { name: "important", builtin: true, position: 0 },
  { name: "other", builtin: true, position: 1 },
];

export function applyTheme(theme: string, font: string) {
  document.documentElement.setAttribute("data-theme", theme);
  document.documentElement.setAttribute("data-font", font);
}

export function useMessages() {
  const [tab, setTab] = useState("important");
  const [messages, setMessages] = useState<Message[]>([]);
  const [counts, setCounts] = useState<MessageCounts>({ counts: {} });
  const [categories, setCategories] = useState<Category[]>(DEFAULT_CATEGORIES);
  const [selectedIndex, setSelectedIndex] = useState(0);
  const [loading, setLoading] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  const [lastRefreshResult, setLastRefreshResult] = useState<RefreshResult | null>(null);
  const refreshInterval = useRef<ReturnType<typeof setInterval> | null>(null);

  const loadCategories = useCallback(async () => {
    try {
      const settings = await getSettings();
      const cats = settings.categories ?? DEFAULT_CATEGORIES;
      setCategories(cats.sort((a, b) => a.position - b.position));
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
        getMessages(tab, "inbox"),
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

  const doSnooze = useCallback(async (id: string, until: number) => {
    await snoozeMessage(id, until);
    await fetchMessages();
  }, [fetchMessages]);

  const doStar = useCallback(async (id: string) => {
    await starMessage(id);
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

  const switchTab = useCallback((newTab: string) => {
    setTab(newTab);
    setSelectedIndex(0);
  }, []);

  const cycleTab = useCallback(() => {
    setTab(prev => {
      const idx = categories.findIndex(c => c.name === prev);
      const next = (idx + 1) % categories.length;
      return categories[next].name;
    });
    setSelectedIndex(0);
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
    loading,
    refreshing,
    lastRefreshResult,
    setSelectedIndex,
    switchTab,
    cycleTab,
    moveSelection,
    doRefresh,
    doArchive,
    doSnooze,
    doStar,
    doOpenLink,
    fetchMessages,
    loadCategories,
  };
}
