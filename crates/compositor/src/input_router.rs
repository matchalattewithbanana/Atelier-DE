//! Common input routing for nested and native backends.
use crate::state::Atelier;
use smithay::{
    backend::input::{AbsolutePositionEvent, Axis, AxisSource, ButtonState, Event, InputBackend, InputEvent, KeyboardKeyEvent, KeyState, PointerAxisEvent, PointerButtonEvent, PointerMotionEvent},
    input::{keyboard::FilterResult, pointer::{AxisFrame, ButtonEvent, MotionEvent}},
    utils::SERIAL_COUNTER,
};
use std::collections::HashSet;

fn clamp_position(x: f64, y: f64, width: i32, height: i32) -> (f64, f64) {
    (x.clamp(0.0, f64::from(width.saturating_sub(1).max(0))),
     y.clamp(0.0, f64::from(height.saturating_sub(1).max(0))))
}

pub fn handle_input<B: InputBackend>(state: &mut Atelier, input: InputEvent<B>, intercepted: &mut HashSet<smithay::input::keyboard::Keycode>) {
    // No ordinary seat dispatch in a protected state. A future lock-surface
    // router must own input, including grabs and pressed-key bookkeeping.
    if !state.desktop_visible(){return;}
    let serial = SERIAL_COUNTER.next_serial();
    match input {
        InputEvent::Keyboard { event } => {
            if let Some(keyboard) = state.seat.get_keyboard() {
                let code = event.key_code();
                let pressed = event.state() == KeyState::Pressed;
                let grabbed = keyboard.is_grabbed();
                let action = keyboard.input(state, code, event.state(), serial, event.time_msec(), |_, modifiers, key| {
                    if !pressed && intercepted.remove(&code) { return FilterResult::Intercept(None); }
                    if pressed && intercepted.contains(&code) { return FilterResult::Intercept(None); }
                    if !pressed || grabbed || !modifiers.alt || modifiers.ctrl || modifiers.logo { return FilterResult::Forward; }
                    let Some(sym) = key.raw_syms().first().map(|sym| sym.raw()) else { return FilterResult::Forward; };
                    use smithay::input::keyboard::keysyms::*;
                    let target = match sym { KEY_1 => 0, KEY_2 => 1, KEY_3 => 2, KEY_4 => 3, _ => return FilterResult::Forward };
                    intercepted.insert(code);
                    FilterResult::Intercept(Some((target, modifiers.shift)))
                });
                if let Some(Some((target, move_focused))) = action {
                    state.select_workspace(target, move_focused, serial);
                    if let Some(pointer) = state.seat.get_pointer() {
                        let focus = state.surface_under_pointer();
                        let location = state.pointer_position;
                        pointer.motion(state, focus, &MotionEvent { location, serial, time: event.time_msec() });
                        pointer.frame(state);
                    }
                }
            }
        }
        InputEvent::PointerMotion { event } => {
            if let Some(region) = state.space.output_geometry(&state.output) {
                let (x, y) = clamp_position(
                    state.pointer_position.x + event.delta_x(),
                    state.pointer_position.y + event.delta_y(),
                    region.size.w, region.size.h,
                );
                state.pointer_position = (x, y).into();
                let focus = state.surface_under_pointer();
                let location = state.pointer_position;
                if let Some(pointer) = state.seat.get_pointer() {
                    pointer.motion(state, focus, &MotionEvent { location, serial, time: event.time_msec() });
                    pointer.frame(state);
                }
            }
        }
        InputEvent::PointerMotionAbsolute { event } => {
            if let Some(region) = state.space.output_geometry(&state.output) {
                state.pointer_position = event.position_transformed(region.size);
                let focus = state.surface_under_pointer();
                let location = state.pointer_position;
                if let Some(pointer) = state.seat.get_pointer() {
                    pointer.motion(state, focus, &MotionEvent { location, serial, time: event.time_msec() });
                    pointer.frame(state);
                }
            }
        }
        InputEvent::PointerButton { event } => {
            if let Some(pointer) = state.seat.get_pointer() {
                if event.state() == ButtonState::Pressed && !pointer.is_grabbed() {
                    state.focus_pointer(serial);
                }
                pointer.button(state, &ButtonEvent { button: event.button_code(), state: event.state(), serial, time: event.time_msec() });
                pointer.frame(state);
            }
        }
        InputEvent::PointerAxis { event } => {
            if let Some(pointer) = state.seat.get_pointer() {
                let source = event.source();
                let mut frame = AxisFrame::new(event.time_msec()).source(source);
                for axis in [Axis::Horizontal, Axis::Vertical] {
                    let steps = event.amount_v120(axis);
                    if let Some(steps) = steps { frame = frame.v120(axis, steps.round() as i32); }
                    let value = event.amount(axis).or_else(|| steps.map(|steps| steps * 15.0 / 120.0)).unwrap_or(0.0);
                    frame = frame.relative_direction(axis, event.relative_direction(axis));
                    if value != 0.0 { frame = frame.value(axis, value); }
                    else if source == AxisSource::Finger { frame = frame.stop(axis); }
                }
                pointer.axis(state, frame);
                pointer.frame(state);
            }
        }
        _ => {}
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn relative_pointer_remains_inside_output() {
        assert_eq!(clamp_position(-5.0, 999.0, 100, 80), (0.0, 79.0));
        assert_eq!(clamp_position(12.5, 14.0, 100, 80), (12.5, 14.0));
        assert_eq!(clamp_position(10.0, 10.0, 0, 0), (0.0, 0.0));
    }
}
