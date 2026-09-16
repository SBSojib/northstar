import { Download, Upload, X } from "lucide-react";
import { useState } from "react";

interface BackupDialogProps {
  mode: "backup" | "restore";
  busy: boolean;
  onSubmit: (password: string) => void;
  onClose: () => void;
}

export function BackupDialog({ mode, busy, onSubmit, onClose }: BackupDialogProps) {
  const [password, setPassword] = useState("");
  const [confirmation, setConfirmation] = useState("");
  const isBackup = mode === "backup";
  const valid =
    password.length >= 8 && (!isBackup || (confirmation.length > 0 && password === confirmation));

  return (
    <div className="dialog-backdrop" onMouseDown={onClose}>
      <form
        className="dialog small-dialog"
        onMouseDown={(event) => event.stopPropagation()}
        onSubmit={(event) => {
          event.preventDefault();
          if (valid) onSubmit(password);
        }}
      >
        <header className="dialog-header">
          <div>
            <h2>{isBackup ? "Create portable backup" : "Restore backup"}</h2>
            <p>
              {isBackup
                ? "Protect hosts and credentials with a backup password."
                : "Imported hosts and groups will be added to your current library."}
            </p>
          </div>
          <button type="button" className="icon-button" onClick={onClose}>
            <X size={19} />
          </button>
        </header>
        <div className="form-body">
          <label className="field">
            <span>Backup password</span>
            <input
              autoFocus
              type="password"
              value={password}
              onChange={(event) => setPassword(event.target.value)}
              placeholder="At least 8 characters"
            />
          </label>
          {isBackup && (
            <label className="field">
              <span>Confirm password</span>
              <input
                type="password"
                value={confirmation}
                onChange={(event) => setConfirmation(event.target.value)}
              />
            </label>
          )}
          {isBackup && confirmation && password !== confirmation && (
            <p className="field-error">Passwords do not match.</p>
          )}
          <p className="security-note">
            Keep this password safe. The backup cannot be restored without it.
          </p>
        </div>
        <footer className="dialog-footer">
          <span className="footer-spacer" />
          <button type="button" className="secondary-button" onClick={onClose}>
            Cancel
          </button>
          <button className="primary-button" disabled={busy || !valid}>
            {busy ? (
              "Working…"
            ) : isBackup ? (
              <><Download size={15} /> Create backup</>
            ) : (
              <><Upload size={15} /> Restore</>
            )}
          </button>
        </footer>
      </form>
    </div>
  );
}
