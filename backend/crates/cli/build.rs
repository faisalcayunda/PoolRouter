// `sqlx::migrate!` menyematkan berkas yang ada saat kompilasi. Tanpa ini, berkas migrasi BARU tidak
// memicu build ulang (cargo hanya melacak berkas yang sudah dibaca).
fn main() {
    println!("cargo:rerun-if-changed=../../migrations");
}
