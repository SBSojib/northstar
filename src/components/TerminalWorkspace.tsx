import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { FitAddon } from "@xterm/addon-fit";
import { Terminal } from "@xterm/xterm";
import { LoaderCircle, Plus, Server, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { api, errorMessage } from "../api";
import type { TerminalTab } from "../types";

interface TerminalOutput {
  sessionId: string;
  data: number[];
}

interface TerminalStatus {
  sessionId: string;
  status: "connecting" | "connected" | "closed" | "error";
  message?: string;
}

interface TerminalWorkspaceProps {
  tabs: TerminalTab[];
  activeTabId: string | null;
  onActivate: (id: string) => void;
  onClose: (id: string) => void;
  onNewHost: () => void;
}

export function TerminalWorkspace({
  tabs,
  activeTabId,
  onActivate,
  onClose,
  onNewHost,
}: TerminalWorkspaceProps) {
  return (
    <main className="workspace">
      <div className="tab-bar">
        {tabs.map((tab) => (
          <button
            key={tab.id}
            className={`terminal-tab ${activeTabId === tab.id ? "active" : ""}`}
            onClick={() => onActivate(tab.id)}
          >
            <span className="tab-status" />
            <span>{tab.host.name}</span>
            <span
              className="tab-close"
              role="button"
              onClick={(event) => {
                event.stopPropagation();
                onClose(tab.id);
              }}
            >
              <X size={14} />
            </span>
          </button>
        ))}
        <button className="new-tab-button" title="Add host" onClick={onNewHost}>
          <Plus size={16} />
        </button>
      </div>

      <div className="terminal-stage">
        {!tabs.length && (
          <div className="welcome">
            <div className="welcome-icon"><Server size={28} /></div>
            <h1>Ready to connect</h1>
            <p>Add a host, then double-click it in the sidebar to open an SSH session.</p>
            <button className="primary-button" onClick={onNewHost}>
              <Plus size={16} /> Add your first host
            </button>
          </div>
        )}
        {tabs.map((tab) => (
          <TerminalPane key={tab.id} tab={tab} active={activeTabId === tab.id} />
        ))}
      </div>
    </main>
  );
}

function TerminalPane({ tab, active }: { tab: TerminalTab; active: boolean }) {
  const containerRef = useRef<HTMLDivElement>(null);
  const terminalRef = useRef<Terminal | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const sessionRef = useRef<string | null>(null);
  const [status, setStatus] = useState<TerminalStatus["status"]>("connecting");
  const [message, setMessage] = useState("");

  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    const terminal = new Terminal({
      cursorBlink: true,
      cursorStyle: "bar",
      fontFamily: "'JetBrains Mono', 'Ubuntu Mono', monospace",
      fontSize: 14,
      lineHeight: 1.25,
      scrollback: 5000,
      allowProposedApi: false,
      theme: {
        background: "#0b0d11",
        foreground: "#d8dee9",
        cursor: "#7aa2f7",
        cursorAccent: "#0b0d11",
        selectionBackground: "#314269",
        black: "#151820",
        red: "#f7768e",
        green: "#9ece6a",
        yellow: "#e0af68",
        blue: "#7aa2f7",
        magenta: "#bb9af7",
        cyan: "#7dcfff",
        white: "#c0caf5",
      },
    });
    const fit = new FitAddon();
    terminal.loadAddon(fit);
    terminal.open(container);
    fit.fit();
    terminalRef.current = terminal;
    fitRef.current = fit;
    terminal.writeln(`\x1b[90mConnecting to ${tab.host.username}@${tab.host.address}…\x1b[0m`);

    let outputUnlisten: UnlistenFn | undefined;
    let statusUnlisten: UnlistenFn | undefined;
    let disposed = false;

    const dataDisposable = terminal.onData((data) => {
      if (sessionRef.current) {
        void api.write(sessionRef.current, Array.from(new TextEncoder().encode(data)));
      }
    });

    const start = async () => {
      try {
        [outputUnlisten, statusUnlisten] = await Promise.all([
          listen<TerminalOutput>("terminal-output", ({ payload }) => {
            if (payload.sessionId === sessionRef.current) {
              terminal.write(new Uint8Array(payload.data));
            }
          }),
          listen<TerminalStatus>("terminal-status", ({ payload }) => {
            if (payload.sessionId !== sessionRef.current) return;
            setStatus(payload.status);
            setMessage(payload.message ?? "");
            if (payload.status === "error") {
              terminal.writeln(`\r\n\x1b[31m${payload.message ?? "Connection failed"}\x1b[0m`);
            } else if (payload.status === "closed") {
              terminal.writeln("\r\n\x1b[90mConnection closed.\x1b[0m");
            }
          }),
        ]);
        if (disposed) {
          outputUnlisten?.();
          statusUnlisten?.();
          return;
        }
        const sessionId = crypto.randomUUID();
        sessionRef.current = sessionId;
        await api.connect(tab.host.id, sessionId, terminal.cols, terminal.rows);
        if (disposed) void api.disconnect(sessionId).catch(() => undefined);
      } catch (error) {
        if (disposed) return;
        const text = errorMessage(error);
        setStatus("error");
        setMessage(text);
        terminal.writeln(`\r\n\x1b[31m${text}\x1b[0m`);
      }
    };
    void start();

    const observer = new ResizeObserver(() => {
      if (!container.offsetParent) return;
      fit.fit();
      if (sessionRef.current) {
        void api.resize(sessionRef.current, terminal.cols, terminal.rows);
      }
    });
    observer.observe(container);

    return () => {
      disposed = true;
      observer.disconnect();
      dataDisposable.dispose();
      outputUnlisten?.();
      statusUnlisten?.();
      if (sessionRef.current) void api.disconnect(sessionRef.current).catch(() => undefined);
      terminal.dispose();
    };
  }, [tab.host.address, tab.host.id, tab.host.username]);

  useEffect(() => {
    if (active) {
      requestAnimationFrame(() => {
        fitRef.current?.fit();
        terminalRef.current?.focus();
      });
    }
  }, [active]);

  return (
    <section className={`terminal-pane ${active ? "active" : ""}`}>
      <div className="session-strip">
        <span className={`connection-state ${status}`} />
        <span>{tab.host.username}@{tab.host.address}:{tab.host.port}</span>
        {status === "connecting" && <LoaderCircle className="spin" size={13} />}
        {status === "error" && <span className="session-error" title={message}>Connection failed</span>}
      </div>
      <div ref={containerRef} className="terminal-container" />
    </section>
  );
}
