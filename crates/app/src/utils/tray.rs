use std::sync::{Arc, LazyLock};

use image::GenericImageView as _;
use ksni::blocking::{Handle, TrayMethods};

use crate::{app::signals::CaptureControl, utils::types::ProcessState};

#[derive(Clone)]
pub struct TrayCallbacks {
    pub start_recording: Arc<dyn Fn() + Send + Sync>,
    pub stop_recording: Arc<dyn Fn() + Send + Sync>,
    pub pause_recording: Arc<dyn Fn() + Send + Sync>,
    pub resume_recording: Arc<dyn Fn() + Send + Sync>,
    pub hide_window: Arc<dyn Fn() + Send + Sync>,
    pub show_window: Arc<dyn Fn() + Send + Sync>,
    pub quit: Arc<dyn Fn() + Send + Sync>,
}

impl Default for TrayCallbacks {
    fn default() -> Self {
        Self {
            start_recording: Arc::new(|| {}),
            stop_recording: Arc::new(|| {}),
            pause_recording: Arc::new(|| {}),
            resume_recording: Arc::new(|| {}),
            hide_window: Arc::new(|| {}),
            show_window: Arc::new(|| {}),
            quit: Arc::new(|| std::process::exit(0)),
        }
    }
}

struct MyTray {
    state: ProcessState,
    hide_window_when_recording: bool,
    window_hidden: bool,
    control: Option<CaptureControl>,
    callbacks: TrayCallbacks,
}

impl MyTray {
    fn is_running(&self) -> bool {
        matches!(self.state, ProcessState::Running | ProcessState::Paused)
    }

    fn is_paused(&self) -> bool {
        matches!(self.state, ProcessState::Paused)
    }
}

impl ksni::Tray for MyTray {
    fn id(&self) -> String {
        "framepipe".into()
    }

    // fn icon_name(&self) -> String {
    //     "media-record".into()
    // }

    fn title(&self) -> String {
        format!("Framepipe ({})", self.state)
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: "Framepipe".to_string(),
            description: self.state.to_body(),
            icon_name: self.icon_name(),
            icon_pixmap: self.icon_pixmap(),
        }
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        static ICON: LazyLock<ksni::Icon> = LazyLock::new(|| {
            let img = image::load_from_memory_with_format(
                include_bytes!("../../../../assets/icons/icon-64.png"),
                image::ImageFormat::Png,
            )
            .expect("valid image");
            let (width, height) = img.dimensions();
            let mut data = img.into_rgba8().into_vec();
            for pixel in data.chunks_exact_mut(4) {
                pixel.rotate_right(1); // RGBA -> ARGB
            }
            ksni::Icon {
                width: width as i32,
                height: height as i32,
                data,
            }
        });

        vec![ICON.clone()]
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::*;

        let mut items: Vec<ksni::MenuItem<Self>> = Vec::new();

