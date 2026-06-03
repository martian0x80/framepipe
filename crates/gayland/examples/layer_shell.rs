use gayland::{GaylandEvent, InputSource, LayerShellConfig, TrackerConfig, start_tracker};

const HOTKEY_CTRL_SHIFT_R: u64 = 1;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("debug")).init();

    let tracker = start_tracker(
        TrackerConfig::new(InputSource::DirectOpen)
            .with_hotkey_str(HOTKEY_CTRL_SHIFT_R, "Ctrl+Shift+R")?
            .with_layer_shell(LayerShellConfig {
                namespace: "gayland-example".to_string(),
                // Set this above 0.0 if you explicitly want periodic layer-shell re-anchoring.
                // which is ideally what you would want.
                sync_frequency_hz: 0.5,
            }),
    )?;

    //

    while let Ok(event) = tracker.events.recv() {
        match event {
            GaylandEvent::MouseDelta { .. } => {
                // ignore
            }
            GaylandEvent::MousePosition { x, y, anchored, .. } => {
                println!("cursor: {x:.1}, {y:.1}, anchored={anchored}");
            }
            GaylandEvent::KeyboardKey {
                keycode,
                key_name,
                is_modifier,
                pressed,
                ..
            } => {
                println!(
                    "key: code={keycode}, name={}, modifier={is_modifier}, pressed={pressed}",
                    key_name.unwrap_or("<unknown>")
                );
            }
            GaylandEvent::HotkeyTriggered { id, .. } => {
                println!("hotkey triggered: {id}");
            }
            _ => {}
        }
    }

    Ok(())
}
