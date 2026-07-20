//! A minimal GTK4 front end for the `bbcat` library.
//!
//! Files follow one pipeline: `bbcat` decodes them into a `Document`, static
//! screens become one GTK texture, and animations replace that texture from a
//! GTK main-loop timer.

use std::{
    cell::{Cell, RefCell},
    path::Path,
    rc::Rc,
    time::Duration,
};

use gtk4::{gdk, gio, glib, prelude::*};

#[derive(Clone)]
struct ViewerState {
    playback: Rc<Cell<u64>>,
    native_size: Rc<Cell<Option<(i32, i32)>>>,
    document: Rc<RefCell<Option<Rc<bbcat::Document>>>>,
    scale: Rc<Cell<usize>>,
}

fn main() -> glib::ExitCode {
    glib::set_application_name("bbcat");

    // HANDLES_OPEN routes command-line and desktop-entry files to `open`;
    // launching without a file uses the ordinary `activate` signal.
    let app = gtk4::Application::builder()
        .application_id("dev.bbcat.GtkViewer")
        .flags(gio::ApplicationFlags::HANDLES_OPEN)
        .build();

    app.connect_startup(|_| {
        // The containing widget remains visible as black letterboxing whenever
        // the picture and window have different aspect ratios.
        let provider = gtk4::CssProvider::new();
        provider.load_from_data(".artwork { background-color: black; }");
        if let Some(display) = gdk::Display::default() {
            gtk4::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }
    });
    app.connect_activate(|app| show_window(app, None));
    app.connect_open(|app, files, _| {
        show_window(app, files.first().and_then(gio::File::path).as_deref());
    });

    app.run()
}

fn show_window(app: &gtk4::Application, path: Option<&Path>) {
    // Contain preserves the rendered pixel aspect ratio when the picture is in
    // responsive mode. Oversized art switches to native-size scrolling below.
    let picture = gtk4::Picture::builder()
        .can_shrink(true)
        .content_fit(gtk4::ContentFit::Contain)
        .hexpand(true)
        .vexpand(true)
        .build();
    // Each load receives a generation number. Old animation timers become
    // harmless as soon as a newer file increments it.
    let state = ViewerState {
        playback: Rc::new(Cell::new(0_u64)),
        // Native-size mode uses this to center the axis that does not scroll.
        native_size: Rc::new(Cell::new(None)),
        document: Rc::new(RefCell::new(None)),
        scale: Rc::new(Cell::new(1)),
    };
    // Scrollbars start disabled for aspect-fit mode and are enabled only when
    // the artwork is larger than the monitor.
    let scroller = gtk4::ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vscrollbar_policy(gtk4::PolicyType::Never)
        .overlay_scrolling(false)
        .hexpand(true)
        .vexpand(true)
        .child(&picture)
        .build();
    // Overlay gives the scroller the entire black viewing area for alignment,
    // while leaving its natural size out of the window's minimum size.
    let viewer = gtk4::Overlay::builder().hexpand(true).vexpand(true).build();
    viewer.add_css_class("artwork");
    viewer.add_overlay(&scroller);
    for property in ["width", "height"] {
        let viewer_weak = viewer.downgrade();
        let scroller = scroller.downgrade();
        let native_size = state.native_size.clone();
        viewer.connect_notify_local(Some(property), move |_, _| {
            if let (Some(viewer), Some(scroller)) = (viewer_weak.upgrade(), scroller.upgrade()) {
                update_content_layout(&viewer, &scroller, native_size.get());
            }
        });
    }

    let open_button = gtk4::Button::builder()
        .icon_name("document-open-symbolic")
        .tooltip_text("Open ANSI art")
        .build();
    let header = gtk4::HeaderBar::new();
    header.pack_start(&open_button);
    let scale_1 = gtk4::ToggleButton::builder()
        .label("×1")
        .tooltip_text("Render at original size")
        .active(true)
        .build();
    let scale_2 = gtk4::ToggleButton::builder()
        .label("×2")
        .tooltip_text("Render at double size")
        .group(&scale_1)
        .build();
    let scale_buttons = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    scale_buttons.add_css_class("linked");
    scale_buttons.append(&scale_1);
    scale_buttons.append(&scale_2);
    header.pack_end(&scale_buttons);

    let window = gtk4::ApplicationWindow::builder()
        .application(app)
        .title("bbcat")
        .default_width(900)
        .default_height(700)
        .titlebar(&header)
        .child(&viewer)
        .build();

    // Grouped toggle buttons behave like a two-choice scale selector. bbcat
    // performs the integer scaling, keeping text-mode pixels crisp.
    for (button, selected_scale) in [(scale_1, 1), (scale_2, 2)] {
        let window = window.downgrade();
        let picture = picture.downgrade();
        let viewer = viewer.downgrade();
        let scroller = scroller.downgrade();
        let state = state.clone();
        button.connect_toggled(move |button| {
            if !button.is_active() {
                return;
            }
            state.scale.set(selected_scale);
            let Some(document) = state.document.borrow().clone() else {
                return;
            };
            let (Some(window), Some(picture), Some(viewer), Some(scroller)) = (
                window.upgrade(),
                picture.upgrade(),
                viewer.upgrade(),
                scroller.upgrade(),
            ) else {
                return;
            };
            if let Err(error) = display_document(
                document,
                selected_scale,
                &window,
                &picture,
                &viewer,
                &scroller,
                &state,
            ) {
                show_error(&window, &error);
            }
        });
    }

    open_button.connect_clicked({
        // Signal handlers live on their widgets. Weak references avoid keeping
        // the complete window hierarchy alive after the window is closed.
        let window = window.downgrade();
        let picture = picture.downgrade();
        let viewer = viewer.downgrade();
        let scroller = scroller.downgrade();
        let state = state.clone();
        move |_| {
            if let (Some(window), Some(picture), Some(viewer), Some(scroller)) = (
                window.upgrade(),
                picture.upgrade(),
                viewer.upgrade(),
                scroller.upgrade(),
            ) {
                choose_file(&window, &picture, &viewer, &scroller, &state);
            }
        }
    });

    window.present();

    if let Some(path) = path {
        load_file(path, &window, &picture, &viewer, &scroller, &state);
    }
}

