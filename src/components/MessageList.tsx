import { useEffect, useRef } from "react";
import { Message } from "../lib/tauri";
import { MessageRow } from "./MessageRow";

interface MessageListProps {
  messages: Message[];
  selectedIndex: number;
  selectedIds: Set<string>;
  loading: boolean;
  onSelect: (index: number) => void;
  onOpen: (url: string) => void;
  style?: React.CSSProperties;
}

export function MessageList({
  messages,
  selectedIndex,
  selectedIds,
  loading,
  onSelect,
  onOpen,
  style,
}: MessageListProps) {
  const listRef = useRef<HTMLDivElement>(null);

  // Scroll selected row into view
  useEffect(() => {
    if (listRef.current) {
      const selectedEl = listRef.current.querySelector(".message-row.selected");
      if (selectedEl) {
        selectedEl.scrollIntoView({ block: "nearest" });
      }
    }
  }, [selectedIndex]);

  if (loading && messages.length === 0) {
    return <div className="message-list-empty" style={style}>Loading...</div>;
  }

  if (messages.length === 0) {
    return (
      <div className="message-list-empty" style={style}>
        <div className="empty-icon">
          <svg width="36" height="36" viewBox="0 0 24 24" fill="currentColor">
            <circle cx="12" cy="12" r="11" />
            <path d="M7.5 12.5l3 3 6-6" fill="none" stroke="var(--bg)" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round" />
          </svg>
        </div>
        <div>No messages. You're at inbox zero!</div>
        <div className="empty-hint">Press r to refresh</div>
      </div>
    );
  }

  return (
    <div className="message-list" ref={listRef} style={style}>
      {messages.map((msg, i) => (
        <MessageRow
          key={msg.id}
          message={msg}
          selected={i === selectedIndex}
          checked={selectedIds.has(msg.id)}
          onClick={() => onSelect(i)}
          onDoubleClick={() => msg.permalink && onOpen(msg.permalink)}
          onMouseEnter={() => onSelect(i)}
        />
      ))}
    </div>
  );
}
