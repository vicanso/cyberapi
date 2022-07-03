// use tauri::{Env};
use once_cell::sync::OnceCell;
use rusqlite::{params_from_iter, Connection};
use std::{error, fmt, fs, io, path::Path, sync::Mutex, sync::MutexGuard};

use crate::util;

#[derive(Debug)]
pub enum DatabaseError {
    IO(io::Error),
    Sqlite(rusqlite::Error),
}

impl fmt::Display for DatabaseError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            DatabaseError::IO(ref e) => e.fmt(f),
            DatabaseError::Sqlite(ref e) => e.fmt(f),
        }
    }
}

impl error::Error for DatabaseError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match *self {
            DatabaseError::IO(ref e) => Some(e),
            DatabaseError::Sqlite(ref e) => Some(e),
        }
    }
}
impl From<io::Error> for DatabaseError {
    fn from(err: io::Error) -> DatabaseError {
        DatabaseError::IO(err)
    }
}
impl From<rusqlite::Error> for DatabaseError {
    fn from(err: rusqlite::Error) -> DatabaseError {
        DatabaseError::Sqlite(err)
    }
}

fn init_conn() -> &'static Mutex<Connection> {
    static DB_CONN: OnceCell<Mutex<Connection>> = OnceCell::new();
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
