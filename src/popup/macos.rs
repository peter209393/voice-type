static mut NS_WINDOW: Option<*mut objc::runtime::Object> = None;
static mut SCREEN_HEIGHT: f64 = 0.0;

pub fn get_cursor_position() -> (i32, i32) {
    use core_graphics::event::{CGEvent, CGEventSource, CGEventSourceStateID};

    unsafe {
        if let Ok(source) = CGEventSource::new(CGEventSourceStateID::CombinedSessionState) {
            if let Ok(event) = CGEvent::new(source) {
                let point = event.location();
                return (point.x as i32, point.y as i32);
            }
        }
    }
    (0, 0)
}

pub fn init_popup() -> anyhow::Result<()> {
    use objc::runtime::{Class, Object, BOOL, NO};
    use objc::{class, msg_send, sel, sel_impl};

    unsafe {
        let ns_window = class!(NSWindow);
        let window: *mut Object = msg_send![ns_window, alloc];

        let ns_screen = class!(NSScreen);
        let main_screen: *mut Object = msg_send![ns_screen, mainScreen];
        let screen_frame: core_graphics::geometry::CGRect = msg_send![main_screen, frame];
        SCREEN_HEIGHT = screen_frame.size.height as f64;

        let style_mask: u64 = 0;
        let backing_store_type: u64 = 2;
        let defer: BOOL = NO;

        let initial_frame = core_graphics::geometry::CGRect::new(
            &core_graphics::geometry::CGPoint::new(0.0, 0.0),
            &core_graphics::geometry::CGSize::new(350.0, 50.0),
        );

        let window: *mut Object = msg_send![
            window,
            initWithContentRect:initial_frame
            styleMask:style_mask
            backing:backing_store_type
            defer:defer
        ];

        let window_level: i32 = 3;
        let _: () = msg_send![window, setLevel:window_level];

        let _: () = msg_send![window, setOpaque:NO];

        let ns_color = class!(NSColor);
        let color: *mut Object = msg_send![
            ns_color,
            colorWithCalibratedRed:30.0/255.0
            green:30.0/255.0
            blue:30.0/255.0
            alpha:0.95
        ];
        let _: () = msg_send![window, setBackgroundColor:color];

        let content_view: *mut Object = msg_send![window, contentView];

        let ns_text_field = class!(NSTextField);
        let label: *mut Object = msg_send![ns_text_field, alloc];
        let label_frame = core_graphics::geometry::CGRect::new(
            &core_graphics::geometry::CGPoint::new(12.0, 8.0),
            &core_graphics::geometry::CGSize::new(326.0, 34.0),
        );
        let label: *mut Object = msg_send![label, initWithFrame:label_frame];

        let _: () = msg_send![label, setBezeled:NO];
        let _: () = msg_send![label, setDrawsBackground:NO];
        let _: () = msg_send![label, setEditable:NO];
        let _: () = msg_send![label, setSelectable:NO];

        let ns_font = class!(NSFont);
        let menlo_str = std::ffi::CString::new("Menlo").unwrap();
        let menlo_name: *mut Object =
            msg_send![class!(NSString), stringWithUTF8String:menlo_str.as_ptr()];
        let font: *mut Object = msg_send![ns_font, fontWithName:menlo_name size:14.0];
        let _: () = msg_send![label, setFont:font];

        let rec_text = std::ffi::CString::new("\u{25CF} Recording...").unwrap();
        let rec_str: *mut Object =
            msg_send![class!(NSString), stringWithUTF8String:rec_text.as_ptr()];
        let _: () = msg_send![label, setStringValue:rec_str];

        let ns_color_text = class!(NSColor);
        let white_color: *mut Object = msg_send![ns_color_text, whiteColor];
        let _: () = msg_send![label, setTextColor:white_color];

        let _: () = msg_send![content_view, addSubview:label];

        let _: () = msg_send![window, orderOut:objc::runtime::nil];

        NS_WINDOW = Some(window);
    }

    Ok(())
}

pub fn show_popup(x: i32, y: i32) {
    use objc::{class, msg_send, sel, sel_impl};

    unsafe {
        if let Some(window) = NS_WINDOW {
            let flipped_y = SCREEN_HEIGHT - y as f64 - 50.0;
            let origin = core_graphics::geometry::CGPoint::new((x + 20) as f64, flipped_y);
            let _: () = msg_send![window, setFrameOrigin:origin];
            let _: () = msg_send![window, makeKeyAndOrderFront:objc::runtime::nil];
        }
    }
}

pub fn hide_popup() {
    use objc::{class, msg_send, sel, sel_impl};

    unsafe {
        if let Some(window) = NS_WINDOW {
            let _: () = msg_send![window, orderOut:objc::runtime::nil];
        }
    }
}

pub fn process_events() {}
