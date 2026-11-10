use std::{
    collections::HashMap,
    ffi::CString,
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    os::{
        fd::{AsRawFd, RawFd},
        unix::{fs::PermissionsExt, process::CommandExt},
    },
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::{mpsc, Arc, Mutex},
    thread,
    time::Duration,
};

use crate::{
    askpass,
    database::Database,
    error::{AppError, AppResult},
    known_hosts,
    models::{Credential, TerminalOutput, TerminalPrompt, TerminalStatus},
};
use tauri::{AppHandle, Emitter};

enum SessionCommand {
    Input(Vec<u8>),
    Resize(u32, u32),
    Answer(String),
    Disconnect,
}

struct SessionLink {
    commands: mpsc::Sender<SessionCommand>,
    wake: std::os::unix::net::UnixStream,
}

enum SessionEnd {
    Closed,
    Cancelled,
    Failed(String),
    HostKeyChanged(String),
}

pub(crate) enum PromptDecision {
    Answer(String),
    Ask { kind: &'static str, secret: bool },
}

pub struct SshManager {
    sessions: Arc<Mutex<HashMap<String, SessionLink>>>,
}

impl SshManager {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn connect(
        &self,
        app: AppHandle,
        database: &Database,
        host_id: i64,
        session_id: String,
        cols: u32,
        rows: u32,
    ) -> AppResult<String> {
        let (host, credential) = database.host_with_credential(host_id)?;
        let known_hosts_option = known_hosts::option_value(&known_hosts::files_for_session(&app)?);
        let (sender, receiver) = mpsc::channel();
        let (wake_read, wake_write) = std::os::unix::net::UnixStream::pair()?;
        wake_read.set_nonblocking(true)?;
        wake_write.set_nonblocking(true)?;
        self.sessions
            .lock()
            .map_err(|_| AppError::Internal("Session lock was poisoned".into()))?
            .insert(
                session_id.clone(),
                SessionLink {
                    commands: sender,
                    wake: wake_write,
                },
            );

        let thread_session_id = session_id.clone();
        let sessions = Arc::clone(&self.sessions);
        thread::spawn(move || {
            emit_status(&app, &thread_session_id, "connecting", None);
            let end = run_session(
                &app,
                &thread_session_id,
                &host.address,
                host.port,
                &host.username,
                credential,
                cols,
                rows,
                &known_hosts_option,
                receiver,
                wake_read,
            );
            match end {
                SessionEnd::Closed | SessionEnd::Cancelled => {
                    emit_status(&app, &thread_session_id, "closed", None)
                }
                SessionEnd::Failed(message) => {
                    emit_status(&app, &thread_session_id, "error", Some(message))
                }
                SessionEnd::HostKeyChanged(message) => emit_status(
                    &app,
                    &thread_session_id,
                    "host_key_changed",
                    Some(message),
                ),
            }
            if let Ok(mut sessions) = sessions.lock() {
                sessions.remove(&thread_session_id);
            }
        });

        Ok(session_id)
    }

    pub fn input(&self, session_id: &str, data: Vec<u8>) -> AppResult<()> {
        self.send(session_id, SessionCommand::Input(data))
    }

    pub fn resize(&self, session_id: &str, cols: u32, rows: u32) -> AppResult<()> {
        self.send(session_id, SessionCommand::Resize(cols, rows))
    }

    pub fn answer(&self, session_id: &str, answer: String) -> AppResult<()> {
        self.send(session_id, SessionCommand::Answer(answer))
    }

    pub fn disconnect(&self, session_id: &str) -> AppResult<()> {
        let link = self
            .sessions
            .lock()
            .map_err(|_| AppError::Internal("Session lock was poisoned".into()))?
            .remove(session_id)
            .ok_or(AppError::SessionNotFound)?;
        link.commands
            .send(SessionCommand::Disconnect)
            .map_err(|_| AppError::SessionNotFound)?;
        poke(&link.wake);
        Ok(())
    }

    fn send(&self, session_id: &str, command: SessionCommand) -> AppResult<()> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| AppError::Internal("Session lock was poisoned".into()))?;
        let Some(link) = sessions.get(session_id) else {
            return Err(AppError::SessionNotFound);
        };
        if link.commands.send(command).is_err() {
            sessions.remove(session_id);
            return Err(AppError::SessionNotFound);
        }
        poke(&link.wake);
        Ok(())
    }
}

