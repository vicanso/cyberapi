//! The strings are compiled in (`rust_i18n::i18n!`), so a change to a locale
//! file must rebuild the crate.

fn main() {
    println!("cargo:rerun-if-changed=locales");
}
