import { AlertCircle, X } from "lucide-react";
import { open, save } from "@tauri-apps/plugin-dialog";
import { useCallback, useEffect, useState } from "react";
import { api, errorMessage } from "./api";
import { BackupDialog } from "./components/BackupDialog";
import { GroupDialog } from "./components/GroupDialog";
import { HostDialog } from "./components/HostDialog";
import { Sidebar } from "./components/Sidebar";
import { TerminalWorkspace } from "./components/TerminalWorkspace";
import type { Group, GroupInput, Host, HostInput, Library, TerminalTab } from "./types";

type Dialog =
  | { type: "new-host" }
  | { type: "edit-host"; host: Host }
  | { type: "new-group" }
  | { type: "edit-group"; group: Group }
  | { type: "transfer"; mode: "backup" | "restore"; path: string }
  | null;

const EMPTY_LIBRARY: Library = { groups: [], hosts: [] };

export default function App() {
  const [library, setLibrary] = useState<Library>(EMPTY_LIBRARY);
  const [query, setQuery] = useState("");
  const [selectedGroupId, setSelectedGroupId] = useState<number | null>(null);
  const [tabs, setTabs] = useState<TerminalTab[]>([]);
  const [activeTabId, setActiveTabId] = useState<string | null>(null);
  const [dialog, setDialog] = useState<Dialog>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");

  const loadLibrary = useCallback(async (search: string) => {
    try {
      setLibrary(await api.getLibrary(search));
    } catch (loadError) {
      setError(errorMessage(loadError));
    }
  }, []);

  useEffect(() => {
    const timer = window.setTimeout(() => void loadLibrary(query), 180);
    return () => window.clearTimeout(timer);
  }, [loadLibrary, query]);

  const openHost = (host: Host) => {
    const tab: TerminalTab = { id: crypto.randomUUID(), host };
    setTabs((current) => [...current, tab]);
    setActiveTabId(tab.id);
  };

  const closeTab = (id: string) => {
    setTabs((current) => {
      const index = current.findIndex((tab) => tab.id === id);
      const next = current.filter((tab) => tab.id !== id);
      if (activeTabId === id) {
        setActiveTabId(next[Math.min(index, next.length - 1)]?.id ?? null);
      }
      return next;
    });
  };

  const saveHost = async (input: HostInput) => {
    setBusy(true);
    try {
      await api.saveHost(input);
      setDialog(null);
      await loadLibrary(query);
    } catch (saveError) {
      setError(errorMessage(saveError));
    } finally {
      setBusy(false);
    }
  };

  const saveGroup = async (input: GroupInput) => {
    setBusy(true);
    try {
      await api.saveGroup(input);
      setDialog(null);
      await loadLibrary(query);
    } catch (saveError) {
      setError(errorMessage(saveError));
    } finally {
      setBusy(false);
    }
  };

  const removeHost = async (host: Host) => {
    if (!window.confirm(`Delete “${host.name}”?`)) return;
    setBusy(true);
    try {
      await api.deleteHost(host.id);
      setDialog(null);
      await loadLibrary(query);
    } catch (deleteError) {
      setError(errorMessage(deleteError));
    } finally {
      setBusy(false);
    }
  };

  const removeGroup = async (group: Group) => {
    if (!window.confirm(`Delete “${group.name}” and its subgroups? Hosts will be kept ungrouped.`)) {
      return;
    }
    setBusy(true);
    try {
      await api.deleteGroup(group.id);
      if (selectedGroupId === group.id) setSelectedGroupId(null);
      setDialog(null);
      await loadLibrary(query);
    } catch (deleteError) {
      setError(errorMessage(deleteError));
    } finally {
      setBusy(false);
    }
  };

  const beginBackup = async () => {
    try {
      const path = await save({
        title: "Save Northstar backup",
        defaultPath: "northstar-backup.northstar",
        filters: [{ name: "Northstar backup", extensions: ["northstar"] }],
      });
      if (path) setDialog({ type: "transfer", mode: "backup", path });
    } catch (backupError) {
      setError(errorMessage(backupError));
    }
  };

  const beginRestore = async () => {
    try {
      const path = await open({
        title: "Open Northstar backup",
        multiple: false,
        directory: false,
        filters: [{ name: "Northstar backup", extensions: ["northstar"] }],
      });
      if (typeof path === "string") setDialog({ type: "transfer", mode: "restore", path });
    } catch (restoreError) {
      setError(errorMessage(restoreError));
    }
  };

  const transferData = async (password: string) => {
    if (dialog?.type !== "transfer") return;
    setBusy(true);
    try {
      if (dialog.mode === "backup") {
        await api.backupLibrary(dialog.path, password);
        setNotice("Portable backup created successfully.");
      } else {
        const result = await api.restoreLibrary(dialog.path, password);
        await loadLibrary(query);
        setNotice(result);
      }
      setDialog(null);
    } catch (transferError) {
      setError(errorMessage(transferError));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="app-shell">
      <Sidebar
        library={library}
        query={query}
        selectedGroupId={selectedGroupId}
        onQueryChange={setQuery}
        onSelectGroup={setSelectedGroupId}
        onOpenHost={openHost}
        onNewHost={() => setDialog({ type: "new-host" })}
        onNewGroup={() => setDialog({ type: "new-group" })}
        onEditHost={(host) => setDialog({ type: "edit-host", host })}
        onEditGroup={(group) => setDialog({ type: "edit-group", group })}
        onBackup={() => void beginBackup()}
        onRestore={() => void beginRestore()}
      />
      <TerminalWorkspace
        tabs={tabs}
        activeTabId={activeTabId}
        onActivate={setActiveTabId}
        onClose={closeTab}
        onNewHost={() => setDialog({ type: "new-host" })}
      />

      {error && (
        <div className="error-toast">
          <AlertCircle size={17} />
          <span>{error}</span>
          <button className="icon-button" onClick={() => setError("")}><X size={16} /></button>
        </div>
      )}
      {notice && (
        <div className="notice-toast">
          <span className="security-dot" />
          <span>{notice}</span>
          <button className="icon-button" onClick={() => setNotice("")}><X size={16} /></button>
        </div>
      )}

      {(dialog?.type === "new-host" || dialog?.type === "edit-host") && (
        <HostDialog
          host={dialog.type === "edit-host" ? dialog.host : undefined}
          groups={library.groups}
          defaultGroupId={selectedGroupId}
          busy={busy}
          onSave={saveHost}
          onDelete={
            dialog.type === "edit-host" ? () => void removeHost(dialog.host) : undefined
          }
          onClose={() => setDialog(null)}
        />
      )}

      {(dialog?.type === "new-group" || dialog?.type === "edit-group") && (
        <GroupDialog
          group={dialog.type === "edit-group" ? dialog.group : undefined}
          groups={library.groups}
          defaultParentId={selectedGroupId}
          busy={busy}
          onSave={saveGroup}
          onDelete={
            dialog.type === "edit-group" ? () => void removeGroup(dialog.group) : undefined
          }
          onClose={() => setDialog(null)}
        />
      )}

      {dialog?.type === "transfer" && (
        <BackupDialog
          mode={dialog.mode}
          busy={busy}
          onSubmit={(password) => void transferData(password)}
          onClose={() => setDialog(null)}
        />
      )}
    </div>
  );
}
