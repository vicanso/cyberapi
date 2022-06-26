// use tauri::{Env};
use rusqlite::{Connection};
use once_cell::sync::OnceCell;
use std::{path::Path, fs, error::Error, sync::Once};

use crate::util;

static DB_CONN: OnceCell<Connection> = OnceCell::new();

pub fn init_conn() {
    let dir = Path::new(util::get_app_dir() );
    fs::create_dir_all(dir).unwrap();
    let file =  dir.join("my_db.db");
    let conn = Connection::open(file).unwrap();
    DB_CONN.set(conn);
    // return conn;
    ()
}