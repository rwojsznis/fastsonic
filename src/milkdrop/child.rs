//! The MilkDrop child process: a window of its own, with its own OpenGL
//! context and its own winit event loop.
//!
//! winit allows one event loop per process and eframe owns the app's, so
//! MilkDrop runs as a separate process: the app spawns this same binary with
//! `--milkdrop-child` (see `super::host`). It reads the sound from the
//! shared-memory ring the app fills, takes its settings and a close request
//! on stdin as JSON lines, and reports where it sits and when it closes on
//! stdout. Everything projectM makes belongs to this process, so it cannot
//! touch the app's windows.

use std::io::{BufRead, Write};
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::glow;
use glutin::config::ConfigTemplateBuilder;
use glutin::context::{ContextAttributesBuilder, NotCurrentGlContext, PossiblyCurrentContext};
use glutin::display::{GetGlDisplay, GlDisplay};
use glutin::surface::{GlSurface, Surface, SurfaceAttributesBuilder, SwapInterval, WindowSurface};
use glutin_winit::DisplayBuilder;
use raw_window_handle::HasWindowHandle;
use serde::{Deserialize, Serialize};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalPosition, LogicalSize, PhysicalPosition};
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Fullscreen, ResizeDirection, Window, WindowId};

use super::engine::Engine;
use super::overlay::{Backing, Overlay, Place, Row, Span};
use super::shm::Ring;
use super::{DEFAULT_FPS, DEFAULT_SECONDS, LAG, MIN_SIZE, Presets, Request};

/// What the child is told to do, on stdin, as one JSON object per line.
#[derive(Debug, Default, Deserialize)]
struct Control {
    fps: Option<u32>,
    seconds: Option<u32>,
    scale: Option<u32>,
    /// The playing song, as lines to overlay when it changes.
    song: Option<Vec<String>>,
    close: Option<bool>,
}

/// What the child reports back, on stdout, as one JSON object per line.
#[derive(Debug, Default, Serialize)]
struct Event {
    #[serde(skip_serializing_if = "Option::is_none")]
    closed: Option<bool>,
    /// What the window asks the app to do with the player.
    #[serde(skip_serializing_if = "Option::is_none")]
    command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pos: Option<[f32; 2]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    size: Option<[f32; 2]>,
}

/// How the child was started, from the command line.
pub struct Args {
    pub shm: PathBuf,
    pub presets: PathBuf,
    pub size: [f32; 2],
    pub pos: Option<[f32; 2]>,
    pub fullscreen: bool,
    pub fps: u32,
    pub seconds: u32,
    pub scale: u32,
}

impl Args {
    /// Reads the child's arguments from the command line; `None` when this is
    /// not a child process.
    pub fn parse() -> Option<Self> {
        let mut all = std::env::args().skip(1);
        if !std::env::args().any(|arg| arg == "--milkdrop-child") {
            return None;
        }
        let mut shm = None;
        let mut presets = None;
        let mut size = super::DEFAULT_SIZE;
        let mut pos = None;
        let mut fullscreen = false;
        let mut fps = DEFAULT_FPS;
        let mut seconds = DEFAULT_SECONDS;
        let mut scale = 1u32;
        while let Some(arg) = all.next() {
            match arg.as_str() {
                "--milkdrop-shm" => shm = all.next().map(PathBuf::from),
                "--milkdrop-presets" => presets = all.next().map(PathBuf::from),
                "--milkdrop-size" => {
                    if let Some(pair) = all.next().and_then(|value| parse_pair(&value)) {
                        size = pair;
                    }
                }
                "--milkdrop-pos" => pos = all.next().and_then(|value| parse_pair(&value)),
                "--milkdrop-fullscreen" => fullscreen = true,
                "--milkdrop-fps" => fps = all.next().and_then(|v| v.parse().ok()).unwrap_or(fps),
                "--milkdrop-seconds" => {
                    seconds = all.next().and_then(|v| v.parse().ok()).unwrap_or(seconds);
                }
                "--milkdrop-scale" => {
                    scale = all.next().and_then(|v| v.parse().ok()).unwrap_or(scale);
                }
                _ => {}
            }
        }
        Some(Self {
            shm: shm?,
            presets: presets?,
            size,
            pos,
            fullscreen,
            fps,
            seconds,
            scale,
        })
    }
}

