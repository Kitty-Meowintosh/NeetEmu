//! Keyboard and mouse, against `graphics/screens/BoilerplateScreen.java`.

use neetemu::events::{Event, Value};
use neetemu::input::{self, Button, Mouse};

fn shape(event: &Event) -> (String, Vec<Value>) {
    (event.name.clone(), event.args.clone())
}

fn want(name: &str, args: Vec<Value>) -> (String, Vec<Value>) {
    (name.to_string(), args)
}

fn flush(mouse: &mut Mouse) -> Vec<(String, Vec<Value>)> {
    mouse.flush().iter().map(shape).collect()
}

fn int(n: i64) -> Value {
    Value::Int(n)
}

fn text(s: &str) -> Value {
    Value::Str(s.to_string())
}

#[test]
fn a_key_carries_code_char_and_modifiers() {
    let event = input::key(true, 97, 0x02).expect("an event");
    assert_eq!(
        shape(&event),
        want("keyPressed", vec![int(97), text("a"), int(2)])
    );

    let event = input::key(false, 97, 0).expect("an event");
    assert_eq!(
        shape(&event),
        want("keyReleased", vec![int(97), text("a"), int(0)])
    );
}

#[test]
fn char_is_empty_outside_the_printable_range() {
    let of = |code| match input::key(true, code, 0).expect("an event").args[1].clone() {
        Value::Str(s) => s,
        other => panic!("want a string, got {other:?}"),
    };
    assert_eq!(of(32), " ");
    assert_eq!(of(97), "a");
    assert_eq!(of(127), "\u{7f}");
    assert_eq!(of(13), "", "enter is below the printable range");
    assert_eq!(of(8), "", "backspace is below it too");
    assert_eq!(of(128), "", "the arrows are above it");
    assert_eq!(of(157), "", "and so is F24");
}

#[test]
fn an_unmapped_key_sends_nothing() {
    assert!(input::key(true, 0, 0).is_none());
    assert!(input::key(false, 0, 0x08).is_none());
}

#[test]
fn a_click_reports_the_button_minecraft_numbers_it_by() {
    let mut mouse = Mouse::new();
    let event = mouse.press(10, 20, Button::Left);
    assert_eq!(
        shape(&event),
        want("mouseClicked", vec![int(10), int(20), int(0), int(0)])
    );
    let event = mouse.release(10, 20, Button::Right);
    assert_eq!(
        shape(&event),
        want("mouseReleased", vec![int(10), int(20), int(1), int(0)])
    );
    assert_eq!(Button::Middle.code(), 2);
}

#[test]
fn a_move_is_reported_once_per_position() {
    let mut mouse = Mouse::new();
    mouse.moved(4, 5);
    assert_eq!(
        flush(&mut mouse),
        vec![want("mouseMoved", vec![int(4), int(5)])]
    );

    mouse.moved(4, 5);
    assert!(mouse.flush().is_empty(), "the same position sends nothing");

    mouse.moved(4, 6);
    assert_eq!(
        flush(&mut mouse),
        vec![want("mouseMoved", vec![int(4), int(6)])]
    );
}

#[test]
fn a_held_button_drags_as_well_as_moves() {
    let mut mouse = Mouse::new();
    mouse.press(1, 1, Button::Left);
    mouse.flush();

    mouse.moved(2, 3);
    assert_eq!(
        flush(&mut mouse),
        vec![
            want("mouseDragged", vec![int(2), int(3), int(0), int(0)]),
            want("mouseMoved", vec![int(2), int(3)]),
        ]
    );

    assert!(mouse.flush().is_empty(), "neither repeats at a standstill");
}

#[test]
fn a_released_button_stops_dragging() {
    let mut mouse = Mouse::new();
    mouse.press(1, 1, Button::Left);
    mouse.release(1, 1, Button::Left);
    mouse.moved(9, 9);
    assert_eq!(
        flush(&mut mouse),
        vec![want("mouseMoved", vec![int(9), int(9)])]
    );
}

#[test]
fn leaving_the_framebuffer_releases_what_was_held() {
    let mut mouse = Mouse::new();
    mouse.press(7, 8, Button::Left);
    mouse.press(7, 8, Button::Right);
    mouse.moved(7, 9);
    mouse.flush();

    let left: Vec<_> = mouse.left().iter().map(shape).collect();
    assert_eq!(
        left,
        vec![
            want("mouseReleased", vec![int(7), int(9), int(0), int(0)]),
            want("mouseReleased", vec![int(7), int(9), int(1), int(0)]),
        ],
        "each at the last position it was seen at"
    );
    assert!(mouse.flush().is_empty(), "and the pointer is gone");
}

