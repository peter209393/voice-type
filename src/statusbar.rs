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
use std::num::NonZeroU32;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;
use wayland_client::{
    globals::registry_queue_init,
    protocol::{wl_output, wl_seat, wl_shm, wl_surface},
    Connection, QueueHandle,
};

use crate::UiState;

const STATUSBAR_HEIGHT: u32 = 28;
const ICON_SIZE: u32 = 24;

pub enum StatusBarCmd {
    UpdateState(UiState),
    Quit,
}

struct StatusBarInner {
    width: u32,
    height: u32,
    breath_phase: f32,
}

impl StatusBarInner {
    fn new() -> Self {
        Self {
            width: 256,
            height: STATUSBAR_HEIGHT,
            breath_phase: 0.0,
        }
    }

    fn update(&mut self, dt: f32) {
        self.breath_phase += dt * 2.0;
        if self.breath_phase > std::f32::consts::TAU {
            self.breath_phase -= std::f32::consts::TAU;
        }
    }

    fn render(&self, canvas: &mut [u8], state: &UiState) {
        let width = self.width as i32;
        let height = self.height as i32;

        // Clear with semi-transparent dark background
        canvas
            .chunks_exact_mut(4)
            .enumerate()
            .for_each(|(index, chunk)| {
                let _x = (index % width as usize) as i32;
                let y = (index / width as usize) as i32;

                // Fade edges
                let alpha = if y < 2 {
                    y as f32 / 2.0 * 0.7
                } else if y > height - 3 {
                    (height - y - 1) as f32 / 2.0 * 0.7
                } else {
                    0.7
                };

                let r = (15.0 * alpha) as u8;
                let g = (15.0 * alpha) as u8;
                let b = (20.0 * alpha) as u8;
                let a = (alpha * 255.0) as u8;

                let color =
                    ((a as u32) << 24) | ((b as u32) << 16) | ((g as u32) << 8) | (r as u32);
                chunk.copy_from_slice(&color.to_le_bytes());
            });

        // Icon colors based on state
        let (icon_color, pulsing) = match state {
            UiState::Idle => ((100, 100, 120), false),
            UiState::Recording { .. } => ((255, 60, 60), true),
            UiState::Transcribing { .. } => ((255, 200, 60), false),
            UiState::Typing { .. } => ((200, 60, 255), false),
            UiState::Done { .. } => ((60, 255, 140), false),
            UiState::Error { .. } => ((255, 60, 60), true),
        };

        let pulse = if pulsing {
            (self.breath_phase.sin() * 0.3 + 0.7) as f32
        } else {
            1.0
        };

        // Center position
        let cx = width / 2;
        let cy = height / 2;
        let radius = (ICON_SIZE as f32 / 2.0) as i32;

        // Draw icon based on state
        match state {
            UiState::Idle | UiState::Done { .. } => {
                // Circle (microphone standby)
                self.draw_circle(canvas, width, height, cx, cy, radius, icon_color, pulse);
                // Small stand
                self.draw_line(canvas, width, height, cx, cy + radius, cx, cy + radius + 3, icon_color, 1.0);
            }
            UiState::Recording { .. } => {
                // Pulsing circle (recording)
                self.draw_circle(canvas, width, height, cx, cy, radius, icon_color, pulse);
                // Inner dot
                let inner_r = radius / 3;
                self.draw_circle_filled(canvas, width, height, cx, cy, inner_r, icon_color, pulse);
            }
            UiState::Transcribing { .. } => {
                // Waveform
                let wave_width = ICON_SIZE as i32;
                let wave_height = 8i32;
                for x in -wave_width / 2..wave_width / 2 {
                    let t = (x as f32) / (wave_width as f32) * std::f32::consts::PI;
                    let y = (t.sin() * wave_height as f32) as i32;
                    let px = cx + x;
                    let py = cy + y;
                    self.draw_pixel_safe(canvas, width, height, px, py, icon_color, pulse);
                    // Draw 3px wide line
                    self.draw_pixel_safe(canvas, width, height, px, py + 1, icon_color, pulse);
                    self.draw_pixel_safe(canvas, width, height, px, py - 1, icon_color, pulse);
                }
            }
            UiState::Typing { .. } => {
                // Rectangle (text)
                let rw = 18i32;
                let rh = 12i32;
                self.draw_rect_outline(canvas, width, height, cx - rw / 2, cy - rh / 2, rw, rh, icon_color, pulse);
                // Small lines representing text
                for i in 0..3 {
                    let ly = cy - rh / 2 + 3 + i * 3;
                    self.draw_line(canvas, width, height, cx - rw / 2 + 3, ly, cx + rw / 2 - 3, ly, icon_color, pulse * 0.7);
                }
            }
            UiState::Error { .. } => {
                // X mark
                let padding = 4;
                self.draw_line(canvas, width, height, cx - radius + padding, cy - radius + padding,
                               cx + radius - padding, cy + radius - padding, icon_color, pulse);
                self.draw_line(canvas, width, height, cx + radius - padding, cy - radius + padding,
                               cx - radius + padding, cy + radius - padding, icon_color, pulse);
            }
        }
    }