fn parse_pair(value: &str) -> Option<[f32; 2]> {
    let (a, b) = value.split_once([',', 'x'])?;
    Some([a.trim().parse().ok()?, b.trim().parse().ok()?])
}

/// Runs the child process to the end; returns its exit code.
pub fn run(args: Args) -> i32 {
    let ring = match Ring::open(&args.shm) {
        Ok(ring) => ring,
        Err(error) => {
            eprintln!("MilkDrop: no audio ring: {error}");
            return 1;
        }
    };
    let event_loop = match EventLoop::<Control>::with_user_event().build() {
        Ok(event_loop) => event_loop,
        Err(error) => {
            eprintln!("MilkDrop: no event loop: {error}");
            return 1;
        }
    };
    // A thread reads control lines from the app and wakes the loop with them;
    // end of input (the app is gone) closes the window.
    let proxy = event_loop.create_proxy();
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        for line in stdin.lock().lines() {
            let Ok(line) = line else { break };
            let control: Control = serde_json::from_str(&line).unwrap_or_default();
            if proxy.send_event(control).is_err() {
                break;
            }
        }
        let _ = proxy.send_event(Control {
            close: Some(true),
            ..Default::default()
        });
    });

    let mut app = Child::new(args, ring);
    let _ = event_loop.run_app(&mut app);
    0
}

/// The live window and everything on its graphics context. Fields drop in
/// order and the engine frees projectM with GL calls, so it comes first.
struct Live {
    engine: Engine,
    gl: Arc<glow::Context>,
    overlay: Option<Overlay>,
    /// What is switched on to stay: the song, the preset's name, the
    /// frame rate. It sits in a corner and does not fade.
    status: Option<Overlay>,
    context: PossiblyCurrentContext,
    surface: Surface<WindowSurface>,
    window: Window,
    fullscreen: bool,
}

struct Child {
    args: Args,
    ring: Ring,
    cursor: u64,
    presets: Presets,
    live: Option<Live>,
    song: Option<Vec<String>>,
    /// When the last frames were drawn, for the count D shows.
    drawn: Vec<Instant>,
    /// Whether the keys are what is on show, so the same key hides them.
    showing_keys: bool,
    /// What the corner is asked to keep showing.
    song_on: bool,
    preset_on: bool,
    fps_on: bool,
    /// What it says now, and when that was written, so it is redrawn
    /// only when it would say something else.
    status_lines: Vec<String>,
    status_written: Option<Instant>,
    modifiers: winit::keyboard::ModifiersState,
    fps: u32,
    scale: u32,
    seconds: u32,
    pointer: PhysicalPosition<f64>,
    last_click: Option<Instant>,
    next_frame: Instant,
    reported: Option<([f32; 2], [f32; 2])>,
}

impl Child {
    fn new(args: Args, ring: Ring) -> Self {
        let fps = args.fps;
        let scale = args.scale;
        let seconds = args.seconds;
        Self {
            args,
            ring,
            cursor: 0,
            presets: Presets::new(),
            live: None,
            song: None,
            drawn: Vec::new(),
            showing_keys: false,
            song_on: false,
            preset_on: false,
            fps_on: false,
            status_lines: Vec::new(),
            status_written: None,
            modifiers: winit::keyboard::ModifiersState::empty(),
            fps,
            scale,
            seconds,
            pointer: PhysicalPosition::new(0.0, 0.0),
            last_click: None,
            next_frame: Instant::now(),
            reported: None,
        }
    }