fn poke(wake: &std::os::unix::net::UnixStream) {
    let byte = [1_u8];
    let _ = unsafe { libc::write(wake.as_raw_fd(), byte.as_ptr().cast(), 1) };
}

struct TempSession {
    dir: PathBuf,
}

impl TempSession {
    fn create(session_id: &str) -> AppResult<Self> {
        let dir = std::env::temp_dir().join(format!("northstar-{session_id}"));
        fs::create_dir_all(&dir)?;
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
        Ok(Self { dir })
    }

    fn write_key(&self, private_key: &str) -> AppResult<PathBuf> {
        let path = self.dir.join("id_key");
        let mut contents = private_key.to_owned();
        if !contents.ends_with('\n') {
            contents.push('\n');
        }
        fs::write(&path, contents)?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        Ok(path)
    }
}

impl Drop for TempSession {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

struct Pty {
    master: RawFd,
    slave: RawFd,
}

impl Drop for Pty {
    fn drop(&mut self) {
        close_fd(&mut self.master);
        close_fd(&mut self.slave);
    }
}

struct ChildGuard(Option<Child>);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(child) = self.0.as_mut() {
            stop_child(child);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn run_session(
    app: &AppHandle,
    session_id: &str,
    address: &str,
    port: u16,
    username: &str,
    credential: Credential,
    cols: u32,
    rows: u32,
    known_hosts_option: &str,
    receiver: mpsc::Receiver<SessionCommand>,
    wake: std::os::unix::net::UnixStream,
) -> SessionEnd {
    let temp = match TempSession::create(session_id) {
        Ok(temp) => temp,
        Err(error) => return SessionEnd::Failed(error.to_string()),
    };
    let key_path = match &credential {
        Credential::PrivateKey { private_key, .. } => match temp.write_key(private_key) {
            Ok(path) => path,
            Err(error) => return SessionEnd::Failed(error.to_string()),
        },
        _ => temp.dir.join("id_key"),
    };
    let log_path = temp.dir.join("ssh.log");
    let args = ssh_arguments(
        address,
        port,
        username,
        &credential,
        known_hosts_option,
        &log_path,
        &key_path,
    );
    if let Err(error) = prepare_prompt_fifos(&temp.dir) {
        return SessionEnd::Failed(error.to_string());
    }
    let request_fd = match open_fifo(&temp.dir.join("request"), true) {
        Ok(fd) => fd,
        Err(error) => return SessionEnd::Failed(error.to_string()),
    };
    let response_fd = match open_fifo(&temp.dir.join("response"), false) {
        Ok(fd) => fd,
        Err(error) => {
            close_raw(request_fd);
            return SessionEnd::Failed(error.to_string());
        }
    };
    let mut pty = match open_pty(cols, rows) {
        Ok(pty) => pty,
        Err(error) => {
            close_raw(request_fd);
            close_raw(response_fd);
            return SessionEnd::Failed(error.to_string());
        }
    };
    if let Err(error) = set_raw(pty.slave) {
        close_raw(request_fd);
        close_raw(response_fd);
        return SessionEnd::Failed(error.to_string());
    }
    let child = match spawn_ssh(&args, pty.master, pty.slave, &temp.dir) {
        Ok(child) => child,
        Err(error) => {
            close_raw(request_fd);
            close_raw(response_fd);
            return SessionEnd::Failed(error.to_string());
        }
    };
    close_fd(&mut pty.slave);
    if let Err(error) = set_nonblocking(pty.master) {
        let mut child = child;
        stop_child(&mut child);
        close_raw(request_fd);
        close_raw(response_fd);
        return SessionEnd::Failed(error.to_string());
    }

    let end = session_loop(
        app,
        session_id,
        &credential,
        child,
        pty.master,
        request_fd,
        response_fd,
        &wake,
        &receiver,
        &log_path,
    );
    close_raw(request_fd);
    close_raw(response_fd);
    end
}

#[allow(clippy::too_many_arguments)]
fn session_loop(
    app: &AppHandle,
    session_id: &str,
    credential: &Credential,
    child: Child,
    master: RawFd,
    request_fd: RawFd,
    response_fd: RawFd,
    wake: &std::os::unix::net::UnixStream,
    receiver: &mpsc::Receiver<SessionCommand>,
    log_path: &Path,
) -> SessionEnd {
    let mut guard = ChildGuard(Some(child));
    let mut log = LogFollow::default();
    let mut log_text = String::new();
    let mut pty_tail = Vec::<u8>::new();
    let mut pending_input = Vec::<u8>::new();
    let mut prompt_buf = Vec::<u8>::new();
    let mut awaiting_prompt = false;
    let mut authenticated = false;
    let mut key_changed = false;
    let wake_fd = wake.as_raw_fd();

    let end = loop {
        if let Err(error) = try_flush(master, &mut pending_input) {
            break SessionEnd::Failed(error.to_string());
        }
        let mut events = libc::POLLIN;
        if !pending_input.is_empty() {
            events |= libc::POLLOUT;
        }
        let mut polls = [
            libc::pollfd {
                fd: master,
                events,
                revents: 0,
            },
            libc::pollfd {
                fd: wake_fd,
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: request_fd,
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        let polled = unsafe { libc::poll(polls.as_mut_ptr(), polls.len() as libc::nfds_t, 200) };
        if polled < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() != std::io::ErrorKind::Interrupted {
                break SessionEnd::Failed(error.to_string());
            }
        }
        if polls[1].revents != 0 {
            drain_fd(wake_fd);
        }

        let mut stop = None;
        while let Ok(command) = receiver.try_recv() {
            match command {
                SessionCommand::Input(data) => pending_input.extend(data),
                SessionCommand::Resize(cols, rows) => {
                    if let Err(error) = set_winsize(master, cols, rows) {
                        stop = Some(SessionEnd::Failed(error.to_string()));
                        break;
                    }
                }
                SessionCommand::Answer(answer) if awaiting_prompt => {
                    if let Err(error) = write_all_fd(response_fd, &askpass::encode_answer(&answer))
                    {
                        stop = Some(SessionEnd::Failed(error.to_string()));
                        break;
                    }
                    awaiting_prompt = false;
                    emit_status(app, session_id, "connecting", None);
                }
                SessionCommand::Answer(_) => {}
                SessionCommand::Disconnect => {
                    stop = Some(SessionEnd::Cancelled);
                    break;
                }
            }
        }
        if let Some(end) = stop {
            break end;
        }

        if polls[0].revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) != 0 {
            match read_available(master) {
                Ok(data) if !data.is_empty() => {
                    append_tail(&mut pty_tail, &data);
                    if host_key_changed(&String::from_utf8_lossy(&data)) {
                        key_changed = true;
                    }
                    emit_output(app, session_id, &data);
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => {}
                Err(error) if is_retryable(&error) => {}
                Err(error) => break SessionEnd::Failed(error.to_string()),
            }
        }

        if polls[2].revents & libc::POLLIN != 0 {
            match read_prompt_line(request_fd, &mut prompt_buf) {
                Ok(Some(prompt)) => {
                    let mirrored = pull_log(&mut log, log_path, &mut log_text);
                    if !mirrored.is_empty() {
                        emit_output(app, session_id, mirrored.as_bytes());
                    }
                    key_changed = key_changed || host_key_changed(&log_text);
                    if prompt_is_host_key(&prompt) {
                        for _ in 0..8 {
                            let seen = format!(
                                "{log_text}\n{}",
                                String::from_utf8_lossy(&pty_tail)
                            );
                            if seen.to_ascii_lowercase().contains("fingerprint") {
                                break;
                            }
                            thread::sleep(Duration::from_millis(25));
                            let waited = pull_log(&mut log, log_path, &mut log_text);
                            if !waited.is_empty() {
                                emit_output(app, session_id, waited.as_bytes());
                            }
                            if let Ok(data) = read_available(master) {
                                if !data.is_empty() {
                                    append_tail(&mut pty_tail, &data);
                                    emit_output(app, session_id, &data);
                                }
                            }
                        }
                    }
                    match decide_prompt(&prompt, credential) {
                        PromptDecision::Answer(secret) => {
                            if let Err(error) =
                                write_all_fd(response_fd, &askpass::encode_answer(&secret))
                            {
                                break SessionEnd::Failed(error.to_string());
                            }
                        }
                        PromptDecision::Ask { kind, secret } => {
                            let message = prompt_context(&log_text, &pty_tail, &prompt);
                            awaiting_prompt = true;
                            emit_prompt(app, session_id, kind, &prompt, &message, secret);
                            let notice = if kind == "host_key" {
                                "Waiting for host key approval"
                            } else {
                                "Waiting for a response"
                            };
                            emit_status(app, session_id, "connecting", Some(notice.into()));
                        }
                    }
                }
                Ok(None) => {}
                Err(error) => break SessionEnd::Failed(error.to_string()),
            }
        }

        let mirrored = pull_log(&mut log, log_path, &mut log_text);
        if !mirrored.is_empty() {
            emit_output(app, session_id, mirrored.as_bytes());
        }
        if !authenticated && log_text.contains("Authenticated to ") {
            authenticated = true;
            emit_status(app, session_id, "connected", None);
        }
        key_changed = key_changed || host_key_changed(&log_text);

        let waited = guard
            .0
            .as_mut()
            .expect("child is waited for once")
            .try_wait();
        match waited {
            Ok(Some(status)) => {
                if let Ok(data) = read_available(master) {
                    if !data.is_empty() {
                        append_tail(&mut pty_tail, &data);
                        emit_output(app, session_id, &data);
                    }
                }
                let mirrored = pull_log(&mut log, log_path, &mut log_text);
                if !mirrored.is_empty() {
                    emit_output(app, session_id, mirrored.as_bytes());
                }
                key_changed = key_changed || host_key_changed(&log_text);
                guard.0.take();
                break classify_exit(status, &log_text, &pty_tail, key_changed);
            }
            Ok(None) => {}
            Err(error) => break SessionEnd::Failed(error.to_string()),
        }
    };
    end
}

fn prompt_is_host_key(prompt: &str) -> bool {
    let lower = prompt.to_ascii_lowercase();
    lower.contains("continue connecting")
        || (lower.contains("fingerprint") && (lower.contains("yes") || lower.contains("no")))
}

fn classify_exit(
    status: ExitStatus,
    log_text: &str,
    pty_tail: &[u8],
    key_changed: bool,
) -> SessionEnd {
    let combined = format!("{log_text}\n{}", String::from_utf8_lossy(pty_tail));
    if key_changed || host_key_changed(&combined) {
        return SessionEnd::HostKeyChanged(prompt_context(log_text, pty_tail, ""));
    }
    if status.success() {
        return SessionEnd::Closed;
    }
    let excerpt = host_key_excerpt(&format!(
        "{log_text}\n{}",
        String::from_utf8_lossy(pty_tail)
    ));
    let message = excerpt
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("Connection failed")
        .to_owned();
    SessionEnd::Failed(if message.is_empty() {
        format!(
            "Connection failed (ssh exited with status {})",
            status.code().unwrap_or(-1)
        )
    } else {
        message
    })
}

fn prepare_prompt_fifos(dir: &Path) -> AppResult<()> {
    mkfifo(&dir.join("request"))?;
    mkfifo(&dir.join("response"))?;
    Ok(())
}

fn spawn_ssh(args: &[String], master: RawFd, slave: RawFd, session_dir: &Path) -> AppResult<Child> {
    let askpass = std::env::current_exe()?;
    let mut command = Command::new("ssh");
    command
        .args(args)
        .env("SSH_ASKPASS", askpass)
        .env("SSH_ASKPASS_REQUIRE", "force")
        .env("NORTHSTAR_ASKPASS", "1")
        .env("NORTHSTAR_ASKPASS_DIR", session_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // Runs in the forked child, before exec. Only async-signal-safe libc calls.
    unsafe {
        command.pre_exec(move || {
            libc::close(master);
            if libc::login_tty(slave) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    command.spawn().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            AppError::InvalidInput(
                "The OpenSSH client was not found. Install the openssh-client package.".into(),
            )
        } else {
            AppError::Io(error)
        }
    })
}

fn stop_child(child: &mut Child) {
    let pid = child.id() as i32;
    if pid > 0 {
        unsafe { libc::kill(-pid, libc::SIGKILL) };
    }
    let _ = child.kill();
    let _ = child.wait();
}

pub(crate) fn ssh_arguments(
    address: &str,
    port: u16,
    username: &str,
    credential: &Credential,
    known_hosts_option: &str,
    log_path: &Path,
    key_path: &Path,
) -> Vec<String> {
    let mut args = vec![
        "-v".into(),
        "-E".into(),
        log_path.display().to_string(),
        "-o".into(),
        "StrictHostKeyChecking=ask".into(),
        "-o".into(),
        "UpdateHostKeys=yes".into(),
        "-o".into(),
        "ServerAliveInterval=15".into(),
        "-o".into(),
        "ServerAliveCountMax=3".into(),
        "-o".into(),
        "RequestTTY=yes".into(),
        "-o".into(),
        "NumberOfPasswordPrompts=1".into(),
        "-o".into(),
        known_hosts_option.to_owned(),
        "-p".into(),
        port.to_string(),
    ];
    match credential {
        Credential::Password { .. } => args.extend([
            "-o".into(),
            "PreferredAuthentications=password,keyboard-interactive".into(),
            "-o".into(),
            "PubkeyAuthentication=no".into(),
            "-o".into(),
            "IdentitiesOnly=yes".into(),
            "-o".into(),
            "IdentityAgent=none".into(),
        ]),
        Credential::PrivateKey { .. } => args.extend([
            "-i".into(),
            key_path.display().to_string(),
            "-o".into(),
            "IdentitiesOnly=yes".into(),
            "-o".into(),
            "IdentityAgent=none".into(),
            "-o".into(),
            "PreferredAuthentications=publickey".into(),
            "-o".into(),
            "PasswordAuthentication=no".into(),
        ]),
        Credential::Agent => args.extend([
            "-o".into(),
            "PreferredAuthentications=publickey".into(),
            "-o".into(),
            "PasswordAuthentication=no".into(),
            "-o".into(),
            "KbdInteractiveAuthentication=no".into(),
        ]),
    }
    args.push("-l".into());
    args.push(username.to_owned());
    args.push(address.to_owned());
    args
}

pub(crate) fn decide_prompt(prompt: &str, credential: &Credential) -> PromptDecision {
    let lower = prompt.to_ascii_lowercase();
    if lower.contains("continue connecting")
        || (lower.contains("fingerprint") && (lower.contains("yes") || lower.contains("no")))
    {
        return PromptDecision::Ask {
            kind: "host_key",
            secret: false,
        };
    }
    if lower.contains("passphrase") {
        if let Credential::PrivateKey {
            passphrase: Some(value),
            ..
        } = credential
        {
            if !value.is_empty() {
                return PromptDecision::Answer(value.clone());
            }
        }
        return PromptDecision::Ask {
            kind: "interactive",
            secret: true,
        };
    }
    if lower.contains("password") {
        if let Credential::Password { password } = credential {
            if !password.is_empty() {
                return PromptDecision::Answer(password.clone());
            }
        }
        return PromptDecision::Ask {
            kind: "interactive",
            secret: true,
        };
    }
    PromptDecision::Ask {
        kind: "interactive",
        secret: false,
    }
}

pub(crate) fn host_key_changed(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("remote host identification has changed")
        || (lower.contains("host key") && lower.contains("has changed"))
}

pub(crate) fn user_visible_log_line(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with("debug") {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    lower.contains("fingerprint")
        || lower.contains("authenticity of host")
        || lower.contains("warning:")
        || lower.contains("host key")
        || lower.contains("known_hosts")
        || lower.contains("permission denied")
        || lower.contains("connection refused")
        || lower.contains("timed out")
        || lower.contains("could not resolve")
        || lower.contains("no route to host")
        || lower.contains("connection closed")
        || lower.contains("offending")
        || lower.contains("verification failed")
}

pub(crate) fn host_key_excerpt(text: &str) -> String {
    let clean = strip_ansi(text);
    let lines = clean
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("debug"))
        .collect::<Vec<_>>();
    let start = lines.len().saturating_sub(12);
    lines[start..].join("\n")
}

fn prompt_context(log_text: &str, pty_tail: &[u8], prompt: &str) -> String {
    let mut message = host_key_excerpt(&format!(
        "{log_text}\n{}",
        String::from_utf8_lossy(pty_tail)
    ));
    if message.is_empty() {
        message = prompt.to_owned();
    } else if !prompt.is_empty() && !message.contains(prompt.trim()) {
        message.push_str("\n\n");
        message.push_str(prompt.trim());
    }
    if message.len() > 4000 {
        let mut end = 4000;
        while !message.is_char_boundary(end) {
            end -= 1;
        }
        message.truncate(end);
    }
    message
}

fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(character) = chars.next() {
        if character == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for next in chars.by_ref() {
                    if ('@'..='~').contains(&next) {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(character);
    }
    out
}

#[derive(Default)]
struct LogFollow {
    file: Option<File>,
    offset: u64,
    partial: String,
}

impl LogFollow {
    fn read_new(&mut self, path: &Path) -> std::io::Result<String> {
        if self.file.is_none() {
            match File::open(path) {
                Ok(file) => self.file = Some(file),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    return Ok(String::new())
                }
                Err(error) => return Err(error),
            }
        }
        let file = self.file.as_mut().expect("log file is open");
        file.seek(SeekFrom::Start(self.offset))?;
        let mut buffer = Vec::new();
        let read = file.read_to_end(&mut buffer)?;
        self.offset += read as u64;
        Ok(String::from_utf8_lossy(&buffer).into_owned())
    }
}

fn pull_log(log: &mut LogFollow, path: &Path, accumulated: &mut String) -> String {
    let chunk = log.read_new(path).unwrap_or_default();
    if chunk.is_empty() {
        return String::new();
    }
    accumulated.push_str(&chunk);
    if accumulated.len() > 64_000 {
        let extra = accumulated.len() - 48_000;
        accumulated.drain(..extra);
    }
    log.partial.push_str(&chunk);
    let mut mirrored = String::new();
    while let Some(index) = log.partial.find('\n') {
        let line: String = log.partial.drain(..=index).collect();
        let trimmed = line.trim_end_matches(['\n', '\r']);
        if user_visible_log_line(trimmed) {
            mirrored.push_str(trimmed);
            mirrored.push_str("\r\n");
        }
    }
    mirrored
}

fn open_pty(cols: u32, rows: u32) -> std::io::Result<Pty> {
    let mut master = -1;
    let mut slave = -1;
    let mut size = libc::winsize {
        ws_row: rows.max(1) as u16,
        ws_col: cols.max(1) as u16,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let result = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut size,
        )
    };
    if result != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(Pty { master, slave })
}

fn set_raw(fd: RawFd) -> std::io::Result<()> {
    let mut termios = unsafe { std::mem::zeroed::<libc::termios>() };
    if unsafe { libc::tcgetattr(fd, &mut termios) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    unsafe { libc::cfmakeraw(&mut termios) };
    if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &termios) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

fn set_winsize(fd: RawFd, cols: u32, rows: u32) -> std::io::Result<()> {
    if cols == 0 || rows == 0 {
        return Ok(());
    }
    let size = libc::winsize {
        ws_row: rows as u16,
        ws_col: cols as u16,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let result = unsafe { libc::ioctl(fd, libc::TIOCSWINSZ, &size) };
    if result == -1 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn set_nonblocking(fd: RawFd) -> std::io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let result = unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) };
    if result < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn mkfifo(path: &Path) -> std::io::Result<()> {
    let text = path.to_string_lossy();
    let c_path = CString::new(text.as_bytes())
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid fifo path"))?;
    let result = unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

fn open_fifo(path: &Path, nonblocking: bool) -> std::io::Result<RawFd> {
    let text = path.to_string_lossy();
    let c_path = CString::new(text.as_bytes())
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid fifo path"))?;
    let mut flags = libc::O_RDWR;
    if nonblocking {
        flags |= libc::O_NONBLOCK;
    }
    let fd = unsafe { libc::open(c_path.as_ptr(), flags) };
    if fd < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(fd)
    }
}

fn close_fd(fd: &mut RawFd) {
    if *fd >= 0 {
        unsafe { libc::close(*fd) };
        *fd = -1;
    }
}

fn close_raw(mut fd: RawFd) {
    close_fd(&mut fd);
}

fn read_available(fd: RawFd) -> std::io::Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let read = unsafe { libc::read(fd, buffer.as_mut_ptr().cast(), buffer.len()) };
        if read > 0 {
            out.extend_from_slice(&buffer[..read as usize]);
            continue;
        }
        if read == 0 {
            if out.is_empty() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "pty closed",
                ));
            }
            return Ok(out);
        }
        let error = std::io::Error::last_os_error();
        if error.kind() == std::io::ErrorKind::Interrupted {
            continue;
        }
        if is_retryable(&error) {
            return Ok(out);
        }
        return Err(error);
    }
}

