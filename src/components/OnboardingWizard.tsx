import { useState, useEffect, useRef, useCallback } from "react";
import {
  CodexStatus,
  Settings,
  SlackFilter,
  SlackChannel,
  SlackUser,
  SlackConnectionInfo,
  getSettings,
  saveSettings,
  testSlackConnection,
  populateSlackCache,
  searchSlackChannels,
  getOnboardingSuggestions,
  getCodexStatus,
} from "../lib/tauri";
import { TypeaheadInput, TypeaheadItem } from "./TypeaheadInput";
import { useLoadingTimer } from "../hooks/useLoadingTimer";

interface OnboardingWizardProps {
  onComplete: () => void;
  initialSettings?: Settings;
}

const TOKEN_CMD = `Object.entries(JSON.parse(localStorage.localConfig_v2).teams).forEach(([,t])=>console.log(t.name,t.token))`;
const COOKIE_CMD = `document.cookie.split("; ").find(c=>c.startsWith("d=")).slice(2)`;

type WorkspaceLoadPhase = "idle" | "loading_cache" | "loading_suggestions" | "ready" | "error";

function CopyBlock({ code }: { code: string }) {
  const [copied, setCopied] = useState(false);
  const handleCopy = () => {
    navigator.clipboard.writeText(code);
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  };
  return (
    <div className="onboarding-code">
      <code>{code}</code>
      <button className="onboarding-copy-btn" onClick={handleCopy}>
        {copied ? "Copied!" : "Copy"}
      </button>
    </div>
  );
}

