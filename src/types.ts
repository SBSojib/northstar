export type AuthType = "password" | "private_key";

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
}
