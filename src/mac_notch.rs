//! The MacBook notch's Now Playing card, drawn by AppKit.
//!
//! A borderless panel the size of the notch sits over it while Fastsonic is
//! in the background. Resting the pointer there grows the panel and drops a
//! card out of the notch: the cover, the song, a seek bar, and the player
//! bar's buttons. What it shows comes from the application each pass
//! (`notch::sync`); what it is asked to do goes back through a
//! [`NotchLink`]. The decisions are in `notch.rs`, where they are tested.
//!
//! The panel never becomes the key window and does not activate the app
//! when clicked: pausing from the notch leaves the app you were in in front.
//!
//! # Lifetime
//!
//! Every AppKit object here belongs to one `Card`, kept in a main-thread
//! slot, and nothing outside it holds one. The card is Fastsonic's own
//! window: it puts no view, observer or delegate into winit's window, so
//! the main window closing does not leave anything of the card's behind in
//! it. In the other direction, [`shutdown`] empties the slot first, then
//! clears each button's target (AppKit does not retain a target), removes
//! the tracking area, orders the panel out and closes it, and only then
//! lets go of it.
//!
//! Anything that calls back later finds the card through the slot, and an
//! empty slot means it does nothing: a hover timer and the cover loader
//! carry a number or nothing, never a pointer, and drawing reads a copy of
//! what to draw rather than the card. The application calls [`shutdown`]
//! whenever its window is replaced or goes (the Winamp switch, closing to
//! the tray) and before it quits, and the next pass builds a fresh card if
//! one is still wanted, so no card made under one event loop lives on into
//! the next.

use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Sel};
use objc2::{AnyThread, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAccessibility, NSAnimatablePropertyContainer, NSAnimationContext, NSApplication,
    NSBackingStoreType, NSBezierPath, NSButton, NSCellImagePosition, NSColor, NSEvent, NSFont,
    NSImage, NSImageScaling, NSImageView, NSLineBreakMode, NSPanel, NSScreen, NSStatusWindowLevel,
    NSTextAlignment, NSTextField, NSTrackingArea, NSTrackingAreaOptions, NSView,
    NSVisualEffectBlendingMode, NSVisualEffectMaterial, NSVisualEffectState, NSVisualEffectView,
    NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_foundation::{MainThreadMarker, NSData, NSObject, NSPoint, NSRect, NSSize, NSString};

use crate::engine::RepeatMode;
use crate::notch::{
    CARD_HEIGHT, CARD_RADIUS, CardLayout, Frame, Geometry, Hover, NotchCommand, NotchLink,
    NotchTrack, SeekBar, Step, Wait,
};
use crate::theme::Icon;

thread_local! {
    /// The card, while there is one. Only the main thread touches it.
    static CARD: RefCell<Option<Card>> = const { RefCell::new(None) };
    /// What the canvas draws, copied out of the card so that drawing,
    /// which AppKit may ask for in the middle of any other call, never
    /// needs to borrow it.
    static CANVAS: Cell<Canvas> = const { Cell::new(Canvas::EMPTY) };
}

/// A cover read from disk, waiting for the main thread.
static LOADED_ART: Mutex<Option<(PathBuf, Vec<u8>)>> = Mutex::new(None);
/// Moves on with every cover asked for, and on shutdown, so a read that
/// finishes after it was overtaken is thrown away.
static ART_GENERATION: AtomicU64 = AtomicU64::new(0);
/// Covers larger than this are not the art cache's and are not read.
const MAX_ART_BYTES: u64 = 10 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Canvas {
    card_w: f64,
    fraction: f64,
    has_art: bool,
}

impl Canvas {
    const EMPTY: Self = Self {
        card_w: crate::notch::CARD_WIDTH,
        fraction: 0.0,
        has_art: false,
    };
}

unsafe extern "C" {
    static _dispatch_main_q: c_void;
    fn dispatch_time(when: u64, delta: i64) -> u64;
    fn dispatch_after_f(
        when: u64,
        queue: *const c_void,
        context: *mut c_void,
        work: extern "C" fn(*mut c_void),
    );
    fn dispatch_async_f(
        queue: *const c_void,
        context: *mut c_void,
        work: extern "C" fn(*mut c_void),
    );
}

fn main_queue() -> *const c_void {
    // A static libdispatch defines for the life of the process; only its
    // address is taken.
    &raw const _dispatch_main_q
}

/// Runs `wait`'s step on the main thread once its delay has passed. The
/// timer carries which wait it is and nothing else; [`Hover`] ignores one
/// that is no longer the wait in progress.
fn start_timer(wait: Wait) {
    let context = match wait {
        Wait::Expand => 1usize,
        Wait::Collapse => 2,
        Wait::Shrink => 3,
    };
    let delay = i64::try_from(wait.delay().as_nanos()).unwrap_or(i64::MAX);
    // Safety: DISPATCH_TIME_NOW plus a delay, on the main queue, with a
    // context that is a plain number.
    unsafe {
        dispatch_after_f(
            dispatch_time(0, delay),
            main_queue(),
            context as *mut c_void,
            on_timer,
        );
    }
}

extern "C" fn on_timer(context: *mut c_void) {
    let wait = match context as usize {
        1 => Wait::Expand,
        2 => Wait::Collapse,
        3 => Wait::Shrink,
        _ => return,
    };
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    with_card(|card| card.due(mtm, wait));
}

/// The card, if there is one and nothing else on this thread is using it.
/// AppKit can call back in the middle of a call the card is making; such a
/// call does nothing rather than borrow the card twice.
fn with_card<R>(f: impl FnOnce(&mut Card) -> R) -> Option<R> {
    CARD.with(|slot| {
        let mut slot = slot.try_borrow_mut().ok()?;
        slot.as_mut().map(f)
    })
}

/// Reads a cover off the main thread and hands it back to it.
fn load_art(path: PathBuf) {
    let generation = ART_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    let spawned = std::thread::Builder::new()
        .name("fastsonic-notch-art".into())
        .spawn(move || {
            let small = std::fs::metadata(&path).is_ok_and(|meta| meta.len() <= MAX_ART_BYTES);
            let Some(bytes) = small.then(|| std::fs::read(&path).ok()).flatten() else {
                return;
            };
            if ART_GENERATION.load(Ordering::SeqCst) != generation {
                return;
            }
            *LOADED_ART.lock().unwrap_or_else(|p| p.into_inner()) = Some((path, bytes));
            // Safety: the main queue, and a context that is nothing at all.
            unsafe { dispatch_async_f(main_queue(), std::ptr::null_mut(), on_art_loaded) };
        });
    if let Err(error) = spawned {
        log::warn!("cannot read the notch card's cover: {error}");
    }
}

extern "C" fn on_art_loaded(_context: *mut c_void) {
    with_card(Card::take_loaded_art);
}

/// Draws one of the app's icons as a template image AppKit tints.
///
/// Rasterized here rather than handed to AppKit as SVG, which `NSImage`
/// reads only on recent macOS releases.
fn icon_image(icon: Icon, points: f64) -> Option<Retained<NSImage>> {
    let png = rasterize(icon.svg(), (points * 2.0).round() as u32)?;
    let image = NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(&png))?;
    image.setTemplate(true);
    image.setSize(NSSize::new(points, points));
    Some(image)
}

