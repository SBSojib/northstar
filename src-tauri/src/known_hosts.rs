use std::{
    collections::HashSet,
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
    process::Command,
};

use tauri::{AppHandle, Manager};

use crate::error::{AppError, AppResult};

pub fn ensure(app: &AppHandle) -> AppResult<PathBuf> {
    let path = northstar_file(app)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    if !path.exists() {
        let file = OpenOptions::new().create(true).write(true).open(&path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(fs::Permissions::from_mode(0o600))?;
        }
    }
    Ok(path)
}

pub fn read(app: &AppHandle) -> AppResult<String> {
    let path = northstar_file(app)?;
    if !path.exists() {
        return Ok(String::new());
    }
    Ok(fs::read_to_string(path)?)
}

pub fn merge(app: &AppHandle, incoming: &str) -> AppResult<usize> {
    if incoming.trim().is_empty() {
        return Ok(0);
    }
    merge_into(&ensure(app)?, incoming)
}

pub fn files_for_session(app: &AppHandle) -> AppResult<Vec<PathBuf>> {
    let mut files = vec![ensure(app)?];
    if let Some(user_file) = user_file() {
        if user_file != files[0] {
            files.push(user_file);
        }
    }
    Ok(files)
}

pub fn option_value(files: &[PathBuf]) -> String {
    let joined = files
        .iter()
        .map(|path| quote_known_hosts_path(&path.display().to_string()))
        .collect::<Vec<_>>()
        .join(" ");
    format!("UserKnownHostsFile={joined}")
}

pub fn forget(app: &AppHandle, address: &str, port: u16) -> AppResult<()> {
    let mut files = vec![ensure(app)?];
    if let Some(user_file) = user_file() {
        if user_file != files[0] {
            files.push(user_file);
        }
    }
    for file in files {
        for target in removal_targets(address, port) {
            remove_host(&file, &target)?;
        }
    }
    Ok(())
}

pub fn removal_targets(address: &str, port: u16) -> Vec<String> {
    let mut targets = vec![address.to_owned(), format!("[{address}]:{port}")];
    if port == 22 {
        targets.push(format!("{address}:22"));
    }
    targets
}

pub fn merge_into(path: &Path, incoming: &str) -> AppResult<usize> {
    let existing = if path.exists() {
        fs::read_to_string(path)?
    } else {
        String::new()
    };
    let mut seen = existing
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect::<HashSet<_>>();
    let mut added = Vec::new();
    for line in incoming.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || !seen.insert(trimmed.to_owned()) {
            continue;
        }
        added.push(trimmed.to_owned());
    }
    if added.is_empty() {
        return Ok(0);
    }
    let mut output = existing;
    if !output.is_empty() && !output.ends_with('\n') {
        output.push('\n');
    }
    for line in &added {
        output.push_str(line);
        output.push('\n');
    }
    fs::write(path, output)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(added.len())
}

fn northstar_file(app: &AppHandle) -> AppResult<PathBuf> {
    let mut path = app
        .path()
        .app_data_dir()
        .map_err(|error| AppError::Internal(error.to_string()))?;
    path.push("known_hosts");
    Ok(path)
}

fn user_file() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    let path = PathBuf::from(home).join(".ssh").join("known_hosts");
    path.is_file().then_some(path)
}

fn quote_known_hosts_path(path: &str) -> String {
    if path.contains([' ', '\t', '"']) {
        format!("\"{}\"", path.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        path.to_owned()
    }
}

fn remove_host(file: &Path, host: &str) -> AppResult<()> {
    if !file.exists() {
        return Ok(());
    }
    let output = Command::new("ssh-keygen")
        .arg("-f")
        .arg(file)
        .arg("-R")
        .arg(host)
        .output()?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.to_ascii_lowercase().contains("not found") {
        return Ok(());
    }
    Err(AppError::InvalidInput(format!(
        "Could not update known hosts: {}",
        stderr.trim()
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_only_new_known_host_lines() {
        let path = std::env::temp_dir().join(format!("northstar-known-hosts-{}", std::process::id()));
        let _ = fs::remove_file(&path);
        assert_eq!(
            merge_into(&path, "kept.example ssh-ed25519 AAAAkept\n").expect("first merge"),
            1
        );
        assert_eq!(
            merge_into(
                &path,
                "kept.example ssh-ed25519 AAAAkept\nnew.example ssh-ed25519 AAAAnew\n"
            )
            .expect("second merge"),
            1
        );
        let stored = fs::read_to_string(&path).expect("read merged file");
        assert!(stored.contains("kept.example"));
        assert!(stored.contains("new.example"));
        assert_eq!(stored.lines().filter(|line| !line.is_empty()).count(), 2);
        fs::remove_file(path).expect("remove test known_hosts");
    }

    #[test]
    fn removal_targets_cover_port_forms() {
        assert_eq!(
            removal_targets("db.internal", 22),
            vec![
                "db.internal".to_owned(),
                "[db.internal]:22".to_owned(),
                "db.internal:22".to_owned()
            ]
        );
        assert_eq!(
            removal_targets("db.internal", 2222),
            vec!["db.internal".to_owned(), "[db.internal]:2222".to_owned()]
        );
    }

    #[test]
    fn ssh_keygen_forgets_a_saved_host() {
        let dir = std::env::temp_dir().join(format!(
            "northstar-known-host-remove-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("known_hosts");
        fs::write(
            &path,
            "forget.example ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIFakeKeyMaterialForTestOnly\n",
        )
        .expect("write known_hosts");
        remove_host(&path, "forget.example").expect("remove host");
        let stored = fs::read_to_string(&path).unwrap_or_default();
        assert!(
            !stored.contains("forget.example"),
            "known_hosts still contains the host: {stored}"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn quotes_known_hosts_paths_that_contain_spaces() {
        let option = option_value(&[
            PathBuf::from("/tmp/northstar/known_hosts"),
            PathBuf::from("/home/first last/.ssh/known_hosts"),
        ]);
        assert_eq!(
            option,
            "UserKnownHostsFile=/tmp/northstar/known_hosts \"/home/first last/.ssh/known_hosts\""
        );
    }
}
