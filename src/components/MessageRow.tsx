import { Message } from "../lib/tauri";

interface MessageRowProps {
  message: Message;
  selected: boolean;
  checked: boolean;
  onClick: () => void;
  onDoubleClick: () => void;
}

function formatRelativeTime(timestamp: number): string {
  const now = Math.floor(Date.now() / 1000);
  const diff = now - timestamp;

  if (diff < 60) return "now";
  if (diff < 3600) return `${Math.floor(diff / 60)}m`;
  if (diff < 86400) return `${Math.floor(diff / 3600)}h`;
  if (diff < 604800) return `${Math.floor(diff / 86400)}d`;
  return `${Math.floor(diff / 604800)}w`;
}

function truncateText(text: string, maxLen: number): string {
  if (text.length <= maxLen) return text;
  return text.slice(0, maxLen) + "…";
}

export function MessageRow({ message, selected, checked, onClick, onDoubleClick }: MessageRowProps) {
  const source = message.subject
    ? `#${message.subject}`
    : message.source;

  const className = [
    "message-row",
    selected ? "selected" : "",
    checked ? "checked" : "",
  ].filter(Boolean).join(" ");

  return (
    <div
      className={className}
      onClick={onClick}
      onDoubleClick={onDoubleClick}
    >
      <span className="msg-check">{checked ? "●" : ""}</span>
      <span className="msg-star">{message.starred ? "★" : " "}</span>
      <span className="msg-source">{source}</span>
      <span className="msg-dot">·</span>
      <span className="msg-sender">{message.sender}</span>
      <span className="msg-body">{truncateText(message.body, 80)}</span>
      <span className="msg-time">{formatRelativeTime(message.timestamp)}</span>
    </div>
  );
}