/// An SVG drawn square at `pixels`, as PNG bytes.
fn rasterize(svg: &[u8], pixels: u32) -> Option<Vec<u8>> {
    use resvg::{tiny_skia, usvg};
    let tree = usvg::Tree::from_data(svg, &usvg::Options::default()).ok()?;
    let mut pixmap = tiny_skia::Pixmap::new(pixels, pixels)?;
    let size = tree.size();
    let scale = pixels as f32 / size.width().max(size.height());
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    pixmap.encode_png().ok()
}

struct Icons {
    heart: Option<Retained<NSImage>>,
    heart_filled: Option<Retained<NSImage>>,
    previous: Option<Retained<NSImage>>,
    next: Option<Retained<NSImage>>,
    play: Option<Retained<NSImage>>,
    pause: Option<Retained<NSImage>>,
    shuffle: Option<Retained<NSImage>>,
    repeat: Option<Retained<NSImage>>,
    repeat_one: Option<Retained<NSImage>>,
}

impl Icons {
    fn new() -> Self {
        Self {
            heart: icon_image(Icon::Heart, 18.0),
            heart_filled: icon_image(Icon::HeartFilled, 18.0),
            previous: icon_image(Icon::SkipBackFilled, 18.0),
            next: icon_image(Icon::SkipForwardFilled, 18.0),
            play: icon_image(Icon::PlayFilled, 16.0),
            pause: icon_image(Icon::PauseFilled, 16.0),
            shuffle: icon_image(Icon::Shuffle, 16.0),
            repeat: icon_image(Icon::Repeat, 16.0),
            repeat_one: icon_image(Icon::Repeat1, 16.0),
        }
    }
}

/// Inter, the face the rest of the app draws with, at `weight`.
fn inter(size: f64, weight: f64) -> Retained<NSFont> {
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFNumberCreate(
            allocator: *const c_void,
            kind: isize,
            value: *const c_void,
        ) -> *const c_void;
        fn CFRelease(handle: *const c_void);
    }
    #[link(name = "CoreText", kind = "framework")]
    unsafe extern "C" {
        fn CTFontManagerCreateFontDescriptorFromData(data: *const c_void) -> *const c_void;
        fn CTFontDescriptorCreateCopyWithVariation(
            descriptor: *const c_void,
            axis: *const c_void,
            value: f64,
        ) -> *const c_void;
        fn CTFontCreateWithFontDescriptor(
            descriptor: *const c_void,
            size: f64,
            matrix: *const c_void,
        ) -> *const c_void;
    }
    const SINT32: isize = 3;
    let fallback = || {
        if weight >= 600.0 {
            NSFont::boldSystemFontOfSize(size)
        } else {
            NSFont::systemFontOfSize(size)
        }
    };
    let data = NSData::with_bytes(include_bytes!("../assets/fonts/InterVariable.ttf"));
    // Safety: each Create is released once, NSData is toll-free bridged
    // to CFData, and a CTFont is toll-free bridged to an NSFont whose
    // ownership `from_raw` takes over.
    unsafe {
        let base = CTFontManagerCreateFontDescriptorFromData(Retained::as_ptr(&data).cast());
        if base.is_null() {
            return fallback();
        }
        let axis = i32::from_be_bytes(*b"wght");
        let axis = CFNumberCreate(std::ptr::null(), SINT32, (&raw const axis).cast());
        let weighted = if axis.is_null() {
            base
        } else {
            let weighted = CTFontDescriptorCreateCopyWithVariation(base, axis, weight);
            CFRelease(axis);
            if weighted.is_null() {
                base
            } else {
                CFRelease(base);
                weighted
            }
        };
        let font = CTFontCreateWithFontDescriptor(weighted, size, std::ptr::null());
        CFRelease(weighted);
        Retained::from_raw(font.cast::<NSFont>().cast_mut()).unwrap_or_else(fallback)
    }
}

