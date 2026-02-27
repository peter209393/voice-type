# Plan: Change Hotkey to Right Alt + Add Top Bar Icons

## Overview
This plan covers two main tasks:
1. Change the recording hotkey from Meta/Super to Right Alt (AltGr)
2. Add a status bar at the top of the screen with icons showing the application state

---

## Task 1: Change Hotkey to Right Alt

### File: `src/hotkey.rs`

**Changes:**
- Modify the `target_keys` array (line 26-29) to use `Key::KEY_RIGHTALT` instead of Meta keys
- Update the error message to reference Alt instead of Meta

**Before:**
```rust
let target_keys = [
    (Key::KEY_RIGHTMETA, "Right Meta"),
    (Key::KEY_LEFTMETA, "Left Meta"),
];
```

**After:**
```rust
let target_keys = [
    (Key::KEY_RIGHTALT, "Right Alt"),
];
```

### File: `src/main.rs`

**Changes:**
- Update the user-facing message (line 77) from "Hold Meta/Win key" to "Hold Right Alt"

---

## Task 2: Add Top Bar with Icons

### Approach
Create a new status bar overlay positioned at the top of the screen using the same Wayland layer shell approach as the existing bottom overlay. This will be a separate surface showing status icons.

### New File: `src/statusbar.rs`

Create a new module that implements a top status bar similar to `overlay.rs` but:
- Uses `Layer::Top` instead of `Layer::Overlay`
- Anchors to `Anchor::TOP | Anchor::LEFT | Anchor::RIGHT`
- Smaller height (e.g., 24-32 pixels)
- Renders status icons based on application state

**Icon states to render:**
- Idle: Gray microphone icon
- Recording: Red microphone icon with pulse animation
- Transcribing: Yellow/waveform icon
- Typing: Purple keyboard/text icon
- Error: Red warning icon

**Rendering approach:**
- Use `ab_glyph` (already in dependencies) to render simple text/icon shapes
- Or draw simple geometric shapes as icons (circles, rectangles, lines)
- Canvas is already using softbuffer + tiny-skia for rendering

### File: `src/main.rs`

**Changes:**
- Import the new `statusbar` module
- Spawn the status bar thread (similar to overlay thread)
- Send status updates to the status bar via a channel
- Update status when state changes (Recording start/stop, Transcribing, Typing, Error)

### File: `Cargo.toml`

No new dependencies needed - all required crates are already present:
- `wayland-client`, `smithay-client-toolkit` - for Wayland layer shell
- `softbuffer`, `tiny-skia` - for rendering
- `ab_glyph` - for text/icon rendering

---

## Implementation Steps

1. **Modify `src/hotkey.rs`** (5 min)
   - Change target keys to Right Alt
   - Update error messages

2. **Modify `src/main.rs`** (5 min)
   - Update console message about hotkey

3. **Create `src/statusbar.rs`** (30-45 min)
   - Set up Wayland layer surface for top bar
   - Implement icon rendering for each state
   - Handle status updates via channel
   - Add simple animations (pulse for recording state)

4. **Integrate status bar into `src/main.rs`** (10-15 min)
   - Import and spawn status bar thread
   - Create channel for status updates
   - Send status updates at appropriate state transitions
   - Clean shutdown of status bar

---

## Design Considerations

### Icon Design (Geometric approach - simplest)
- **Idle**: Gray circle representing microphone
- **Recording**: Red circle with pulsing glow
- **Transcribing**: Yellow waveform (sine wave)
- **Typing**: Purple rectangle (text)
- **Error**: Red triangle or X mark

### Positioning
- Top bar height: 28 pixels
- Centered icon (~24x24px)
- Semi-transparent background matching overlay style

### Performance
- Minimal CPU usage when idle
- Only re-render on state change or animation frame
- Separate thread to not block audio processing
