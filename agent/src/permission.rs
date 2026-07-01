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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_returns_true_off_macos() {
        // Off macOS the check is a no-op that must report available.
        #[cfg(not(target_os = "macos"))]
        assert!(check_input_permission());
        // On macOS it reflects the live Accessibility trust state, which a unit
        // test can't assert; just confirm it runs without panicking.
        #[cfg(target_os = "macos")]
        {
            let _trusted: bool = check_input_permission();
        }
    }
}