fn choose_file(
    window: &gtk4::ApplicationWindow,
    picture: &gtk4::Picture,
    viewer: &gtk4::Overlay,
    scroller: &gtk4::ScrolledWindow,
    state: &ViewerState,
) {
    // FileChooserNative uses the desktop's preferred chooser rather than a
    // custom GTK file-browser window.
    let chooser = gtk4::FileChooserNative::builder()
        .title("Open ANSI art")
        .transient_for(window)
        .modal(true)
        .action(gtk4::FileChooserAction::Open)
        .accept_label("Open")
        .cancel_label("Cancel")
        .build();

    chooser.connect_response({
        let window = window.clone();
        let picture = picture.clone();
        let viewer = viewer.clone();
        let scroller = scroller.clone();
        let state = state.clone();
        move |chooser, response| {
            if response == gtk4::ResponseType::Accept
                && let Some(path) = chooser.file().and_then(|file| file.path())
            {
                load_file(&path, &window, &picture, &viewer, &scroller, &state);
            }
            chooser.destroy();
        }
    });
    chooser.show();
}

fn load_file(
    path: &Path,
    window: &gtk4::ApplicationWindow,
    picture: &gtk4::Picture,
    viewer: &gtk4::Overlay,
    scroller: &gtk4::ScrolledWindow,
    state: &ViewerState,
) {
    // Invalidating the previous generation stops its next timer callback from
    // scheduling another animation frame.
    state.playback.set(state.playback.get().wrapping_add(1));

    let result = std::fs::read(path)
        .map_err(|error| error.to_string())
        .and_then(|data| {
            // The filename lets bbcat use extension hints for formats whose
            // contents alone are ambiguous, such as ADF, DDW, and RIPscrip.
            bbcat::decode_with_options(
                &data,
                bbcat::DecodeOptions {
                    file_name: Some(path),
                    width: None,
                },
            )
            .map_err(|error| error.to_string())
        })
        .map(|document| {
            // SAUCE metadata is presentation data here; bbcat has already
            // decoded its fixed-width CP437 fields into Rust strings.
            let (sauce_title, sauce_details) = document.sauce.as_ref().map_or_else(
                || (None, Vec::new()),
                |sauce| {
                    let title = non_empty(&sauce.title).map(str::to_owned);
                    let mut details = Vec::new();
                    if let Some(author) = non_empty(&sauce.author) {
                        details.push(format!("by {author}"));
                    }
                    if let Some(date) = non_empty(&sauce.date) {
                        details.push(format_sauce_date(date));
                    }
                    (title, details)
                },
            );
            (document, sauce_title, sauce_details)
        });

    let (document, sauce_title, sauce_details) = match result {
        Ok(result) => result,
        Err(error) => {
            show_error(window, &error);
            return;
        }
    };
    let title = sauce_title.unwrap_or_else(|| {
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("ANSI art")
            .to_owned()
    });
    let title = if sauce_details.is_empty() {
        title
    } else {
        format!("{title} — {}", sauce_details.join(" · "))
    };
    window.set_title(Some(&title));
    let document = Rc::new(document);
    if let Err(error) = display_document(
        document.clone(),
        state.scale.get(),
        window,
        picture,
        viewer,
        scroller,
        state,
    ) {
        show_error(window, &error);
        return;
    }
    *state.document.borrow_mut() = Some(document);
}