    fn draw_pixel_safe(&self, canvas: &mut [u8], width: i32, height: i32, x: i32, y: i32, color: (u8, u8, u8), alpha_mul: f32) {
        if x >= 0 && x < width && y >= 0 && y < height {
            let idx = (y * width + x) as usize * 4;
            if idx + 3 < canvas.len() {
                let (r, g, b) = color;
                let a = (alpha_mul * 255.0) as u8;
                let c = ((a as u32) << 24) | ((b as u32) << 16) | ((g as u32) << 8) | (r as u32);
                canvas[idx..idx + 4].copy_from_slice(&c.to_le_bytes());
            }
        }
    }

    fn draw_line(&self, canvas: &mut [u8], width: i32, height: i32, x0: i32, y0: i32, x1: i32, y1: i32, color: (u8, u8, u8), alpha_mul: f32) {
        let dx = (x1 - x0).abs();
        let dy = (y1 - y0).abs();
        let sx = if x0 < x1 { 1 } else { -1 };
        let sy = if y0 < y1 { 1 } else { -1 };
        let mut err = dx - dy;
        let mut x = x0;
        let mut y = y0;

        loop {
            self.draw_pixel_safe(canvas, width, height, x, y, color, alpha_mul);
            if x == x1 && y == y1 {
                break;
            }
            let e2 = 2 * err;
            if e2 > -dy {
                err -= dy;
                x += sx;
            }
            if e2 < dx {
                err += dx;
                y += sy;
            }
        }
    }

    fn draw_circle(&self, canvas: &mut [u8], width: i32, height: i32, cx: i32, cy: i32, radius: i32, color: (u8, u8, u8), alpha_mul: f32) {
        // Draw circle outline using midpoint algorithm
        let mut x = 0i32;
        let mut y = radius;
        let mut d = 1 - radius;

        while x <= y {
            // Draw 8 symmetric points
            for (px, py) in [
                (cx + x, cy + y), (cx - x, cy + y),
                (cx + x, cy - y), (cx - x, cy - y),
                (cx + y, cy + x), (cx - y, cy + x),
                (cx + y, cy - x), (cx - y, cy - x),
            ] {
                self.draw_pixel_safe(canvas, width, height, px, py, color, alpha_mul);
            }
            x += 1;
            if d < 0 {
                d += 2 * x + 1;
            } else {
                y -= 1;
                d += 2 * (x - y) + 1;
            }
        }
    }

    fn draw_circle_filled(&self, canvas: &mut [u8], width: i32, height: i32, cx: i32, cy: i32, radius: i32, color: (u8, u8, u8), alpha_mul: f32) {
        for y in -radius..=radius {
            for x in -radius..=radius {
                if x * x + y * y <= radius * radius {
                    self.draw_pixel_safe(canvas, width, height, cx + x, cy + y, color, alpha_mul);
                }
            }
        }
    }