    fn frame_interval(&self) -> Option<Duration> {
        (self.fps != 0).then(|| Duration::from_secs_f32(1.0 / self.fps as f32))
    }

    fn create(&mut self, event_loop: &ActiveEventLoop) {
        match build(event_loop, &self.args, self.seconds) {
            Ok(live) => {
                live.window.request_redraw();
                self.live = Some(live);
            }
            Err(error) => {
                eprintln!("MilkDrop: {error}");
                report(&Event {
                    closed: Some(true),
                    ..Default::default()
                });
                event_loop.exit();
            }
        }
    }

    fn set_fullscreen(&mut self, on: bool) {
        if let Some(live) = &mut self.live {
            live.window
                .set_fullscreen(on.then(|| Fullscreen::Borderless(None)));
            live.fullscreen = on;
        }
    }

    fn close(&mut self, event_loop: &ActiveEventLoop) {
        report(&Event {
            closed: Some(true),
            ..Default::default()
        });
        self.live = None;
        event_loop.exit();
    }

    fn render(&mut self) {
        let Some(live) = &mut self.live else {
            return;
        };
        let folder = self.args.presets.clone();
        self.presets.refresh(&folder);
        if self.presets.current().is_none() && self.presets.count() > 0 {
            self.presets.next(false);
        }
        if let Some(hard) = live.engine.switch_wanted()
            && !self.presets.locked
        {
            self.presets.next(hard);
        }
        live.engine.set_seconds(self.seconds);
        live.engine.set_locked(self.presets.locked);
        if let Some(Request::Load { path, smooth }) = self.presets.take_request() {
            live.engine.load(&path, smooth);
        }
        let frames = self.ring.since(&mut self.cursor, LAG);
        live.engine.feed_frames(&frames);
        let size = live.window.inner_size();
        live.engine.render(
            0,
            0,
            size.width.max(1),
            size.height.max(1),
            self.scale.max(1),
        );
        if let Some(overlay) = &mut live.overlay {
            overlay.draw(&live.gl, (size.width, size.height));
        }
        if let Some(status) = &mut live.status {
            status.draw(&live.gl, (size.width, size.height));
        }
        if self.drawn.len() >= 60 {
            self.drawn.remove(0);
        }
        self.drawn.push(Instant::now());
        if let Err(error) = live.surface.swap_buffers(&live.context) {
            eprintln!("MilkDrop: present failed: {error}");
        }
        self.update_status();
        self.report_geometry();
    }

    /// Tells the app where the window is, when it has moved or resized.
    fn report_geometry(&mut self) {
        let Some(live) = &self.live else {
            return;
        };
        let size = live.window.inner_size();
        let size = [size.width as f32, size.height as f32];
        let pos = live
            .window
            .outer_position()
            .map(|pos| [pos.x as f32, pos.y as f32])
            .unwrap_or([0.0, 0.0]);
        if self.reported != Some((pos, size)) {
            self.reported = Some((pos, size));
            report(&Event {
                pos: Some(pos),
                size: Some(size),
                ..Default::default()
            });
        }
    }

    fn over_grip(&self) -> bool {
        let Some(live) = &self.live else {
            return false;
        };
        let size = live.window.inner_size();
        self.pointer.x >= size.width as f64 - 16.0 && self.pointer.y >= size.height as f64 - 16.0
    }
}

