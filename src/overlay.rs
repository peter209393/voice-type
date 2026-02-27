use anyhow::Result;
use crossbeam_channel::Receiver;
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_layer, delegate_output, delegate_registry, delegate_seat,
    delegate_shm,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{SeatHandler, SeatState},
    shell::{
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
        WaylandSurface,
    },
    shm::{slot::SlotPool, Shm, ShmHandler},
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;
use wayland_client::{
    globals::registry_queue_init,
    protocol::{wl_output, wl_seat, wl_shm, wl_surface},
    Connection, QueueHandle,
};

use crate::UiState;

const ICON_SIZE: u32 = 32;
const POPUP_WIDTH: u32 = 500;
const POPUP_HEIGHT: u32 = 120;

pub enum OverlayCmd {
    UpdateState(UiState),
    Quit,
}

struct OverlayApp {
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    shm: Shm,
    exit: bool,
    first_configure: bool,
    pool: SlotPool,
    ui_state: UiState,
    layer: LayerSurface,
    running: Arc<AtomicBool>,
    cmd_rx: Receiver<OverlayCmd>,
    last_time: Instant,
    breath_phase: f32,
}

impl OverlayApp {
    fn draw(&mut self, qh: &QueueHandle<Self>) {
        let now = Instant::now();
        let dt = (now - self.last_time).as_secs_f32();
        self.last_time = now;
        self.breath_phase += dt * 3.0;

        loop {
            match self.cmd_rx.try_recv() {
                Ok(OverlayCmd::UpdateState(state)) => {
                    self.ui_state = state;
                    self.resize_layer();
                }
                Ok(OverlayCmd::Quit) => {
                    self.exit = true;
                    return;
                }
                Err(_) => break,
            }
        }

        if !self.running.load(Ordering::SeqCst) {
            self.exit = true;
            return;
        }

        let (width, height) = match &self.ui_state {
            UiState::Idle => (ICON_SIZE, ICON_SIZE),
            _ => (POPUP_WIDTH, POPUP_HEIGHT),
        };

        let stride = width as i32 * 4;

        let (buffer, mut canvas) = match self.pool.create_buffer(
            width as i32,
            height as i32,
            stride,
            wl_shm::Format::Argb8888,
        ) {
            Ok(result) => result,
            Err(_) => return,
        };

        let breath = self.breath_phase;
        match &self.ui_state {
            UiState::Idle => render_icon(&mut canvas, width, height, breath),
            UiState::Recording { text, .. } => {
                render_popup(&mut canvas, width, height, text, true, breath)
            }
            UiState::Transcribing { .. } => {
                render_popup(&mut canvas, width, height, "Transcribing...", false, breath)
            }
            UiState::Done { text } => render_popup(&mut canvas, width, height, text, false, breath),
            UiState::Error { msg } => render_popup(&mut canvas, width, height, msg, false, breath),
        }

        self.layer
            .wl_surface()
            .damage_buffer(0, 0, width as i32, height as i32);
        self.layer
            .wl_surface()
            .frame(qh, self.layer.wl_surface().clone());

        if buffer.attach_to(self.layer.wl_surface()).is_err() {
            return;
        }
        self.layer.commit();
    }

    fn resize_layer(&mut self) {
        let (width, height) = match &self.ui_state {
            UiState::Idle => (ICON_SIZE, ICON_SIZE),
            _ => (POPUP_WIDTH, POPUP_HEIGHT),
        };

        let anchor = match &self.ui_state {
            UiState::Idle => Anchor::TOP | Anchor::RIGHT,
            _ => Anchor::TOP | Anchor::LEFT | Anchor::RIGHT,
        };

        self.layer.set_anchor(anchor);
        self.layer.set_size(width, height);
        self.layer.commit();
    }
}