fn color([r, g, b]: [u8; 3]) -> Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(
        f64::from(r) / 255.0,
        f64::from(g) / 255.0,
        f64::from(b) / 255.0,
        1.0,
    )
}

fn quiet() -> Retained<NSColor> {
    NSColor::colorWithWhite_alpha(1.0, 0.7)
}

fn rect(frame: Frame) -> NSRect {
    NSRect::new(
        NSPoint::new(frame.x, frame.y),
        NSSize::new(frame.w, frame.h),
    )
}

fn set_corner_radius(view: &NSView, radius: f64) {
    view.setWantsLayer(true);
    // Safety: a layer-backed view has a CALayer, which takes both.
    unsafe {
        let layer: Option<Retained<AnyObject>> = msg_send![view, layer];
        if let Some(layer) = layer {
            let () = msg_send![&*layer, setCornerRadius: radius];
            let () = msg_send![&*layer, setMasksToBounds: true];
        }
    }
}

/// The safe area's top inset, which only a notched display has. The
/// property is new in macOS 12; earlier releases ran on no notched Mac.
fn top_inset(screen: &NSScreen) -> f64 {
    let answers: bool = unsafe { msg_send![screen, respondsToSelector: sel!(safeAreaInsets)] };
    if answers {
        screen.safeAreaInsets().top
    } else {
        0.0
    }
}

/// Where the card goes, on the first display with a notch.
fn notch_geometry(mtm: MainThreadMarker) -> Option<Geometry> {
    NSScreen::screens(mtm).iter().find_map(|screen| {
        let frame = screen.frame();
        Geometry::new(
            frame.origin.x,
            frame.origin.y,
            frame.size.width,
            frame.size.height,
            top_inset(&screen),
        )
    })
}

/// Turns the views' events into calls on the card.
fn local_point(view: &NSView, event: &NSEvent) -> NSPoint {
    view.convertPoint_fromView(event.locationInWindow(), None)
}

fn push(command: NotchCommand) {
    with_card(|card| card.link.push(command));
}

define_class!(
    /// The object the card's buttons call. AppKit does not retain a
    /// button's target; the card does, and clears the targets before it
    /// lets go of this.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "FastsonicNotchActions"]
    struct Actions;

    impl Actions {
        #[unsafe(method(playPause:))]
        fn play_pause(&self, _sender: Option<&AnyObject>) {
            with_card(Card::play_pause);
        }

        #[unsafe(method(next:))]
        fn next(&self, _sender: Option<&AnyObject>) {
            push(NotchCommand::Next);
        }

        #[unsafe(method(previous:))]
        fn previous(&self, _sender: Option<&AnyObject>) {
            push(NotchCommand::Previous);
        }

        #[unsafe(method(shuffle:))]
        fn shuffle(&self, _sender: Option<&AnyObject>) {
            with_card(Card::toggle_shuffle);
        }

        #[unsafe(method(repeat:))]
        fn repeat(&self, _sender: Option<&AnyObject>) {
            with_card(Card::cycle_repeat);
        }

        #[unsafe(method(like:))]
        fn like(&self, _sender: Option<&AnyObject>) {
            with_card(Card::toggle_saved);
        }
    }
);

define_class!(
    /// A button that answers the first click, while Fastsonic is not the
    /// active app, rather than spending it on activation.
    #[unsafe(super(NSButton))]
    #[thread_kind = MainThreadOnly]
    #[name = "FastsonicNotchButton"]
    struct Button;

    impl Button {
        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
            true
        }
    }
);

define_class!(
    /// The panel's content: the notch itself, which tracks the pointer.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "FastsonicNotchView"]
    struct RootView;

    impl RootView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
            true
        }

        #[unsafe(method(mouseEntered:))]
        fn mouse_entered(&self, _event: &NSEvent) {
            if let Some(mtm) = MainThreadMarker::new() {
                with_card(|card| card.entered(mtm));
            }
        }

        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, _event: &NSEvent) {
            with_card(Card::exited);
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, _event: &NSEvent) {
            push(NotchCommand::ShowWindow);
        }
    }
);

define_class!(
    /// The card's own drawing under its labels and buttons: the panel, the
    /// cover's placeholder, the seek bar, and the disc behind Play. A press
    /// on the bar seeks; a press anywhere else opens the window.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "FastsonicNotchCanvas"]
    struct CanvasView;

    impl CanvasView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
            true
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            let point = local_point(self, event);
            let scrubbing = with_card(|card| card.press(point)).unwrap_or(false);
            if !scrubbing {
                push(NotchCommand::ShowWindow);
            }
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, event: &NSEvent) {
            let point = local_point(self, event);
            with_card(|card| card.drag(point));
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, event: &NSEvent) {
            let point = local_point(self, event);
            with_card(|card| card.release(point));
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty: NSRect) {
            draw(CANVAS.with(Cell::get));
        }
    }
);

define_class!(
    /// The card: a flipped container, so its views are laid out from the
    /// top. A press that reaches it opens the window.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "FastsonicNotchCard"]
    struct CardView;

    impl CardView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
            true
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, _event: &NSEvent) {
            push(NotchCommand::ShowWindow);
        }
    }
);