    fn draw_rect_outline(&self, canvas: &mut [u8], width: i32, height: i32, x: i32, y: i32, w: i32, h: i32, color: (u8, u8, u8), alpha_mul: f32) {
        // Top and bottom
        for px in x..x + w {
            self.draw_pixel_safe(canvas, width, height, px, y, color, alpha_mul);
            self.draw_pixel_safe(canvas, width, height, px, y + h - 1, color, alpha_mul);
        }
        // Left and right
        for py in y..y + h {
            self.draw_pixel_safe(canvas, width, height, x, py, color, alpha_mul);
            self.draw_pixel_safe(canvas, width, height, x + w - 1, py, color, alpha_mul);
        }
    }
}

struct StatusBarApp {
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    shm: Shm,

    exit: bool,
    first_configure: bool,
    pool: SlotPool,
    inner: StatusBarInner,
    ui_state: UiState,
    layer: LayerSurface,
    running: Arc<AtomicBool>,
    cmd_rx: Receiver<StatusBarCmd>,
    last_time: Instant,
}

impl CompositorHandler for StatusBarApp {
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

impl OutputHandler for StatusBarApp {
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

impl LayerShellHandler for StatusBarApp {
    fn closed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _layer: &LayerSurface) {
        self.exit = true;
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        _layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        self.inner.width = NonZeroU32::new(configure.new_size.0).map_or(256, NonZeroU32::get);
        self.inner.height =
            NonZeroU32::new(configure.new_size.1).map_or(STATUSBAR_HEIGHT, NonZeroU32::get);

        if self.first_configure {
            self.first_configure = false;
            self.draw(qh);
        }
    }
}

impl SeatHandler for StatusBarApp {
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

impl ShmHandler for StatusBarApp {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for StatusBarApp {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState];
}

impl StatusBarApp {
    fn draw(&mut self, qh: &QueueHandle<Self>) {
        let now = Instant::now();
        let dt = (now - self.last_time).as_secs_f32();
        self.last_time = now;

        self.inner.update(dt);

        // Check for commands
        loop {
            match self.cmd_rx.try_recv() {
                Ok(StatusBarCmd::UpdateState(state)) => {
                    self.ui_state = state;
                }
                Ok(StatusBarCmd::Quit) => {
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

        let width = self.inner.width;
        let height = self.inner.height;
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

        self.inner.render(&mut canvas, &self.ui_state);

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
}

delegate_compositor!(StatusBarApp);
delegate_output!(StatusBarApp);
delegate_shm!(StatusBarApp);
delegate_seat!(StatusBarApp);
delegate_layer!(StatusBarApp);
delegate_registry!(StatusBarApp);

pub fn run_status_bar(
    running: Arc<AtomicBool>,
    cmd_rx: Receiver<StatusBarCmd>,
) -> Result<()> {
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
        Layer::Top,
        Some("sway-voice-type-status"),
        None,
    );
    layer.set_anchor(Anchor::TOP | Anchor::LEFT | Anchor::RIGHT);
    layer.set_keyboard_interactivity(KeyboardInteractivity::None);
    layer.set_exclusive_zone(-1);
    layer.set_margin(0, 0, 0, 0);
    layer.set_size(256, STATUSBAR_HEIGHT);
    layer.commit();

    let pool = SlotPool::new(256 * STATUSBAR_HEIGHT as usize * 4, &shm)?;

    let mut app = StatusBarApp {
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        output_state: OutputState::new(&globals, &qh),
        shm,
        exit: false,
        first_configure: true,
        pool,
        inner: StatusBarInner::new(),
        ui_state: UiState::Idle,
        layer,
        running,
        cmd_rx,
        last_time: Instant::now(),
    };

    loop {
        event_queue.blocking_dispatch(&mut app)?;

        if app.exit {
            break;
        }
    }

    Ok(())
}
