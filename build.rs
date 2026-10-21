//! The strings are compiled in (`rust_i18n::i18n!`), so a change to a locale
//! file must rebuild the crate. A Windows executable carries the app's icon.

fn main() {
    println!("cargo:rerun-if-changed=locales");
    // The target's OS: a build script runs on the host.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rerun-if-changed=resources/icons/icon.ico");
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("resources/icons/icon.ico");
        // Without the Windows SDK's resource compiler the app still builds,
        // with the default icon.
        if let Err(error) = resource.compile() {
            println!("cargo:warning=the icon was not compiled in: {error}");
        }
    }
}