fn read_prompt_line(fd: RawFd, buffer: &mut Vec<u8>) -> std::io::Result<Option<String>> {
    let mut temporary = [0_u8; 1024];
    loop {
        let read = unsafe { libc::read(fd, temporary.as_mut_ptr().cast(), temporary.len()) };
        if read > 0 {
            buffer.extend_from_slice(&temporary[..read as usize]);
            continue;
        }
        if read == 0 {
            break;
        }
        let error = std::io::Error::last_os_error();
        if error.kind() == std::io::ErrorKind::Interrupted {
            continue;
        }
        if is_retryable(&error) {
            break;
        }
        return Err(error);
    }
    let Some(position) = buffer.iter().position(|byte| *byte == b'\n') else {
        return Ok(None);
    };
    let line = buffer.drain(..=position).collect::<Vec<_>>();
    Ok(Some(
        String::from_utf8_lossy(&line).trim().to_owned(),
    ))
}

fn try_flush(fd: RawFd, pending: &mut Vec<u8>) -> std::io::Result<()> {
    while !pending.is_empty() {
        let written = unsafe { libc::write(fd, pending.as_ptr().cast(), pending.len()) };
        if written > 0 {
            pending.drain(..written as usize);
            continue;
        }
        if written == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::WriteZero,
                "pty closed",
            ));
        }
        let error = std::io::Error::last_os_error();
        if error.kind() == std::io::ErrorKind::Interrupted {
            continue;
        }
        if is_retryable(&error) {
            return Ok(());
        }
        return Err(error);
    }
    Ok(())
}

