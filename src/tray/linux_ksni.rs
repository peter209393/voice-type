use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use crossbeam_channel::Receiver;
use ksni::{Tray, TrayService};

use crate::UiState;
use crate::tray::icons::{IconData, icon_for_state};

pub enum TrayCmd {
    UpdateState(UiState),
    Quit,
}

#[derive(Debug, Clone)]
struct VoiceTypeTray {
    state: UiState,
    /// Pre-rendered pixmap per state, in ksni's ARGB32 big-endian format.
    pixmaps: StatePixmaps,
}

#[derive(Debug, Clone)]
struct StatePixmaps {
    idle: ksni::Icon,
    recording: ksni::Icon,
    transcribing: ksni::Icon,
    done: ksni::Icon,
    error: ksni::Icon,
}

impl StatePixmaps {
    fn build() -> Self {
        let render = |state: &UiState| -> ksni::Icon {
            let icon: IconData = icon_for_state(state);
            ksni::Icon {
                width: icon.width,
                height: icon.height,
                data: icon.to_argb32_be(),
            }
        };
        Self {
            idle: render(&UiState::Idle),
            recording: render(&UiState::Recording { started_at: Instant::now() }),
            transcribing: render(&UiState::Transcribing { started_at: Instant::now() }),
            done: render(&UiState::Done { text: String::new() }),
            error: render(&UiState::Error { msg: String::new() }),
        }
    }

    fn for_state(&self, state: &UiState) -> &ksni::Icon {
        match state {
            UiState::Idle => &self.idle,
            UiState::Recording { .. } => &self.recording,
            UiState::Transcribing { .. } => &self.transcribing,
            UiState::Done { .. } => &self.done,
            UiState::Error { .. } => &self.error,
        }
    }
}

impl Tray for VoiceTypeTray {
    fn id(&self) -> String {
        // A stable id prevents some DEs from spawning duplicate / mismatched items.
        "voice-type".to_string()
    }

    fn icon_name(&self) -> String {
        // Intentionally empty: we ship raw ARGB32 data via `icon_pixmap` so the
        // rendered icon never depends on the active system icon theme.
        String::new()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        vec![self.pixmaps.for_state(&self.state).clone()]
    }

    fn title(&self) -> String {
        "Voice Type".to_string()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        let (title, desc) = match &self.state {
            UiState::Idle => ("Idle", "Press hotkey to start recording"),
            UiState::Recording { .. } => ("Recording", "Speak now..."),
            UiState::Transcribing { .. } => ("Transcribing", "Processing audio..."),
            UiState::Done { text } => ("Done", text.as_str()),
            UiState::Error { msg } => ("Error", msg.as_str()),
        };
        ksni::ToolTip {
            title: title.to_string(),
            description: desc.to_string(),
            ..Default::default()
        }
    }
}

pub fn run_tray(running: Arc<AtomicBool>, cmd_rx: Receiver<TrayCmd>) -> anyhow::Result<()> {
    let tray = VoiceTypeTray {
        state: UiState::Idle,
        pixmaps: StatePixmaps::build(),
    };
    let service = TrayService::new(tray);
    let handle = service.handle();

    std::thread::spawn(move || {
        while let Ok(TrayCmd::UpdateState(state)) = cmd_rx.recv() {
            handle.update(|t: &mut VoiceTypeTray| {
                t.state = state;
            });
        }
    });

    service.run()?;

    running.store(false, Ordering::SeqCst);
    Ok(())
}