impl ApplicationHandler<Control> for Child {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.live.is_none() {
            self.create(event_loop);
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, control: Control) {
        if let Some(scale) = control.scale {
            self.scale = scale.clamp(1, 4);
        }
        if let Some(song) = control.song
            && self.song.as_ref() != Some(&song)
        {
            // The way MilkDrop showed the title when the song turned over.
            self.song = Some(song);
            self.show_song();
        }
        if let Some(fps) = control.fps {
            self.fps = fps;
        }
        if let Some(seconds) = control.seconds {
            self.seconds = seconds;
        }
        if control.close == Some(true) {
            self.close(event_loop);
        } else if let Some(live) = &self.live {
            live.window.request_redraw();
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => self.close(event_loop),
            WindowEvent::Resized(size) => {
                if let Some(live) = &self.live
                    && let (Some(width), Some(height)) =
                        (NonZeroU32::new(size.width), NonZeroU32::new(size.height))
                {
                    live.surface.resize(&live.context, width, height);
                    live.window.request_redraw();
                }
            }
            WindowEvent::CursorMoved { position, .. } => self.pointer = position,
            WindowEvent::ModifiersChanged(modifiers) => self.modifiers = modifiers.state(),
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button,
                ..
            } => self.on_press(button),
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                self.on_key(event.logical_key, event_loop);
            }
            WindowEvent::RedrawRequested => {
                self.render();
                self.next_frame = Instant::now() + self.frame_interval().unwrap_or(Duration::ZERO);
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let Some(live) = &self.live else {
            event_loop.set_control_flow(winit::event_loop::ControlFlow::Wait);
            return;
        };
        match self.frame_interval() {
            None => {
                live.window.request_redraw();
                event_loop.set_control_flow(winit::event_loop::ControlFlow::Poll);
            }
            Some(_) => {
                if Instant::now() >= self.next_frame {
                    live.window.request_redraw();
                } else {
                    event_loop.set_control_flow(winit::event_loop::ControlFlow::WaitUntil(
                        self.next_frame,
                    ));
                }
            }
        }
    }
}

impl Child {
    fn on_press(&mut self, button: MouseButton) {
        let Some(live) = &self.live else {
            return;
        };
        match button {
            MouseButton::Left => {
                if self.over_grip() {
                    let _ = live.window.drag_resize_window(ResizeDirection::SouthEast);
                    return;
                }
                let now = Instant::now();
                let double = self
                    .last_click
                    .is_some_and(|last| now.duration_since(last) < Duration::from_millis(350));
                if double {
                    self.last_click = None;
                    let on = !live.fullscreen;
                    self.set_fullscreen(on);
                } else {
                    self.last_click = Some(now);
                    let _ = live.window.drag_window();
                }
            }
            MouseButton::Right => self.presets.next(false),
            _ => {}
        }
    }

    fn on_key(&mut self, key: Key, event_loop: &ActiveEventLoop) {
        let fullscreen = self.live.as_ref().is_some_and(|live| live.fullscreen);
        // Control (Command on the Mac) is what the app's own playback
        // shortcuts are held with; a plain key is the window's own.
        let control = self.modifiers.control_key() || self.modifiers.super_key();
        let plain = !control && !self.modifiers.alt_key();
        match key.as_ref() {
            Key::Named(NamedKey::Escape) => {
                if fullscreen {
                    self.set_fullscreen(false);
                } else {
                    self.close(event_loop);
                }
            }
            // Presets are what this window is for, so they have the
            // plain keys; playback keeps the app's own bindings, so the
            // hands that know one know the other.
            Key::Named(NamedKey::ArrowRight) if plain => self.presets.next(false),
            Key::Named(NamedKey::ArrowLeft) if plain => self.presets.previous(),
            Key::Character("n") | Key::Character("N") if plain => self.presets.next(false),
            Key::Character("p") | Key::Character("P") if plain => self.presets.previous(),
            Key::Character("h") | Key::Character("H") if plain => self.presets.next(true),
            Key::Character("l") | Key::Character("L") if plain => {
                self.presets.locked = !self.presets.locked;
                let note = if self.presets.locked {
                    "Preset kept"
                } else {
                    "Preset free again"
                };
                self.show_note(note.into());
            }
            Key::Character("r") | Key::Character("R") if plain => {
                let note = if self.presets.toggle_order() {
                    "Random order"
                } else {
                    "Folder order"
                };
                self.show_note(note.into());
            }
            // Playback, in the app's own bindings.
            Key::Named(NamedKey::Space) if plain => command("play-pause"),
            Key::Named(NamedKey::ArrowLeft) if control => command("previous"),
            Key::Named(NamedKey::ArrowRight) if control => command("next"),
            Key::Named(NamedKey::ArrowUp) if control => command("volume-up"),
            Key::Named(NamedKey::ArrowDown) if control => command("volume-down"),
            Key::Character("m") | Key::Character("M") if plain => command("mute"),
            Key::Character("s") | Key::Character("S") if plain => command("shuffle"),
            // The window itself.
            Key::Named(NamedKey::Enter) if self.modifiers.alt_key() => {
                self.set_fullscreen(!fullscreen)
            }
            Key::Character("f") | Key::Character("F") if plain => self.set_fullscreen(!fullscreen),
            // What it can tell you.
            Key::Character("?") | Key::Named(NamedKey::F1) => self.show_keys(),
            Key::Character("i") | Key::Character("I") if plain => self.toggle_status(Status::Song),
            Key::Character("t") | Key::Character("T") if plain => {
                self.toggle_status(Status::Preset)
            }
            Key::Character("d") | Key::Character("D") if plain => self.toggle_status(Status::Fps),
            _ => {}
        }
    }

