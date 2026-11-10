import { ShieldAlert } from "lucide-react";
import { useState } from "react";

export function fingerprintOf(text: string): string | null {
  return text.match(/SHA256:[A-Za-z0-9+/]+={0,2}/)?.[0] ?? null;
}

interface HostPromptDialogProps {
  title: string;
  detail: string;
  message: string;
  secret: boolean;
  confirmLabel: string;
  busy: boolean;
  onConfirm: (answer: string) => void;
  onCancel: () => void;
}

export function HostPromptDialog({
  title,
  detail,
  message,
  secret,
  confirmLabel,
  busy,
  onConfirm,
  onCancel,
}: HostPromptDialogProps) {
  const [answer, setAnswer] = useState("");
  const fingerprint = fingerprintOf(message);
  const fixedAnswer = confirmLabel === "Trust this key";

  return (
    <div className="dialog-backdrop" onMouseDown={onCancel}>
      <form
        className="dialog"
        onMouseDown={(event) => event.stopPropagation()}
        onSubmit={(event) => {
          event.preventDefault();
          onConfirm(fixedAnswer ? "yes" : answer);
        }}
      >
        <header className="dialog-header">
          <div>
            <h2>{title}</h2>
            <p>{detail}</p>
          </div>
          <ShieldAlert size={18} />
        </header>
        <div className="form-body">
          {fingerprint && (
            <div className="field">
              <span>Fingerprint</span>
              <strong className="fingerprint-value">{fingerprint}</strong>
            </div>
          )}
          {message && <pre className="host-key-excerpt">{message}</pre>}
          {!fixedAnswer && (
            <label className="field">
              <span>Response</span>
              <input
                autoFocus
                type={secret ? "password" : "text"}
                value={answer}
                onChange={(event) => setAnswer(event.target.value)}
              />
            </label>
          )}
          <p className="security-note">
            {fixedAnswer
              ? "Trust this key only if the fingerprint matches the one given to you by the server’s administrator."
              : "This response is sent to OpenSSH for the current prompt and is not saved."}
          </p>
        </div>
        <footer className="dialog-footer">
          <span className="footer-spacer" />
          <button type="button" className="secondary-button" onClick={onCancel} disabled={busy}>
            {fixedAnswer ? "Reject" : "Cancel"}
          </button>
          <button className="primary-button" disabled={busy || (!fixedAnswer && !answer)}>
            {busy ? "Working…" : confirmLabel}
          </button>
        </footer>
      </form>
    </div>
  );
}

interface HostKeyChangedDialogProps {
  address: string;
  message: string;
  busy: boolean;
  onReplace: () => void;
  onClose: () => void;
}

export function HostKeyChangedDialog({
  address,
  message,
  busy,
  onReplace,
  onClose,
}: HostKeyChangedDialogProps) {
  const fingerprint = fingerprintOf(message);

  return (
    <div className="dialog-backdrop" onMouseDown={onClose}>
      <form
        className="dialog"
        onMouseDown={(event) => event.stopPropagation()}
        onSubmit={(event) => {
          event.preventDefault();
          onReplace();
        }}
      >
        <header className="dialog-header">
          <div>
            <h2>Host key changed</h2>
            <p>{address} presented a different key from the one that was saved.</p>
          </div>
          <ShieldAlert size={18} />
        </header>
        <div className="form-body">
          {fingerprint && (
            <div className="field">
              <span>New fingerprint</span>
              <strong className="fingerprint-value">{fingerprint}</strong>
            </div>
          )}
          {message && <pre className="host-key-excerpt">{message}</pre>}
          <p className="security-note">
            This can mean the server was rebuilt, or that the connection is being intercepted.
            Continue only if you expected this key to change. The saved key is removed from
            Northstar and from your OpenSSH known_hosts file.
          </p>
        </div>
        <footer className="dialog-footer">
          <span className="footer-spacer" />
          <button type="button" className="secondary-button" onClick={onClose} disabled={busy}>
            Keep the old key
          </button>
          <button className="primary-button" disabled={busy}>
            {busy ? "Working…" : "Replace saved key and reconnect"}
          </button>
        </footer>
      </form>
    </div>
  );
}