fn draw(canvas: Canvas) {
    let layout = CardLayout::new(canvas.card_w);
    let card = NSRect::new(NSPoint::ZERO, NSSize::new(canvas.card_w, CARD_HEIGHT));

    // A dark glass over the blur, and a hairline round it.
    let panel =
        NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(card, CARD_RADIUS, CARD_RADIUS);
    NSColor::colorWithSRGBRed_green_blue_alpha(0.08, 0.09, 0.11, 0.78).set();
    panel.fill();
    let edge = NSRect::new(
        NSPoint::new(0.5, 0.5),
        NSSize::new(canvas.card_w - 1.0, CARD_HEIGHT - 1.0),
    );
    let outline =
        NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(edge, CARD_RADIUS, CARD_RADIUS);
    outline.setLineWidth(1.0);
    NSColor::colorWithSRGBRed_green_blue_alpha(0.16, 0.19, 0.22, 0.75).set();
    outline.stroke();

    if !canvas.has_art {
        let art =
            NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(rect(layout.art), 8.0, 8.0);
        NSColor::colorWithWhite_alpha(0.18, 0.9).set();
        art.fill();
    }

    let bar = layout.seek;
    let track = NSRect::new(
        NSPoint::new(bar.start, bar.center_y - 2.0),
        NSSize::new(bar.width(), 4.0),
    );
    NSColor::colorWithWhite_alpha(1.0, 0.2).set();
    NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(track, 2.0, 2.0).fill();
    let played = bar.width() * canvas.fraction;
    NSColor::whiteColor().set();
    if played > 0.0 {
        let fill = NSRect::new(track.origin, NSSize::new(played, 4.0));
        NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(fill, 2.0, 2.0).fill();
    }
    let thumb = NSRect::new(
        NSPoint::new(bar.start + played - 6.0, bar.center_y - 6.0),
        NSSize::new(12.0, 12.0),
    );
    NSBezierPath::bezierPathWithOvalInRect(thumb).fill();

    NSColor::colorWithSRGBRed_green_blue_alpha(0.95, 0.96, 0.97, 1.0).set();
    NSBezierPath::bezierPathWithOvalInRect(rect(layout.play)).fill();
}

fn label(mtm: MainThreadMarker, font: &NSFont, alpha: f64) -> Retained<NSTextField> {
    let field = NSTextField::labelWithString(&NSString::from_str(""), mtm);
    field.setFont(Some(font));
    field.setTextColor(Some(&NSColor::colorWithWhite_alpha(1.0, alpha)));
    field.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
    field
}

fn button(
    mtm: MainThreadMarker,
    actions: &Actions,
    action: Sel,
    image: Option<&NSImage>,
    name: &str,
) -> Retained<Button> {
    let button: Retained<Button> =
        unsafe { msg_send![Button::alloc(mtm), initWithFrame: NSRect::ZERO] };
    button.setBordered(false);
    button.setImagePosition(NSCellImagePosition::ImageOnly);
    // Icons keep the size they are drawn at rather than fill the button.
    button.setImageScaling(NSImageScaling::ScaleProportionallyDown);
    set_image(&button, image, name);
    button.setContentTintColor(Some(&quiet()));
    // Safety: the card keeps `actions` alive for as long as the button
    // has it as a target, and clears the target before letting go.
    unsafe {
        button.setTarget(Some(actions));
        button.setAction(Some(action));
    }
    button
}

/// Shows `image`, or `name` as text if the icon could not be drawn, and
/// names the button for VoiceOver either way.
fn set_image(button: &NSButton, image: Option<&NSImage>, name: &str) {
    let name = NSString::from_str(name);
    match image {
        Some(image) => {
            button.setImage(Some(image));
            button.setTitle(&NSString::from_str(""));
        }
        None => {
            button.setImage(None);
            button.setTitle(&name);
        }
    }
    button.setAccessibilityLabel(Some(&name));
}

/// Fades `views` in from nothing over `seconds`.
fn fade_in(views: &[&NSView], seconds: f64) {
    for view in views {
        view.setAlphaValue(0.0);
    }
    NSAnimationContext::beginGrouping();
    let context = NSAnimationContext::currentContext();
    context.setDuration(seconds);
    context.setAllowsImplicitAnimation(true);
    for view in views {
        view.animator().setAlphaValue(1.0);
    }
    NSAnimationContext::endGrouping();
}

/// Everything the card is made of, and what it shows.
struct Card {
    link: NotchLink,
    panel: Retained<NSPanel>,
    root: Retained<RootView>,
    tracking: Retained<NSTrackingArea>,
    card: Retained<CardView>,
    effect: Retained<NSVisualEffectView>,
    canvas: Retained<CanvasView>,
    art: Retained<NSImageView>,
    title: Retained<NSTextField>,
    artist: Retained<NSTextField>,
    elapsed: Retained<NSTextField>,
    duration: Retained<NSTextField>,
    like: Retained<Button>,
    shuffle: Retained<Button>,
    previous: Retained<Button>,
    play: Retained<Button>,
    next: Retained<Button>,
    repeat: Retained<Button>,
    /// Kept for as long as the buttons name it as their target.
    _actions: Retained<Actions>,
    icons: Icons,
    geometry: Geometry,
    hover: Hover,
    on_screen: bool,
    window_background: bool,
    track: Option<NotchTrack>,
    art_path: Option<PathBuf>,
    /// Where a press on the seek bar is, until it is let go.
    scrub: Option<u32>,
}

