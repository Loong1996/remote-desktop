/// Check whether the process can inject input, logging actionable guidance when
/// it can't. On macOS this queries the Accessibility trust state (and prompts on
/// first run); elsewhere it's a no-op that returns true.
pub fn check_input_permission() -> bool {
    #[cfg(target_os = "macos")]
    {
        let trusted = macos_accessibility_client::accessibility::application_is_trusted_with_prompt();
        if !trusted {
            tracing::warn!(
                "macOS Accessibility permission not granted — mouse/keyboard \
                 injection will not work. Approve this program under System \
                 Settings → Privacy & Security → Accessibility, then restart it."
            );
        }
        trusted
    }
    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}

/// Check whether the process can capture the screen on macOS, logging guidance
/// if not. Elsewhere it's a no-op returning true. MVP heuristic: on macOS,
/// SCShareableContent::get() succeeds only when Screen Recording is authorized.
pub fn check_screen_recording_permission() -> bool {
    #[cfg(target_os = "macos")]
    {
        match screencapturekit::shareable_content::SCShareableContent::get() {
            Ok(_) => true,
            Err(_) => {
                tracing::warn!(
                    "macOS Screen Recording permission not granted — the remote screen \
                     will be blank. Approve this program under System Settings → Privacy & \
                     Security → Screen Recording, then restart it."
                );
                false
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn check_returns_true_off_macos() {
        // Off macOS the check is a no-op that must report available.
        #[cfg(not(target_os = "macos"))]
        assert!(super::check_input_permission());
        // On macOS, do NOT call check_input_permission() here — it uses the
        // *prompting* Accessibility API and would surface a system dialog during
        // `cargo test` / CI. Verify the underlying non-prompting trust query is
        // callable instead (no dialog); the prompting variant is exercised at
        // real startup, not in tests.
        #[cfg(target_os = "macos")]
        {
            let _trusted: bool =
                macos_accessibility_client::accessibility::application_is_trusted();
        }
    }
}
