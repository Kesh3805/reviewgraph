// Re-embed the migrations whenever they change (sqlx::migrate! reads them at compile time).
fn main() {
    println!("cargo:rerun-if-changed=../../migrations");
}
