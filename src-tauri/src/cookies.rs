use cookie_store::CookieStore;
use serde::Serialize;
use std::fs::File;
use std::{fs::OpenOptions, io::BufReader, io::BufWriter, path::Path};

use crate::error::CyberAPIError;
use crate::util::get_app_dir;

const COOKIE_FILE: &str = "cookies.json";
const SESSION_COOKIE_EXPIRES: &str = "Fri, 31 Jan 2200 16:00:00 GMT";

#[derive(Debug, Serialize)]
pub struct Cookie {
    name: String,
    value: String,
    path: String,
    domain: String,
    expires: String,
}

pub fn get_session_cookie_expires() -> String {
    let mut str = String::from("; Expires=");
    str.push_str(SESSION_COOKIE_EXPIRES);
    str
}

pub fn get_cookie_store() -> Result<CookieStore, CyberAPIError> {
    let filename = Path::new(get_app_dir()).join(COOKIE_FILE);

    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .read(true)
        .open(filename)
        .map(BufReader::new)?;
    let store = cookie_store::CookieStore::load_json(file)?;
    Ok(store)
}

pub fn save_cookie_store(store: CookieStore) -> Result<(), CyberAPIError> {
    let filename = Path::new(get_app_dir()).join(COOKIE_FILE);

    let mut writer = File::create(filename).map(BufWriter::new)?;

    store.save_json(&mut writer)?;

    Ok(())
}

pub fn list_cookie() -> Result<Vec<Cookie>, CyberAPIError> {
    let store = get_cookie_store()?;
    let mut result: Vec<Cookie> = Vec::new();
    store.iter_any().for_each(|ele| {
        // TODO domain expires 无法获取的问题
        let domain = match ele.domain_raw() {
            Some(str) => str,
            _ => "",
        };
        println!("{:?}", ele);

        let expires = match ele.expires_datetime() {
            Some(t) => t.to_string(),
            _ => "".to_string(),
        };
        let c = Cookie {
            name: ele.name().to_string(),
            value: ele.value().to_string(),
            path: ele.path.to_string(),
            domain: domain.to_string(),
            expires,
        };
        result.push(c);
    });
    Ok(result)
}
