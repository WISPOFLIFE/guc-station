//! Embeds the app icon in the Windows executable.

fn main() {
    println!("cargo:rerun-if-changed=assets/brand/app-icon.ico");
    #[cfg(windows)]
    winresource::WindowsResource::new()
        .set_icon("assets/brand/app-icon.ico")
        .compile()
        .expect("embed the app icon");
}
