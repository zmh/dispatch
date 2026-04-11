import { useEffect } from "react";
import { relaunch } from "@tauri-apps/plugin-process";

export type UpdateStatus =
  | { state: "checking" }
  | { state: "up-to-date"; version: string }
  | { state: "downloading"; version: string }
  | { state: "installed"; version: string }
  | { state: "error"; message: string };

interface UpdateDialogProps {
  status: UpdateStatus;
  onClose: () => void;
}

export function UpdateDialog({ status, onClose }: UpdateDialogProps) {
  // Auto-close "up to date" after 3 seconds
  useEffect(() => {
    if (status.state === "up-to-date") {
      const timer = setTimeout(onClose, 3000);
      return () => clearTimeout(timer);
    }
  }, [status.state, onClose]);

  return (
    <div className="dialog-overlay" onClick={onClose}>
      <div className="update-dialog-native" onClick={(e) => e.stopPropagation()}>
        <div className="settings-titlebar">
          <button className="settings-close" onClick={onClose} title="Close" />
          <span className="settings-titlebar-text">Software Update</span>
        </div>
        <div className="update-body-native">
          {status.state === "checking" && (
            <p>Checking for updates...</p>
          )}
          {status.state === "up-to-date" && (
            <p>You're up to date. (v{status.version})</p>
          )}
          {status.state === "downloading" && (
            <p>Downloading update v{status.version}...</p>
          )}
          {status.state === "installed" && (
            <p>Update v{status.version} installed. Restart Dispatch to apply.</p>
          )}
          {status.state === "error" && (
            <>
              <p>Could not check for updates.</p>
              <p className="update-error-detail">{status.message}</p>
            </>
          )}
        </div>
        {status.state === "installed" && (
          <div className="update-footer">
            <button className="dialog-cancel" onClick={onClose}>Later</button>
            <button className="dialog-save" onClick={() => relaunch()}>Restart</button>
          </div>
        )}
      </div>
    </div>
  );
}
