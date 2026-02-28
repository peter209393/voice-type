use gtk::prelude::*;

static mut GTK_WINDOW: Option<gtk::ApplicationWindow> = None;

pub fn init_gtk() -> anyhow::Result<()> {
    gtk::init()?;

    let window = gtk::ApplicationWindow::builder()
        .window_position(gtk::WindowPosition::Mouse)
        .decorated(false)
        .resizable(false)
        .skip_taskbar_hint(true)
        .accept_focus(false)
        .type_hint(gtk::gdk::WindowTypeHint::Notification)
        .build();

    window.set_keep_above(true);
    window.set_default_width(300);

    let css = gtk::CssProvider::new();
    let _ = css.load_from_data(
        b"
        window {
            background-color: rgba(30, 30, 30, 0.95);
            border-radius: 8px;
            border: 1px solid rgba(255, 255, 255, 0.1);
        }
        label {
            color: #ffffff;
            font-size: 14px;
            font-family: monospace;
        }
    ",
    );

    if let Some(screen) = gtk::gdk::Screen::default() {
        gtk::StyleContext::add_provider_for_screen(
            &screen,
            &css,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }

    let label = gtk::Label::builder()
        .label("● Recording...")
        .halign(gtk::Align::Start)
        .valign(gtk::Align::Start)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();

    window.add(&label);

    unsafe {
        GTK_WINDOW = Some(window);
    }

    Ok(())
}

pub fn get_cursor_position() -> (i32, i32) {
    if let Some(display) = gtk::gdk::Display::default() {
        if let Some(seat) = display.default_seat() {
            if let Some(pointer) = seat.pointer() {
                let (_, x, y) = pointer.position();
                return (x, y);
            }
        }
    }
    (0, 0)
}

pub fn show_popup(x: i32, y: i32) {
    unsafe {
        if let Some(ref window) = GTK_WINDOW {
            window.move_(x + 20, y + 20);
            window.show_all();
        }
    }
}

pub fn hide_popup() {
    unsafe {
        if let Some(ref window) = GTK_WINDOW {
            window.hide();
        }
    }
}

pub fn process_events() {
    while gtk::events_pending() {
        gtk::main_iteration();
    }
}