fn write_all_fd(fd: RawFd, data: &[u8]) -> std::io::Result<()> {
    let mut pending = data.to_vec();
    while !pending.is_empty() {
        let written = unsafe { libc::write(fd, pending.as_ptr().cast(), pending.len()) };
        if written > 0 {
            pending.drain(..written as usize);
            continue;
        }
        let error = std::io::Error::last_os_error();
        if error.kind() == std::io::ErrorKind::Interrupted {
            continue;
        }
        if is_retryable(&error) {
            thread::sleep(Duration::from_millis(4));
            continue;
        }
        return Err(error);
    }
    Ok(())
}

fn drain_fd(fd: RawFd) {
    let mut buffer = [0_u8; 64];
    loop {
        let read = unsafe { libc::read(fd, buffer.as_mut_ptr().cast(), buffer.len()) };
        if read <= 0 {
            break;
        }
    }
}

fn append_tail(tail: &mut Vec<u8>, data: &[u8]) {
    tail.extend_from_slice(data);
    if tail.len() > 16_000 {
        let extra = tail.len() - 12_000;
        tail.drain(..extra);
    }
}

fn is_retryable(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
    )
}

fn emit_output(app: &AppHandle, session_id: &str, data: &[u8]) {
    let _ = app.emit(
        "terminal-output",
        TerminalOutput {
            session_id: session_id.to_owned(),
            data: data.to_vec(),
        },
    );
}

