use std::{collections::HashMap, fs, path::PathBuf, sync::Mutex};

use rusqlite::{params, Connection, OptionalExtension};
use tauri::{AppHandle, Manager};

use crate::{
    error::{AppError, AppResult},
    models::{
        BackupData, BackupGroup, BackupHost, Credential, Group, GroupInput, Host, HostInput,
        Library,
    },
    vault::Vault,
};

pub struct Database {
    connection: Mutex<Connection>,
    vault: Vault,
}

impl Database {
    pub fn open(app: &AppHandle) -> AppResult<Self> {
        let data_dir = app
            .path()
            .app_data_dir()
            .map_err(|error| AppError::Internal(error.to_string()))?;
        fs::create_dir_all(&data_dir)?;
        let connection = Connection::open(database_path(data_dir))?;
        Self::from_connection(connection, Vault::load()?)
    }

    fn from_connection(connection: Connection, vault: Vault) -> AppResult<Self> {
        connection.execute_batch(
            "
            PRAGMA foreign_keys = ON;
            PRAGMA journal_mode = WAL;

            CREATE TABLE IF NOT EXISTS groups (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                parent_id INTEGER REFERENCES groups(id) ON DELETE CASCADE,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );

            CREATE TABLE IF NOT EXISTS hosts (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                address TEXT NOT NULL,
                port INTEGER NOT NULL DEFAULT 22,
                username TEXT NOT NULL,
                group_id INTEGER REFERENCES groups(id) ON DELETE SET NULL,
                auth_type TEXT NOT NULL CHECK(auth_type IN ('password', 'private_key')),
                encrypted_credential BLOB NOT NULL,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );

            CREATE INDEX IF NOT EXISTS idx_groups_parent_id ON groups(parent_id);
            CREATE INDEX IF NOT EXISTS idx_hosts_group_id ON hosts(group_id);
            CREATE INDEX IF NOT EXISTS idx_hosts_name ON hosts(name);
            ",
        )?;

        Ok(Self {
            connection: Mutex::new(connection),
            vault,
        })
    }