fn display_document(
    document: Rc<bbcat::Document>,
    scale: usize,
    window: &gtk4::ApplicationWindow,
    picture: &gtk4::Picture,
    viewer: &gtk4::Overlay,
    scroller: &gtk4::ScrolledWindow,
    state: &ViewerState,
) -> Result<(), String> {
    let generation = state.playback.get().wrapping_add(1);
    state.playback.set(generation);
    let content_size = match render_document(document.clone(), scale)? {
        Rendered::Static(texture) => {
            let size = (texture.width(), texture.height());
            picture.set_paintable(Some(&texture));
            size
        }
        Rendered::Animation(document) => show_animation_frame(
            picture,
            document,
            scale,
            state.playback.clone(),
            generation,
            0,
            window.downgrade(),
        )?,
    };
    let (window_size, scrollbars) = window_fit_for_content(window, content_size.0, content_size.1);
    configure_content_view(
        picture,
        viewer,
        scroller,
        scrollbars,
        content_size.0,
        content_size.1,
        &state.native_size,
    );
    // Request the window size after removing the previous scale's larger
    // widget requests, otherwise GTK cannot shrink from ×2 back to ×1.
    window.set_default_size(window_size.0, window_size.1);
    Ok(())
}

enum Rendered {
    // Both variants describe what GTK needs after bbcat has decoded the input.
    Static(gdk::Texture),
    Animation(Rc<bbcat::Document>),
}

fn render_document(document: Rc<bbcat::Document>, scale: usize) -> Result<Rendered, String> {
    // Retain decoded screens rather than pre-rendering every texture. Large
    // animations would otherwise consume width * height * 4 bytes per frame.
    if let Some(animation) = document.animation.as_ref()
        && !animation.frames.is_empty()
    {
        return Ok(Rendered::Animation(document));
    }

    // Static documents can use bbcat's high-level PNG convenience method.
    document
        .encode_png(scale)
        .map_err(|error| error.to_string())
        .and_then(texture_from_png)
        .map(Rendered::Static)
}

fn texture_from_png(png: Vec<u8>) -> Result<gdk::Texture, String> {
    // Bytes takes ownership of the Vec, and the resulting texture keeps the
    // encoded data alive for as long as GTK needs it.
    gdk::Texture::from_bytes(&glib::Bytes::from_owned(png)).map_err(|error| error.to_string())
}

