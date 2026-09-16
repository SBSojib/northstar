import { useState } from "react";
import { X } from "lucide-react";
import type { Group, GroupInput } from "../types";

interface GroupDialogProps {
  group?: Group;
  groups: Group[];
  defaultParentId: number | null;
  busy: boolean;
  onSave: (input: GroupInput) => void;
  onDelete?: () => void;
  onClose: () => void;
}

export function GroupDialog({
  group,
  groups,
  defaultParentId,
  busy,
  onSave,
  onDelete,
  onClose,
}: GroupDialogProps) {
  const [name, setName] = useState(group?.name ?? "");
  const [parentId, setParentId] = useState<number | null>(
    group?.parentId ?? defaultParentId,
  );

  return (
    <div className="dialog-backdrop" onMouseDown={onClose}>
      <form
        className="dialog small-dialog"
        onMouseDown={(event) => event.stopPropagation()}
        onSubmit={(event) => {
          event.preventDefault();
          onSave({ id: group?.id, name, parentId });
        }}
      >
        <header className="dialog-header">
          <div>
            <h2>{group ? "Edit group" : "New group"}</h2>
            <p>Organize hosts with nested folders.</p>
          </div>
          <button type="button" className="icon-button" onClick={onClose}>
            <X size={19} />
          </button>
        </header>

        <div className="form-body">
          <label className="field">
            <span>Name</span>
            <input autoFocus value={name} onChange={(event) => setName(event.target.value)} />
          </label>
          <label className="field">
            <span>Parent group</span>
            <select
              value={parentId ?? ""}
              onChange={(event) =>
                setParentId(event.target.value ? Number(event.target.value) : null)
              }
            >
              <option value="">None</option>
              {groups
                .filter((candidate) => candidate.id !== group?.id)
                .map((candidate) => (
                  <option key={candidate.id} value={candidate.id}>
                    {candidate.name}
                  </option>
                ))}
            </select>
          </label>
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
          <button className="primary-button" disabled={busy || !name.trim()}>
            {busy ? "Saving…" : "Save group"}
          </button>
        </footer>
      </form>
    </div>
  );
}
