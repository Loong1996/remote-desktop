// The transitive `apple-metal` dependency (pulled in by `screencapturekit`)
// links a Swift static bridge that needs Swift runtime compatibility archives
// (libswiftCompatibility56, …). apple-metal's build script only adds the full
// Xcode toolchain path (`.../XcodeDefault.xctoolchain/usr/lib/swift/macosx`).
// On a machine with only the Command Line Tools installed, those archives live
// under `<developer-dir>/usr/lib/swift/macosx` instead, so the link fails with
// "Undefined symbols: __swift_FORCE_LOAD_$_swiftCompatibility56". Add that
// directory to the link search path when — and only when — it exists, which is
// a no-op on full-Xcode setups (path absent there) and off macOS.
fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    #[cfg(target_os = "macos")]
    {
        // Link search path for the Swift compatibility archives (link time).
        if let Ok(out) = std::process::Command::new("xcode-select").arg("-p").output() {
            if out.status.success() {
                let dev = String::from_utf8_lossy(&out.stdout).trim().to_string();
                let swift_dir = format!("{dev}/usr/lib/swift/macosx");
                if std::path::Path::new(&swift_dir).is_dir() {
                    println!("cargo:rustc-link-search=native={swift_dir}");
                }
            }
        }
        // Runtime search path for the OS Swift dynamic libraries
        // (libswift_Concurrency.dylib, …). apple-metal adds this rpath via
        // `cargo:rustc-link-arg`, but link-args from a dependency's build
        // script do not propagate to the dependent binary/tests — so add it
        // here for our own bin and test artifacts.
        println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
    }
}
