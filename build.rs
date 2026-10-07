//! Incrusta el icono y la información de versión en el `.exe` de Windows.
//!
//! Con MSVC usa `rc.exe` (viene con el Windows SDK de las Build Tools); al
//! compilar desde Linux usa `x86_64-w64-mingw32-windres`. Si no hay
//! compilador de recursos, el ejecutable se genera igual, sin icono propio.

fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("assets/icon.ico")
        .set("ProductName", "YT Mini Player")
        .set("FileDescription", "YT Mini Player")
        .set("LegalCopyright", "Proyecto personal");
    if let Err(err) = res.compile() {
        println!("cargo:warning=no se incrustó el icono en el .exe: {err}");
    }
}
