import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { FitAddon } from "@xterm/addon-fit";
import { Terminal } from "@xterm/xterm";
import { LoaderCircle, Plus, RefreshCw, Server, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { api, errorMessage } from "../api";
import type { TerminalPrompt, TerminalTab } from "../types";
import { HostKeyChangedDialog, HostPromptDialog } from "./HostKeyDialog";

interface TerminalOutput {
  sessionId: string;
  data: number[];
}

interface TerminalStatus {
  sessionId: string;
  status: "connecting" | "connected" | "closed" | "error" | "host_key_changed";
  message?: string;
}

interface PromptRequest extends TerminalPrompt {
  tabId: string;
}

interface KeyChange {
  tabId: string;
  hostId: number;
  address: string;
  message: string;
}

interface TerminalWorkspaceProps {
  tabs: TerminalTab[];
  activeTabId: string | null;
  onActivate: (id: string) => void;
  onClose: (id: string) => void;
  onNewHost: () => void;
  onReconnect: (id: string) => void;
  onError: (message: string) => void;
}

export function TerminalWorkspace({
  tabs,
  activeTabId,
  onActivate,
  onClose,
  onNewHost,
  onReconnect,
  onError,
}: TerminalWorkspaceProps) {
  const [prompts, setPrompts] = useState<PromptRequest[]>([]);
  const [keyChanges, setKeyChanges] = useState<KeyChange[]>([]);
  const [busy, setBusy] = useState(false);
  const prompt = prompts[0];
  const keyChange = keyChanges[0];

  const dismissPrompt = () => setPrompts((current) => current.slice(1));
  const dismissKeyChange = () => setKeyChanges((current) => current.slice(1));

  const answerPrompt = async (answer: string) => {
    if (!prompt) return;
    setBusy(true);
    try {
      await api.answerPrompt(prompt.sessionId, answer);
      dismissPrompt();
    } catch (error) {
      onError(errorMessage(error));
    } finally {
      setBusy(false);
    }
  };

  const cancelPrompt = async () => {
    if (!prompt) return;
    const current = prompt;
    dismissPrompt();
    if (current.kind === "host_key") {
      await api.answerPrompt(current.sessionId, "no").catch(() => undefined);
      return;
    }
    await api.disconnect(current.sessionId).catch(() => undefined);
  };

  const replaceHostKey = async () => {
    if (!keyChange) return;
    setBusy(true);
    try {
      await api.forgetHostKey(keyChange.hostId);
      const tabId = keyChange.tabId;
      dismissKeyChange();
      onReconnect(tabId);
    } catch (error) {
      onError(errorMessage(error));
    } finally {
      setBusy(false);
    }
  };

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
          <TerminalPane
            key={tab.id}
            tab={tab}
            active={activeTabId === tab.id}
            onPrompt={(request) => {
              setPrompts((current) => [...current.filter((item) => item.tabId !== tab.id), request]);
              onActivate(tab.id);
            }}
            onHostKeyChanged={(change) => {
              setKeyChanges((current) => [
                ...current.filter((item) => item.tabId !== tab.id),
                change,
              ]);
              onActivate(tab.id);
            }}
          />
        ))}
      </div>

      {prompt && (
        <HostPromptDialog
          title={prompt.kind === "host_key" ? "Verify host key" : prompt.prompt}
          detail={
            prompt.kind === "host_key"
              ? "This host is not in known hosts yet. Compare the fingerprint, then trust it once."
              : "OpenSSH is waiting for a response before the session can continue."
          }
          message={prompt.kind === "host_key" ? prompt.message : ""}
          secret={prompt.secret}
          confirmLabel={prompt.kind === "host_key" ? "Trust this key" : "Send"}
          busy={busy}
          onConfirm={(answer) => void answerPrompt(answer)}
          onCancel={() => void cancelPrompt()}
        />
      )}
      {!prompt && keyChange && (
        <HostKeyChangedDialog
          address={keyChange.address}
          message={keyChange.message}
          busy={busy}
          onReplace={() => void replaceHostKey()}
          onClose={dismissKeyChange}
        />
      )}
    </main>
  );
}