fn render_icon(canvas: &mut [u8], width: u32, height: u32, breath_phase: f32) {
    let breath = (breath_phase.sin() * 0.3 + 0.7) as f32;

    canvas.chunks_exact_mut(4).for_each(|chunk| {
        let color = 0x00_10_10_10u32;
        chunk.copy_from_slice(&color.to_le_bytes());
    });

    let cx = width as i32 / 2;
    let cy = height as i32 / 2;
    let r = 10;

    for y in -r..=r {
        for x in -r..=r {
            if x * x + y * y <= r * r {
                let px = cx + x;
                let py = cy + y;
                if px >= 0 && px < width as i32 && py >= 0 && py < height as i32 {
                    let idx = (py * width as i32 + px) as usize * 4;
                    if idx + 3 < canvas.len() {
                        let alpha = (breath * 255.0) as u8;
                        let color = ((alpha as u32) << 24) | 0x00_FF_A8_50u32;
                        canvas[idx..idx + 4].copy_from_slice(&color.to_le_bytes());
                    }
                }
            }
        }
    }
}

fn render_popup(
    canvas: &mut [u8],
    width: u32,
    height: u32,
    text: &str,
    is_recording: bool,
    breath_phase: f32,
) {
    let bg_alpha = if is_recording {
        ((breath_phase.sin() * 0.1 + 0.5) * 255.0) as u8
    } else {
        180
    };

    canvas.chunks_exact_mut(4).for_each(|chunk| {
        let color = ((bg_alpha as u32) << 24) | 0x00_18_18_20u32;
        chunk.copy_from_slice(&color.to_le_bytes());
    });

    if is_recording {
        let dot_r = 6;
        let cx = 20;
        let cy = height as i32 / 2;
        let breath = (breath_phase.sin() * 0.5 + 0.5) as f32;
        let r = (dot_r as f32 * (0.8 + breath * 0.4)) as i32;

        for y in -r..=r {
            for x in -r..=r {
                if x * x + y * y <= r * r {
                    let px = cx + x;
                    let py = cy + y;
                    if px >= 0 && px < width as i32 && py >= 0 && py < height as i32 {
                        let idx = (py * width as i32 + px) as usize * 4;
                        if idx + 3 < canvas.len() {
                            let alpha = (breath * 255.0) as u8;
                            let color = ((alpha as u32) << 24) | 0x00_FF_64_64u32;
                            canvas[idx..idx + 4].copy_from_slice(&color.to_le_bytes());
                        }
                    }
                }
            }
        }
    }

    let text_x = if is_recording { 40 } else { 20 };
    render_text(canvas, width as i32, height as i32, text, text_x, 20);
}

fn render_text(canvas: &mut [u8], width: i32, height: i32, text: &str, x_start: i32, y_start: i32) {
    let display_text = if text.len() > 80 {
        format!("...{}", &text[text.len() - 77..])
    } else {
        text.to_string()
    };

    let mut x = x_start;
    let mut y = y_start;
    let char_width = 9;
    let line_height = 22;
    let max_chars_per_line = (width - x_start - 20) / char_width;
    let mut char_count = 0;

    for c in display_text.chars() {
        if c == '\n' || char_count >= max_chars_per_line as usize {
            x = x_start;
            y += line_height;
            char_count = 0;
            if y > height - line_height {
                break;
            }
            if c == '\n' {
                continue;
            }
        }

        draw_char(canvas, width, height, c, x, y, (255, 255, 255));
        x += char_width;
        char_count += 1;
    }
}