#[test]
fn the_wheel_carries_both_axes_as_floats() {
    let event = input::wheel(3, 4, -1.0, 2.5, 0x08);
    assert_eq!(
        shape(&event),
        want(
            "mouseScrolled",
            vec![int(3), int(4), Value::Num(-1.0), Value::Num(2.5), int(8)]
        )
    );
}

#[test]
fn button_events_carry_the_modifiers_held() {
    let mut mouse = Mouse::new();
    mouse.modifiers(0x08);
    let event = mouse.press(1, 2, Button::Left);
    assert_eq!(
        shape(&event),
        want("mouseClicked", vec![int(1), int(2), int(0), int(8)])
    );

    mouse.modifiers(0x09);
    mouse.moved(3, 4);
    assert_eq!(
        flush(&mut mouse),
        vec![
            want("mouseDragged", vec![int(3), int(4), int(0), int(9)]),
            want("mouseMoved", vec![int(3), int(4)]),
        ]
    );

    mouse.modifiers(0);
    let event = mouse.release(3, 4, Button::Left);
    assert_eq!(
        shape(&event),
        want("mouseReleased", vec![int(3), int(4), int(0), int(0)]),
        "a modifier let go shows at once"
    );
}

/// The table in csrc/display.c, checked against SDL's own constants.
#[cfg(sdl)]
mod mapping {
    use neetemu::display::keys;

    fn code(key: &str, mods: &[&str]) -> i32 {
        let mods = mods.iter().fold(0u16, |acc, m| acc | keys::modifier(m));
        keys::code(keys::keycode(key), mods)
    }

    #[test]
    fn letters_and_digits_fold_shift_in() {
        assert_eq!(code("A", &[]), 'a' as i32);
        assert_eq!(code("A", &["shift"]), 'A' as i32);
        assert_eq!(code("1", &[]), '1' as i32);
        assert_eq!(code("1", &["shift"]), '!' as i32);
        assert_eq!(code("0", &["shift"]), ')' as i32);
        assert_eq!(code("/", &["shift"]), '?' as i32);
        assert_eq!(code("`", &["shift"]), '~' as i32);
        assert_eq!(code("A", &["ctrl"]), 'a' as i32, "only shift folds in");
    }

    #[test]
    fn the_named_keys_match_the_mod() {
        assert_eq!(code("Return", &[]), 13);
        assert_eq!(code("Tab", &[]), 9);
        assert_eq!(code("Backspace", &[]), 8);
        assert_eq!(code("Delete", &[]), 8, "delete is backspace upstream");
        assert_eq!(code("Left Shift", &[]), 14);
        assert_eq!(code("Right Shift", &[]), 14);
        assert_eq!(code("Left", &[]), 128);
        assert_eq!(code("Right", &[]), 129);
        assert_eq!(code("Up", &[]), 130);
        assert_eq!(code("Down", &[]), 131);
        assert_eq!(code("Left Ctrl", &[]), 132);
        assert_eq!(code("Right Alt", &[]), 133);
        assert_eq!(code("F1", &[]), 134);
        assert_eq!(code("F12", &[]), 145);
        assert_eq!(code("F13", &[]), 146);
        assert_eq!(code("F24", &[]), 157);
    }

    #[test]
    fn unmapped_keys_are_suppressed() {
        assert_eq!(code("Escape", &[]), 0);
        assert_eq!(code("Left GUI", &[]), 0, "super sends nothing at all");
        assert_eq!(code("Keypad Enter", &[]), 0);
        assert_eq!(code("Insert", &[]), 0);
    }

    #[test]
    fn only_the_four_modifier_bits_minecraft_sets_are_reported() {
        let bits =
            |names: &[&str]| keys::mods(names.iter().fold(0u16, |acc, m| acc | keys::modifier(m)));
        assert_eq!(bits(&[]), 0);
        assert_eq!(bits(&["shift"]), 0x01);
        assert_eq!(bits(&["ctrl"]), 0x02);
        assert_eq!(bits(&["alt"]), 0x04);
        assert_eq!(bits(&["super"]), 0x08);
        assert_eq!(bits(&["ctrl", "shift"]), 0x03);
        assert_eq!(bits(&["caps"]), 0, "GLFW_LOCK_KEY_MODS is never set");
        assert_eq!(bits(&["num"]), 0);
    }

    #[test]
    fn buttons_are_left_right_middle_from_zero() {
        assert_eq!(keys::button(1), 0);
        assert_eq!(keys::button(3), 1);
        assert_eq!(keys::button(2), 2);
    }
}
