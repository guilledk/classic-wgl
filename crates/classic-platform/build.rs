use std::process::Command;

fn main() {
    let target_arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();

    if target_arch == "wasm32" {
        return;
    }

    let feature_native = std::env::var("CARGO_FEATURE_NATIVE").is_ok();
    if !feature_native {
        return;
    }

    if let Ok(output) = Command::new("pkg-config").args(["--libs", "x11"]).output() {
        if output.status.success() {
            let flags = String::from_utf8_lossy(&output.stdout);
            for flag in flags.split_whitespace() {
                if let Some(path) = flag.strip_prefix("-L") {
                    println!("cargo:rustc-link-search=native={path}");
                } else if flag == "-lX11" {
                    println!("cargo:rustc-link-lib=X11");
                }
            }
            return;
        }
    }
    println!("cargo:rustc-link-lib=X11");
}
