//! Windows executables carry their icon and version information as
//! resources; everywhere else there is nothing to do.

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
    }
}
