use crate::schema::{EventEnvelope, ResourceContext};
use chrono::Utc;
use core_foundation::runloop::{kCFRunLoopCommonModes, CFRunLoop};
use core_graphics::event::{
    CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement, CGEventType,
};
use serde_json::json;
use std::thread;
use tokio::sync::mpsc;
use uuid::Uuid;

// Hardcoded for MVP to avoid crate version mismatches
// kCGKeyboardEventKeycode = 9
const KEYCODE_FIELD: u32 = 9;

pub fn start_event_tap(tx: mpsc::Sender<String>) -> anyhow::Result<()> {
    println!("[MacOS] Starting Native Event Tap...");

    thread::spawn(move || {
        // Events of interest
        let events = vec![
            CGEventType::KeyDown,
            CGEventType::KeyUp,
            CGEventType::LeftMouseDown,
        ];

        let tap_result = CGEventTap::new(
            CGEventTapLocation::HID,
            CGEventTapPlacement::HeadInsertEventTap,
            CGEventTapOptions::ListenOnly,
            events,
            move |_proxy, type_, event| {
                let log_json = match type_ {
                    CGEventType::KeyDown | CGEventType::KeyUp => {
                        // CGEventField represents the keycode field index
                        let keycode = event.get_integer_value_field(KEYCODE_FIELD);
                        let envelope = base_envelope(
                            "native_tap",
                            "system",
                            "key_input",
                            "P2",
                            Some(ResourceContext {
                                resource_type: "input".to_string(),
                                id: "keyboard".to_string(),
                            }),
                            json!({ "keycode": keycode }),
                        );
                        serde_json::to_value(envelope).unwrap_or_else(|_| json!({}))
                    }
                    CGEventType::LeftMouseDown => {
                        let loc = event.location();
                        let envelope = base_envelope(
                            "native_tap",
                            "system",
                            "click",
                            "P2",
                            Some(ResourceContext {
                                resource_type: "input".to_string(),
                                id: "mouse".to_string(),
                            }),
                            json!({ "location": { "x": loc.x, "y": loc.y } }),
                        );
                        serde_json::to_value(envelope).unwrap_or_else(|_| json!({}))
                    }
                    _ => return Some(event.to_owned()),
                };

                // Non-blocking send
                if let Ok(log) = serde_json::to_string(&log_json) {
                    if let Err(e) = tx.try_send(log) {
                        use tokio::sync::mpsc::error::TrySendError;
                        match e {
                            TrySendError::Full(_) => eprintln!(
                                "⚠️ [MacOS] Event Channel Full! Dropping event (increase buffer?)."
                            ),
                            TrySendError::Closed(_) => {
                                eprintln!("⚠️ [MacOS] Event Channel Closed.")
                            }
                        }
                    }
                } else {
                    eprintln!("⚠️ [MacOS] Failed to serialize event; dropping.");
                }

                Some(event.to_owned())
            },
        );

        match tap_result {
            Ok(tap) => match tap.mach_port.create_runloop_source(0) {
                Ok(source) => {
                    let current_loop = CFRunLoop::get_current();
                    current_loop.add_source(&source, unsafe { kCFRunLoopCommonModes });

                    println!("[MacOS] Event Tap Loop Running...");
                    unsafe {
                        CFRunLoopRun();
                    }
                }
                Err(_) => eprintln!(
                    "❌ Failed to create RunLoop source. Accessibility Access might be missing."
                ),
            },
            Err(e) => eprintln!("❌ Failed to create CGEventTap: {:?}", e),
        }
    });

    Ok(())
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRunLoopRun();
}

fn base_envelope(
    source: &str,
    app: &str,
    event_type: &str,
    priority: &str,
    resource: Option<ResourceContext>,
    payload: serde_json::Value,
) -> EventEnvelope {
    EventEnvelope {
        schema_version: "1.0".to_string(),
        event_id: Uuid::new_v4().to_string(),
        ts: Utc::now().to_rfc3339(),
        source: source.to_string(),
        app: app.to_string(),
        event_type: event_type.to_string(),
        priority: priority.to_string(),
        resource,
        payload,
        privacy: None,
        pid: None,
        window_id: None,
        window_title: None,
        browser_url: None,
        raw: None,
    }
}