function TerminalPane({
  tab,
  active,
  onPrompt,
  onHostKeyChanged,
}: {
  tab: TerminalTab;
  active: boolean;
  onPrompt: (prompt: PromptRequest) => void;
  onHostKeyChanged: (change: KeyChange) => void;
}) {
  const containerRef = useRef<HTMLDivElement>(null);
  const terminalRef = useRef<Terminal | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const sessionRef = useRef<string | null>(null);
  const onPromptRef = useRef(onPrompt);
  const onHostKeyChangedRef = useRef(onHostKeyChanged);
  const [status, setStatus] = useState<TerminalStatus["status"]>("connecting");
  const [message, setMessage] = useState("");
  const [retryKey, setRetryKey] = useState(0);
  onPromptRef.current = onPrompt;
  onHostKeyChangedRef.current = onHostKeyChanged;

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
    setStatus("connecting");
    setMessage("");
    terminal.writeln(`\x1b[90mConnecting to ${tab.host.username}@${tab.host.address}…\x1b[0m`);

    const pasteText = (text: string) => {
      if (!text) return;
      if (
        text.length > 2048 &&
        !window.confirm(`Paste ${text.length.toLocaleString()} characters into this session?`)
      ) {
        return;
      }
      terminal.paste(text);
    };

    terminal.attachCustomKeyEventHandler((event) => {
      if (event.type !== "keydown" || !event.ctrlKey || !event.shiftKey) return true;
      if (event.key === "C" || event.key === "c") {
        const selection = terminal.getSelection();
        if (selection) void navigator.clipboard.writeText(selection).catch(() => undefined);
        return false;
      }
      if (event.key === "V" || event.key === "v") {
        void navigator.clipboard.readText().then(pasteText).catch(() => undefined);
        return false;
      }
      return true;
    });
    const selection = terminal.onSelectionChange(() => {
      const text = terminal.getSelection();
      if (text) void navigator.clipboard.writeText(text).catch(() => undefined);
    });
    const onMiddleClick = (event: MouseEvent) => {
      if (event.button !== 1) return;
      event.preventDefault();
      void navigator.clipboard.readText().then(pasteText).catch(() => undefined);
    };
    container.addEventListener("mousedown", onMiddleClick);

    let outputUnlisten: UnlistenFn | undefined;
    let statusUnlisten: UnlistenFn | undefined;
    let promptUnlisten: UnlistenFn | undefined;
    let disposed = false;

    const dataDisposable = terminal.onData((data) => {
      if (sessionRef.current) {
        void api.write(sessionRef.current, Array.from(new TextEncoder().encode(data)));
      }
    });

    const start = async () => {
      try {
        [outputUnlisten, statusUnlisten, promptUnlisten] = await Promise.all([
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
            } else if (payload.status === "host_key_changed") {
              terminal.writeln("\r\n\x1b[33mHost key changed. Confirm the new key before reconnecting.\x1b[0m");
              onHostKeyChangedRef.current({
                tabId: tab.id,
                hostId: tab.host.id,
                address: tab.host.address,
                message: payload.message ?? "",
              });
            }
          }),
          listen<TerminalPrompt>("terminal-prompt", ({ payload }) => {
            if (payload.sessionId !== sessionRef.current) return;
            onPromptRef.current({ ...payload, tabId: tab.id });
          }),
        ]);
        if (disposed) {
          outputUnlisten?.();
          statusUnlisten?.();
          promptUnlisten?.();
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
      container.removeEventListener("mousedown", onMiddleClick);
      dataDisposable.dispose();
      selection.dispose();
      outputUnlisten?.();
      statusUnlisten?.();
      promptUnlisten?.();
      if (sessionRef.current) void api.disconnect(sessionRef.current).catch(() => undefined);
      terminal.dispose();
    };
  }, [retryKey, tab.generation, tab.host.address, tab.host.id, tab.host.username, tab.id]);

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
        {status === "connecting" && message && <span className="session-message">{message}</span>}
        {(status === "error" || status === "closed" || status === "host_key_changed") && (
          <div className="session-disconnected">
            <span className="session-error" title={message}>
              {status === "host_key_changed"
                ? "Host key changed"
                : status === "error"
                  ? "Connection failed"
                  : "Connection closed"}
            </span>
            {status === "host_key_changed" && (
              <button
                className="session-retry"
                type="button"
                onClick={() =>
                  onHostKeyChanged({
                    tabId: tab.id,
                    hostId: tab.host.id,
                    address: tab.host.address,
                    message,
                  })
                }
              >
                Review key
              </button>
            )}
            <button
              className="session-retry"
              type="button"
              onClick={() => setRetryKey((key) => key + 1)}
              title="Start a new SSH session"
            >
              <RefreshCw size={12} /> Retry
            </button>
          </div>
        )}
      </div>
      <div ref={containerRef} className="terminal-container" />
    </section>
  );
}
