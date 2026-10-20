//! A release bridge carries the page (`static_files::WebBuild`): one built
//! before the page would ship without the module, so that is refused, and a
//! changed page builds it again. A debug bridge reads the page from disk and
//! needs neither.

use std::path::Path;

fn main() {
    if std::env::var("PROFILE").as_deref() != Ok("release") {
        println!("cargo:rerun-if-changed=build.rs");
        return;
    }
    println!("cargo:rerun-if-changed=../cyberapi-web/www");
    if !Path::new("../cyberapi-web/www/wasm/cyberapi_web_bg.wasm").exists() {
        panic!("the page is not built: run `scripts/web-bundle.sh --release` before a release build of the bridge");
    }
}