impl Card {
    fn new(mtm: MainThreadMarker, link: NotchLink, geometry: Geometry) -> Self {
        let panel: Retained<NSPanel> = unsafe {
            msg_send![
                NSPanel::alloc(mtm),
                initWithContentRect: rect(geometry.collapsed),
                styleMask: NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel,
                backing: NSBackingStoreType::Buffered,
                defer: false,
            ]
        };
        // Safety: the card owns the panel and closes it before dropping it.
        unsafe { panel.setReleasedWhenClosed(false) };
        // A panel hides whenever its app is inactive, which is all the
        // time this one is meant to be seen.
        panel.setHidesOnDeactivate(false);
        panel.setFloatingPanel(true);
        panel.setBecomesKeyOnlyIfNeeded(true);
        // Above the menu bar the notch sits in.
        panel.setLevel(NSStatusWindowLevel);
        panel.setOpaque(false);
        panel.setBackgroundColor(Some(&NSColor::clearColor()));
        panel.setHasShadow(true);
        panel.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::Stationary
                | NSWindowCollectionBehavior::FullScreenAuxiliary
                | NSWindowCollectionBehavior::IgnoresCycle,
        );

        let root: Retained<RootView> = unsafe {
            msg_send![
                RootView::alloc(mtm),
                initWithFrame: NSRect::new(NSPoint::ZERO, rect(geometry.collapsed).size),
            ]
        };
        // Safety: the root view owns the area and the card removes it
        // before the view goes.
        let tracking = unsafe {
            NSTrackingArea::initWithRect_options_owner_userInfo(
                NSTrackingArea::alloc(),
                NSRect::ZERO,
                NSTrackingAreaOptions::MouseEnteredAndExited
                    | NSTrackingAreaOptions::ActiveAlways
                    | NSTrackingAreaOptions::InVisibleRect,
                Some(&root),
                None,
            )
        };
        root.addTrackingArea(&tracking);
        panel.setContentView(Some(&root));

        let card: Retained<CardView> = unsafe {
            msg_send![CardView::alloc(mtm), initWithFrame: rect(geometry.card_collapsed)]
        };
        card.setAlphaValue(0.0);
        set_corner_radius(&card, CARD_RADIUS);
        root.addSubview(&card);

        let effect =
            NSVisualEffectView::initWithFrame(NSVisualEffectView::alloc(mtm), NSRect::ZERO);
        effect.setMaterial(NSVisualEffectMaterial::HUDWindow);
        effect.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
        effect.setState(NSVisualEffectState::Active);
        set_corner_radius(&effect, CARD_RADIUS);
        card.addSubview(&effect);

        let canvas: Retained<CanvasView> =
            unsafe { msg_send![CanvasView::alloc(mtm), initWithFrame: NSRect::ZERO] };
        canvas.setWantsLayer(true);
        canvas.setAccessibilityElement(true);
        canvas.setAccessibilityLabel(Some(&NSString::from_str("Playback position")));
        card.addSubview(&canvas);

