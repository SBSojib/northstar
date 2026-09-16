import { open } from "@tauri-apps/plugin-dialog";
import { FileKey, X } from "lucide-react";
import { useState } from "react";
import type { AuthType, Group, Host, HostInput } from "../types";

interface HostDialogProps {
  host?: Host;
  groups: Group[];
  defaultGroupId: number | null;
  busy: boolean;
  onSave: (input: HostInput) => void;
  onDelete?: () => void;
  onClose: () => void;
}

export function HostDialog({
  host,
  groups,
  defaultGroupId,
  busy,
  onSave,
  onDelete,
  onClose,
}: HostDialogProps) {
  const [name, setName] = useState(host?.name ?? "");
  const [address, setAddress] = useState(host?.address ?? "");
  const [port, setPort] = useState(host?.port ?? 22);
  const [username, setUsername] = useState(host?.username ?? "");
  const [groupId, setGroupId] = useState<number | null>(host?.groupId ?? defaultGroupId);
  const [authType, setAuthType] = useState<AuthType>(host?.authType ?? "password");
  const [password, setPassword] = useState("");
  const [privateKeyPath, setPrivateKeyPath] = useState("");
  const [privateKeyPassphrase, setPrivateKeyPassphrase] = useState("");

  const chooseKey = async () => {
    const selected = await open({
      multiple: false,
      directory: false,
    });
    if (selected) setPrivateKeyPath(selected);
  };

  return (
    <div className="dialog-backdrop" onMouseDown={onClose}>
      <form
        className="dialog"
        onMouseDown={(event) => event.stopPropagation()}
        onSubmit={(event) => {
          event.preventDefault();
          onSave({
            id: host?.id,
            name,
            address,
            port,
            username,
            groupId,
            authType,
            password: password || undefined,
            privateKeyPath: privateKeyPath || undefined,
            privateKeyPassphrase: privateKeyPassphrase || undefined,
          });
        }}
      >
        <header className="dialog-header">
          <div>
            <h2>{host ? "Edit host" : "New SSH host"}</h2>
            <p>Connection details stay on this device.</p>
          </div>
          <button type="button" className="icon-button" onClick={onClose}>
            <X size={19} />
          </button>
        </header>

        <div className="form-body">
          <div className="form-grid">
            <label className="field full">
              <span>Label</span>
              <input
                autoFocus
                value={name}
                onChange={(event) => setName(event.target.value)}
                placeholder="Production server"
              />
            </label>
            <label className="field address-field">
              <span>Address</span>
              <input
                value={address}
                onChange={(event) => setAddress(event.target.value)}
                placeholder="192.168.1.10 or example.com"
              />
            </label>
            <label className="field port-field">
              <span>Port</span>
              <input
                type="number"
                min={1}
                max={65535}
                value={port}
                onChange={(event) => setPort(Number(event.target.value))}
              />
            </label>
            <label className="field full">
              <span>Username</span>
              <input
                value={username}
                onChange={(event) => setUsername(event.target.value)}
                placeholder="ubuntu"
              />
            </label>
            <label className="field full">
              <span>Group</span>
              <select
                value={groupId ?? ""}
                onChange={(event) =>
                  setGroupId(event.target.value ? Number(event.target.value) : null)
                }
              >
                <option value="">No group</option>
                {groups.map((group) => (
                  <option key={group.id} value={group.id}>
                    {group.name}
                  </option>
                ))}
              </select>
            </label>
          </div>

          <div className="form-section-title">Authentication</div>
          <div className="segmented">
            <button
              type="button"
              className={authType === "password" ? "active" : ""}
              onClick={() => setAuthType("password")}
            >
              Password
            </button>
            <button
              type="button"
              className={authType === "private_key" ? "active" : ""}
              onClick={() => setAuthType("private_key")}
            >
              Private key
            </button>
          </div>

          {authType === "password" ? (
            <label className="field">
              <span>Password</span>
              <input
                type="password"
                value={password}
                onChange={(event) => setPassword(event.target.value)}
                placeholder={host?.authType === "password" ? "Leave blank to keep saved password" : ""}
              />
            </label>
          ) : (
            <>
              <label className="field">
                <span>Private key file</span>
                <div className="file-picker">
                  <input
                    readOnly
                    value={privateKeyPath}
                    placeholder={
                      host?.authType === "private_key"
                        ? "Leave blank to keep saved key"
                        : "Choose a .pem or private key file"
                    }
                  />
                  <button type="button" className="secondary-button" onClick={chooseKey}>
                    <FileKey size={15} /> Browse
                  </button>
                </div>
              </label>
              <label className="field">
                <span>Key passphrase <em>optional</em></span>
                <input
                  type="password"
                  value={privateKeyPassphrase}
                  onChange={(event) => setPrivateKeyPassphrase(event.target.value)}
                />
              </label>
            </>
          )}
          <p className="security-note">
            The credential is encrypted before it enters SQLite. Private key files are imported;
            the original path is not saved.
          </p>
        </div>

        <footer className="dialog-footer">
          {onDelete && (
            <button type="button" className="danger-button" onClick={onDelete} disabled={busy}>
              Delete
            </button>
          )}
          <span className="footer-spacer" />
          <button type="button" className="secondary-button" onClick={onClose}>
            Cancel
          </button>
          <button
            className="primary-button"
            disabled={busy || !name.trim() || !address.trim() || !username.trim()}
          >
            {busy ? "Saving…" : "Save host"}
          </button>
        </footer>
      </form>
    </div>
  );
}
