mod backup;
mod database;
mod error;
mod models;
mod ssh;
mod vault;

use database::Database;
use error::AppResult;
use models::{GroupInput, HostInput, Library};
use ssh::SshManager;
use tauri::{AppHandle, Manager, State};

#[tauri::command]
fn get_library(database: State<'_, Database>, query: Option<String>) -> AppResult<Library> {
    database.library(query.as_deref())
}

#[tauri::command]
fn save_group(database: State<'_, Database>, input: GroupInput) -> AppResult<i64> {
    database.save_group(input)
}

#[tauri::command]
fn delete_group(database: State<'_, Database>, id: i64) -> AppResult<()> {
    database.delete_group(id)
}

#[tauri::command]
fn save_host(database: State<'_, Database>, input: HostInput) -> AppResult<i64> {
    database.save_host(input)
}

#[tauri::command]
fn delete_host(database: State<'_, Database>, id: i64) -> AppResult<()> {
    database.delete_host(id)
}

#[tauri::command]
fn backup_library(database: State<'_, Database>, path: String, password: String) -> AppResult<()> {
    let data = database.export_data()?;
    backup::write(std::path::Path::new(&path), &password, &data)
}

#[tauri::command]
fn restore_library(
    database: State<'_, Database>,
    path: String,
    password: String,
) -> AppResult<String> {
    let data = backup::read(std::path::Path::new(&path), &password)?;
    let (groups, hosts) = database.import_data(data)?;
    Ok(format!("Imported {hosts} hosts and {groups} groups"))
}

#[tauri::command]
fn connect_ssh(
    app: AppHandle,
    database: State<'_, Database>,
    ssh: State<'_, SshManager>,
    host_id: i64,
    session_id: String,
    cols: u32,
    rows: u32,
) -> AppResult<String> {
    ssh.connect(app, &database, host_id, session_id, cols, rows)
}

#[tauri::command]
fn write_ssh(ssh: State<'_, SshManager>, session_id: String, data: Vec<u8>) -> AppResult<()> {
    ssh.input(&session_id, data)
}

#[tauri::command]
fn resize_ssh(
    ssh: State<'_, SshManager>,
    session_id: String,
    cols: u32,
    rows: u32,
) -> AppResult<()> {
    ssh.resize(&session_id, cols, rows)
}

#[tauri::command]
fn disconnect_ssh(ssh: State<'_, SshManager>, session_id: String) -> AppResult<()> {
    ssh.disconnect(&session_id)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            app.manage(Database::open(app.handle())?);
            app.manage(SshManager::new());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_library,
            save_group,
            delete_group,
            save_host,
            delete_host,
            backup_library,
            restore_library,
            connect_ssh,
            write_ssh,
            resize_ssh,
            disconnect_ssh
        ])
        .run(tauri::generate_context!())
        .expect("error while running Northstar SSH");
}
