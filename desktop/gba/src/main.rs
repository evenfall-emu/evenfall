//! A desktop GBA player: one window, the keyboard and the default sound
//! device around the GBA core, drawn by the shared presenter.

mod audio;
mod pacing;
mod screen;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use evenfall_audio::StereoSample;
use evenfall_gba::bus::KeyState;
use evenfall_gba::host::host_save;
use evenfall_gba::machine::Machine;
use evenfall_gba::session::{Config, Session};
use evenfall_host::{PixelFormat, VideoFormat};
use evenfall_present::{Preset, Settings};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Fullscreen, Window, WindowId};

use audio::Audio;
use pacing::Pacer;
use screen::Screen;

const USAGE: &str = "usage: evenfall-gba-desktop [--bios <path>] [--save-file <path>] [--preset pixel|vanilla-plus] [--integer-scale] <rom.gba|rom.zip>

Keys: arrows (D-pad), X and Z (A and B), Enter and Backspace (Start and Select),
A and S (L and R), Space (pause), Tab (hold to fast-forward),
F2 (Pixel or Vanilla+), F3 (whole-number scaling), F11 (fullscreen).";

const VIDEO: VideoFormat = VideoFormat { width: 240, height: 160, pixel: PixelFormat::Bgr555 };

/// Frames run per paced frame while Tab is held.
const FAST_FORWARD: u32 = 8;

/// How often a changed battery save is written while playing.
const SAVE_INTERVAL: Duration = Duration::from_secs(5);

struct Arguments {
    rom: String,
    bios: Option<String>,
    save_file: Option<String>,
    settings: Settings,
}

