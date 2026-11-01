use std::{
    collections::HashMap,
    io::{ErrorKind, Read, Write},
    net::{TcpStream, ToSocketAddrs},
    sync::{mpsc, Arc, Mutex},
    thread,
    time::Duration,
};

/// Long enough for Linux TCP SYN retransmits. The kernel gives a new connection
/// about two minutes; 15 seconds was failing with "connection timed out" on a
/// lossy path while an already-open `ssh` session stayed up.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(120);
/// SSH-level keepalive. Traffic in either direction keeps NAT state alive.
const KEEPALIVE_INTERVAL_SECS: u32 = 15;
/// libssh2's LIBSSH2_ERROR_EAGAIN. A non-blocking call must be retried.
const LIBSSH2_ERROR_EAGAIN: i32 = -37;

use crate::{
    database::Database,
    error::{AppError, AppResult},
    models::{Credential, TerminalOutput, TerminalStatus},
};
use ssh2::Session;
use tauri::{AppHandle, Emitter};

enum SessionCommand {
    Input(Vec<u8>),
    Resize(u32, u32),
    Disconnect,
}

pub struct SshManager {
    sessions: Arc<Mutex<HashMap<String, mpsc::Sender<SessionCommand>>>>,
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
        let (sender, receiver) = mpsc::channel();
        self.sessions
            .lock()
            .map_err(|_| AppError::Internal("Session lock was poisoned".into()))?
            .insert(session_id.clone(), sender);

        let thread_session_id = session_id.clone();
        let sessions = Arc::clone(&self.sessions);
        thread::spawn(move || {
            emit_status(&app, &thread_session_id, "connecting", None);
            let result = run_session(
                &app,
                &thread_session_id,
                &host.address,
                host.port,
                &host.username,
                credential,
                cols,
                rows,
                receiver,
            );
            match result {
                Ok(()) => emit_status(&app, &thread_session_id, "closed", None),
                Err(error) => {
                    emit_status(&app, &thread_session_id, "error", Some(error.to_string()))
                }
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

    pub fn disconnect(&self, session_id: &str) -> AppResult<()> {
        let sender = self
            .sessions
            .lock()
            .map_err(|_| AppError::Internal("Session lock was poisoned".into()))?
            .remove(session_id)
            .ok_or(AppError::SessionNotFound)?;
        sender
            .send(SessionCommand::Disconnect)
            .map_err(|_| AppError::SessionNotFound)
    }

    fn send(&self, session_id: &str, command: SessionCommand) -> AppResult<()> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| AppError::Internal("Session lock was poisoned".into()))?;
        let result = sessions
            .get(session_id)
            .ok_or(AppError::SessionNotFound)?
            .send(command);
        if result.is_err() {
            sessions.remove(session_id);
            return Err(AppError::SessionNotFound);
        }
        Ok(())
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
    receiver: mpsc::Receiver<SessionCommand>,
) -> AppResult<()> {
    let tcp = connect_tcp(address, port)?;

    let mut ssh = Session::new()?;
    ssh.set_tcp_stream(tcp);
    // Blocking API calls wait on poll, not on a socket read deadline.
    ssh.set_timeout(0);
    ssh.handshake()?;
    match credential {
        Credential::Password { password } => ssh.userauth_password(username, &password)?,
        Credential::PrivateKey {
            private_key,
            passphrase,
        } => ssh.userauth_pubkey_memory(username, None, &private_key, passphrase.as_deref())?,
    }
    if !ssh.authenticated() {
        return Err(AppError::InvalidInput("Authentication failed".into()));
    }

    let mut channel = ssh.channel_session()?;
    channel.request_pty("xterm-256color", None, Some((cols, rows, 0, 0)))?;
    channel.shell()?;
    // want_reply so a dead path fails the send instead of looking connected.
    ssh.set_keepalive(true, KEEPALIVE_INTERVAL_SECS);
    ssh.set_blocking(false);
    emit_status(app, session_id, "connected", None);

    let mut buffer = [0_u8; 16 * 1024];
    loop {
        while let Ok(command) = receiver.try_recv() {
            match command {
                SessionCommand::Input(data) => write_nonblocking(&mut channel, &data)?,
                SessionCommand::Resize(cols, rows) => {
                    finish_ssh(|| channel.request_pty_size(cols, rows, None, None))?
                }
                SessionCommand::Disconnect => {
                    let _ = channel.close();
                    return Ok(());
                }
            }
        }

        // Idle shells send nothing. Without this, firewalls drop the TCP
        // session that an interactive terminal keeps warm by typing.
        ssh.keepalive_send()?;

        match channel.read(&mut buffer) {
            Ok(0) if channel.eof() => break,
            Ok(0) => {}
            Ok(count) => emit_output(app, session_id, &buffer[..count]),
            Err(error) if is_retryable_io(&error) => {}
            Err(error) => return Err(error.into()),
        }

        match channel.stderr().read(&mut buffer) {
            Ok(0) => {}
            Ok(count) => emit_output(app, session_id, &buffer[..count]),
            Err(error) if is_retryable_io(&error) => {}
            Err(error) => return Err(error.into()),
        }

        if channel.eof() {
            break;
        }
        thread::sleep(Duration::from_millis(8));
    }

    ssh.set_blocking(true);
    channel.wait_close()?;
    Ok(())
}

fn connect_tcp(address: &str, port: u16) -> AppResult<TcpStream> {
    let mut last_error = None;
    let mut saw_address = false;
    for socket_address in (address, port).to_socket_addrs()? {
        saw_address = true;
        match TcpStream::connect_timeout(&socket_address, CONNECT_TIMEOUT) {
            Ok(stream) => {
                prepare_socket(&stream)?;
                return Ok(stream);
            }
            Err(error) => last_error = Some(error),
        }
    }
    if !saw_address {
        return Err(AppError::InvalidInput(
            "Could not resolve host address".into(),
        ));
    }
    Err(last_error.expect("a failed attempt was recorded").into())
}

fn prepare_socket(stream: &TcpStream) -> std::io::Result<()> {
    stream.set_nodelay(true)?;
    // An idle prompt is normal. A socket read/write deadline is reported as a
    // timeout on some platforms, which used to drop the session.
    stream.set_read_timeout(None)?;
    stream.set_write_timeout(None)?;
    enable_tcp_keepalive(stream)
}

fn enable_tcp_keepalive(stream: &TcpStream) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        let fd = stream.as_raw_fd();
        set_int_option(fd, libc::SOL_SOCKET, libc::SO_KEEPALIVE, 1)?;
        #[cfg(target_os = "linux")]
        {
            set_int_option(fd, libc::IPPROTO_TCP, libc::TCP_KEEPIDLE, 30)?;
            set_int_option(fd, libc::IPPROTO_TCP, libc::TCP_KEEPINTVL, 10)?;
            set_int_option(fd, libc::IPPROTO_TCP, libc::TCP_KEEPCNT, 3)?;
        }
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        {
            set_int_option(fd, libc::IPPROTO_TCP, libc::TCP_KEEPALIVE, 30)?;
        }
    }
    #[cfg(not(unix))]
    {
        let _ = stream;
    }
    Ok(())
}

