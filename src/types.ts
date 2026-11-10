export type AuthType = "password" | "private_key" | "agent";

export interface Group {
  id: number;
  name: string;
  parentId: number | null;
}

export interface Host {
  id: number;
  name: string;
  address: string;
  port: number;
  username: string;
  groupId: number | null;
  authType: AuthType;
}

export interface Library {
  groups: Group[];
  hosts: Host[];
}

export interface GroupInput {
  id?: number;
  name: string;
  parentId: number | null;
}

export interface HostInput {
  id?: number;
  name: string;
  address: string;
  port: number;
  username: string;
  groupId: number | null;
  authType: AuthType;
  password?: string;
  privateKeyPath?: string;
  privateKeyPassphrase?: string;
}

export interface TerminalTab {
  id: string;
  host: Host;
  generation: number;
}

export interface TerminalPrompt {
  sessionId: string;
  kind: "host_key" | "interactive";
  prompt: string;
  message: string;
  secret: boolean;
}
