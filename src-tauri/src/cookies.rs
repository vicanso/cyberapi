use cookie_store::CookieStore;
use once_cell::sync::OnceCell;
use serde::{Deserialize, Serialize};
use std::{
    fs::File, fs::OpenOptions, io::BufReader, io::BufWriter, path::Path, sync::Mutex,
    sync::MutexGuard,
};
use url::Url;

use crate::error::CyberAPIError;
use crate::util::get_app_dir;

static COOKIE_STORE: OnceCell<Mutex<CookieStore>> = OnceCell::new();

fn init_store() -> &'static Mutex<CookieStore> {
    COOKIE_STORE.get_or_init(|| {
        let filename = Path::new(get_app_dir()).join(COOKIE_FILE);
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .read(true)
            .open(filename)
            .map(BufReader::new)
            .unwrap();
        let store = cookie_store::CookieStore::load_json(file).unwrap();
        Mutex::new(store)
    })
}

const COOKIE_FILE: &str = "cookies.json";

#[derive(Debug, Serialize, Deserialize)]
pub struct Cookie {
    name: String,
    value: String,
    path: String,
    domain: String,
    expires: String,
}

impl Cookie {
    fn to_set_cookie_string(&self) -> String {
        let mut arr = Vec::new();
        arr.push(format!("{}={}", self.name, self.value));

        if !self.path.is_empty() {
            arr.push(format!("Path={}", self.path));
        }
        if !self.domain.is_empty() {
            arr.push(format!("Domain={}", self.domain));
        }
        if !self.expires.is_empty() {
            arr.push(format!("Expires={}", self.expires));
        }

        arr.join(";")
    }
    fn get_url(&self) -> String {
        let mut path = self.path.clone();
        if path.is_empty() {
            path = "/".to_string()
        }

        format!("http://{}{}", self.domain, path)
    }
}

pub fn get_cookie_store() -> MutexGuard<'static, CookieStore> {
    let result = init_store();
    result.lock().unwrap()
}

fn save_store(store: MutexGuard<CookieStore>) -> Result<(), CyberAPIError> {
    let filename = Path::new(get_app_dir()).join(COOKIE_FILE);
    let mut writer = File::create(filename).map(BufWriter::new)?;
    store.save_json(&mut writer)?;
    Ok(())
}

pub fn delete_cookie_from_store(c: Cookie) -> Result<(), CyberAPIError> {
    let name = c.name;
    if name.is_empty() {
        return Ok(());
    }
    let mut store = get_cookie_store();
    let domain = c.domain.as_str();
    let path = c.path.as_str();

    store.remove(domain, path, name.as_str());
    save_store(store)?;

    Ok(())
}

pub fn save_cookie_store(set_cookies: Vec<String>, current_url: &Url) -> Result<(), CyberAPIError> {
    let mut store = get_cookie_store();
    for ele in set_cookies {
        store.parse(ele.as_str(), current_url)?;
    }

    save_store(store)?;

    Ok(())
}

pub fn list_cookie() -> Result<Vec<String>, CyberAPIError> {
    let store = get_cookie_store();
    let mut result: Vec<String> = Vec::new();

    for ele in store.iter_any() {
        let data = serde_json::to_string(ele)?;
        if !data.is_empty() {
            result.push(data);
        }
    }
    Ok(result)
}

pub fn add_cookie(c: Cookie) -> Result<(), CyberAPIError> {
    let mut store = get_cookie_store();

    let url = c.get_url();
    let request_url = Url::parse(&url)?;
    let cookie_str = c.to_set_cookie_string();


    store.parse(&cookie_str, &request_url)?;

    save_store(store)?;

    Ok(())
}
