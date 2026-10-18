//! Conversion from winit keyboard events to the W3C `key` and `code` values Servo expects.

use servo::{Code, Key, KeyState, KeyboardEvent, Location, Modifiers, NamedKey};
use std::str::FromStr;
use winit::event::{ElementState, KeyEvent};
use winit::keyboard::{Key as WinitKey, KeyLocation, ModifiersState, NamedKey as WinitNamedKey, PhysicalKey};

pub fn modifiers(state: ModifiersState) -> Modifiers {
    let mut m = Modifiers::empty();
    m.set(Modifiers::CONTROL, state.control_key());
    m.set(Modifiers::SHIFT, state.shift_key());
    m.set(Modifiers::ALT, state.alt_key());
    m.set(Modifiers::META, state.super_key());
    m
}

fn key(event: &KeyEvent) -> Key {
    match &event.logical_key {
        WinitKey::Character(s) => Key::Character(s.to_string()),
        WinitKey::Named(WinitNamedKey::Space) => Key::Character(" ".to_string()),
        WinitKey::Named(named) => NamedKey::from_str(&format!("{:?}", named))
            .map(Key::Named)
            .unwrap_or(Key::Named(NamedKey::Unidentified)),
        _ => Key::Named(NamedKey::Unidentified),
    }
}

fn code(event: &KeyEvent) -> Code {
    let PhysicalKey::Code(c) = event.physical_key else {
        return Code::Unidentified;
    };
    let name = match format!("{:?}", c).as_str() {
        "SuperLeft" => "MetaLeft".to_string(),
        "SuperRight" => "MetaRight".to_string(),
        other => other.to_string(),
    };
    Code::from_str(&name).unwrap_or(Code::Unidentified)
}

fn location(event: &KeyEvent) -> Location {
    match event.location {
        KeyLocation::Standard => Location::Standard,
        KeyLocation::Left => Location::Left,
        KeyLocation::Right => Location::Right,
        KeyLocation::Numpad => Location::Numpad,
    }
}

/// Builds Servo's keyboard event from a winit event and the current modifier state.
pub fn keyboard_event(event: &KeyEvent, mods: ModifiersState) -> KeyboardEvent {
    let state = match event.state {
        ElementState::Pressed => KeyState::Down,
        ElementState::Released => KeyState::Up,
    };
    KeyboardEvent::new_without_event(
        state,
        key(event),
        code(event),
        location(event),
        modifiers(mods),
        event.repeat,
        false,
    )
}
