use once_cell::sync::OnceCell;

static APP_DIR: OnceCell<String> = OnceCell::new();
pub fn set_app_dir(dir: String) {
    APP_DIR.set(dir).unwrap();
}

pub fn get_app_dir() -> &'static String {
    APP_DIR.get().unwrap()
}