    /// Every key this window answers, in two columns over the picture.
    /// The list is the window's own bindings: what is here is what works.
    fn show_keys(&mut self) {
        let Some(live) = &mut self.live else {
            return;
        };
        let Some(overlay) = &mut live.overlay else {
            return;
        };
        if overlay.showing() && self.showing_keys {
            // The same key that opened it puts it away again.
            overlay.hide();
            self.showing_keys = false;
            return;
        }
        let heading = |text: &str| Row::Heading(Span::new(text, 12.0).weight(700.0).tint(0.62));
        let keys = |key: &str, does: &str| Row::Keys {
            key: Span::new(key, 14.0).weight(600.0),
            does: Span::new(does, 14.0).tint(0.86),
        };
        let rows = [
            Row::Line(Span::new("MilkDrop", 22.0).weight(700.0)),
            Row::Gap(10.0),
            heading("PRESETS"),
            Row::Gap(3.0),
            keys("\u{2192}  or  N", "Next preset"),
            keys("\u{2190}  or  P", "Previous preset"),
            keys("H", "Next preset, cut on the beat"),
            keys("L", "Keep this preset"),
            keys("R", "Random or folder order"),
            keys("Right-click", "Next preset"),
            Row::Gap(9.0),
            heading("PLAYBACK"),
            Row::Gap(3.0),
            keys("Space", "Play or pause"),
            keys("Ctrl+\u{2190}  /  Ctrl+\u{2192}", "Previous or next song"),
            keys("Ctrl+\u{2191}  /  Ctrl+\u{2193}", "Volume up or down"),
            keys("M", "Mute or unmute"),
            keys("S", "Shuffle"),
            Row::Gap(9.0),
            heading("WINDOW"),
            Row::Gap(3.0),
            keys("F, Alt+Enter, double-click", "Full screen"),
            keys("Esc", "Leave full screen, or close"),
            keys("Drag", "Move it; drag a corner to resize"),
            Row::Gap(9.0),
            heading("SHOW"),
            Row::Gap(3.0),
            keys("?  or  F1", "These keys"),
            keys("I", "What is playing, on or off"),
            keys("T", "This preset's name, on or off"),
            keys("D", "FPS, on or off"),
        ];
        overlay.show(
            &live.gl,
            &rows,
            Place::Center,
            Backing::Box,
            Duration::from_secs(12),
            window_size(&live.window),
        );
        self.showing_keys = true;
        live.window.request_redraw();
    }

