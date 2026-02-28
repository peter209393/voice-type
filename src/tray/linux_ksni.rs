use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crossbeam_channel::Receiver;
use ksni::{Tray, TrayService};

use crate::UiState;

pub enum TrayCmd {
    UpdateState(UiState),
    Quit,
}

#[derive(Debug, Clone)]
struct VoiceTypeTray {
    state: UiState,
}

impl Tray for VoiceTypeTray {
    fn icon_name(&self) -> String {
        match &self.state {
            UiState::Idle => "microphone-sensitivity-muted",
            UiState::Recording { .. } => "media-record",
            UiState::Transcribing { .. } => "audio-input-microphone",
            UiState::Done { .. } => "dialog-ok",
            UiState::Error { .. } => "dialog-error",
        }
        .to_string()
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
