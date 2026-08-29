//! Windows executables carry their icon and version information as
//! resources, and link GLEW for libprojectM; everywhere else there is
//! nothing to do.

fn main() {
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rerun-if-changed=packaging/windows/fastpotify.ico");
        let mut resource = winresource::WindowsResource::new();
        resource
            .set_icon("packaging/windows/fastpotify.ico")
            .set("ProductName", "Fastpotify")
            .set("FileDescription", "Fastpotify");
        if let Err(error) = resource.compile() {
            println!("cargo:warning=Windows resources not embedded: {error}");
        }
        // libprojectM reads OpenGL through GLEW on Windows, and as a static
        // library leaves GLEW itself to whoever links it: the copy vcpkg
        // installed (`vcpkg install glew:x64-windows-static-md`), from the
        // same root projectm-sys builds with.
        if std::env::var_os("CARGO_FEATURE_MILKDROP").is_some() {
            println!("cargo:rerun-if-env-changed=VCPKG_INSTALLATION_ROOT");
            if let Some(root) = std::env::var_os("VCPKG_INSTALLATION_ROOT") {
                let lib = std::path::Path::new(&root)
                    .join("installed")
                    .join("x64-windows-static-md")
                    .join("lib");
                println!("cargo:rustc-link-search=native={}", lib.display());
                println!("cargo:rustc-link-lib=static=glew32s");
            }
        }
    }
}
