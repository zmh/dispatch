import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getVersion } from "@tauri-apps/api/app";

interface AboutDialogProps {
  onClose: () => void;
}

export function AboutDialog({ onClose }: AboutDialogProps) {
  const [version, setVersion] = useState("");

  useEffect(() => {
    getVersion().then(setVersion);
  }, []);

  const handleLink = (url: string) => {
    invoke("open_link", { url });
  };

  return (
    <div className="dialog-overlay" onClick={onClose}>
      <div className="dialog about-dialog" onClick={(e) => e.stopPropagation()}>
        <div className="about-icon">
          <img src="/icon.png" alt="Haystack" width={64} height={64} />
        </div>
        <div className="about-name">Haystack</div>
        <div className="about-version">Version {version}</div>
        <div className="about-description">CEO Inbox — Aggregated message triage</div>
        <div className="about-links">
          <button className="about-link" onClick={() => handleLink("https://github.com/zmh/haystack")}>
            GitHub
          </button>
          <span className="about-link-sep">&middot;</span>
          <button className="about-link" onClick={() => handleLink("https://zmh.org")}>
            zmh.org
          </button>
        </div>
        <div className="about-copyright">&copy; 2025 Zachary Hamed. All rights reserved.</div>
        <div className="dialog-footer">
          <button className="dialog-cancel" onClick={onClose}>Close</button>
        </div>
      </div>
    </div>
  );
}
