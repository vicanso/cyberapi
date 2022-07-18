// use tauri::{Env};
use once_cell::sync::OnceCell;
use rusqlite::{params_from_iter, Connection};
use std::{fs, path::Path, sync::Mutex, sync::MutexGuard};

use crate::util;

static DB_CONN: OnceCell<Mutex<Connection>> = OnceCell::new();
fn init_conn() -> &'static Mutex<Connection> {
    // 初始化数据库
    DB_CONN.get_or_init(|| {
        let dir = Path::new(util::get_app_dir());
        fs::create_dir_all(dir).unwrap();
        let file = dir.join("my_db.db");
        let conn = Connection::open(file).unwrap();
        Mutex::new(conn)
    })
}

pub fn get_conn() -> MutexGuard<'static, Connection> {
    let result = init_conn();
    result.lock().unwrap()
}

pub fn add_or_update_record(
    table: &str,
    keys: Vec<String>,
    values: Vec<String>,
) -> Result<usize, rusqlite::Error> {
    let conn = get_conn();

    let p = params_from_iter(values);

    let mut params_values = Vec::new();
    for n in 0..keys.len() {
        params_values.push(format!("?{}", n + 1));
    }
    let sql = format!(
        "INSERT OR REPLACE INTO {} ({}) VALUES ({})",
        table,
        keys.join(", "),
        params_values.join(", "),
    );
    conn.execute(&sql, p)
}

pub trait NewFromRow<T> {
    fn from_row(data: &rusqlite::Row) -> Result<T, rusqlite::Error>;
}

pub fn list_records<T: NewFromRow<T>>(
    table: &str,
    keys: Vec<String>,
) -> Result<Vec<T>, rusqlite::Error> {
    let conn = get_conn();

    let sql = format!("SELECT {} FROM {}", keys.join(", "), table);
    let mut statement = conn.prepare(&sql)?;
    let mut rows = statement.query([])?;

    let mut result = Vec::new();
    let mut done = false;
    while !done {
        let item = rows.next()?;
        match item {
            Some(data) => result.push(T::from_row(data)?),
            None => done = true,
        }
    }
    Ok(result)
}

pub fn delete_by_ids(table: &str, ids: Vec<String>) -> Result<usize, rusqlite::Error> {
    if ids.is_empty() {
        return Ok(0);
    }
    let sql = format!("DELETE FROM {} WHERE id IN ({})", table, ids.join(", "));
    get_conn().execute(&sql, [])
}
