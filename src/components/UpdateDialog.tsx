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
      <div className="dialog update-dialog" onClick={(e) => e.stopPropagation()}>
        <div className="dialog-title">Software Update</div>
        <div className="update-body">
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
            <>
              <p>Update v{status.version} installed. Restart Dispatch to apply.</p>
            </>
          )}
          {status.state === "error" && (
            <>
              <p>Could not check for updates.</p>
              <p className="update-error-detail">{status.message}</p>
            </>
          )}
        </div>
        <div className="dialog-footer">
          {status.state === "installed" ? (
            <>
              <button className="dialog-cancel" onClick={onClose}>Later</button>
              <button className="dialog-save" onClick={() => relaunch()}>Restart</button>
            </>
          ) : (
            <button className="dialog-cancel" onClick={onClose}>
              {status.state === "checking" || status.state === "downloading" ? "Dismiss" : "Close"}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