fn draw_char(
    canvas: &mut [u8],
    width: i32,
    height: i32,
    c: char,
    x: i32,
    y: i32,
    color: (u8, u8, u8),
) {
    let pattern: &[&str] = match c {
        'a' | 'A' => &[
            "01110", "10001", "11111", "10001", "10001", "10001", "10001",
        ],
        'b' | 'B' => &[
            "11110", "10001", "10001", "11110", "10001", "10001", "11110",
        ],
        'c' | 'C' => &[
            "01111", "10000", "10000", "10000", "10000", "10000", "01111",
        ],
        'd' | 'D' => &[
            "11110", "10001", "10001", "10001", "10001", "10001", "11110",
        ],
        'e' | 'E' => &[
            "11111", "10000", "10000", "11110", "10000", "10000", "11111",
        ],
        'f' | 'F' => &[
            "11111", "10000", "10000", "11110", "10000", "10000", "10000",
        ],
        'g' | 'G' => &[
            "01111", "10000", "10000", "10111", "10001", "10001", "01110",
        ],
        'h' | 'H' => &[
            "10001", "10001", "10001", "11111", "10001", "10001", "10001",
        ],
        'i' | 'I' => &[
            "11111", "00100", "00100", "00100", "00100", "00100", "11111",
        ],
        'j' | 'J' => &[
            "11111", "00010", "00010", "00010", "00010", "10010", "01100",
        ],
        'k' | 'K' => &[
            "10001", "10010", "10100", "11000", "10100", "10010", "10001",
        ],
        'l' | 'L' => &[
            "10000", "10000", "10000", "10000", "10000", "10000", "11111",
        ],
        'm' | 'M' => &[
            "10001", "11011", "10101", "10101", "10001", "10001", "10001",
        ],
        'n' | 'N' => &[
            "10001", "11001", "10101", "10011", "10001", "10001", "10001",
        ],
        'o' | 'O' => &[
            "01110", "10001", "10001", "10001", "10001", "10001", "01110",
        ],
        'p' | 'P' => &[
            "11110", "10001", "10001", "11110", "10000", "10000", "10000",
        ],
        'q' | 'Q' => &[
            "01110", "10001", "10001", "10001", "10101", "10010", "01101",
        ],
        'r' | 'R' => &[
            "11110", "10001", "10001", "11110", "10100", "10010", "10001",
        ],
        's' | 'S' => &[
            "01111", "10000", "10000", "01110", "00001", "00001", "11110",
        ],
        't' | 'T' => &[
            "11111", "00100", "00100", "00100", "00100", "00100", "00100",
        ],
        'u' | 'U' => &[
            "10001", "10001", "10001", "10001", "10001", "10001", "01110",
        ],
        'v' | 'V' => &[
            "10001", "10001", "10001", "10001", "10001", "01010", "00100",
        ],
        'w' | 'W' => &[
            "10001", "10001", "10001", "10101", "10101", "10101", "01010",
        ],
        'x' | 'X' => &[
            "10001", "10001", "01010", "00100", "01010", "10001", "10001",
        ],
        'y' | 'Y' => &[
            "10001", "10001", "01010", "00100", "00100", "00100", "00100",
        ],
        'z' | 'Z' => &[
            "11111", "00001", "00010", "00100", "01000", "10000", "11111",
        ],
        ' ' => &[
            "     ", "     ", "     ", "     ", "     ", "     ", "     ",
        ],
        '.' => &[
            "     ", "     ", "     ", "     ", "     ", "00100", "00100",
        ],
        ',' => &[
            "     ", "     ", "     ", "     ", "00100", "00100", "01000",
        ],
        '?' => &[
            "01110", "10001", "00010", "00100", "00100", "     ", "00100",
        ],
        '!' => &[
            "00100", "00100", "00100", "00100", "00100", "     ", "00100",
        ],
        '-' => &[
            "     ", "     ", "     ", "11111", "     ", "     ", "     ",
        ],
        '\'' => &[
            "00100", "00100", "01000", "     ", "     ", "     ", "     ",
        ],
        ':' => &[
            "     ", "00100", "00100", "     ", "00100", "00100", "     ",
        ],
        ';' => &[
            "     ", "00100", "00100", "     ", "00100", "01000", "     ",
        ],
        '(' => &[
            "00010", "00100", "01000", "01000", "01000", "00100", "00010",
        ],
        ')' => &[
            "01000", "00100", "00010", "00010", "00010", "00100", "01000",
        ],
        _ => &[
            "11111", "10001", "10001", "10001", "10001", "10001", "11111",
        ],
    };

    let (r, g, b) = color;
    for (dy, row) in pattern.iter().enumerate() {
        for (dx, ch) in row.chars().enumerate() {
            if ch == '1' {
                let px = x + dx as i32;
                let py = y + dy as i32;
                if px >= 0 && px < width && py >= 0 && py < height {
                    let idx = (py * width + px) as usize * 4;
                    if idx + 3 < canvas.len() {
                        let c =
                            0xFF00_0000u32 | ((b as u32) << 16) | ((g as u32) << 8) | (r as u32);
                        canvas[idx..idx + 4].copy_from_slice(&c.to_le_bytes());
                    }
                }
            }
        }
    }
}

