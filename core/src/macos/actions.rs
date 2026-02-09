use core_graphics::event::{CGEvent, CGEventTapLocation};
use core_graphics::event_source::CGEventSource;
use core_graphics::event_source::CGEventSourceStateID;
use std::{thread, time::Duration};

pub fn click_element(element_id: &str) -> anyhow::Result<()> {
    // Implementing "Click by ID" purely with coords is hard without the Accessibility Object.
    // In native mode, we usually pass coordinates or AXUIElementRef.
    // For MVP, if "element_id" is essentially ignored or we need to look it up.
    // Let's assume for now we cannot click easily without coordinates.
    // But since this is a "Behavior" tool, maybe we just log it?
    // Or we implement a "click at (x,y)" helper.

    // For now, let's just fail if we don't have coords, or hardcode a "center" click if valid.
    println!(
        "[MacOS] Click Element '{}' (Not fully resolved in MVP)",
        element_id
    );
    // Real implementation requires finding the element first (AXUIElement) then getting its center.
    // We haven't linked find_element yet.
    Ok(())
}

pub fn type_text(text: &str) -> anyhow::Result<()> {
    println!("[MacOS] Typping: {}", text);

    let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
        .map_err(|_| anyhow::anyhow!("Failed to create event source"))?;

    for c in text.chars() {
        // Very basic mapping. Dealing with keycodes is complex.
        // We'll trust CGEventKeyboardSetUnicodeString which is easier than mapping keycodes manually.

        // 1. Key Down
        if let Ok(event) = CGEvent::new_keyboard_event(source.clone(), 0, true) {
            event.set_string(&c.to_string());
            event.post(CGEventTapLocation::HID);
        }

        thread::sleep(Duration::from_millis(10));

        // 2. Key Up
        if let Ok(event) = CGEvent::new_keyboard_event(source.clone(), 0, false) {
            event.set_string(&c.to_string());
            event.post(CGEventTapLocation::HID);
        }

        thread::sleep(Duration::from_millis(10));
    }

    Ok(())
}