fn frame_duration(frame: &bbcat::AnimationFrame) -> Duration {
    // DDW frames carry a native duration. ANSI frames instead record how many
    // source bytes produced them, matching bbcat's default playback rate.
    frame.duration.unwrap_or_else(|| {
        let nanoseconds = (frame.source_bytes as u128).saturating_mul(1_000_000_000)
            / u128::from(bbcat::DEFAULT_ANIMATION_BAUD);
        Duration::from_nanos(nanoseconds.min(u128::from(u64::MAX)) as u64)
    })
}

fn non_empty(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()).then_some(value)
}

fn format_sauce_date(date: &str) -> String {
    if date.len() == 8 && date.bytes().all(|byte| byte.is_ascii_digit()) {
        format!("{}-{}-{}", &date[..4], &date[4..6], &date[6..])
    } else {
        date.to_owned()
    }
}

fn show_animation_frame(
    picture: &gtk4::Picture,
    document: Rc<bbcat::Document>,
    scale: usize,
    playback: Rc<Cell<u64>>,
    generation: u64,
    index: usize,
    window: glib::WeakRef<gtk4::ApplicationWindow>,
) -> Result<(i32, i32), String> {
    // A callback belonging to an older file must not replace the new artwork.
    if playback.get() != generation {
        return Ok((0, 0));
    }

    // Rendering only the current screen keeps the texture working set small.
    let frames = &document
        .animation
        .as_ref()
        .expect("animated render requires animation frames")
        .frames;
    let frame = &frames[index];
    let png = bbcat::encode_screen_scaled(&frame.screen, 0, frame.screen.height, scale)?;
    let texture = texture_from_png(png)?;
    let size = (texture.width(), texture.height());
    picture.set_paintable(Some(&texture));
    let delay = frame_duration(frame).max(Duration::from_millis(1));
    let next = (index + 1) % frames.len();
    let picture = picture.downgrade();
    // One-shot timers allow every frame to have a different duration. The weak
    // picture reference also stops the loop naturally when its window closes.
    glib::timeout_add_local_once(delay, move || {
        if let Some(picture) = picture.upgrade() {
            let next_window = window.clone();
            if let Err(error) = show_animation_frame(
                &picture,
                document,
                scale,
                playback,
                generation,
                next,
                next_window,
            ) && let Some(window) = window.upgrade()
            {
                show_error(&window, &error);
            }
        }
    });
    Ok(size)
}

fn window_fit_for_content(
    window: &gtk4::ApplicationWindow,
    content_width: i32,
    content_height: i32,
) -> ((i32, i32), (bool, bool)) {
    // GDK monitor geometry is in the same logical units used by GTK widget
    // sizes, including on scaled displays.
    let titlebar_height = window
        .titlebar()
        .map(|titlebar| titlebar.measure(gtk4::Orientation::Vertical, -1).1)
        .unwrap_or(0);
    let monitor_size = window
        .surface()
        .and_then(|surface| surface.display().monitor_at_surface(&surface))
        .map(|monitor| {
            let geometry = monitor.geometry();
            (geometry.width(), geometry.height())
        })
        .unwrap_or((1200, 800));
    let size = fitted_window_size(
        content_width,
        content_height,
        titlebar_height,
        monitor_size.0,
        monitor_size.1,
    );
    (
        size,
        required_scrollbars(
            content_width,
            content_height,
            titlebar_height,
            monitor_size.0,
            monitor_size.1,
        ),
    )
}

