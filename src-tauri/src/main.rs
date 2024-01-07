// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{path::Path, process::exit, time::Duration};

use gilrs::{Axis, Button, Event, EventType, Gilrs};
use serde::{Deserialize, Serialize};
use tauri::{async_runtime::Mutex, AppHandle, Manager, Menu, MenuEntry, Runtime, Submenu};
use tauri_plugin_sql::{Migration, MigrationKind};
#[allow(unused_imports)]
use window_vibrancy::{apply_blur, apply_mica, apply_vibrancy, NSVisualEffectMaterial};

// Learn more about Tauri commands at https://tauri.app/v1/guides/features/command

#[tauri::command]
fn get_home_dir() -> Option<String> {
    dirs::home_dir()
        .as_ref()
        .and_then(|path| path.to_str())
        .map(|s| s.to_string())
}

#[tauri::command]
fn get_target() -> &'static str {
    env!("RUSTC_TARGET")
}

#[derive(Serialize, Deserialize, Default, Clone)]
struct Args {
    workspace: Option<String>,
    code: Option<String>,
}

#[tauri::command]
fn get_args<R: Runtime>(app: AppHandle<R>) -> Args {
    let args = app.state::<Args>();
    (*args).clone()
}

struct GamepadInput {
    method: Mutex<Gilrs>,
}

#[derive(Serialize, Clone)]
enum GamepadDigital {
    L1,
    L2,
    R1,
    R2,
    Up,
    Down,
    Left,
    Right,
    X,
    B,
    Y,
    A,
}

impl TryFrom<Button> for GamepadDigital {
    type Error = ();

    fn try_from(button: Button) -> Result<Self, Self::Error> {
        match button {
            Button::LeftTrigger => Ok(Self::L1),
            Button::RightTrigger => Ok(Self::R1),
            Button::LeftTrigger2 => Ok(Self::L2),
            Button::RightTrigger2 => Ok(Self::R2),
            Button::DPadUp => Ok(Self::Up),
            Button::DPadDown => Ok(Self::Down),
            Button::DPadLeft => Ok(Self::Left),
            Button::DPadRight => Ok(Self::Right),
            Button::North => Ok(Self::X),
            Button::South => Ok(Self::B),
            Button::West => Ok(Self::Y),
            Button::East => Ok(Self::A),
            _ => Err(()),
        }
    }
}

#[derive(Serialize, Clone)]
enum GamepadAnalog {
    LeftX,
    LeftY,
    RightX,
    RightY,
}

impl TryFrom<Axis> for GamepadAnalog {
    type Error = ();

    fn try_from(axis: Axis) -> Result<Self, Self::Error> {
        match axis {
            Axis::LeftStickX => Ok(Self::LeftX),
            Axis::LeftStickY => Ok(Self::LeftY),
            Axis::RightStickX => Ok(Self::RightX),
            Axis::RightStickY => Ok(Self::RightY),
            _ => Err(()),
        }
    }
}

#[derive(Serialize, Clone)]
enum GamepadUpdate {
    Analog { name: GamepadAnalog, state: f32 },
    Digital { name: GamepadDigital, state: bool },
    Connected,
    Disconnected,
}

#[derive(Serialize, Clone)]
struct GamepadEvent {
    id: usize,
    name: String,
    uuid: [u8; 16],
    update: GamepadUpdate,
}