        let art = NSImageView::initWithFrame(NSImageView::alloc(mtm), NSRect::ZERO);
        art.setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);
        set_corner_radius(&art, 8.0);
        art.setAccessibilityLabel(Some(&NSString::from_str("Album artwork")));
        card.addSubview(&art);

        let title_font = inter(14.5, 600.0);
        let body_font = inter(12.5, 400.0);
        let time_font = inter(11.0, 400.0);
        let title = label(mtm, &title_font, 1.0);
        let artist = label(mtm, &body_font, 0.65);
        let elapsed = label(mtm, &time_font, 0.6);
        let duration = label(mtm, &time_font, 0.6);
        duration.setAlignment(NSTextAlignment::Right);
        for field in [&title, &artist, &elapsed, &duration] {
            card.addSubview(field);
        }

        let actions: Retained<Actions> = unsafe { msg_send![Actions::alloc(mtm), init] };
        let icons = Icons::new();
        let like = button(
            mtm,
            &actions,
            sel!(like:),
            icons.heart.as_deref(),
            "Save to Liked Songs",
        );
        let shuffle = button(
            mtm,
            &actions,
            sel!(shuffle:),
            icons.shuffle.as_deref(),
            "Shuffle",
        );
        let previous = button(
            mtm,
            &actions,
            sel!(previous:),
            icons.previous.as_deref(),
            "Previous",
        );
        let play = button(
            mtm,
            &actions,
            sel!(playPause:),
            icons.play.as_deref(),
            "Play",
        );
        let next = button(mtm, &actions, sel!(next:), icons.next.as_deref(), "Next");
        let repeat = button(
            mtm,
            &actions,
            sel!(repeat:),
            icons.repeat.as_deref(),
            "Repeat",
        );
        for button in [&like, &shuffle, &previous, &play, &next, &repeat] {
            card.addSubview(button);
        }

        let mut card = Self {
            link,
            panel,
            root,
            tracking,
            card,
            effect,
            canvas,
            art,
            title,
            artist,
            elapsed,
            duration,
            like,
            shuffle,
            previous,
            play,
            next,
            repeat,
            _actions: actions,
            icons,
            geometry,
            hover: Hover::default(),
            on_screen: false,
            window_background: true,
            track: None,
            art_path: None,
            scrub: None,
        };
        card.lay_out();
        card
    }

    /// Puts the panel where `self.geometry` says and the card's views
    /// where the card's width puts them.
    fn lay_out(&mut self) {
        let geometry = self.geometry;
        let (panel, card) = if self.hover.expanded() {
            (geometry.expanded, geometry.card_expanded)
        } else {
            (geometry.collapsed, geometry.card_collapsed)
        };
        self.panel.setFrame_display(rect(panel), false);
        self.card.setFrame(rect(card));
        let card_w = geometry.card_expanded.w;
        let bounds = rect(Frame::new(0.0, 0.0, card_w, CARD_HEIGHT));
        self.effect.setFrame(bounds);
        self.canvas.setFrame(bounds);
        let layout = CardLayout::new(card_w);
        self.art.setFrame(rect(layout.art));
        self.title.setFrame(rect(layout.title));
        self.artist.setFrame(rect(layout.artist));
        self.elapsed.setFrame(rect(layout.elapsed));
        self.duration.setFrame(rect(layout.duration));
        self.like.setFrame(rect(layout.like));
        self.shuffle.setFrame(rect(layout.shuffle));
        self.previous.setFrame(rect(layout.previous));
        self.play.setFrame(rect(layout.play));
        self.next.setFrame(rect(layout.next));
        self.repeat.setFrame(rect(layout.repeat));
        self.redraw();
    }

    /// Whether Fastsonic is what the person is looking at.
    fn foreground(&self, mtm: MainThreadMarker) -> bool {
        let app = NSApplication::sharedApplication(mtm);
        let panel = Retained::as_ptr(&self.panel).cast::<c_void>();
        let window_on_screen = app.windows().iter().any(|window| {
            Retained::as_ptr(&window).cast::<c_void>() != panel
                && window.isVisible()
                && !window.isMiniaturized()
        });
        crate::notch::in_foreground(app.isActive(), window_on_screen, self.window_background)
    }

    fn can_expand(&self, mtm: MainThreadMarker) -> bool {
        self.on_screen && self.track.is_some() && !self.foreground(mtm)
    }

    fn sync(
        &mut self,
        mtm: MainThreadMarker,
        window_background: bool,
        geometry: Option<Geometry>,
        track: Option<&NotchTrack>,
    ) {
        self.window_background = window_background;
        if let Some(geometry) = geometry
            && geometry.differs(&self.geometry)
        {
            self.geometry = geometry;
            self.lay_out();
        }
        self.show(track);
        let foreground = self.foreground(mtm);
        let wanted =
            crate::notch::should_show(true, foreground, track.is_some(), geometry.is_some());
        if wanted && !self.on_screen {
            self.panel.orderFrontRegardless();
            self.on_screen = true;
        } else if !wanted && self.on_screen {
            self.hide();
        }
        self.take_loaded_art();
    }

    /// Off screen, closed, waiting for nothing.
    fn hide(&mut self) {
        self.panel.orderOut(None);
        self.on_screen = false;
        self.hover.reset();
        self.scrub = None;
        self.card.setAlphaValue(0.0);
        self.lay_out();
    }

    fn show(&mut self, next: Option<&NotchTrack>) {
        if crate::notch::is_another_track(self.track.as_ref(), next) {
            let fade = crate::notch::crossfades(self.hover.expanded(), self.track.as_ref(), next);
            self.scrub = None;
            self.track = next.cloned();
            if let Some(track) = next {
                self.fill(track);
                if fade {
                    fade_in(&[&self.title, &self.artist, &self.art], 0.3);
                }
            }
            return;
        }
        let (Some(shown), Some(next)) = (self.track.as_mut(), next) else {
            return;
        };
        let changes = crate::notch::changes(shown, next);
        *shown = next.clone();
        let accent = next.accent;
        if changes.playing {
            self.show_playing(next.playing);
        }
        if changes.saved || changes.accent {
            self.show_saved(next.saved, accent);
        }
        if changes.shuffle || changes.accent {
            self.show_shuffle(next.shuffle, accent);
        }
        if changes.repeat || changes.accent {
            self.show_repeat(next.repeat, accent);
        }
        if changes.art {
            self.show_art(next.art_path.clone());
        }
        if changes.time && self.scrub.is_none() {
            self.show_time(next.position_ms, next.duration_ms);
        }
    }

    fn fill(&mut self, track: &NotchTrack) {
        let title = crate::notch::one_line(&track.title, "Fastsonic");
        self.title.setStringValue(&NSString::from_str(&title));
        let artist = crate::notch::one_line(&track.artist, "");
        self.artist.setStringValue(&NSString::from_str(&artist));
        self.show_playing(track.playing);
        self.show_saved(track.saved, track.accent);
        self.show_shuffle(track.shuffle, track.accent);
        self.show_repeat(track.repeat, track.accent);
        self.show_art(track.art_path.clone());
        self.show_time(track.position_ms, track.duration_ms);
    }

    fn show_playing(&self, playing: bool) {
        let (image, name) = if playing {
            (self.icons.pause.as_deref(), "Pause")
        } else {
            (self.icons.play.as_deref(), "Play")
        };
        set_image(&self.play, image, name);
        // Dark on the light disc.
        self.play
            .setContentTintColor(Some(&NSColor::colorWithSRGBRed_green_blue_alpha(
                0.06, 0.07, 0.08, 1.0,
            )));
    }

    fn show_saved(&self, saved: bool, accent: [u8; 3]) {
        let (image, tint, name) = if saved {
            (
                self.icons.heart_filled.as_deref(),
                color(accent),
                "Remove from Liked Songs",
            )
        } else {
            (self.icons.heart.as_deref(), quiet(), "Save to Liked Songs")
        };
        set_image(&self.like, image, name);
        self.like.setContentTintColor(Some(&tint));
    }

    fn show_shuffle(&self, shuffle: bool, accent: [u8; 3]) {
        let tint = if shuffle { color(accent) } else { quiet() };
        self.shuffle.setContentTintColor(Some(&tint));
    }

    fn show_repeat(&self, repeat: RepeatMode, accent: [u8; 3]) {
        let (image, tint, name) = match repeat {
            RepeatMode::Off => (self.icons.repeat.as_deref(), quiet(), "Repeat"),
            RepeatMode::Context => (self.icons.repeat.as_deref(), color(accent), "Repeat one"),
            RepeatMode::Track => (
                self.icons.repeat_one.as_deref(),
                color(accent),
                "Repeat off",
            ),
        };
        set_image(&self.repeat, image, name);
        self.repeat.setContentTintColor(Some(&tint));
    }

    fn show_art(&mut self, path: Option<PathBuf>) {
        if self.art_path == path {
            return;
        }
        self.art_path = path.clone();
        self.art.setImage(None);
        self.redraw();
        match path {
            Some(path) => load_art(path),
            None => {
                ART_GENERATION.fetch_add(1, Ordering::SeqCst);
            }
        }
    }

    fn take_loaded_art(&mut self) {
        let loaded = LOADED_ART.lock().unwrap_or_else(|p| p.into_inner()).take();
        let Some((path, bytes)) = loaded else {
            return;
        };
        if self.art_path.as_ref() != Some(&path) {
            return;
        }
        let image = NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(&bytes));
        self.art.setImage(image.as_deref());
        self.redraw();
    }

    fn show_time(&self, position_ms: u32, duration_ms: u32) {
        let elapsed = crate::util::format_duration_ms(position_ms);
        let total = crate::util::format_duration_ms(duration_ms);
        self.elapsed.setStringValue(&NSString::from_str(&elapsed));
        self.duration.setStringValue(&NSString::from_str(&total));
        let value = NSString::from_str(&format!("{elapsed} of {total}"));
        // Safety: an accessibility value is any object; this one is a string.
        unsafe { self.canvas.setAccessibilityValue(Some(&value)) };
        self.redraw();
    }

    /// Hands the canvas what to draw and asks for a drawing.
    fn redraw(&self) {
        let position = self
            .scrub
            .or_else(|| self.track.as_ref().map(|track| track.position_ms))
            .unwrap_or(0);
        let duration = self.track.as_ref().map_or(0, |track| track.duration_ms);
        CANVAS.with(|canvas| {
            canvas.set(Canvas {
                card_w: self.geometry.card_expanded.w,
                fraction: SeekBar::fraction(position, duration),
                has_art: self.art.image().is_some(),
            })
        });
        self.canvas.setNeedsDisplay(true);
    }

    fn entered(&mut self, mtm: MainThreadMarker) {
        let step = self.hover.entered(self.can_expand(mtm));
        self.run(step);
    }

    fn exited(&mut self) {
        let step = self.hover.exited();
        self.run(step);
    }

    fn due(&mut self, mtm: MainThreadMarker, wait: Wait) {
        let step = self.hover.due(wait, self.can_expand(mtm));
        self.run(step);
    }

    fn run(&mut self, step: Step) {
        match step {
            Step::Nothing => {}
            Step::Start(wait) => start_timer(wait),
            Step::Expand => {
                self.panel
                    .setFrame_display(rect(self.geometry.expanded), false);
                self.animate_card(
                    self.geometry.card_expanded,
                    1.0,
                    crate::notch::EXPAND_ANIMATION.as_secs_f64(),
                );
                self.panel.invalidateShadow();
            }
            Step::Collapse => {
                self.animate_card(
                    self.geometry.card_collapsed,
                    0.0,
                    crate::notch::COLLAPSE_ANIMATION.as_secs_f64(),
                );
                start_timer(Wait::Shrink);
            }
            Step::Shrink => {
                self.card.setAlphaValue(0.0);
                self.card.setFrame(rect(self.geometry.card_collapsed));
                self.panel
                    .setFrame_display(rect(self.geometry.collapsed), false);
            }
        }
    }

    fn animate_card(&self, frame: Frame, alpha: f64, seconds: f64) {
        NSAnimationContext::beginGrouping();
        let context = NSAnimationContext::currentContext();
        context.setDuration(seconds);
        context.setAllowsImplicitAnimation(true);
        self.card.animator().setFrame(rect(frame));
        self.card.animator().setAlphaValue(alpha);
        NSAnimationContext::endGrouping();
    }

    /// The controls answer at once; the application then makes it true.
    fn play_pause(&mut self) {
        if let Some(track) = self.track.as_mut() {
            track.playing = !track.playing;
            let playing = track.playing;
            self.show_playing(playing);
        }
        self.link.push(NotchCommand::PlayPause);
    }

    fn toggle_shuffle(&mut self) {
        if let Some(track) = self.track.as_mut() {
            track.shuffle = !track.shuffle;
            let (shuffle, accent) = (track.shuffle, track.accent);
            self.show_shuffle(shuffle, accent);
        }
        self.link.push(NotchCommand::ToggleShuffle);
    }

    fn cycle_repeat(&mut self) {
        if let Some(track) = self.track.as_mut() {
            track.repeat = track.repeat.next();
            let (repeat, accent) = (track.repeat, track.accent);
            self.show_repeat(repeat, accent);
        }
        self.link.push(NotchCommand::CycleRepeat);
    }

    fn toggle_saved(&mut self) {
        let Some(track) = self.track.as_mut() else {
            return;
        };
        track.saved = !track.saved;
        let (saved, accent, uri) = (track.saved, track.accent, track.uri.clone());
        self.show_saved(saved, accent);
        self.link.push(NotchCommand::ToggleSaved(uri));
    }

    /// A press on the card. On the seek bar it starts scrubbing and is
    /// answered `true`; anywhere else is left to open the window.
    fn press(&mut self, point: NSPoint) -> bool {
        let bar = CardLayout::new(self.geometry.card_expanded.w).seek;
        let duration = self.track.as_ref().map_or(0, |track| track.duration_ms);
        if !bar.hit(point.x, point.y) || duration == 0 {
            return false;
        }
        self.scrub_to(bar.position_at(point.x, duration), duration);
        true
    }

    fn drag(&mut self, point: NSPoint) {
        if self.scrub.is_none() {
            return;
        }
        let bar = CardLayout::new(self.geometry.card_expanded.w).seek;
        let duration = self.track.as_ref().map_or(0, |track| track.duration_ms);
        self.scrub_to(bar.position_at(point.x, duration), duration);
    }

    /// Letting go of the bar seeks there, once, as the player bar does.
    fn release(&mut self, point: NSPoint) {
        if self.scrub.take().is_none() {
            return;
        }
        let bar = CardLayout::new(self.geometry.card_expanded.w).seek;
        let Some(track) = self.track.as_mut() else {
            return;
        };
        let position = bar.position_at(point.x, track.duration_ms);
        track.position_ms = position;
        let duration = track.duration_ms;
        self.show_time(position, duration);
        self.link.push(NotchCommand::Seek(position));
    }

    fn scrub_to(&mut self, position: u32, duration: u32) {
        self.scrub = Some(position);
        self.show_time(position, duration);
    }

    /// Takes every tie to AppKit down, in the order that leaves nothing
    /// able to call into what is about to go.
    fn tear_down(self) {
        for button in [
            &self.like,
            &self.shuffle,
            &self.previous,
            &self.play,
            &self.next,
            &self.repeat,
        ] {
            // Safety: clearing a target is always sound.
            unsafe {
                button.setTarget(None);
                button.setAction(None);
            }
        }
        self.root.removeTrackingArea(&self.tracking);
        self.panel.orderOut(None);
        self.panel.close();
    }
}