fn parse(args: &[String]) -> Result<Arguments, String> {
    let mut rom = None;
    let mut bios = None;
    let mut save_file = None;
    let mut settings = Settings::default();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let mut value = || args.next().cloned().ok_or_else(|| format!("{arg} needs a value"));
        match arg.as_str() {
            "--bios" => bios = Some(value()?),
            "--save-file" => save_file = Some(value()?),
            "--preset" => {
                settings.preset = match value()?.as_str() {
                    "pixel" => Preset::Pixel,
                    "vanilla-plus" => Preset::VanillaPlus,
                    other => return Err(format!("unknown preset {other:?}")),
                }
            }
            "--integer-scale" => settings.integer_scale = true,
            "-h" | "--help" => return Err(String::new()),
            flag if flag.starts_with('-') => return Err(format!("unknown option {flag}")),
            path if rom.is_none() => rom = Some(path.to_owned()),
            _ => return Err("only one ROM".to_owned()),
        }
    }
    Ok(Arguments { rom: rom.ok_or("no ROM given")?, bios, save_file, settings })
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arguments = match parse(&args) {
        Ok(arguments) => arguments,
        Err(error) => {
            if !error.is_empty() {
                eprintln!("{error}");
            }
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
    };
    if let Err(error) = run(arguments) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run(arguments: Arguments) -> Result<(), String> {
    let options = host_save::Options {
        rom_path: &arguments.rom,
        bios_path: arguments.bios.as_deref(),
        save_path_override: arguments.save_file.as_deref(),
        ..Default::default()
    };
    let rom = host_save::load_rom_file(&arguments.rom).map_err(|error| format!("{error:?}"))?;
    let bios = host_save::load_bios_file(&options).map_err(|error| format!("{error:?}"))?;
    let prepared = host_save::prepare(&rom, &options).map_err(|error| format!("{error:?}"))?;

    // Sound is optional: without a device the game still plays.
    let audio =
        Audio::open().inspect_err(|error| eprintln!("sound: {error}; playing without sound")).ok();
    let rtc_epoch = host_save::resolve_rtc_epoch(
        &rom,
        host_save::RtcOption::Auto,
        host_save::current_unix_epoch(),
    );
    let session = Session::create(Config {
        image: &rom,
        bios: &bios,
        save: prepared.save_data.as_deref(),
        save_type: Some(prepared.save_type),
        rtc_enabled: Some(rtc_epoch.is_some()),
        rtc_epoch: rtc_epoch.unwrap_or(0),
        output_rate: audio.as_ref().map_or(48_000, Audio::rate),
        ..Config::default()
    })
    .map_err(|error| format!("{error:?}"))?;

    let title = std::path::Path::new(&arguments.rom)
        .file_stem()
        .map_or_else(|| "Evenfall".to_owned(), |stem| stem.to_string_lossy().into_owned());
    let mut app = App {
        title,
        save_path: prepared.save_path,
        saved: session.machine.save_data().to_vec(),
        session,
        audio,
        settings: arguments.settings,
        screen: None,
        pacer: Pacer::new(Instant::now()),
        next_cycle: 0,
        keys: KeyState::default(),
        paused: false,
        fast_forward: false,
        last_save: Instant::now(),
        error: None,
    };
    app.next_cycle = app.session.machine.now();
    let event_loop = EventLoop::new().map_err(|error| error.to_string())?;
    event_loop.run_app(&mut app).map_err(|error| error.to_string())?;
    app.save();
    app.error.map_or(Ok(()), Err)
}

struct App {
    title: String,
    save_path: PathBuf,
    /// The battery save as last written.
    saved: Vec<u8>,
    session: Session,
    audio: Option<Audio>,
    settings: Settings,
    screen: Option<Screen>,
    pacer: Pacer,
    /// Where the next frame's run ends, in master cycles. Kept apart from the
    /// machine's clock, which an instruction can carry past it.
    next_cycle: u64,
    keys: KeyState,
    paused: bool,
    fast_forward: bool,
    last_save: Instant,
    error: Option<String>,
}

impl App {
    fn run_frame(&mut self) -> Result<(), String> {
        self.next_cycle += Machine::CYCLES_PER_FRAME;
        self.session.advance(self.next_cycle).map_err(|error| format!("{error:?}"))?;
        if let Some(fault) = self.session.machine.fault() {
            return Err(format!("the game stopped: {fault:?}"));
        }
        let mut samples = [StereoSample::default(); 4096];
        let count = self.session.drain_audio(&mut samples).map_err(|error| format!("{error:?}"))?;
        if let Some(audio) = &self.audio
            && !self.fast_forward
        {
            audio.push(&samples[..count]);
        }
        Ok(())
    }

    /// Writes the battery save when it changed since the last write.
    fn save(&mut self) {
        let data = self.session.machine.save_data();
        if data != self.saved.as_slice() {
            match host_save::write_atomic(&self.save_path, data) {
                Ok(()) => self.saved = data.to_vec(),
                Err(error) => eprintln!("save {}: {error:?}", self.save_path.display()),
            }
        }
        self.last_save = Instant::now();
    }

    fn update_title(&self) {
        if let Some(screen) = &self.screen {
            let state = if self.paused { " (paused)" } else { "" };
            screen.window().set_title(&format!("{}{state} - Evenfall", self.title));
        }
    }

    fn key(&mut self, event_loop: &ActiveEventLoop, code: KeyCode, pressed: bool, repeat: bool) {
        let game_key = match code {
            KeyCode::ArrowUp => Some(&mut self.keys.up),
            KeyCode::ArrowDown => Some(&mut self.keys.down),
            KeyCode::ArrowLeft => Some(&mut self.keys.left),
            KeyCode::ArrowRight => Some(&mut self.keys.right),
            KeyCode::KeyX => Some(&mut self.keys.a),
            KeyCode::KeyZ => Some(&mut self.keys.b),
            KeyCode::Enter => Some(&mut self.keys.start),
            KeyCode::Backspace => Some(&mut self.keys.select),
            KeyCode::KeyA => Some(&mut self.keys.l),
            KeyCode::KeyS => Some(&mut self.keys.r),
            _ => None,
        };
        if let Some(key) = game_key {
            *key = pressed;
            let keys = self.keys;
            if let Err(error) = self.session.set_keys(keys) {
                self.fail(event_loop, format!("{error:?}"));
            }
            return;
        }
        if !pressed || repeat {
            if code == KeyCode::Tab && !pressed {
                self.fast_forward = false;
            }
            return;
        }
        match code {
            KeyCode::Space => {
                self.paused = !self.paused;
                self.pacer = Pacer::new(Instant::now());
                if let Some(audio) = &self.audio {
                    audio.clear();
                }
                self.update_title();
            }
            KeyCode::Tab => self.fast_forward = true,
            KeyCode::F2 => {
                self.settings.preset = match self.settings.preset {
                    Preset::Pixel => Preset::VanillaPlus,
                    Preset::VanillaPlus => Preset::Pixel,
                };
                self.apply_settings();
            }
            KeyCode::F3 => {
                self.settings.integer_scale = !self.settings.integer_scale;
                self.apply_settings();
            }
            KeyCode::F11 => {
                if let Some(screen) = &self.screen {
                    let window = screen.window();
                    let fullscreen = window.fullscreen().is_none();
                    window.set_fullscreen(fullscreen.then_some(Fullscreen::Borderless(None)));
                }
            }
            _ => {}
        }
    }

    fn apply_settings(&mut self) {
        if let Some(screen) = &mut self.screen {
            screen.settings = self.settings;
            screen.window().request_redraw();
        }
    }

    fn release_keys(&mut self) {
        self.keys = KeyState::default();
        self.fast_forward = false;
        let _ = self.session.set_keys(self.keys);
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, error: String) {
        self.error = Some(error);
        event_loop.exit();
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.screen.is_some() {
            return;
        }
        let attributes = Window::default_attributes()
            .with_title(format!("{} - Evenfall", self.title))
            .with_inner_size(LogicalSize::new(720, 480))
            .with_min_inner_size(LogicalSize::new(240, 160));
        let screen = event_loop
            .create_window(attributes)
            .map_err(|error| error.to_string())
            .and_then(|window| Screen::new(event_loop, Arc::new(window), VIDEO, self.settings));
        match screen {
            Ok(screen) => self.screen = Some(screen),
            Err(error) => self.fail(event_loop, error),
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(screen) = &mut self.screen {
                    screen.resize(size.width, size.height);
                    screen.window().request_redraw();
                }
            }
            WindowEvent::RedrawRequested => {
                if let Some(screen) = &mut self.screen {
                    screen.draw(self.session.machine.framebuffer());
                }
            }
            WindowEvent::Focused(false) => self.release_keys(),
            WindowEvent::KeyboardInput {
                event: KeyEvent { physical_key: PhysicalKey::Code(code), state, repeat, .. },
                ..
            } => self.key(event_loop, code, state == ElementState::Pressed, repeat),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.paused || self.screen.is_none() {
            event_loop.set_control_flow(ControlFlow::Wait);
            return;
        }
        let mut ran = 0;
        // A few frames at most per wake-up, so input and drawing keep up.
        while ran < 4 && self.pacer.due(Instant::now()) {
            let frames = if self.fast_forward { FAST_FORWARD } else { 1 };
            for _ in 0..frames {
                if let Err(error) = self.run_frame() {
                    self.fail(event_loop, error);
                    return;
                }
            }
            self.pacer.ran(Instant::now());
            ran += 1;
        }
        if ran > 0
            && let Some(screen) = &self.screen
        {
            screen.window().request_redraw();
        }
        if self.last_save.elapsed() >= SAVE_INTERVAL {
            self.save();
        }
        event_loop.set_control_flow(ControlFlow::WaitUntil(self.pacer.next()));
    }
}