#[cfg(unix)]
fn set_int_option(
    fd: std::os::fd::RawFd,
    level: libc::c_int,
    name: libc::c_int,
    value: libc::c_int,
) -> std::io::Result<()> {
    let result = unsafe {
        libc::setsockopt(
            fd,
            level,
            name,
            &value as *const libc::c_int as *const libc::c_void,
            std::mem::size_of_val(&value) as libc::socklen_t,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

fn is_retryable_io(error: &std::io::Error) -> bool {
    matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted)
}

fn is_ssh_retryable(error: &ssh2::Error) -> bool {
    matches!(error.code(), ssh2::ErrorCode::Session(LIBSSH2_ERROR_EAGAIN))
}

fn finish_ssh(mut operation: impl FnMut() -> Result<(), ssh2::Error>) -> AppResult<()> {
    loop {
        match operation() {
            Ok(()) => return Ok(()),
            Err(error) if is_ssh_retryable(&error) => thread::sleep(Duration::from_millis(4)),
            Err(error) => return Err(error.into()),
        }
    }
}

fn write_nonblocking(channel: &mut ssh2::Channel, mut data: &[u8]) -> AppResult<()> {
    while !data.is_empty() {
        match channel.write(data) {
            Ok(0) => thread::sleep(Duration::from_millis(4)),
            Ok(count) => data = &data[count..],
            Err(error) if is_retryable_io(&error) => thread::sleep(Duration::from_millis(4)),
            Err(error) => return Err(error.into()),
        }
    }
    loop {
        match channel.flush() {
            Ok(()) => return Ok(()),
            Err(error) if is_retryable_io(&error) => thread::sleep(Duration::from_millis(4)),
            Err(error) => return Err(error.into()),
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::time::Instant;

    #[test]
    fn connect_timeout_covers_syn_retransmits() {
        assert!(CONNECT_TIMEOUT >= Duration::from_secs(60));
    }

    #[test]
    fn eagain_does_not_end_the_session() {
        let error = ssh2::Error::new(
            ssh2::ErrorCode::Session(LIBSSH2_ERROR_EAGAIN),
            "operation would block",
        );
        assert!(is_ssh_retryable(&error));
        let fatal = ssh2::Error::new(ssh2::ErrorCode::Session(-9), "timed out");
        assert!(!is_ssh_retryable(&fatal));
    }

    #[test]
    fn connects_to_a_local_listener() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let stream = connect_tcp("127.0.0.1", port).expect("connect");
        assert_eq!(stream.peer_addr().expect("peer").port(), port);
        assert!(stream.nodelay().expect("nodelay"));
        assert_eq!(stream.read_timeout().expect("read timeout"), None);
        assert_eq!(stream.write_timeout().expect("write timeout"), None);
    }

    #[test]
    fn refused_port_fails_without_waiting_out_the_timeout() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        drop(listener);
        let started = Instant::now();
        let error = connect_tcp("127.0.0.1", port).expect_err("closed port");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "refused connect waited {:?}: {error}",
            started.elapsed()
        );
    }
}