fn emit_status(app: &AppHandle, session_id: &str, status: &str, message: Option<String>) {
    let _ = app.emit(
        "terminal-status",
        TerminalStatus {
            session_id: session_id.to_owned(),
            status: status.to_owned(),
            message,
        },
    );
}

fn emit_prompt(
    app: &AppHandle,
    session_id: &str,
    kind: &str,
    prompt: &str,
    message: &str,
    secret: bool,
) {
    let _ = app.emit(
        "terminal-prompt",
        TerminalPrompt {
            session_id: session_id.to_owned(),
            kind: kind.to_owned(),
            prompt: prompt.to_owned(),
            message: message.to_owned(),
            secret,
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn stored_password_is_not_shown_as_a_prompt() {
        let credential = Credential::Password {
            password: "secret".into(),
        };
        assert!(matches!(
            decide_prompt("ubuntu@host's password: ", &credential),
            PromptDecision::Answer(value) if value == "secret"
        ));
        assert!(matches!(
            decide_prompt(
                "Are you sure you want to continue connecting (yes/no/[fingerprint])? ",
                &credential
            ),
            PromptDecision::Ask { kind: "host_key", secret: false }
        ));
    }

    #[test]
    fn missing_passphrase_is_asked_in_the_ui() {
        let credential = Credential::PrivateKey {
            private_key: "key".into(),
            passphrase: None,
        };
        assert!(matches!(
            decide_prompt("Enter passphrase for key '/tmp/id_key': ", &credential),
            PromptDecision::Ask { kind: "interactive", secret: true }
        ));
    }

    #[test]
    fn changed_host_key_is_detected_and_excerpted() {
        let log = "\
debug1: Connecting to db.internal [10.0.0.8] port 22.\n\
@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@\n\
@    WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED!     @\n\
The fingerprint for the ED25519 key sent by the remote host is\n\
SHA256:abcdefghijklmnopqrstuvwxyz0123456789ABCD.\n\
Host key for db.internal has changed and you have requested strict checking.\n";
        assert!(host_key_changed(log));
        let excerpt = host_key_excerpt(log);
        assert!(!excerpt.contains("debug1"));
        assert!(excerpt.contains("SHA256:abcdefghijklmnopqrstuvwxyz0123456789ABCD"));
    }

    #[test]
    fn private_key_sessions_do_not_consult_the_agent() {
        let args = ssh_arguments(
            "db.internal",
            2222,
            "ubuntu",
            &Credential::PrivateKey {
                private_key: "key".into(),
                passphrase: None,
            },
            "UserKnownHostsFile=/tmp/known_hosts",
            Path::new("/tmp/ssh.log"),
            Path::new("/tmp/id_key"),
        );
        assert!(args.windows(2).any(|pair| pair == ["-i", "/tmp/id_key"]));
        assert!(args.iter().any(|arg| arg == "IdentityAgent=none"));
        assert!(args.iter().any(|arg| arg == "StrictHostKeyChecking=ask"));
        assert_eq!(args[args.len() - 3..], ["-l", "ubuntu", "db.internal"]);
    }

    #[test]
    fn agent_sessions_keep_the_user_ssh_config() {
        let args = ssh_arguments(
            "db.internal",
            22,
            "ubuntu",
            &Credential::Agent,
            "UserKnownHostsFile=/tmp/known_hosts",
            Path::new("/tmp/ssh.log"),
            Path::new("/tmp/unused"),
        );
        assert!(!args.iter().any(|arg| arg == "IdentityAgent=none"));
        assert!(!args.windows(2).any(|pair| pair[0] == "-i"));
        assert!(args.iter().any(|arg| arg == "PasswordAuthentication=no"));
    }

    #[test]
    fn pty_runs_a_command() {
        let pty = match open_pty(80, 24) {
            Ok(pty) => pty,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
            Err(error) => panic!("open pty: {error}"),
        };
        set_raw(pty.slave).expect("raw pty");
        let mut command = Command::new("/bin/echo");
        command
            .arg("northstar-pty")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let master = pty.master;
        let slave = pty.slave;
        unsafe {
            command.pre_exec(move || {
                libc::close(master);
                if libc::login_tty(slave) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = command.spawn().expect("spawn echo");
        set_nonblocking(pty.master).expect("nonblocking");
        let started = Instant::now();
        let mut collected = Vec::new();
        while started.elapsed() < Duration::from_secs(2) {
            if let Ok(chunk) = read_available(pty.master) {
                collected.extend(chunk);
            }
            if child.try_wait().ok().flatten().is_some() && !collected.is_empty() {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        let _ = child.wait();
        let text = String::from_utf8_lossy(&collected);
        assert!(
            text.contains("northstar-pty"),
            "pty output was {text:?}"
        );
    }
}
