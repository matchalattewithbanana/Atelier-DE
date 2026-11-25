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
    state.sync_input_boundary(0);
    let desktop=state.desktop_input_ready();
    let blocked=state.input_gate.waiting()||(!desktop&&state.desktop_visible());
    let serial = SERIAL_COUNTER.next_serial();
    match input {
        InputEvent::Keyboard { event } => {
            let code=event.key_code();let pressed=event.state()==KeyState::Pressed;
            if let Some(keyboard) = state.seat.get_keyboard() {
                if !desktop{
                    // No shortcuts in the lock domain. When quarantined the
                    // seat has no focus; update its physical/XKB state anyway.
                    intercepted.clear();
                    if blocked{keyboard.set_focus(state,None,serial);}
                    keyboard.input(state,code,event.state(),serial,event.time_msec(),|_,_,_|FilterResult::<()>::Forward);
                    state.input_gate.key(code,pressed);
                    state.sync_input_boundary(event.time_msec());
                    return;
                }
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
                        let focus = state.input_pointer_focus();
                        let location = state.pointer_position;
                        pointer.motion(state, focus, &MotionEvent { location, serial, time: event.time_msec() });
                        pointer.frame(state);
                    }
                }
            }
            state.input_gate.key(code,pressed);
        }
        InputEvent::PointerMotion { event } => {
            if let Some(region) = state.space.output_geometry(&state.output) {
                let (x, y) = clamp_position(
                    state.pointer_position.x + event.delta_x(),
                    state.pointer_position.y + event.delta_y(),
                    region.size.w, region.size.h,
                );
                state.pointer_position = (x, y).into();
                let focus = state.input_pointer_focus();
                let location = state.pointer_position;
                if let Some(pointer) = state.seat.get_pointer() {
                    pointer.motion(state, focus, &MotionEvent { location, serial, time: event.time_msec() });
                    pointer.frame(state);
                }
            }
        }
        InputEvent::PointerMotionAbsolute { event } => {
            if let Some(region) = state.space.output_geometry(&state.output) {
                let p=event.position_transformed(region.size);
                state.pointer_position=clamp_position(p.x,p.y,region.size.w,region.size.h).into();
                let focus = state.input_pointer_focus();
                let location = state.pointer_position;
                if let Some(pointer) = state.seat.get_pointer() {
                    pointer.motion(state, focus, &MotionEvent { location, serial, time: event.time_msec() });
                    pointer.frame(state);
                }
            }
        }
        InputEvent::PointerButton { event } => {
            let button=event.button_code();let pressed=event.state()==ButtonState::Pressed;
            if let Some(pointer) = state.seat.get_pointer() {
                if desktop&&pressed&&!pointer.is_grabbed(){state.focus_pointer(serial);}
                if blocked{
                    let location=state.pointer_position;
                    let motion=MotionEvent{location,serial,time:event.time_msec()};
                    pointer.motion(state,None,&motion);
                    if pointer.is_grabbed(){pointer.unset_grab(state,serial,event.time_msec());}
                    pointer.motion(state,None,&motion);
                }
                pointer.button(state,&ButtonEvent{button,state:event.state(),serial,time:event.time_msec()});
                if blocked&&pointer.is_grabbed(){pointer.unset_grab(state,serial,event.time_msec());}
                pointer.frame(state);
            }
            state.input_gate.button(button,pressed);
            state.sync_input_boundary(event.time_msec());
        }
        InputEvent::PointerAxis { event } => {
            if blocked{return;}
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