fn configure_content_view(
    picture: &gtk4::Picture,
    viewer: &gtk4::Overlay,
    scroller: &gtk4::ScrolledWindow,
    scrollbars: (bool, bool),
    content_width: i32,
    content_height: i32,
    native_size: &Rc<Cell<Option<(i32, i32)>>>,
) {
    let (horizontal_scrollbar, vertical_scrollbar) = scrollbars;
    if horizontal_scrollbar || vertical_scrollbar {
        // The scroller always fills the window so its bars stay on the window
        // edges. The picture, rather than the scroller, is centered on an axis
        // where the native-size artwork fits.
        scroller.set_propagate_natural_width(false);
        scroller.set_propagate_natural_height(false);
        scroller.set_halign(gtk4::Align::Fill);
        scroller.set_valign(gtk4::Align::Fill);
        scroller.set_hexpand(true);
        scroller.set_vexpand(true);
        picture.set_size_request(content_width, content_height);
        picture.set_can_shrink(false);
        picture.set_halign(gtk4::Align::Center);
        picture.set_valign(gtk4::Align::Center);
        picture.set_hexpand(false);
        picture.set_vexpand(false);
        native_size.set(Some((content_width, content_height)));
    } else {
        // Responsive mode fills the resizable viewer. Picture handles the
        // aspect-preserving scale and letterboxing without a size request.
        scroller.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Never);
        scroller.set_propagate_natural_width(false);
        scroller.set_propagate_natural_height(false);
        scroller.set_halign(gtk4::Align::Fill);
        scroller.set_valign(gtk4::Align::Fill);
        scroller.set_hexpand(true);
        scroller.set_vexpand(true);
        native_size.set(None);
        picture.set_size_request(-1, -1);
        picture.set_can_shrink(true);
        picture.set_halign(gtk4::Align::Fill);
        picture.set_valign(gtk4::Align::Fill);
        picture.set_hexpand(true);
        picture.set_vexpand(true);
    }
    update_content_layout(viewer, scroller, native_size.get());
}

fn update_content_layout(
    viewer: &gtk4::Overlay,
    scroller: &gtk4::ScrolledWindow,
    native_size: Option<(i32, i32)>,
) {
    let Some((content_width, content_height)) = native_size else {
        return;
    };
    // Choose scrollbar axes from the full viewer allocation, independently of
    // the picture's native size.
    let vertical_scrollbar = scroller.vscrollbar();
    let horizontal_scrollbar = scroller.hscrollbar();
    let scrollbar_width = vertical_scrollbar
        .measure(gtk4::Orientation::Horizontal, -1)
        .1;
    let scrollbar_height = horizontal_scrollbar
        .measure(gtk4::Orientation::Vertical, -1)
        .1;
    let scrollbars = viewport_scrollbars(
        content_width,
        content_height,
        viewer.width(),
        viewer.height(),
        scrollbar_width,
        scrollbar_height,
    );
    scroller.set_policy(
        if scrollbars.0 {
            gtk4::PolicyType::Automatic
        } else {
            gtk4::PolicyType::Never
        },
        if scrollbars.1 {
            gtk4::PolicyType::Automatic
        } else {
            gtk4::PolicyType::Never
        },
    );
}

fn viewport_scrollbars(
    content_width: i32,
    content_height: i32,
    viewport_width: i32,
    viewport_height: i32,
    scrollbar_width: i32,
    scrollbar_height: i32,
) -> (bool, bool) {
    let mut horizontal = content_width > viewport_width;
    let mut vertical = content_height > viewport_height;

    // One scrollbar reduces the other axis. Two passes reach the stable pair
    // without relying on GTK's child measurements during a scroll operation.
    for _ in 0..2 {
        horizontal |= content_width
            > viewport_width.saturating_sub(if vertical { scrollbar_width } else { 0 });
        vertical |= content_height
            > viewport_height.saturating_sub(if horizontal { scrollbar_height } else { 0 });
    }
    (horizontal, vertical)
}

