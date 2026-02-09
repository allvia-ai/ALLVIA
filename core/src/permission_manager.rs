// Permission Manager - Inspired by clawdbot-main/apps/macos/Sources/Clawdbot/PermissionManager.swift
// Unified macOS permission handling for Steer

use core_foundation::base::TCFType;
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::CFDictionary;
use core_foundation::string::CFString;

/// Permission capabilities that Steer may require
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Capability {
    Accessibility,
    ScreenRecording,
    // Future: Microphone, Camera, etc.
}

impl std::fmt::Display for Capability {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Capability::Accessibility => write!(f, "Accessibility"),
            Capability::ScreenRecording => write!(f, "Screen Recording"),
        }
    }
}

/// Result of permission status check
#[derive(Debug, Clone)]
pub struct PermissionStatus {
    pub capability: Capability,
    pub granted: bool,
}

/// Permission Manager for macOS
/// Provides unified interface for checking and requesting system permissions
pub struct PermissionManager;

impl PermissionManager {
    /// Check if Accessibility permission is granted
    pub fn check_accessibility() -> bool {
        #[link(name = "ApplicationServices", kind = "framework")]
        extern "C" {
            fn AXIsProcessTrusted() -> bool;
        }
        unsafe { AXIsProcessTrusted() }
    }

    /// Request Accessibility permission with system prompt
    pub fn request_accessibility() -> bool {
        #[link(name = "ApplicationServices", kind = "framework")]
        extern "C" {
            fn AXIsProcessTrustedWithOptions(options: *const std::ffi::c_void) -> bool;
        }

        // Create options dictionary with prompt key
        let key = CFString::new("AXTrustedCheckOptionPrompt");
        let value = CFBoolean::true_value();

        let dict = CFDictionary::from_CFType_pairs(&[(key.as_CFType(), value.as_CFType())]);

        unsafe {
            AXIsProcessTrustedWithOptions(dict.as_concrete_TypeRef() as *const std::ffi::c_void)
        }
    }

    /// Check if Screen Recording permission is granted
    pub fn check_screen_recording() -> bool {
        #[link(name = "CoreGraphics", kind = "framework")]
        extern "C" {
            fn CGPreflightScreenCaptureAccess() -> bool;
        }
        unsafe { CGPreflightScreenCaptureAccess() }
    }

    /// Request Screen Recording permission
    pub fn request_screen_recording() -> bool {
        #[link(name = "CoreGraphics", kind = "framework")]
        extern "C" {
            fn CGRequestScreenCaptureAccess() -> bool;
        }
        unsafe { CGRequestScreenCaptureAccess() }
    }

    /// Check status of all required permissions
    pub fn status_all() -> Vec<PermissionStatus> {
        vec![
            PermissionStatus {
                capability: Capability::Accessibility,
                granted: Self::check_accessibility(),
            },
            PermissionStatus {
                capability: Capability::ScreenRecording,
                granted: Self::check_screen_recording(),
            },
        ]
    }

    /// Check if all required permissions are granted
    pub fn all_granted() -> bool {
        Self::check_accessibility() && Self::check_screen_recording()
    }

    /// Request all missing permissions (with prompts)
    pub fn ensure_all(interactive: bool) -> Vec<PermissionStatus> {
        let mut results = Vec::new();

        // Accessibility
        let acc_granted = if Self::check_accessibility() {
            true
        } else if interactive {
            Self::request_accessibility()
        } else {
            false
        };
        results.push(PermissionStatus {
            capability: Capability::Accessibility,
            granted: acc_granted,
        });

        // Screen Recording
        let sr_granted = if Self::check_screen_recording() {
            true
        } else if interactive {
            Self::request_screen_recording()
        } else {
            false
        };
        results.push(PermissionStatus {
            capability: Capability::ScreenRecording,
            granted: sr_granted,
        });

        results
    }

    /// Print permission status to console
    pub fn print_status() {
        println!("🔐 Permission Status:");
        for status in Self::status_all() {
            let icon = if status.granted { "✅" } else { "❌" };
            println!(
                "   {} {}: {}",
                icon,
                status.capability,
                if status.granted { "Granted" } else { "Denied" }
            );
        }
    }

    /// Open System Preferences to the relevant privacy pane
    pub fn open_privacy_settings(capability: Capability) {
        let url = match capability {
            Capability::Accessibility => {
                "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"
            }
            Capability::ScreenRecording => {
                "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture"
            }
        };

        let _ = std::process::Command::new("open").arg(url).spawn();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_accessibility_check() {
        // Just verify it doesn't crash
        let _ = PermissionManager::check_accessibility();
    }

    #[test]
    fn test_screen_recording_check() {
        // Just verify it doesn't crash
        let _ = PermissionManager::check_screen_recording();
    }

    #[test]
    fn test_status_all() {
        let statuses = PermissionManager::status_all();
        assert_eq!(statuses.len(), 2);
    }
}