    /// A line of its own, low in the picture: a short answer to a key.
    fn show_note(&mut self, text: String) {
        let Some(live) = &mut self.live else {
            return;
        };
        let Some(overlay) = &mut live.overlay else {
            return;
        };
        overlay.show(
            &live.gl,
            &[Row::Line(Span::new(text, 15.0).weight(600.0))],
            Place::BottomLeft,
            Backing::Shadow,
            Duration::from_secs(3),
            window_size(&live.window),
        );
        self.showing_keys = false;
        live.window.request_redraw();
    }

    /// Keeps something in the corner, or stops keeping it there. What is
    /// on is written into one block, so two of them cannot overlap.
    fn toggle_status(&mut self, what: Status) {
        match what {
            Status::Song => self.song_on = !self.song_on,
            Status::Preset => self.preset_on = !self.preset_on,
            Status::Fps => self.fps_on = !self.fps_on,
        }
        // A key press answers at once, whatever the pace above.
        self.status_written = None;
        self.update_status();
        if let Some(live) = &self.live {
            live.window.request_redraw();
        }
    }

    /// What the corner says now: the song, the preset, the frame rate,
    /// in that order, and each only if it was asked for.
    fn status_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        if self.song_on {
            let playing = self
                .song
                .as_ref()
                .map(|song| {
                    let named: Vec<&str> = song
                        .iter()
                        .take(2)
                        .map(String::as_str)
                        .filter(|line| !line.is_empty())
                        .collect();
                    named.join("  \u{00b7}  ")
                })
                .unwrap_or_default();
            lines.push(if playing.is_empty() {
                "Nothing playing".to_string()
            } else {
                playing
            });
        }
        if self.preset_on {
            lines.push(
                self.presets
                    .current()
                    .and_then(|path| path.file_stem())
                    .map(|stem| stem.to_string_lossy().to_string())
                    .unwrap_or_else(|| "No preset".into()),
            );
        }
        if self.fps_on {
            lines.push(format!("{:.0} FPS", frames_per_second(&self.drawn)));
        }
        lines
    }

    /// Writes the corner again when it would say something else, and no
    /// more often than a few times a second while the count is moving.
    fn update_status(&mut self) {
        let lines = self.status_lines();
        if lines.is_empty() {
            self.status_lines.clear();
            if let Some(live) = &mut self.live
                && let Some(status) = &mut live.status
            {
                status.hide();
            }
            return;
        }
        // Saying the same thing again is not worth a new bitmap, and a
        // frame rate that flickers between two numbers is not worth one
        // several times a second either.
        if lines == self.status_lines {
            return;
        }
        let waited = self
            .status_written
            .is_none_or(|at| at.elapsed() >= Duration::from_millis(400));
        if !waited {
            return;
        }
        let rows: Vec<Row> = lines
            .iter()
            .map(|line| Row::Line(Span::new(line.clone(), 13.0).weight(600.0)))
            .collect();
        let Some(live) = &mut self.live else {
            return;
        };
        let Some(status) = &mut live.status else {
            return;
        };
        self.status_lines = lines;
        self.status_written = Some(Instant::now());
        status.show(
            &live.gl,
            &rows,
            Place::TopLeft,
            Backing::Shadow,
            // Long enough that it never fades on its own; the key that
            // turned it on is what turns it off.
            Duration::from_secs(60 * 60),
            window_size(&live.window),
        );
    }

    /// What is playing, big in the middle of the picture: the title, then
    /// the artist, then the album, fading out into the visuals again.
    fn show_song(&mut self) {
        let Some(live) = &mut self.live else {
            return;
        };
        let (Some(overlay), Some(song)) = (&mut live.overlay, &self.song) else {
            return;
        };
        let mut rows: Vec<Row> = Vec::new();
        for (index, text) in song.iter().take(3).enumerate() {
            if text.is_empty() {
                continue;
            }
            let span = match index {
                0 => Span::new(text.clone(), 27.0).weight(700.0),
                1 => Span::new(text.clone(), 19.0).weight(500.0).tint(0.92),
                _ => Span::new(text.clone(), 15.0).tint(0.68),
            };
            if index > 0 {
                rows.push(Row::Gap(3.0));
            }
            rows.push(Row::Line(span));
        }
        if rows.is_empty() {
            return;
        }
        overlay.show(
            &live.gl,
            &rows,
            Place::Center,
            Backing::Shadow,
            Duration::from_secs(4),
            window_size(&live.window),
        );
        self.showing_keys = false;
        live.window.request_redraw();
    }
}

