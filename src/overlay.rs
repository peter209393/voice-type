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

use crate::dsp::{LevelMeter, Oscilloscope};
use crate::UiState;

const OVERLAY_HEIGHT: u32 = 100;

pub enum OverlayCmd {
    StartRecording,
    StopRecording,
    Quit,
}

struct OverlayInner {
    width: u32,
    height: u32,
    breath_phase: f32,
    meter: LevelMeter,
    scope: Oscilloscope,
    last_rms: f32,
    last_peak: f32,
    last_osc: Vec<f32>,
}

impl OverlayInner {
    fn new() -> Self {
        Self {
            width: 256,
            height: OVERLAY_HEIGHT,
            breath_phase: 0.0,
            meter: LevelMeter::new(),
            scope: Oscilloscope::new(2048),
            last_rms: 0.0,
            last_peak: 0.0,
            last_osc: vec![0.0; 512],
        }
    }

    fn update(&mut self, dt: f32) {
        self.breath_phase += dt * 1.5;
        if self.breath_phase > std::f32::consts::TAU {
            self.breath_phase -= std::f32::consts::TAU;
        }
    }

    fn drain_audio(&mut self, rx: &Receiver<Vec<f32>>) {
        use crossbeam_channel::TryRecvError;
        loop {
            match rx.try_recv() {
                Ok(chunk) => {
                    self.meter.push_samples(&chunk);
                    self.scope.push_samples(&chunk);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }
        let (rms, peak, _) = self.meter.current();
        self.last_rms = rms;
        self.last_peak = peak;
        self.last_osc = self.scope.render_points(512);
    }

    fn render(&self, canvas: &mut [u8], state: &UiState) {
        let width = self.width as i32;
        let height = self.height as i32;

        let breath = (self.breath_phase.sin() * 0.5 + 0.5) as f32;
        let base_alpha = match state {
            UiState::Recording => 0.6 + breath * 0.3,
            UiState::Transcribing { .. } | UiState::Typing { .. } => 0.5 + breath * 0.2,
            _ => 0.3,
        };

        canvas
            .chunks_exact_mut(4)
            .enumerate()
            .for_each(|(index, chunk)| {
                let _x = (index % width as usize) as i32;
                let y = (index / width as usize) as i32;

                let alpha_mul = if y < 4 || y > height - 4 {
                    let edge = if y < 4 {
                        y as f32 / 4.0
                    } else {
                        (height - y - 1) as f32 / 4.0
                    };
                    edge * base_alpha
                } else {
                    base_alpha
                };

                let r = (20.0 * alpha_mul) as u8;
                let g = (20.0 * alpha_mul) as u8;
                let b = (30.0 * alpha_mul) as u8;
                let a = (alpha_mul * 255.0) as u8;

                let color =
                    ((a as u32) << 24) | ((b as u32) << 16) | ((g as u32) << 8) | (r as u32);
                chunk.copy_from_slice(&color.to_le_bytes());
            });

        let accent_color = match state {
            UiState::Recording => (80u8, 200u8, 255u8),
            UiState::Transcribing { .. } => (255, 200, 80),
            UiState::Typing { .. } => (200, 80, 255),
            UiState::Done { .. } => (80, 255, 140),
            UiState::Error { .. } => (255, 80, 80),
            UiState::Idle => (120, 120, 140),
        };

        let meter_y = height - 20;
        let meter_height = 8i32;
        let rms_width = (self.last_rms.clamp(0.0, 1.0) * width as f32) as i32;
        let peak_x = (self.last_peak.clamp(0.0, 1.0) * width as f32) as i32;

        for y in meter_y..meter_y + meter_height {
            if y >= height {
                break;
            }
            for x in 0..rms_width.min(width) {
                let idx = (y * width + x) as usize * 4;
                if idx + 3 < canvas.len() {
                    let (ar, ag, ab) = accent_color;
                    let color =
                        (0xFFu32 << 24) | ((ab as u32) << 16) | ((ag as u32) << 8) | (ar as u32);
                    let bytes = color.to_le_bytes();
                    canvas[idx..idx + 4].copy_from_slice(&bytes);
                }
            }
        }

        if peak_x > 0 && peak_x < width {
            for y in meter_y..meter_y + meter_height {
                if y >= height {
                    break;
                }
                let idx = (y * width + peak_x) as usize * 4;
                if idx + 3 < canvas.len() {
                    let color = 0xFF_FF_50_50u32;
                    canvas[idx..idx + 4].copy_from_slice(&color.to_le_bytes());
                }
            }
        }

        let osc_mid_y = height as f32 * 0.35;
        let osc_amp = height as f32 * 0.25;
        let n_points = self.last_osc.len().max(2);

        for i in 0..n_points - 1 {
            let t0 = i as f32 / (n_points - 1) as f32;
            let t1 = (i + 1) as f32 / (n_points - 1) as f32;
            let x0 = (t0 * width as f32) as i32;
            let x1 = (t1 * width as f32) as i32;
            let y0 = (osc_mid_y - self.last_osc[i].clamp(-1.0, 1.0) * osc_amp) as i32;
            let y1 = (osc_mid_y - self.last_osc[i + 1].clamp(-1.0, 1.0) * osc_amp) as i32;

            let dx = (x1 - x0).abs();
            let dy = (y1 - y0).abs();
            let sx = if x0 < x1 { 1 } else { -1 };
            let sy = if y0 < y1 { 1 } else { -1 };
            let mut err = dx - dy;
            let mut cx = x0;
            let mut cy = y0;

            let (ar, ag, ab) = accent_color;
            let color = (0xFFu32 << 24) | ((ab as u32) << 16) | ((ag as u32) << 8) | (ar as u32);

            loop {
                if cx >= 0 && cx < width && cy >= 0 && cy < height {
                    let idx = (cy * width + cx) as usize * 4;
                    if idx + 3 < canvas.len() {
                        canvas[idx..idx + 4].copy_from_slice(&color.to_le_bytes());
                    }
                }
                if cx == x1 && cy == y1 {
                    break;
                }
                let e2 = 2 * err;
                if e2 > -dy {
                    err -= dy;
                    cx += sx;
                }
                if e2 < dx {
                    err += dx;
                    cy += sy;
                }
            }
        }
    }
}

struct OverlayApp {
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    shm: Shm,

    exit: bool,
    first_configure: bool,
    pool: SlotPool,
    inner: OverlayInner,
    ui_state: UiState,
    layer: LayerSurface,
    running: Arc<AtomicBool>,
    cmd_rx: Receiver<OverlayCmd>,
    audio_rx: Receiver<Vec<f32>>,
    last_time: Instant,
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
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        self.inner.width = NonZeroU32::new(configure.new_size.0).map_or(256, NonZeroU32::get);
        self.inner.height =
            NonZeroU32::new(configure.new_size.1).map_or(OVERLAY_HEIGHT, NonZeroU32::get);

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

impl OverlayApp {
    fn draw(&mut self, qh: &QueueHandle<Self>) {
        let now = Instant::now();
        let dt = (now - self.last_time).as_secs_f32();
        self.last_time = now;

        self.inner.update(dt);
        self.inner.drain_audio(&self.audio_rx);

        loop {
            match self.cmd_rx.try_recv() {
                Ok(OverlayCmd::Quit) => {
                    self.exit = true;
                    return;
                }
                _ => break,
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

delegate_compositor!(OverlayApp);
delegate_output!(OverlayApp);
delegate_shm!(OverlayApp);
delegate_seat!(OverlayApp);
delegate_layer!(OverlayApp);
delegate_registry!(OverlayApp);

pub fn run_overlay(
    audio_rx: Receiver<Vec<f32>>,
    running: Arc<AtomicBool>,
    cmd_rx: Receiver<OverlayCmd>,
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
        Layer::Overlay,
        Some("sway-voice-type"),
        None,
    );
    layer.set_anchor(Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
    layer.set_keyboard_interactivity(KeyboardInteractivity::None);
    layer.set_exclusive_zone(-1);
    layer.set_margin(0, 0, 80, 0);
    layer.set_size(256, OVERLAY_HEIGHT);
    layer.commit();

    let pool = SlotPool::new(256 * OVERLAY_HEIGHT as usize * 4, &shm)?;

    let mut app = OverlayApp {
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        output_state: OutputState::new(&globals, &qh),
        shm,
        exit: false,
        first_configure: true,
        pool,
        inner: OverlayInner::new(),
        ui_state: UiState::Idle,
        layer,
        running,
        cmd_rx,
        audio_rx,
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
