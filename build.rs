//! Incrusta icono, versión y manifiesto en el `.exe` de Windows.
//!
//! Con MSVC usa `rc.exe` (viene con el Windows SDK de las Build Tools); al
//! compilar desde Linux usa `x86_64-w64-mingw32-windres`. Si no hay
//! compilador de recursos, el ejecutable se genera igual, sin icono propio.
//!
//! `YTMP_UIACCESS=1` genera la variante de **superposición máxima**: el
//! manifiesto pide `uiAccess="true"`, lo que coloca las ventanas "siempre
//! visible" en la banda de accesibilidad de Windows (la del teclado en
//! pantalla), por encima de las apps a pantalla completa. Windows solo la
//! deja arrancar si el `.exe` está firmado y en Archivos de programa; de eso
//! se encarga `dist/install.ps1 -Overlay`.

fn manifest(uiaccess: bool) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
      <requestedPrivileges>
        <requestedExecutionLevel level="asInvoker" uiAccess="{uiaccess}"/>
      </requestedPrivileges>
    </security>
  </trustInfo>
  <compatibility xmlns="urn:schemas-microsoft-com:compatibility.v1">
    <application>
      <supportedOS Id="{{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}}"/>
    </application>
  </compatibility>
  <application xmlns="urn:schemas-microsoft-com:asm.v3">
    <windowsSettings>
      <dpiAware xmlns="http://schemas.microsoft.com/SMI/2005/WindowsSettings">true/pm</dpiAware>
      <dpiAwareness xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">PerMonitorV2</dpiAwareness>
    </windowsSettings>
  </application>
  <dependency>
    <dependentAssembly>
      <assemblyIdentity type="win32" name="Microsoft.Windows.Common-Controls" version="6.0.0.0"
        processorArchitecture="*" publicKeyToken="6595b64144ccf1df" language="*"/>
    </dependentAssembly>
  </dependency>
</assembly>
"#
    )
}

fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=YTMP_UIACCESS");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let uiaccess = std::env::var("YTMP_UIACCESS").is_ok_and(|v| v == "1");
    let mut res = winresource::WindowsResource::new();
    res.set_icon("assets/icon.ico")
        .set("ProductName", "YT Mini Player")
        .set(
            "FileDescription",
            if uiaccess { "YT Mini Player (superposición máxima)" } else { "YT Mini Player" },
        )
        .set("LegalCopyright", "Proyecto personal")
        .set_manifest(&manifest(uiaccess));
    if let Err(err) = res.compile() {
        println!("cargo:warning=no se incrustaron icono ni manifiesto en el .exe: {err}");
    }
}