fn broadcast_gamepad<R: Runtime>(app: AppHandle<R>) {
    tauri::async_runtime::spawn_blocking(move || {
        let input = app.state::<GamepadInput>();
        loop {
            let mut gilrs = input.method.blocking_lock();
            while let Some(Event { id, event, .. }) =
                gilrs.next_event_blocking(Some(Duration::from_millis(1000 / 60)))
            {
                let update = match event {
                    EventType::Connected => Some(GamepadUpdate::Connected),
                    EventType::Disconnected => Some(GamepadUpdate::Disconnected),
                    EventType::AxisChanged(axis, val, _) => {
                        if let Ok(analog) = GamepadAnalog::try_from(axis) {
                            Some(GamepadUpdate::Analog {
                                name: analog,
                                state: val,
                            })
                        } else {
                            None
                        }
                    }
                    EventType::ButtonChanged(button, val, _) => {
                        if let Ok(digital) = GamepadDigital::try_from(button) {
                            Some(GamepadUpdate::Digital {
                                name: digital,
                                state: val >= 0.5,
                            })
                        } else {
                            None
                        }
                    }
                    _ => None,
                };
                if let Some(update) = update {
                    let gamepad = gilrs.gamepad(id);
                    _ = app.emit_all(
                        "gamepad",
                        GamepadEvent {
                            id: id.into(),
                            name: gamepad.name().to_string(),
                            uuid: gamepad.uuid(),
                            update,
                        },
                    );
                }
            }
        }
    });
}

#[tauri::command]
async fn connect_all_gamepads<R: Runtime>(
    app: AppHandle<R>,
    input: tauri::State<'_, GamepadInput>,
) -> tauri::Result<()> {
    let gilrs = input.method.lock().await;
    for (id, _) in gilrs.gamepads() {
        let gamepad = gilrs.gamepad(id);
        _ = app.emit_all(
            "gamepad",
            GamepadEvent {
                id: id.into(),
                name: gamepad.name().to_string(),
                uuid: gamepad.uuid(),
                update: GamepadUpdate::Connected,
            },
        );
    }

    Ok(())
}

fn main() {
    _ = fix_path_env::fix();
    let db = tauri_plugin_sql::Builder::default()
        .add_migrations(
            "sqlite:pros_rs.sqlite",
            vec![Migration {
                version: 1,
                description: "v1",
                sql: include_str!("../migrations/1.sql"),
                kind: MigrationKind::Up,
            }],
        )
        .build();
    tauri::Builder::default()
        .manage(GamepadInput {
            method: Mutex::new(Gilrs::new().unwrap()),
        })
        .setup(|app| {
            match app.get_cli_matches() {
                Ok(matches) => {
                    let mut args = Args::default();
                    if let Some(help) = matches.args.get("help") {
                        eprintln!("{}", help.value.as_str().unwrap());
                        exit(1);
                    }
                    if matches.args.get("version").is_some() {
                        println!("{}", app.package_info().version);
                        exit(0);
                    }
                    if let Some(ws) = matches.args.get("workspace").and_then(|m| m.value.as_str()) {
                        let path = Path::new(ws)
                            .canonicalize()
                            .expect("Failed to open workspace");
                        if !path.is_dir() {
                            eprintln!("Workspace not a directory: {ws}");
                            exit(1);
                        }
                        app.fs_scope().allow_directory(&path, true).unwrap();
                        args.workspace = Some(format!("{}", path.display()));
                    }
                    if let Some(user_code) = matches.args.get("code").and_then(|m| m.value.as_str())
                    {
                        if let Ok(path) = Path::new(user_code).canonicalize() {
                            app.fs_scope().allow_file(&path).unwrap();
                            args.code = Some(format!("{}", path.display()));
                        } else {
                            eprintln!("Cannot open robot code: {user_code}");
                            exit(1);
                        }
                    }

                    app.manage(args);
                }
                Err(e) => {
                    eprintln!("{e}");
                    exit(1);
                }
            }

            broadcast_gamepad(app.app_handle());

            let window = app.get_window("main").unwrap();

            #[cfg(target_os = "macos")]
            apply_vibrancy(
                &window,
                NSVisualEffectMaterial::WindowBackground,
                None,
                None,
            )
            .unwrap();

            #[cfg(target_os = "windows")]
            {
                if let Err(_) = apply_mica(&window, None) {}
                window.set_decorations(true).unwrap();
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_home_dir,
            get_target,
            connect_all_gamepads,
            get_args,
        ])
        .plugin(tauri_plugin_persisted_scope::init())
        .plugin(db)
        .plugin(tauri_plugin_upload::init())
        .plugin(tauri_plugin_gamepad::init())
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