export function OnboardingWizard({ onComplete, initialSettings }: OnboardingWizardProps) {
  const initialAiProvider = (initialSettings?.ai_provider || "").trim().toLowerCase();
  const [step, setStep] = useState(0);
  const [slackToken, setSlackToken] = useState(initialSettings?.slack_token || "");
  const [slackCookie, setSlackCookie] = useState(initialSettings?.slack_cookie || "");
  const [filters, setFilters] = useState<SlackFilter[]>(initialSettings?.slack_filters || []);
  const [aiProvider, setAiProvider] = useState(initialAiProvider);
  const [savedAiProvider, setSavedAiProvider] = useState(initialAiProvider);
  const [claudeApiKey, setClaudeApiKey] = useState(initialSettings?.claude_api_key || "");
  const [openaiApiKey, setOpenaiApiKey] = useState(initialSettings?.openai_api_key || "");
  const [codexStatus, setCodexStatus] = useState<CodexStatus | null>(null);
  const [connectionInfo, setConnectionInfo] = useState<SlackConnectionInfo | null>(null);
  const [suggestedChannels, setSuggestedChannels] = useState<SlackChannel[]>([]);
  const [suggestedPeople, setSuggestedPeople] = useState<SlackUser[]>([]);
  const [workspaceLoadPhase, setWorkspaceLoadPhase] = useState<WorkspaceLoadPhase>("idle");
  const [workspaceLoadError, setWorkspaceLoadError] = useState<string | null>(null);
  const [testing, setTesting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [savingAi, setSavingAi] = useState(false);
  const [loadingCodexStatus, setLoadingCodexStatus] = useState(false);
  const autoAdvanceRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const workspaceLoadSeqRef = useRef(0);
  const {
    elapsedSeconds: cacheLoadElapsedSeconds,
    isSlow: cacheLoadIsSlow,
  } = useLoadingTimer(workspaceLoadPhase === "loading_cache");
  const {
    elapsedSeconds: suggestionsLoadElapsedSeconds,
    isSlow: suggestionsLoadIsSlow,
  } = useLoadingTimer(workspaceLoadPhase === "loading_suggestions");

  const applyWorkspaceSuggestions = useCallback((people: SlackUser[], channels: SlackChannel[]) => {
    const topPeople = people.slice(0, 10);
    const topChannels = channels.slice(0, 15);
    setSuggestedPeople(topPeople);
    setSuggestedChannels(topChannels);
    setFilters((prev) => {
      if (prev.length > 0) return prev;
      const preselected: SlackFilter[] = [];
      for (const u of topPeople.slice(0, 3)) {
        preselected.push({ filter_type: "user", id: u.id, display_name: u.real_name || u.name });
      }
      for (const ch of topChannels.slice(0, 3)) {
        preselected.push({ filter_type: "channel", id: ch.id, display_name: ch.name });
      }
      return preselected;
    });
  }, []);

  const loadWorkspaceData = useCallback(async () => {
    if (!slackToken || !slackCookie) return;

    const loadSeq = ++workspaceLoadSeqRef.current;
    setWorkspaceLoadError(null);
    setWorkspaceLoadPhase("loading_cache");

    let cacheFailed = false;
    try {
      await populateSlackCache();
    } catch (cacheError) {
      console.error("Workspace cache preload failed:", cacheError);
      cacheFailed = true;
    }
    if (workspaceLoadSeqRef.current !== loadSeq) return;

    setWorkspaceLoadPhase("loading_suggestions");

    let people: SlackUser[] = [];
    let channels: SlackChannel[] = [];
    let suggestionsLoaded = false;

    try {
      const suggestions = await getOnboardingSuggestions();
      if (workspaceLoadSeqRef.current !== loadSeq) return;
      people = suggestions.suggested_people;
      channels = suggestions.suggested_channels;
      suggestionsLoaded = true;
    } catch (suggestionsError) {
      console.error("Failed to load onboarding suggestions:", suggestionsError);
    }

    if (!suggestionsLoaded || channels.length === 0) {
      try {
        const liveChannels = await searchSlackChannels("");
        if (workspaceLoadSeqRef.current !== loadSeq) return;
        if (channels.length === 0) {
          channels = liveChannels;
        }
      } catch (searchError) {
        console.error("Fallback workspace channel search failed:", searchError);
      }
    }

    if (workspaceLoadSeqRef.current !== loadSeq) return;
    applyWorkspaceSuggestions(people, channels);
    setWorkspaceLoadPhase("ready");

    if (cacheFailed) {
      setWorkspaceLoadError("Workspace preload was partial. You can still search manually, or retry.");
    } else {
      setWorkspaceLoadError(null);
    }
  }, [applyWorkspaceSuggestions, slackCookie, slackToken]);

  useEffect(() => {
    if (step !== 2) return;
    if (workspaceLoadPhase !== "idle") return;
    if (!connectionInfo) return;
    void loadWorkspaceData();
  }, [step, workspaceLoadPhase, connectionInfo, loadWorkspaceData]);

  useEffect(() => {
    if (step === 2) return;
    workspaceLoadSeqRef.current += 1;
    setWorkspaceLoadPhase((prev) =>
      prev === "loading_cache" || prev === "loading_suggestions" ? "idle" : prev
    );
  }, [step]);

  useEffect(() => {
    if (step !== 3 || aiProvider !== "codex") return;
    setLoadingCodexStatus(true);
    getCodexStatus()
      .then((status) => setCodexStatus(status))
      .catch((e) => {
        console.error("Failed to load Codex status:", e);
        setCodexStatus({
          installed: false,
          authenticated: false,
          auth_mode: null,
          has_codex_subscription: false,
          message: "Could not read Codex status",
        });
      })
      .finally(() => setLoadingCodexStatus(false));
  }, [step, aiProvider]);

  // Keyboard handler
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Enter" && !(e.target instanceof HTMLInputElement)) {
        e.preventDefault();
        if (step === 0) setStep(1);
        else if (step === 4) handleFinish();
      }
      if (e.key === "Escape" && !(e.target instanceof HTMLInputElement)) {
        handleSkipAll();
      }
    };
    document.addEventListener("keydown", handleKeyDown);
    return () => document.removeEventListener("keydown", handleKeyDown);
  }, [step]);

  const handleTestConnection = async () => {
    setTesting(true);
    setError(null);
    setWorkspaceLoadError(null);
    setWorkspaceLoadPhase("idle");

    try {
      // Save credentials first
      const currentSettings = await getSettings();
      await saveSettings({
        ...currentSettings,
        slack_token: slackToken || null,
        slack_cookie: slackCookie || null,
      });

      // Test the connection
      const info = await testSlackConnection(slackToken, slackCookie);
      setConnectionInfo(info);

      // Start workspace cache + suggestions load in background
      void loadWorkspaceData();

      // Auto-advance after showing success
      autoAdvanceRef.current = setTimeout(() => setStep(2), 1200);
    } catch (e) {
      setError("Couldn't connect to Slack. Please check your token and cookie.");
      console.error("Connection test failed:", e);
    } finally {
      setTesting(false);
    }
  };

  const handleSkipCredentials = async () => {
    // Skip Slack setup and continue onboarding
    setStep(3);
  };

  const handleSkipAll = () => {
    onComplete();
  };

  const handleFiltersNext = async () => {
    setSaving(true);
    try {
      const currentSettings = await getSettings();
      await saveSettings({
        ...currentSettings,
        slack_filters: filters.length > 0 ? filters : null,
      });
    } catch (e) {
      console.error("Failed to save filters:", e);
    } finally {
      setSaving(false);
      setStep(3);
    }
  };

  const handleAiNext = async () => {
    setSavingAi(true);
    try {
      const currentSettings = await getSettings();
      await saveSettings({
        ...currentSettings,
        ai_provider: aiProvider || null,
        claude_api_key: claudeApiKey || null,
        openai_api_key: openaiApiKey || null,
      });
      setSavedAiProvider(aiProvider);
    } catch (e) {
      console.error("Failed to save AI settings:", e);
    } finally {
      setSavingAi(false);
      setStep(4);
    }
  };

  const handleFinish = async () => {
    onComplete();
  };

  const addFilter = (item: TypeaheadItem) => {
    if (filters.some((f) => f.id === item.id)) return;
    setFilters([...filters, {
      filter_type: item.type,
      id: item.id,
      display_name: item.label,
    }]);
  };

  const removeFilter = (id: string) => {
    setFilters(filters.filter((f) => f.id !== id));
  };

  const toggleChannel = (channel: SlackChannel) => {
    const existing = filters.find((f) => f.id === channel.id);
    if (existing) {
      removeFilter(channel.id);
    } else {
      setFilters([...filters, {
        filter_type: "channel",
        id: channel.id,
        display_name: channel.name,
      }]);
    }
  };

  const togglePerson = (user: SlackUser) => {
    const existing = filters.find((f) => f.id === user.id);
    if (existing) {
      removeFilter(user.id);
    } else {
      setFilters([...filters, {
        filter_type: "user",
        id: user.id,
        display_name: user.real_name || user.name,
      }]);
    }
  };

  const tokenValid = slackToken.startsWith("xoxc-");
  const cookieValid = slackCookie.startsWith("xoxd-");
  const canConnect = slackToken.length > 0 && slackCookie.length > 0;
  const workspaceIsLoading =
    workspaceLoadPhase === "loading_cache" || workspaceLoadPhase === "loading_suggestions";
  const workspaceLoadElapsedSeconds =
    workspaceLoadPhase === "loading_cache" ? cacheLoadElapsedSeconds : suggestionsLoadElapsedSeconds;
  const workspaceLoadIsSlow =
    workspaceLoadPhase === "loading_cache" ? cacheLoadIsSlow : suggestionsLoadIsSlow;

  // Cleanup auto-advance timer
  useEffect(() => {
    return () => {
      workspaceLoadSeqRef.current += 1;
      if (autoAdvanceRef.current) clearTimeout(autoAdvanceRef.current);
    };
  }, []);

  return (
    <div className="onboarding-overlay">
      <div className="onboarding-dialog">
        {/* Step indicator */}
        <div className="onboarding-steps">
          {[0, 1, 2, 3, 4].map((s) => (
            <div key={s} className={`onboarding-dot ${s === step ? "active" : s < step ? "completed" : ""}`} />
          ))}
        </div>

        {/* Step 0: Welcome */}
        {step === 0 && (
          <div className="onboarding-content">
            <div className="onboarding-welcome-icon">
              <img src="/icon.png" alt="Dispatch" width="64" height="64" />
            </div>
            <h1 className="onboarding-title">Welcome to Dispatch</h1>
            <p className="onboarding-subtitle">A focused inbox for the messages that matter.</p>
            <div className="onboarding-footer onboarding-footer-center">
              <button className="dialog-save" onClick={() => setStep(1)}>
                Get Started
              </button>
            </div>
          </div>
        )}

        {/* Step 1: Connect to Slack */}
        {step === 1 && (
          <div className="onboarding-content">
            <h1 className="onboarding-title">Connect to Slack</h1>
            <p className="onboarding-subtitle">
              Dispatch needs a session token and cookie from your Slack workspace.
            </p>

            <ol className="onboarding-instructions">
              <li>
                Open <a href="https://app.slack.com" target="_blank" rel="noopener noreferrer">app.slack.com</a> in your browser
              </li>
              <li>
                Press <kbd>Cmd+Option+I</kbd> to open DevTools, then click the <strong>Console</strong> tab
              </li>
              <li>
                Paste this command to find your <strong>token</strong>:
                <CopyBlock code={TOKEN_CMD} />
              </li>
              <li>
                <div className="onboarding-input-row">
                  <input
                    type="password"
                    className="settings-input"
                    value={slackToken}
                    onChange={(e) => {
                      workspaceLoadSeqRef.current += 1;
                      setSlackToken(e.target.value);
                      setError(null);
                      setConnectionInfo(null);
                      setSuggestedPeople([]);
                      setSuggestedChannels([]);
                      setWorkspaceLoadError(null);
                      setWorkspaceLoadPhase("idle");
                    }}
                    placeholder="xoxc-..."
                    autoFocus
                  />
                  {slackToken.length > 0 && (
                    <span className={`onboarding-input-status ${tokenValid ? "valid" : "invalid"}`}>
                      {tokenValid ? "\u2713" : "\u2717"}
                    </span>
                  )}
                </div>
              </li>
              <li>
                Now paste this command for your <strong>cookie</strong>:
                <CopyBlock code={COOKIE_CMD} />
              </li>
              <li>
                <div className="onboarding-input-row">
                  <input
                    type="password"
                    className="settings-input"
                    value={slackCookie}
                    onChange={(e) => {
                      workspaceLoadSeqRef.current += 1;
                      setSlackCookie(e.target.value);
                      setError(null);
                      setConnectionInfo(null);
                      setSuggestedPeople([]);
                      setSuggestedChannels([]);
                      setWorkspaceLoadError(null);
                      setWorkspaceLoadPhase("idle");
                    }}
                    placeholder="xoxd-..."
                  />
                  {slackCookie.length > 0 && (
                    <span className={`onboarding-input-status ${cookieValid ? "valid" : "invalid"}`}>
                      {cookieValid ? "\u2713" : "\u2717"}
                    </span>
                  )}
                </div>
              </li>
            </ol>

            {connectionInfo && (
              <div className="onboarding-success">
                Connected to <strong>{connectionInfo.team}</strong> as {connectionInfo.user}
              </div>
            )}

            {error && (
              <div className="onboarding-error">{error}</div>
            )}

            <div className="onboarding-footer">
              <button className="onboarding-skip" onClick={handleSkipCredentials}>
                I'll set this up later
              </button>
              <button
                className="dialog-save"
                disabled={!canConnect || testing || !!connectionInfo}
                onClick={handleTestConnection}
              >
                {testing ? "Connecting..." : connectionInfo ? "Connected" : "Connect"}
              </button>
            </div>
          </div>
        )}

        {/* Step 2: Choose what to follow */}
        {step === 2 && (
          <div className="onboarding-content">
            <h1 className="onboarding-title">What should Dispatch watch?</h1>
            <p className="onboarding-subtitle">
              Messages where you're @mentioned or DMed are always included. Add people and channels to also keep tabs on.
            </p>

            <div className="onboarding-filters-section">
              <div className="filter-chips">
                <span className="filter-chip filter-chip-auto">to:me</span>
                {filters.map((f) => (
                  <span key={f.id} className="filter-chip">
                    {f.filter_type === "user" ? "@" : f.filter_type === "to" ? "to:" : "#"}
                    {f.display_name.replace(/^#/, "")}
                    <button className="chip-remove" onClick={() => removeFilter(f.id)}>
                      ×
                    </button>
                  </span>
                ))}
              </div>

              {suggestedPeople.length > 0 && (
                <div className="onboarding-suggested">
                  <div className="onboarding-suggested-label">Suggested people:</div>
                  <div className="onboarding-channel-grid">
                    {suggestedPeople.map((user) => {
                      const isSelected = filters.some((f) => f.id === user.id);
                      return (
                        <button
                          key={user.id}
                          className={`onboarding-channel-chip ${isSelected ? "selected" : ""}`}
                          onClick={() => togglePerson(user)}
                        >
                          @ {user.real_name || user.name}
                        </button>
                      );
                    })}
                  </div>
                </div>
              )}

              {suggestedChannels.length > 0 && (
                <div className="onboarding-suggested">
                  <div className="onboarding-suggested-label">Suggested channels:</div>
                  <div className="onboarding-channel-grid">
                    {suggestedChannels.map((ch) => {
                      const isSelected = filters.some((f) => f.id === ch.id);
                      return (
                        <button
                          key={ch.id}
                          className={`onboarding-channel-chip ${isSelected ? "selected" : ""}`}
                          onClick={() => toggleChannel(ch)}
                        >
                          # {ch.name}
                        </button>
                      );
                    })}
                  </div>
                </div>
              )}

              <div className="onboarding-loading-row" aria-live="polite">
                {(workspaceLoadPhase === "loading_cache" || workspaceLoadPhase === "loading_suggestions") && (
                  <span className="onboarding-loading">
                    {workspaceLoadPhase === "loading_cache"
                      ? `Loading your workspace... ${workspaceLoadElapsedSeconds}s`
                      : `Loading suggestions... ${workspaceLoadElapsedSeconds}s`}
                    {workspaceLoadIsSlow ? " · slow" : ""}
                  </span>
                )}
                {workspaceLoadPhase === "error" && (
                  <span className="onboarding-loading onboarding-loading-error">
                    {workspaceLoadError || "Couldn't load your workspace data."}
                  </span>
                )}
                {workspaceLoadPhase !== "error" && !workspaceIsLoading && workspaceLoadError && (
                  <span className="onboarding-loading onboarding-loading-note">
                    {workspaceLoadError}
                  </span>
                )}
                {(workspaceLoadPhase === "error" || workspaceLoadError || (workspaceIsLoading && workspaceLoadIsSlow)) && (
                  <button
                    type="button"
                    className="onboarding-loading-action"
                    onClick={() => {
                      setWorkspaceLoadError(null);
                      void loadWorkspaceData();
                    }}
                  >
                    Retry workspace load
                  </button>
                )}
                {!workspaceIsLoading && workspaceLoadPhase !== "error" && !workspaceLoadError && (
                  <span className="onboarding-loading-placeholder">&nbsp;</span>
                )}
              </div>

              <div className="onboarding-search-section">
                <TypeaheadInput
                  placeholder="Search @people or #channels..."
                  onSelect={addFilter}
                />
              </div>
            </div>

            <div className="onboarding-footer">
              <button className="dialog-cancel" onClick={() => setStep(1)}>Back</button>
              <button className="dialog-save" onClick={handleFiltersNext} disabled={saving}>
                {saving ? "Saving..." : "Continue"}
              </button>
            </div>
          </div>
        )}

        {/* Step 3: Optional AI setup */}
        {step === 3 && (
          <div className="onboarding-content">
            <h1 className="onboarding-title">AI classification (optional)</h1>
            <p className="onboarding-subtitle">
              Pick how Dispatch should classify messages beyond your rules.
            </p>

            <div className="onboarding-summary" style={{ marginBottom: 12 }}>
              {([
                { key: "codex", label: "Codex (ChatGPT/Codex subscription)" },
                { key: "openai", label: "OpenAI API key" },
                { key: "claude", label: "Claude API key" },
                { key: "", label: "Not now" },
              ] as const).map((opt) => (
                <label key={opt.key || "none"} className="settings-checkbox-label">
                  <input
                    type="radio"
                    name="onboarding_ai_provider"
                    checked={aiProvider === opt.key}
                    onChange={() => setAiProvider(opt.key)}
                  />
                  {opt.label}
                </label>
              ))}
            </div>

            {aiProvider === "claude" && (
              <div className="onboarding-input-row" style={{ marginBottom: 12 }}>
                <input
                  type="password"
                  className="settings-input"
                  value={claudeApiKey}
                  onChange={(e) => setClaudeApiKey(e.target.value)}
                  placeholder="Claude API key (sk-ant-...)"
                />
              </div>
            )}

            {aiProvider === "openai" && (
              <div className="onboarding-input-row" style={{ marginBottom: 12 }}>
                <input
                  type="password"
                  className="settings-input"
                  value={openaiApiKey}
                  onChange={(e) => setOpenaiApiKey(e.target.value)}
                  placeholder="OpenAI API key (sk-...)"
                />
              </div>
            )}

            {aiProvider === "codex" && (
              <div className="onboarding-summary" style={{ marginBottom: 12 }}>
                <div className="onboarding-summary-item">
                  <span className={`onboarding-summary-status ${codexStatus?.authenticated ? "good" : "skip"}`}>
                    {codexStatus?.authenticated ? "\u2713" : "\u2013"}
                  </span>
                  <span>
                    {loadingCodexStatus
                      ? "Checking Codex status..."
                      : codexStatus
                        ? codexStatus.message
                        : "Status unavailable"}
                  </span>
                </div>
                <div className="onboarding-summary-item">
                  <span className="onboarding-summary-status">{"\u00b7"}</span>
                  <span>
                    Mode: {codexStatus?.auth_mode || "unknown"} · Subscription:{" "}
                    {codexStatus?.has_codex_subscription ? "yes" : "no"}
                  </span>
                </div>
                <div className="onboarding-footer onboarding-footer-center" style={{ marginTop: 8 }}>
                  <button
                    className="dialog-cancel"
                    onClick={() => {
                      setLoadingCodexStatus(true);
                      getCodexStatus()
                        .then((status) => setCodexStatus(status))
                        .catch((e) => {
                          console.error("Failed to load Codex status:", e);
                        })
                        .finally(() => setLoadingCodexStatus(false));
                    }}
                    disabled={loadingCodexStatus}
                  >
                    {loadingCodexStatus ? "Checking..." : "Refresh Codex Status"}
                  </button>
                </div>
              </div>
            )}

            <div className="onboarding-footer">
              <button className="dialog-cancel" onClick={() => setStep(2)}>
                Back
              </button>
              <button className="onboarding-skip" onClick={() => setStep(4)}>
                Skip for now
              </button>
              <button className="dialog-save" onClick={handleAiNext} disabled={savingAi}>
                {savingAi ? "Saving..." : "Continue"}
              </button>
            </div>
          </div>
        )}

        {/* Step 4: All Set */}
        {step === 4 && (
          <div className="onboarding-content">
            <div className="onboarding-welcome-icon">
              <span className="onboarding-check-icon">{"\u2713"}</span>
            </div>
            <h1 className="onboarding-title">You're ready to go</h1>

            <div className="onboarding-summary">
              <div className="onboarding-summary-item">
                <span className={`onboarding-summary-status ${connectionInfo ? "good" : "skip"}`}>
                  {connectionInfo ? "\u2713" : "\u2013"}
                </span>
                <span>
                  {connectionInfo
                    ? <>Connected to <strong>{connectionInfo.team}</strong></>
                    : "Slack: not configured yet"
                  }
                </span>
              </div>
              <div className="onboarding-summary-item">
                <span className="onboarding-summary-status good">{"\u2713"}</span>
                <span>Watching {filters.length + 1} source{filters.length !== 0 ? "s" : ""} (including to:me)</span>
              </div>
              <div className="onboarding-summary-item">
                <span className={`onboarding-summary-status ${savedAiProvider ? "good" : "skip"}`}>
                  {savedAiProvider ? "\u2713" : "\u2013"}
                </span>
                <span>
                  {savedAiProvider
                    ? `AI provider: ${savedAiProvider === "codex" ? "Codex" : savedAiProvider === "openai" ? "OpenAI" : "Claude"}`
                    : "AI provider: not configured yet"}
                </span>
              </div>
            </div>

            <p className="onboarding-hint">
              You can change these anytime in Settings (<kbd>Cmd+,</kbd>)
            </p>

            <div className="onboarding-footer onboarding-footer-center">
              <button className="dialog-save" onClick={handleFinish}>
                Open Inbox
              </button>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