    pub fn library(&self, query: Option<&str>) -> AppResult<Library> {
        let connection = self.lock()?;
        let groups = {
            let mut statement = connection
                .prepare("SELECT id, name, parent_id FROM groups ORDER BY name COLLATE NOCASE")?;
            let groups = statement
                .query_map([], |row| {
                    Ok(Group {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        parent_id: row.get(2)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            groups
        };

        let pattern = format!("%{}%", query.unwrap_or_default().trim());
        let mut statement = connection.prepare(
            "SELECT id, name, address, port, username, group_id, auth_type
             FROM hosts
             WHERE name LIKE ?1 COLLATE NOCASE
                OR address LIKE ?1 COLLATE NOCASE
                OR username LIKE ?1 COLLATE NOCASE
             ORDER BY name COLLATE NOCASE",
        )?;
        let hosts = statement
            .query_map([pattern], |row| {
                Ok(Host {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    address: row.get(2)?,
                    port: row.get(3)?,
                    username: row.get(4)?,
                    group_id: row.get(5)?,
                    auth_type: row.get(6)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Library { groups, hosts })
    }

    pub fn save_group(&self, input: GroupInput) -> AppResult<i64> {
        let name = required(input.name, "Group name")?;
        let connection = self.lock()?;
        if let Some(id) = input.id {
            let creates_cycle = match input.parent_id {
                Some(parent) if parent == id => true,
                Some(parent) => is_descendant(&connection, parent, id)?,
                None => false,
            };
            if creates_cycle {
                return Err(AppError::InvalidInput(
                    "A group cannot be nested inside itself".into(),
                ));
            }
            let changed = connection.execute(
                "UPDATE groups SET name = ?1, parent_id = ?2, updated_at = CURRENT_TIMESTAMP WHERE id = ?3",
                params![name, input.parent_id, id],
            )?;
            return if changed == 0 {
                Err(AppError::NotFound)
            } else {
                Ok(id)
            };
        }

        connection.execute(
            "INSERT INTO groups (name, parent_id) VALUES (?1, ?2)",
            params![name, input.parent_id],
        )?;
        Ok(connection.last_insert_rowid())
    }

    pub fn delete_group(&self, id: i64) -> AppResult<()> {
        let connection = self.lock()?;
        connection.execute("DELETE FROM groups WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn save_host(&self, input: HostInput) -> AppResult<i64> {
        let new_credential = self.credential_from_input(&input)?;
        let name = required(input.name, "Host name")?;
        let address = required(input.address, "Address")?;
        let username = required(input.username, "Username")?;
        if input.port == 0 {
            return Err(AppError::InvalidInput(
                "Port must be between 1 and 65535".into(),
            ));
        }
        if input.auth_type != "password" && input.auth_type != "private_key" {
            return Err(AppError::InvalidInput(
                "Unsupported authentication type".into(),
            ));
        }

        let connection = self.lock()?;
        if let Some(id) = input.id {
            let encrypted = match new_credential {
                Some(value) => self.encrypt_credential(&value)?,
                None => connection
                    .query_row(
                        "SELECT encrypted_credential FROM hosts WHERE id = ?1 AND auth_type = ?2",
                        params![id, input.auth_type],
                        |row| row.get(0),
                    )
                    .optional()?
                    .ok_or_else(|| {
                        AppError::InvalidInput(
                            "Enter the credential when changing authentication type".into(),
                        )
                    })?,
            };
            let changed = connection.execute(
                "UPDATE hosts SET name=?1, address=?2, port=?3, username=?4, group_id=?5,
                 auth_type=?6, encrypted_credential=?7, updated_at=CURRENT_TIMESTAMP WHERE id=?8",
                params![
                    name,
                    address,
                    input.port,
                    username,
                    input.group_id,
                    input.auth_type,
                    encrypted,
                    id
                ],
            )?;
            return if changed == 0 {
                Err(AppError::NotFound)
            } else {
                Ok(id)
            };
        }

        let credential = new_credential
            .ok_or_else(|| AppError::InvalidInput("A credential is required".into()))?;
        let encrypted = self.encrypt_credential(&credential)?;
        connection.execute(
            "INSERT INTO hosts
             (name, address, port, username, group_id, auth_type, encrypted_credential)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                name,
                address,
                input.port,
                username,
                input.group_id,
                input.auth_type,
                encrypted
            ],
        )?;
        Ok(connection.last_insert_rowid())
    }

    pub fn delete_host(&self, id: i64) -> AppResult<()> {
        let connection = self.lock()?;
        connection.execute("DELETE FROM hosts WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn host_with_credential(&self, id: i64) -> AppResult<(Host, Credential)> {
        let connection = self.lock()?;
        let (host, encrypted): (Host, Vec<u8>) = connection
            .query_row(
                "SELECT id, name, address, port, username, group_id, auth_type, encrypted_credential
                 FROM hosts WHERE id = ?1",
                [id],
                |row| {
                    Ok((
                        Host {
                            id: row.get(0)?,
                            name: row.get(1)?,
                            address: row.get(2)?,
                            port: row.get(3)?,
                            username: row.get(4)?,
                            group_id: row.get(5)?,
                            auth_type: row.get(6)?,
                        },
                        row.get(7)?,
                    ))
                },
            )
            .optional()?
            .ok_or(AppError::NotFound)?;
        let decrypted = self.vault.decrypt(&encrypted)?;
        let credential = serde_json::from_slice(&decrypted)
            .map_err(|error| AppError::Internal(error.to_string()))?;
        Ok((host, credential))
    }

    pub fn export_data(&self) -> AppResult<BackupData> {
        let connection = self.lock()?;
        let groups = {
            let mut statement =
                connection.prepare("SELECT id, name, parent_id FROM groups ORDER BY id")?;
            let groups = statement
                .query_map([], |row| {
                    Ok(BackupGroup {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        parent_id: row.get(2)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            groups
        };
        let hosts = {
            let mut statement = connection.prepare(
                "SELECT name, address, port, username, group_id, auth_type, encrypted_credential
                 FROM hosts ORDER BY id",
            )?;
            let rows = statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, u16>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, Option<i64>>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, Vec<u8>>(6)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            rows.into_iter()
                .map(
                    |(name, address, port, username, group_id, auth_type, encrypted)| {
                        let decrypted = self.vault.decrypt(&encrypted)?;
                        let credential = serde_json::from_slice(&decrypted)
                            .map_err(|error| AppError::Internal(error.to_string()))?;
                        Ok(BackupHost {
                            name,
                            address,
                            port,
                            username,
                            group_id,
                            auth_type,
                            credential,
                        })
                    },
                )
                .collect::<AppResult<Vec<_>>>()?
        };
        Ok(BackupData {
            version: 1,
            groups,
            hosts,
        })
    }

    pub fn import_data(&self, data: BackupData) -> AppResult<(usize, usize)> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        let mut group_ids = HashMap::new();
        let mut remaining = data.groups;
        let group_count = remaining.len();

        while !remaining.is_empty() {
            let previous_len = remaining.len();
            let mut next = Vec::new();
            for group in remaining {
                let new_parent_id = match group.parent_id {
                    None => Some(None),
                    Some(parent_id) => group_ids.get(&parent_id).copied().map(Some),
                };
                if let Some(parent_id) = new_parent_id {
                    transaction.execute(
                        "INSERT INTO groups (name, parent_id) VALUES (?1, ?2)",
                        params![group.name, parent_id],
                    )?;
                    group_ids.insert(group.id, transaction.last_insert_rowid());
                } else {
                    next.push(group);
                }
            }
            if next.len() == previous_len {
                return Err(AppError::InvalidInput(
                    "The backup contains an invalid group hierarchy".into(),
                ));
            }
            remaining = next;
        }

        let host_count = data.hosts.len();
        for host in data.hosts {
            let credential_type = match &host.credential {
                Credential::Password { .. } => "password",
                Credential::PrivateKey { .. } => "private_key",
            };
            if host.auth_type != credential_type {
                return Err(AppError::InvalidInput(
                    "The backup contains an invalid host credential".into(),
                ));
            }
            let encrypted = self.encrypt_credential(&host.credential)?;
            let group_id = host
                .group_id
                .and_then(|original_id| group_ids.get(&original_id).copied());
            transaction.execute(
                "INSERT INTO hosts
                 (name, address, port, username, group_id, auth_type, encrypted_credential)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    host.name,
                    host.address,
                    host.port,
                    host.username,
                    group_id,
                    host.auth_type,
                    encrypted
                ],
            )?;
        }
        transaction.commit()?;
        Ok((group_count, host_count))
    }

    fn credential_from_input(&self, input: &HostInput) -> AppResult<Option<Credential>> {
        match input.auth_type.as_str() {
            "password" => Ok(input
                .password
                .as_ref()
                .filter(|password| !password.is_empty())
                .map(|password| Credential::Password {
                    password: password.clone(),
                })),
            "private_key" => {
                let Some(path) = input
                    .private_key_path
                    .as_ref()
                    .filter(|path| !path.is_empty())
                else {
                    return Ok(None);
                };
                let private_key = fs::read_to_string(path)?;
                Ok(Some(Credential::PrivateKey {
                    private_key,
                    passphrase: input
                        .private_key_passphrase
                        .clone()
                        .filter(|value| !value.is_empty()),
                }))
            }
            _ => Ok(None),
        }
    }

    fn encrypt_credential(&self, credential: &Credential) -> AppResult<Vec<u8>> {
        let serialized = serde_json::to_vec(credential)
            .map_err(|error| AppError::Internal(error.to_string()))?;
        self.vault.encrypt(&serialized)
    }

    fn lock(&self) -> AppResult<std::sync::MutexGuard<'_, Connection>> {
        self.connection
            .lock()
            .map_err(|_| AppError::Internal("Database lock was poisoned".into()))
    }
}

fn required(value: String, label: &str) -> AppResult<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        Err(AppError::InvalidInput(format!("{label} is required")))
    } else {
        Ok(trimmed.to_owned())
    }
}

fn database_path(mut data_dir: PathBuf) -> PathBuf {
    data_dir.push("northstar.db");
    data_dir
}

fn is_descendant(connection: &Connection, possible_child: i64, group_id: i64) -> AppResult<bool> {
    let found: Option<i64> = connection
        .query_row(
            "WITH RECURSIVE descendants(id) AS (
                SELECT id FROM groups WHERE parent_id = ?1
                UNION ALL
                SELECT groups.id FROM groups JOIN descendants ON groups.parent_id = descendants.id
             )
             SELECT id FROM descendants WHERE id = ?2 LIMIT 1",
            params![group_id, possible_child],
            |row| row.get(0),
        )
        .optional()?;
    Ok(found.is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_database() -> Database {
        Database::from_connection(
            Connection::open_in_memory().expect("open in-memory database"),
            Vault::for_test(),
        )
        .expect("initialize database")
    }

    #[test]
    fn manages_nested_groups_hosts_search_and_encrypted_credentials() {
        let database = test_database();
        let parent_id = database
            .save_group(GroupInput {
                id: None,
                name: "Production".into(),
                parent_id: None,
            })
            .expect("save parent group");
        let child_id = database
            .save_group(GroupInput {
                id: None,
                name: "Web".into(),
                parent_id: Some(parent_id),
            })
            .expect("save child group");

        let host_id = database
            .save_host(HostInput {
                id: None,
                name: "Production API".into(),
                address: "api.example.com".into(),
                port: 22,
                username: "ubuntu".into(),
                group_id: Some(child_id),
                auth_type: "password".into(),
                password: Some("correct horse battery staple".into()),
                private_key_path: None,
                private_key_passphrase: None,
            })
            .expect("save password host");

        let library = database.library(Some("production")).expect("search hosts");
        assert_eq!(library.hosts.len(), 1);
        assert_eq!(library.hosts[0].id, host_id);
        assert_eq!(library.groups.len(), 2);

        let (_, credential) = database
            .host_with_credential(host_id)
            .expect("decrypt credential");
        assert!(matches!(
            credential,
            Credential::Password { ref password } if password == "correct horse battery staple"
        ));

        let encrypted: Vec<u8> = database
            .lock()
            .expect("lock database")
            .query_row(
                "SELECT encrypted_credential FROM hosts WHERE id = ?1",
                [host_id],
                |row| row.get(0),
            )
            .expect("read encrypted value");
        assert!(!String::from_utf8_lossy(&encrypted).contains("correct horse"));

        let cycle = database.save_group(GroupInput {
            id: Some(parent_id),
            name: "Production".into(),
            parent_id: Some(child_id),
        });
        assert!(matches!(cycle, Err(AppError::InvalidInput(_))));

        database.delete_group(parent_id).expect("delete group tree");
        let library = database.library(None).expect("reload library");
        assert!(library.groups.is_empty());
        assert_eq!(library.hosts[0].group_id, None);

        database.delete_host(host_id).expect("delete host");
        assert!(database
            .library(None)
            .expect("reload hosts")
            .hosts
            .is_empty());
    }

    #[test]
    fn exports_and_merges_data_into_another_database() {
        let source = test_database();
        let group_id = source
            .save_group(GroupInput {
                id: None,
                name: "Imported group".into(),
                parent_id: None,
            })
            .expect("save source group");
        source
            .save_host(HostInput {
                id: None,
                name: "Imported host".into(),
                address: "import.example.com".into(),
                port: 22,
                username: "deployer".into(),
                group_id: Some(group_id),
                auth_type: "password".into(),
                password: Some("portable-secret".into()),
                private_key_path: None,
                private_key_passphrase: None,
            })
            .expect("save source host");

        let target = test_database();
        target
            .save_group(GroupInput {
                id: None,
                name: "Existing group".into(),
                parent_id: None,
            })
            .expect("save existing target data");
        let data = source.export_data().expect("export source data");
        assert_eq!(
            target.import_data(data).expect("import source data"),
            (1, 1)
        );

        let library = target.library(None).expect("read merged library");
        assert_eq!(library.groups.len(), 2);
        assert_eq!(library.hosts.len(), 1);
        let (_, credential) = target
            .host_with_credential(library.hosts[0].id)
            .expect("decrypt imported credential");
        assert!(matches!(
            credential,
            Credential::Password { ref password } if password == "portable-secret"
        ));
    }

    #[test]
    fn imports_and_encrypts_private_key_contents() {
        let database = test_database();
        let key_contents = "-----BEGIN OPENSSH PRIVATE KEY-----\ntest-key-material\n-----END OPENSSH PRIVATE KEY-----";
        let key_path =
            std::env::temp_dir().join(format!("northstar-test-key-{}", std::process::id()));
        fs::write(&key_path, key_contents).expect("write temporary private key");

        let host_id = database
            .save_host(HostInput {
                id: None,
                name: "Key host".into(),
                address: "key.example.com".into(),
                port: 2222,
                username: "admin".into(),
                group_id: None,
                auth_type: "private_key".into(),
                password: None,
                private_key_path: Some(key_path.to_string_lossy().into_owned()),
                private_key_passphrase: Some("key-passphrase".into()),
            })
            .expect("save private-key host");
        fs::remove_file(key_path).expect("remove source key");

        let (_, credential) = database
            .host_with_credential(host_id)
            .expect("decrypt private key");
        assert!(matches!(
            credential,
            Credential::PrivateKey {
                ref private_key,
                passphrase: Some(ref passphrase)
            } if private_key == key_contents && passphrase == "key-passphrase"
        ));

        let encrypted: Vec<u8> = database
            .lock()
            .expect("lock database")
            .query_row(
                "SELECT encrypted_credential FROM hosts WHERE id = ?1",
                [host_id],
                |row| row.get(0),
            )
            .expect("read encrypted private key");
        let stored = String::from_utf8_lossy(&encrypted);
        assert!(!stored.contains("test-key-material"));
        assert!(!stored.contains("key-passphrase"));
    }
}
