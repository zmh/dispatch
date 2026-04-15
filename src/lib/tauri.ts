import { invoke } from "@tauri-apps/api/core";

export interface Message {
  id: string;
  source: string;
  sender: string;
  subject: string | null;
  body: string;
  body_html: string | null;
  permalink: string | null;
  avatar_url: string | null;
  timestamp: number;
  classification: string;
  status: string;
  starred: boolean;
  unread: boolean;
  snoozed_until: number | null;
  created_at: number;
}

export interface SlackUser {
  id: string;
  name: string;
  real_name: string;
  avatar_url?: string | null;
}

export interface SlackChannel {
  id: string;
  name: string;
  is_private: boolean;
}

export interface SlackFilter {
  filter_type: string; // "user" | "channel"
  id: string;
  display_name: string;
}

export interface Category {
  name: string;
  builtin: boolean;
  position: number;
  description?: string;
}

export interface CategoryRule {
  category: string;
  rule_type: string; // "keyword" | "sender" | "channel"
  value: string;
  id: string | null;
}

export interface Settings {
  slack_token: string | null;
  slack_cookie: string | null;
  ai_provider: string | null;
  claude_api_key: string | null;
  openai_api_key: string | null;
  slack_filters: SlackFilter[] | null;
  categories: Category[] | null;
  category_rules: CategoryRule[] | null;
  theme: string | null;
  font: string | null;
  font_size: string | null;
  open_in_slack_app: boolean | null;
  notifications_enabled: boolean | null;
  beta_release_channel: boolean | null;
  after_archive: string | null;
}

export interface RefreshResult {
  new_messages: number;
  classified: number;
  pending_classification: number;
  in_progress: boolean;
  progress_percent: number;
  slack_fetch_ms: number;
  db_write_ms: number;
  classify_ms: number;
  avatar_ms: number;
  errors: string[];
}

export interface MessageCounts {
  counts: Record<string, number>;
}

export interface SlackCacheStatus {
  user_count: number;
  channel_count: number;
}

export async function getMessages(classification: string, status: string = "inbox"): Promise<Message[]> {
  return invoke("get_messages", { classification, status });
}

export async function getMessagesByStatus(status: string): Promise<Message[]> {
  return invoke("get_messages_by_status", { status });
}

export async function getStarredMessages(): Promise<Message[]> {
  return invoke("get_starred_messages");
}

export async function getMessageCounts(status: string = "inbox"): Promise<MessageCounts> {
  return invoke("get_message_counts", { status });
}

export async function refreshInbox(startIfIdle: boolean = true): Promise<RefreshResult> {
  return invoke("refresh_inbox", { startIfIdle });
}

export async function markDoneMessage(id: string): Promise<void> {
  return invoke("mark_done_message", { id });
}

export async function snoozeMessage(id: string, until: number): Promise<void> {
  return invoke("snooze_message", { id, until });
}

export async function starMessage(id: string): Promise<boolean> {
  return invoke("star_message", { id });
}

export async function setUnreadMessage(id: string, unread: boolean): Promise<boolean> {
  return invoke("set_unread_message", { id, unread });
}

export async function openLink(url: string, useSlackApp: boolean = false): Promise<void> {
  return invoke("open_link", { url, useSlackApp });
}

export async function getSettings(): Promise<Settings> {
  return invoke("get_settings");
}

export interface CodexStatus {
  installed: boolean;
  authenticated: boolean;
  auth_mode: string | null;
  has_codex_subscription: boolean;
  message: string;
}

export async function getCodexStatus(): Promise<CodexStatus> {
  return invoke("get_codex_status");
}

export interface SaveSettingsResult {
  classifications_reset: boolean;
  filters_cleaned: boolean;
}

export async function saveSettings(settings: Settings): Promise<SaveSettingsResult> {
  return invoke("save_settings", { settings });
}

export async function populateSlackCache(): Promise<SlackCacheStatus> {
  return invoke("populate_slack_cache");
}

export async function searchSlackUsers(query: string): Promise<SlackUser[]> {
  return invoke("search_slack_users", { query });
}

export async function searchSlackChannels(query: string): Promise<SlackChannel[]> {
  return invoke("search_slack_channels", { query });
}

export async function getSlackCacheStatus(): Promise<SlackCacheStatus> {
  return invoke("get_slack_cache_status");
}

export interface OnboardingSuggestions {
  suggested_people: SlackUser[];
  suggested_channels: SlackChannel[];
}

export async function getOnboardingSuggestions(): Promise<OnboardingSuggestions> {
  return invoke("get_onboarding_suggestions");
}

export async function setWindowTheme(theme: string): Promise<void> {
  return invoke("set_window_theme", { theme });
}

export interface SlackConnectionInfo {
  team: string;
  user: string;
  team_id: string;
  user_id: string;
}

export async function testSlackConnection(token: string, cookie: string): Promise<SlackConnectionInfo> {
  return invoke("test_slack_connection", { token, cookie });
}