/// What the corner can be asked to keep showing.
#[derive(Clone, Copy)]
enum Status {
    Song,
    Preset,
    Fps,
}

/// Frames a second, from when the last frames were drawn.
fn frames_per_second(drawn: &[Instant]) -> f32 {
    let (Some(first), Some(last)) = (drawn.first(), drawn.last()) else {
        return 0.0;
    };
    let span = last.duration_since(*first);
    if drawn.len() < 2 || span.is_zero() {
        return 0.0;
    }
    (drawn.len() - 1) as f32 / span.as_secs_f32()
}

/// The window's size in pixels, which the overlay lays itself out for.
fn window_size(window: &Window) -> (u32, u32) {
    let size = window.inner_size();
    (size.width.max(1), size.height.max(1))
}

/// Asks the app for something only it can do: the player is over there.
fn command(what: &str) {
    report(&Event {
        command: Some(what.to_string()),
        ..Default::default()
    });
}

/// Writes one event line to the app and flushes it. The line is tagged so
/// the app can tell it from libprojectM's own chatter on the same stream.
fn report(event: &Event) {
    if let Ok(line) = serde_json::to_string(event) {
        let mut stdout = std::io::stdout().lock();
        let _ = writeln!(stdout, "{}{line}", super::host::EVENT_PREFIX);
        let _ = stdout.flush();
    }
}