impl CompositorHandler for OverlayApp {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_factor: i32,
    ) {
    }
    fn transform_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_transform: wl_output::Transform,
    ) {
    }
    fn frame(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _time: u32,
    ) {
        self.draw(qh);
    }
    fn surface_enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }
    fn surface_leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for OverlayApp {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }
    fn update_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }
    fn output_destroyed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }
}

impl LayerShellHandler for OverlayApp {
    fn closed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _layer: &LayerSurface) {
        self.exit = true;
    }
    fn configure(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        _layer: &LayerSurface,
        _configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        if self.first_configure {
            self.first_configure = false;
            self.draw(qh);
        }
    }
}

impl SeatHandler for OverlayApp {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }
    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
    fn new_capability(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _seat: wl_seat::WlSeat,
        _capability: smithay_client_toolkit::seat::Capability,
    ) {
    }
    fn remove_capability(
        &mut self,
        _conn: &Connection,
        _: &QueueHandle<Self>,
        _: wl_seat::WlSeat,
        _capability: smithay_client_toolkit::seat::Capability,
    ) {
    }
    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl ShmHandler for OverlayApp {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for OverlayApp {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState];
}

delegate_compositor!(OverlayApp);
delegate_output!(OverlayApp);
delegate_shm!(OverlayApp);
delegate_seat!(OverlayApp);
delegate_layer!(OverlayApp);
delegate_registry!(OverlayApp);

pub fn run_overlay(running: Arc<AtomicBool>, cmd_rx: Receiver<OverlayCmd>) -> Result<()> {
    let conn = Connection::connect_to_env()?;
    let (globals, mut event_queue) = registry_queue_init(&conn)?;
    let qh = event_queue.handle();

    let compositor = CompositorState::bind(&globals, &qh)?;
    let layer_shell = LayerShell::bind(&globals, &qh)?;
    let shm = Shm::bind(&globals, &qh)?;

    let surface = compositor.create_surface(&qh);
    let layer = layer_shell.create_layer_surface(
        &qh,
        surface,
        Layer::Overlay,
        Some("sway-voice-type"),
        None,
    );
    layer.set_anchor(Anchor::TOP | Anchor::RIGHT);
    layer.set_keyboard_interactivity(KeyboardInteractivity::None);
    layer.set_exclusive_zone(-1);
    layer.set_margin(10, 10, 0, 0);
    layer.set_size(ICON_SIZE, ICON_SIZE);
    layer.commit();

    let pool = SlotPool::new(POPUP_WIDTH as usize * POPUP_HEIGHT as usize * 4, &shm)?;

    let mut app = OverlayApp {
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        output_state: OutputState::new(&globals, &qh),
        shm,
        exit: false,
        first_configure: true,
        pool,
        ui_state: UiState::Idle,
        layer,
        running,
        cmd_rx,
        last_time: Instant::now(),
        breath_phase: 0.0,
    };

    loop {
        event_queue.blocking_dispatch(&mut app)?;
        if app.exit {
            break;
        }
    }

    Ok(())
}