/// Keeps the card in step with the application; see `notch::sync`.
pub fn sync(link: &NotchLink, enabled: bool, window_background: bool, track: Option<&NotchTrack>) {
    if !enabled {
        shutdown();
        return;
    }
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let geometry = notch_geometry(mtm);
    let missing = CARD.with(|slot| slot.try_borrow().is_ok_and(|card| card.is_none()));
    if missing {
        // Made when there is a song to show and somewhere to show it.
        let (Some(geometry), Some(_)) = (geometry, track) else {
            return;
        };
        let card = Card::new(mtm, link.clone(), geometry);
        CARD.with(|slot| *slot.borrow_mut() = Some(card));
    }
    with_card(|card| card.sync(mtm, window_background, geometry, track));
}

/// Takes the card down, if there is one; see the module's notes.
pub fn shutdown() {
    let card = CARD.with(|slot| slot.try_borrow_mut().ok().and_then(|mut slot| slot.take()));
    let Some(card) = card else {
        return;
    };
    ART_GENERATION.fetch_add(1, Ordering::SeqCst);
    LOADED_ART.lock().unwrap_or_else(|p| p.into_inner()).take();
    CANVAS.with(|canvas| canvas.set(Canvas::EMPTY));
    card.tear_down();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every icon the card wears draws, and draws something: a template
    /// image is only its alpha.
    #[test]
    fn every_icon_rasterizes_to_a_visible_png() {
        for icon in [
            Icon::Heart,
            Icon::HeartFilled,
            Icon::SkipBackFilled,
            Icon::SkipForwardFilled,
            Icon::PlayFilled,
            Icon::PauseFilled,
            Icon::Shuffle,
            Icon::Repeat,
            Icon::Repeat1,
        ] {
            let png = rasterize(icon.svg(), 36).unwrap_or_else(|| panic!("{icon:?}"));
            let image = image::load_from_memory(&png).expect("a PNG").to_rgba8();
            assert_eq!(image.dimensions(), (36, 36));
            assert!(
                image.pixels().any(|pixel| pixel[3] > 0),
                "{icon:?} drew nothing"
            );
        }
    }

    #[test]
    fn inter_is_the_face_at_the_asked_size() {
        let font = inter(14.0, 600.0);
        assert!(font.fontName().to_string().contains("Inter"));
        assert_eq!(font.pointSize(), 14.0);
    }

    /// Off the main thread, as in every other test, there is no card to
    /// make, and taking down none is harmless.
    #[test]
    fn without_the_main_thread_nothing_is_made() {
        let link = NotchLink::new(|| {});
        let track = NotchTrack {
            title: "Song".into(),
            duration_ms: 1000,
            ..NotchTrack::default()
        };
        sync(&link, true, true, Some(&track));
        assert!(CARD.with(|slot| slot.borrow().is_none()));
        shutdown();
        assert!(link.drain().is_empty());
    }
}
