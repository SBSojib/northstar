import {
  ChevronDown,
  ChevronRight,
  Download,
  Folder,
  FolderPlus,
  KeyRound,
  MoreHorizontal,
  Plus,
  Search,
  Server,
  Upload,
} from "lucide-react";
import { useMemo, useState } from "react";
import type { Group, Host, Library } from "../types";

interface SidebarProps {
  library: Library;
  query: string;
  selectedGroupId: number | null;
  onQueryChange: (query: string) => void;
  onSelectGroup: (id: number | null) => void;
  onOpenHost: (host: Host) => void;
  onNewHost: () => void;
  onNewGroup: () => void;
  onEditHost: (host: Host) => void;
  onEditGroup: (group: Group) => void;
  onBackup: () => void;
  onRestore: () => void;
}

export function Sidebar(props: SidebarProps) {
  const [expanded, setExpanded] = useState<Set<number>>(
    () => new Set(props.library.groups.map((group) => group.id)),
  );

  const groupsByParent = useMemo(() => {
    const result = new Map<number | null, Group[]>();
    for (const group of props.library.groups) {
      const siblings = result.get(group.parentId) ?? [];
      siblings.push(group);
      result.set(group.parentId, siblings);
    }
    return result;
  }, [props.library.groups]);

  const hostsByGroup = useMemo(() => {
    const result = new Map<number | null, Host[]>();
    for (const host of props.library.hosts) {
      const siblings = result.get(host.groupId) ?? [];
      siblings.push(host);
      result.set(host.groupId, siblings);
    }
    return result;
  }, [props.library.hosts]);

  const toggle = (id: number) => {
    setExpanded((current) => {
      const next = new Set(current);
      next.has(id) ? next.delete(id) : next.add(id);
      return next;
    });
  };

  const renderHost = (host: Host, depth: number) => (
    <div
      className="tree-row host-row"
      style={{ paddingLeft: 14 + depth * 16 }}
      key={host.id}
      onDoubleClick={() => props.onOpenHost(host)}
      role="button"
      tabIndex={0}
      onKeyDown={(event) => event.key === "Enter" && props.onOpenHost(host)}
    >
      {host.authType === "private_key" ? <KeyRound size={14} /> : <Server size={14} />}
      <span className="tree-label">
        <span>{host.name}</span>
        <small>{host.username}@{host.address}</small>
      </span>
      <button
        className="icon-button row-action"
        title="Edit host"
        onClick={(event) => {
          event.stopPropagation();
          props.onEditHost(host);
        }}
      >
        <MoreHorizontal size={15} />
      </button>
    </div>
  );

  const renderGroup = (group: Group, depth: number) => {
    const isExpanded = expanded.has(group.id);
    const children = groupsByParent.get(group.id) ?? [];
    const hosts = hostsByGroup.get(group.id) ?? [];
    return (
      <div key={group.id}>
        <div
          className={`tree-row group-row ${props.selectedGroupId === group.id ? "selected" : ""}`}
          style={{ paddingLeft: 8 + depth * 16 }}
          onClick={() => props.onSelectGroup(group.id)}
        >
          <button
            className="disclosure"
            onClick={(event) => {
              event.stopPropagation();
              toggle(group.id);
            }}
          >
            {isExpanded ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
          </button>
          <Folder size={15} />
          <span className="tree-label">{group.name}</span>
          <button
            className="icon-button row-action"
            title="Edit group"
            onClick={(event) => {
              event.stopPropagation();
              props.onEditGroup(group);
            }}
          >
            <MoreHorizontal size={15} />
          </button>
        </div>
        {isExpanded && (
          <>
            {children.map((child) => renderGroup(child, depth + 1))}
            {hosts.map((host) => renderHost(host, depth + 1))}
          </>
        )}
      </div>
    );
  };

  const rootHosts = hostsByGroup.get(null) ?? [];
  const rootGroups = groupsByParent.get(null) ?? [];

  return (
    <aside className="sidebar">
      <header className="brand">
        <div className="brand-mark">N</div>
        <div>
          <strong>Northstar</strong>
          <span>SSH client</span>
        </div>
      </header>

      <div className="sidebar-actions">
        <button className="primary-button" onClick={props.onNewHost}>
          <Plus size={16} /> New host
        </button>
        <button className="icon-button bordered" title="New group" onClick={props.onNewGroup}>
          <FolderPlus size={17} />
        </button>
      </div>

      <label className="search-box">
        <Search size={15} />
        <input
          value={props.query}
          onChange={(event) => props.onQueryChange(event.target.value)}
          placeholder="Search hosts"
        />
      </label>

      <div className="sidebar-section-title">Hosts</div>
      <nav className="tree">
        <div
          className={`tree-row all-hosts ${props.selectedGroupId === null ? "selected" : ""}`}
          onClick={() => props.onSelectGroup(null)}
        >
          <Server size={15} />
          <span className="tree-label">All hosts</span>
          <span className="count">{props.library.hosts.length}</span>
        </div>
        {rootGroups.map((group) => renderGroup(group, 0))}
        {rootHosts.map((host) => renderHost(host, 0))}
        {!props.library.groups.length && !props.library.hosts.length && (
          <div className="empty-sidebar">Add a host to get started.</div>
        )}
      </nav>

      <div className="backup-actions">
        <button onClick={props.onBackup}><Download size={14} /> Backup</button>
        <button onClick={props.onRestore}><Upload size={14} /> Restore</button>
      </div>
      <footer className="sidebar-footer">
        <span className="security-dot" /> Credentials protected locally
      </footer>
    </aside>
  );
}
