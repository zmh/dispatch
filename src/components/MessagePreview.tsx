import DOMPurify from "dompurify";
import { Message } from "../lib/tauri";

interface MessagePreviewProps {
  message: Message | undefined;
  onOpenLink: (permalink: string) => void;
}

function formatRelativeTime(timestamp: number): string {
  const now = Math.floor(Date.now() / 1000);
  const diff = now - timestamp;

  if (diff < 60) return "just now";
  if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
  if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`;
  if (diff < 604800) return `${Math.floor(diff / 86400)}d ago`;
  return `${Math.floor(diff / 604800)}w ago`;
}

function getInitials(name: string): string {
  const parts = name.trim().split(/\s+/);
  if (parts.length >= 2) {
    return (parts[0][0] + parts[parts.length - 1][0]).toUpperCase();
  }
  return name.slice(0, 2).toUpperCase();
}

export function MessagePreview({ message, onOpenLink }: MessagePreviewProps) {
  if (!message) {
    return (
      <div className="preview-panel preview-empty">
        <span className="preview-empty-text">No message selected</span>
      </div>
    );
  }

  const source = message.subject
    ? `#${message.subject}`
    : message.source;

  return (
    <div className="preview-panel">
      <div className="preview-header">
        <div className="preview-header-row">
          {message.avatar_url ? (
            <img className="preview-avatar-img" src={message.avatar_url} alt="" />
          ) : (
            <span className="preview-avatar">{getInitials(message.sender)}</span>
          )}
          <div className="preview-sender-info">
            <span
              className="preview-sender-name"
              onClick={() => message.permalink && onOpenLink(message.permalink)}
              title="Open in Slack"
            >
              {message.sender}
            </span>
            <span
              className="preview-source"
              onClick={() => message.permalink && onOpenLink(message.permalink)}
              title="Open in Slack"
            >
              {source}
            </span>
          </div>
          <span className="preview-star">{message.starred ? "★" : ""}</span>
          <span className="preview-time">{formatRelativeTime(message.timestamp)}</span>
          {message.permalink && (
            <button
              className="preview-open-icon"
              onClick={() => onOpenLink(message.permalink!)}
              title="Open in Slack"
            >
              <svg width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
                <path d="M11 8v3a1 1 0 0 1-1 1H3a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1h3" />
                <path d="M8 2h4v4" />
                <path d="M12 2L6 8" />
              </svg>
            </button>
          )}
        </div>
      </div>

      <div className="preview-body">
        {message.body_html ? (
          <div
            className="preview-html"
            dangerouslySetInnerHTML={{ __html: DOMPurify.sanitize(message.body_html) }}
          />
        ) : (
          <pre className="preview-text">{message.body}</pre>
        )}
      </div>
    </div>
  );
}
