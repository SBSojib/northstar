# Northstar SSH

A focused, local-first desktop SSH client for Ubuntu, built with Tauri 2, React,
TypeScript, Rust, SQLite, and xterm.js.

## MVP features

- Password and imported private-key authentication
- Nested host groups and local search
- Multiple interactive terminal tabs
- Local SQLite storage
- AES-256-GCM encrypted credentials with the master key stored in the Ubuntu
  Secret Service keyring
- Password-protected portable backup and non-destructive restore

Private key contents are imported and encrypted; their source paths are not
saved.

## Ubuntu setup

Install the current Node.js LTS and Rust stable toolchains, then install Tauri's
Linux prerequisites:

```bash
sudo apt update
sudo apt install build-essential curl wget file libwebkit2gtk-4.1-dev \
  libayatana-appindicator3-dev librsvg2-dev patchelf libssl-dev \
  libsecret-1-dev libdbus-1-dev
```

Run the app:

```bash
npm install
npm run tauri dev
```

Build the `.deb` package:

```bash
npm run tauri build
```

The installer is written to:

```text
src-tauri/target/release/bundle/deb/Northstar SSH_<version>_amd64.deb
```

Install it with apt. Use the quoted path because the filename contains a space:

```bash
sudo apt install "./src-tauri/target/release/bundle/deb/Northstar SSH_0.2.1_amd64.deb"
```

Replace `0.2.1` with the version shown in that directory. Upgrades keep saved hosts; they live outside the package in the application data directory.

## Storage

The database is stored in Tauri's application data directory as
`northstar.db`. Passwords, private keys, and key passphrases are encrypted in
the database. The random database master key is held by the desktop keyring
under the service `app.northstar.ssh`.

Normal package upgrades and reinstalls preserve this application-data
directory. Use the **Backup** action in the sidebar to create a portable
`.northstar` file. It contains hosts, nested groups, and credentials encrypted
with your chosen password. On another Northstar installation, choose
**Restore**; imported items are merged with the existing library.

A raw copy of `northstar.db` is not portable because its encryption key remains
in the original machine's system keyring.