        if self.is_running() {
            if self.is_paused() {
                items.push(
                    StandardItem {
                        label: "Resume Recording".into(),
                        icon_name: "media-playback-start".into(),
                        activate: Box::new(|tray: &mut Self| {
                            if let Some(ctrl) = &tray.control {
                                ctrl.request_resume();
                                ctrl.paused
                                    .store(false, std::sync::atomic::Ordering::Relaxed);
                            }
                            (tray.callbacks.resume_recording)();
                            tray.state = ProcessState::Running;
                        }),
                        ..Default::default()
                    }
                    .into(),
                );
            } else {
                items.push(
                    StandardItem {
                        label: "Pause Recording".into(),
                        icon_name: "media-playback-pause".into(),
                        activate: Box::new(|tray: &mut Self| {
                            if let Some(ctrl) = &tray.control {
                                ctrl.request_pause();
                                ctrl.paused
                                    .store(true, std::sync::atomic::Ordering::Relaxed);
                            }
                            (tray.callbacks.pause_recording)();
                            tray.state = ProcessState::Paused;
                        }),
                        ..Default::default()
                    }
                    .into(),
                );
            }

            items.push(
                StandardItem {
                    label: "Stop Recording".into(),
                    icon_name: "media-playback-stop".into(),
                    activate: Box::new(|tray: &mut Self| {
                        if let Some(ctrl) = &tray.control {
                            ctrl.request_stop();
                        }
                        (tray.callbacks.stop_recording)();
                        tray.state = ProcessState::Stopped(None);
                    }),
                    ..Default::default()
                }
                .into(),
            );
        } else {
            items.push(
                StandardItem {
                    label: "Start Recording".into(),
                    icon_name: "media-record".into(),
                    activate: Box::new(|tray: &mut Self| {
                        (tray.callbacks.start_recording)();
                        tray.state = ProcessState::Running;
                        if tray.hide_window_when_recording {
                            tray.window_hidden = true;
                            (tray.callbacks.hide_window)();
                        }
                    }),
                    ..Default::default()
                }
                .into(),
            );
        }

        // let options = SubMenu {
        //     label: "Options".into(),
        //     icon_name: "preferences-system".into(),
        //     submenu: vec![
        //         CheckmarkItem {
        //             label: "Hide Window when recording".into(),
        //             checked: self.hide_window_when_recording,
        //             activate: Box::new(|tray: &mut Self| {
        //                 tray.hide_window_when_recording = !tray.hide_window_when_recording;
        //             }),
        //             ..Default::default()
        //         }
        //         .into(),
        //         StandardItem {
        //             label: if self.window_hidden {
        //                 "Show Window".into()
        //             } else {
        //                 "Hide Window".into()
        //             },
        //             icon_name: "window".into(),
        //             activate: Box::new(|tray: &mut Self| {
        //                 tray.window_hidden = !tray.window_hidden;
        //                 if tray.window_hidden {
        //                     (tray.callbacks.hide_window)();
        //                 } else {
        //                     (tray.callbacks.show_window)();
        //                 }
        //             }),
        //             ..Default::default()
        //         }
        //         .into(),
        //     ],
        //     ..Default::default()
        // };
        // items.push(options.into());

        items.push(ksni::MenuItem::Separator);
        items.push(
            StandardItem {
                label: "Exit".into(),
                icon_name: "application-exit".into(),
                activate: Box::new(|tray: &mut Self| {
                    if let Some(ctrl) = &tray.control {
                        ctrl.request_stop();
                    }
                    (tray.callbacks.quit)();
                }),
                ..Default::default()
            }
            .into(),
        );

        items
    }
}

pub struct TrayController {
    handle: Handle<MyTray>,
}

impl TrayController {
    pub fn set_process_state(&self, state: ProcessState) {
        self.handle.update(|tray| {
            tray.state = state;
            if !matches!(tray.state, ProcessState::Running | ProcessState::Paused) {
                tray.window_hidden = false;
            }
        });
    }

    pub fn set_capture_control(&self, control: Option<CaptureControl>) {
        self.handle.update(move |tray| {
            tray.control = control;
        });
    }

    pub fn set_hide_window_when_recording(&self, enabled: bool) {
        self.handle
            .update(|tray| tray.hide_window_when_recording = enabled);
    }
}

pub fn spawn_tray(
    initial_state: ProcessState,
    callbacks: TrayCallbacks,
) -> Result<TrayController, String> {
    let tray = MyTray {
        state: initial_state,
        hide_window_when_recording: false,
        window_hidden: false,
        control: None,
        callbacks,
    };
    let handle = tray
        .spawn()
        .map_err(|e| format!("tray spawn failed: {e}"))?;
    Ok(TrayController { handle })
}

pub fn run_tray() {
    let _ = spawn_tray(ProcessState::Stopped(None), TrayCallbacks::default());
    loop {
        std::thread::park();
    }
}