/// Builds the window, a GL context and surface for it, and projectM.
fn build(event_loop: &ActiveEventLoop, args: &Args, seconds: u32) -> Result<Live, String> {
    let mut attributes = Window::default_attributes()
        .with_title("MilkDrop")
        .with_decorations(false)
        .with_resizable(true)
        .with_min_inner_size(LogicalSize::new(MIN_SIZE[0], MIN_SIZE[1]))
        .with_inner_size(LogicalSize::new(args.size[0], args.size[1]));
    if let Some(pos) = args.pos {
        attributes = attributes.with_position(LogicalPosition::new(pos[0], pos[1]));
    }

    // Opaque: no alpha, so the window is never see-through.
    let template = ConfigTemplateBuilder::new().with_alpha_size(0);
    let (window, config) = DisplayBuilder::new()
        .with_window_attributes(Some(attributes))
        .build(event_loop, template, |mut configs| {
            configs.next().expect("a GL config for MilkDrop")
        })
        .map_err(|error| format!("no GL config: {error}"))?;
    let window = window.ok_or_else(|| "the window was not created".to_string())?;

    let raw = window
        .window_handle()
        .map_err(|error| format!("no window handle: {error}"))?
        .as_raw();
    let context_attributes = ContextAttributesBuilder::new().build(Some(raw));
    let not_current = unsafe {
        config
            .display()
            .create_context(&config, &context_attributes)
    }
    .map_err(|error| format!("no GL context: {error}"))?;

    let inner = window.inner_size();
    let surface_attributes = SurfaceAttributesBuilder::<WindowSurface>::new().build(
        raw,
        NonZeroU32::new(inner.width).unwrap_or(NonZeroU32::MIN),
        NonZeroU32::new(inner.height).unwrap_or(NonZeroU32::MIN),
    );
    let surface = unsafe {
        config
            .display()
            .create_window_surface(&config, &surface_attributes)
    }
    .map_err(|error| format!("no GL surface: {error}"))?;
    let context = not_current
        .make_current(&surface)
        .map_err(|error| format!("could not make the context current: {error}"))?;
    let _ = surface.set_swap_interval(&context, SwapInterval::DontWait);

    let gl = Arc::new(unsafe {
        glow::Context::from_loader_function_cstr(|symbol| config.display().get_proc_address(symbol))
    });
    let texture_dirs = [args.presets.clone(), args.presets.join("textures")];
    let dirs: Vec<&std::path::Path> = texture_dirs.iter().map(|dir| dir.as_path()).collect();
    let engine = Engine::new(Arc::clone(&gl), &dirs, seconds)
        .ok_or_else(|| "needs OpenGL 3.3".to_string())?;
    // Text over the picture; the picture goes on without it if the
    // shaders will not take.
    let overlay = Overlay::new(&gl);
    let status = Overlay::new(&gl);

    let mut live = Live {
        engine,
        gl,
        overlay,
        status,
        context,
        surface,
        window,
        fullscreen: false,
    };
    if args.fullscreen {
        live.window
            .set_fullscreen(Some(Fullscreen::Borderless(None)));
        live.fullscreen = true;
    }
    Ok(live)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A child with no window, for the parts that need none.
    fn headless_child() -> Child {
        let dir =
            std::env::temp_dir().join(format!("fastpotify-milkdrop-child-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a place for the test's ring");
        let shm = dir.join("ring");
        let ring = Ring::create(&shm).expect("a ring to read");
        Child::new(
            Args {
                shm,
                presets: dir,
                size: [640.0, 480.0],
                pos: None,
                fullscreen: false,
                fps: 30,
                seconds: 30,
                scale: 1,
            },
            ring,
        )
    }

    /// The corner carries what has been switched on, in one block, in
    /// the same order however they were turned on.
    #[test]
    fn the_corner_shows_only_what_is_switched_on() {
        let mut child = headless_child();
        assert!(child.status_lines().is_empty(), "nothing on, nothing shown");

        child.fps_on = true;
        assert_eq!(child.status_lines().len(), 1);
        assert!(child.status_lines()[0].ends_with("FPS"));

        child.song = Some(vec![
            "Wish You Were Here".into(),
            "Incubus".into(),
            "Morning View".into(),
        ]);
        child.song_on = true;
        let lines = child.status_lines();
        assert_eq!(lines.len(), 2, "the song joins the frame rate");
        assert!(
            lines[0].starts_with("Wish You Were Here") && lines[0].contains("Incubus"),
            "the song line names the song and who plays it: {}",
            lines[0]
        );
        assert!(lines[1].ends_with("FPS"), "the frame rate stays last");

        child.presets.files = vec![PathBuf::from("/presets/Geiss - Spiral Artifact.milk")];
        child.presets.next(true);
        child.preset_on = true;
        let lines = child.status_lines();
        assert_eq!(
            lines[1], "Geiss - Spiral Artifact",
            "the preset sits between the song and the frame rate"
        );
        assert_eq!(lines.len(), 3);

        child.song_on = false;
        child.preset_on = false;
        child.fps_on = false;
        assert!(child.status_lines().is_empty(), "and off again");
    }

    /// The count is frames over the time they took, and it says nothing
    /// at all until there are two of them to measure between.
    #[test]
    fn frames_a_second_are_counted_over_the_time_they_took() {
        assert_eq!(frames_per_second(&[]), 0.0);
        let now = Instant::now();
        assert_eq!(frames_per_second(&[now]), 0.0, "one frame measures nothing");

        // Thirty frames, a fiftieth of a second apart: fifty a second.
        let drawn: Vec<Instant> = (0..30)
            .map(|index| now + Duration::from_micros(20_000 * index))
            .collect();
        let fps = frames_per_second(&drawn);
        assert!(
            (fps - 50.0).abs() < 0.001,
            "twenty milliseconds a frame is fifty a second, not {fps}"
        );
    }
}
