import { useState, useEffect, useRef } from "react";
import {
  Settings,
  SlackFilter,
  SlackChannel,
  SlackConnectionInfo,
  getSettings,
  saveSettings,
  testSlackConnection,
  populateSlackCache,
  searchSlackChannels,
} from "../lib/tauri";
import { TypeaheadInput, TypeaheadItem } from "./TypeaheadInput";

interface OnboardingWizardProps {
  onComplete: () => void;
  initialSettings?: Settings;
}

const TOKEN_CMD = `Object.entries(JSON.parse(localStorage.localConfig_v2).teams).forEach(([,t])=>console.log(t.name,t.token))`;
const COOKIE_CMD = `document.cookie.split("; ").find(c=>c.startsWith("d=")).slice(2)`;

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
  const [step, setStep] = useState(0);
  const [slackToken, setSlackToken] = useState(initialSettings?.slack_token || "");
  const [slackCookie, setSlackCookie] = useState(initialSettings?.slack_cookie || "");
  const [filters, setFilters] = useState<SlackFilter[]>(initialSettings?.slack_filters || []);
  const [connectionInfo, setConnectionInfo] = useState<SlackConnectionInfo | null>(null);
  const [suggestedChannels, setSuggestedChannels] = useState<SlackChannel[]>([]);
  const [cacheReady, setCacheReady] = useState(false);
  const [testing, setTesting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const autoAdvanceRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Load suggested channels when cache is ready and we're on step 2
  useEffect(() => {
    if (step === 2 && cacheReady) {
      // Search for common channels — empty-ish query to get popular ones
      searchSlackChannels("").then((channels) => {
        setSuggestedChannels(channels.slice(0, 15));
      }).catch(() => {});
    }
  }, [step, cacheReady]);

  // Keyboard handler
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Enter" && !(e.target instanceof HTMLInputElement)) {
        e.preventDefault();
        if (step === 0) setStep(1);
        else if (step === 3) handleFinish();
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

      // Populate cache in background
      populateSlackCache().then(() => {
        setCacheReady(true);
      }).catch(console.error);

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
    // Jump to final step
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

  const tokenValid = slackToken.startsWith("xoxc-");
  const cookieValid = slackCookie.startsWith("xoxd-");
  const canConnect = slackToken.length > 0 && slackCookie.length > 0;

  // Cleanup auto-advance timer
  useEffect(() => {
    return () => {
      if (autoAdvanceRef.current) clearTimeout(autoAdvanceRef.current);
    };
  }, []);

  return (
    <div className="onboarding-overlay">
      <div className="onboarding-dialog">
        {/* Step indicator */}
        <div className="onboarding-steps">
          {[0, 1, 2, 3].map((s) => (
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
                    onChange={(e) => { setSlackToken(e.target.value); setError(null); setConnectionInfo(null); }}
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
                    onChange={(e) => { setSlackCookie(e.target.value); setError(null); setConnectionInfo(null); }}
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

              {suggestedChannels.length > 0 && (
                <div className="onboarding-suggested">
                  <div className="onboarding-suggested-label">Your channels:</div>
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

              {!cacheReady && connectionInfo && (
                <div className="onboarding-loading">Loading your channels...</div>
              )}

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

        {/* Step 3: All Set */}
        {step === 3 && (
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