fn fitted_window_size(
    content_width: i32,
    content_height: i32,
    titlebar_height: i32,
    monitor_width: i32,
    monitor_height: i32,
) -> (i32, i32) {
    const MIN_WIDTH: i32 = 320;
    const MIN_HEIGHT: i32 = 240;

    // Leave a small margin around an automatically sized window. Users can
    // still maximize it normally.
    let max_width = monitor_width.saturating_mul(9).div_euclid(10).max(1);
    let max_height = monitor_height.saturating_mul(9).div_euclid(10).max(1);
    let (needs_horizontal_scrollbar, needs_vertical_scrollbar) = required_scrollbars(
        content_width,
        content_height,
        titlebar_height,
        monitor_width,
        monitor_height,
    );

    let width = content_width
        .saturating_add(if needs_vertical_scrollbar { 16 } else { 0 })
        .max(MIN_WIDTH.min(max_width))
        .min(max_width);
    let height = content_height
        .saturating_add(titlebar_height)
        .saturating_add(if needs_horizontal_scrollbar { 16 } else { 0 })
        .max(MIN_HEIGHT.min(max_height))
        .min(max_height);

    (width, height)
}

fn required_scrollbars(
    content_width: i32,
    content_height: i32,
    titlebar_height: i32,
    monitor_width: i32,
    monitor_height: i32,
) -> (bool, bool) {
    // This mirrors the initial window cap: content beyond either usable axis
    // stays at native resolution instead of being scaled into illegibility.
    let max_width = monitor_width.saturating_mul(9).div_euclid(10).max(1);
    let max_height = monitor_height.saturating_mul(9).div_euclid(10).max(1);
    (
        content_width > max_width,
        content_height.saturating_add(titlebar_height) > max_height,
    )
}

fn show_error(window: &gtk4::ApplicationWindow, error: &str) {
    let dialog = gtk4::MessageDialog::builder()
        .transient_for(window)
        .modal(true)
        .message_type(gtk4::MessageType::Error)
        .buttons(gtk4::ButtonsType::Close)
        .text("Could not open ANSI art")
        .secondary_text(error)
        .build();
    dialog.connect_response(|dialog, _| dialog.close());
    dialog.present();
}

#[cfg(test)]
mod tests {
    use super::{fitted_window_size, format_sauce_date, required_scrollbars, viewport_scrollbars};

    #[test]
    fn fits_normal_content_exactly() {
        assert_eq!(fitted_window_size(640, 384, 46, 1920, 1080), (640, 430));
    }

    #[test]
    fn caps_oversized_content_to_the_monitor() {
        assert_eq!(fitted_window_size(640, 4000, 46, 1920, 1080), (656, 972));
        assert_eq!(fitted_window_size(4000, 384, 46, 1920, 1080), (1728, 446));
        assert_eq!(fitted_window_size(4000, 4000, 46, 1920, 1080), (1728, 972));
    }

    #[test]
    fn keeps_tiny_content_usable() {
        assert_eq!(fitted_window_size(80, 25, 46, 1920, 1080), (320, 240));
    }

    #[test]
    fn scrolls_native_content_when_either_axis_exceeds_the_monitor() {
        assert_eq!(
            required_scrollbars(640, 384, 46, 1920, 1080),
            (false, false)
        );
        assert_eq!(
            required_scrollbars(4000, 384, 46, 1920, 1080),
            (true, false)
        );
        assert_eq!(
            required_scrollbars(640, 4000, 46, 1920, 1080),
            (false, true)
        );
    }

    #[test]
    fn does_not_add_a_cross_axis_scrollbar() {
        assert_eq!(
            viewport_scrollbars(6400, 640, 1920, 1080, 16, 16),
            (true, false)
        );
        assert_eq!(
            viewport_scrollbars(640, 4000, 1920, 1080, 16, 16),
            (false, true)
        );
    }

    #[test]
    fn accounts_for_space_taken_by_the_first_scrollbar() {
        assert_eq!(
            viewport_scrollbars(6400, 1070, 1920, 1080, 16, 16),
            (true, true)
        );
        assert_eq!(
            viewport_scrollbars(1910, 4000, 1920, 1080, 16, 16),
            (true, true)
        );
    }

    #[test]
    fn formats_sauce_dates_for_display() {
        assert_eq!(format_sauce_date("20260720"), "2026-07-20");
        assert_eq!(format_sauce_date("unknown"), "unknown");
    }
}
