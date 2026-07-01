// The transitive `apple-metal` dependency (pulled in by `screencapturekit`)
// links a Swift static bridge that needs the Swift runtime compatibility
// archives (libswiftCompatibility56, …). apple-metal contributes the toolchain
// path via `cargo:rustc-link-arg`, but link-args from a *dependency's* build
// script do not propagate to the dependent binary/test artifacts — so the test
// binary fails to link with "Undefined symbols: __swift_FORCE_LOAD_$_...".
//
// We re-add the Swift static-archive directory via `cargo:rustc-link-search`
// (which DOES propagate to our bin + tests), trying every candidate location so
// it works both on Command-Line-Tools-only hosts (`<dev>/usr/lib/swift…`) and on
// full-Xcode hosts / CI runners (`<dev>/Toolchains/XcodeDefault.xctoolchain/…`).
// Adding a directory that doesn't exist is skipped; off macOS this is a no-op.
fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    #[cfg(target_os = "macos")]
    {
        if let Ok(out) = std::process::Command::new("xcode-select").arg("-p").output() {
            if out.status.success() {
                let dev = String::from_utf8_lossy(&out.stdout).trim().to_string();
                let candidates = [
                    format!("{dev}/usr/lib/swift/macosx"),
                    format!("{dev}/usr/lib/swift_static/macosx"),
                    format!("{dev}/Toolchains/XcodeDefault.xctoolchain/usr/lib/swift/macosx"),
                    format!("{dev}/Toolchains/XcodeDefault.xctoolchain/usr/lib/swift_static/macosx"),
                ];
                for dir in candidates {
                    if std::path::Path::new(&dir).is_dir() {
                        println!("cargo:rustc-link-search=native={dir}");
                    }
                }
            }
        }
        // Runtime search path for the OS Swift dynamic libraries
        // (libswift_Concurrency.dylib, …), which apple-metal only adds via a
        // non-propagating link-arg.
        println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
    }
}
