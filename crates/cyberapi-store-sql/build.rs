//! `sqlx::migrate!` puts the schema steps into the build. Without this,
//! cargo does not see a step added to `migrations/` and keeps the build
//! made without it (sqlx's `migrate build-script`).

fn main() {
    println!("cargo:rerun-if-changed=migrations");
}
