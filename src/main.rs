//! A minimal GTK4 front end for the `bbcat` library.
//!
//! Files follow one pipeline: `bbcat` decodes them into a `Document`, static
//! screens become one GTK texture, and animations replace that texture from a
//! GTK main-loop timer.

use std::{cell::Cell, path::Path, rc::Rc, time::Duration};

use gtk4::{gdk, gio, glib, prelude::*};

fn main() -> glib::ExitCode {
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
    let playback = Rc::new(Cell::new(0_u64));
    // In native-size scrolling mode this records the artwork size. It is used
    // below to center the non-scrolling axis within the visible viewport.
    let native_size = Rc::new(Cell::new(None));
    // Centering uses child coordinates instead of margins, keeping the scroll
    // origin stable while GTK is showing or hiding scrollbars.
    let canvas = gtk4::Fixed::builder().hexpand(true).vexpand(true).build();
    canvas.put(&picture, 0.0, 0.0);
    // Scrollbars start disabled for aspect-fit mode and are enabled only when
    // the artwork is larger than the monitor.
    let scroller = gtk4::ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vscrollbar_policy(gtk4::PolicyType::Never)
        .overlay_scrolling(false)
        .hexpand(true)
        .vexpand(true)
        .child(&canvas)
        .build();
    // Recalculate the canvas whenever resizing or scrollbar layout changes.
    for adjustment in [scroller.hadjustment(), scroller.vadjustment()] {
        let picture = picture.downgrade();
        let canvas = canvas.downgrade();
        let scroller = scroller.downgrade();
        let native_size = native_size.clone();
        adjustment.connect_page_size_notify(move |_| {
            if let (Some(picture), Some(canvas), Some(scroller)) =
                (picture.upgrade(), canvas.upgrade(), scroller.upgrade())
            {
                update_content_layout(&picture, &canvas, &scroller, native_size.get());
            }
        });
    }
    for property in ["width", "height"] {
        let picture = picture.downgrade();
        let canvas = canvas.downgrade();
        let native_size = native_size.clone();
        scroller.connect_notify_local(Some(property), move |scroller, _| {
            if let (Some(picture), Some(canvas)) = (picture.upgrade(), canvas.upgrade()) {
                update_content_layout(&picture, &canvas, scroller, native_size.get());
            }
        });
    }
    let viewer = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    viewer.add_css_class("artwork");
    viewer.append(&scroller);

    let open_button = gtk4::Button::builder()
        .icon_name("document-open-symbolic")
        .tooltip_text("Open ANSI art")
        .build();
    let header = gtk4::HeaderBar::new();
    header.pack_start(&open_button);

    let window = gtk4::ApplicationWindow::builder()
        .application(app)
        .title("bbcat GTK")
        .default_width(900)
        .default_height(700)
        .titlebar(&header)
        .child(&viewer)
        .build();

    open_button.connect_clicked({
        // Signal handlers live on their widgets. Weak references avoid keeping
        // the complete window hierarchy alive after the window is closed.
        let window = window.downgrade();
        let picture = picture.downgrade();
        let canvas = canvas.downgrade();
        let scroller = scroller.downgrade();
        let playback = playback.clone();
        let native_size = native_size.clone();
        move |_| {
            if let (Some(window), Some(picture), Some(canvas), Some(scroller)) = (
                window.upgrade(),
                picture.upgrade(),
                canvas.upgrade(),
                scroller.upgrade(),
            ) {
                choose_file(
                    &window,
                    &picture,
                    &canvas,
                    &scroller,
                    &playback,
                    &native_size,
                );
            }
        }
    });

    window.present();

    if let Some(path) = path {
        load_file(
            path,
            &window,
            &picture,
            &canvas,
            &scroller,
            &playback,
            &native_size,
        );
    }
}

fn choose_file(
    window: &gtk4::ApplicationWindow,
    picture: &gtk4::Picture,
    canvas: &gtk4::Fixed,
    scroller: &gtk4::ScrolledWindow,
    playback: &Rc<Cell<u64>>,
    native_size: &Rc<Cell<Option<(i32, i32)>>>,
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
        let canvas = canvas.clone();
        let scroller = scroller.clone();
        let playback = playback.clone();
        let native_size = native_size.clone();
        move |chooser, response| {
            if response == gtk4::ResponseType::Accept
                && let Some(path) = chooser.file().and_then(|file| file.path())
            {
                load_file(
                    &path,
                    &window,
                    &picture,
                    &canvas,
                    &scroller,
                    &playback,
                    &native_size,
                );
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
    canvas: &gtk4::Fixed,
    scroller: &gtk4::ScrolledWindow,
    playback: &Rc<Cell<u64>>,
    native_size: &Rc<Cell<Option<(i32, i32)>>>,
) {
    // Invalidating the previous generation stops its next timer callback from
    // scheduling another animation frame.
    let generation = playback.get().wrapping_add(1);
    playback.set(generation);

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
        .and_then(|document| {
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
            render_document(document).map(|rendered| (rendered, sauce_title, sauce_details))
        });

    let (rendered, sauce_title, sauce_details) = match result {
        Ok(result) => result,
        Err(error) => {
            show_error(window, &error);
            return;
        }
    };
    let content_size = match rendered {
        Rendered::Static(texture) => {
            let size = (texture.width(), texture.height());
            picture.set_paintable(Some(&texture));
            size
        }
        Rendered::Animation(frames) => {
            match show_animation_frame(
                picture,
                frames,
                playback.clone(),
                generation,
                0,
                window.downgrade(),
            ) {
                Ok(size) => size,
                Err(error) => {
                    show_error(window, &error);
                    return;
                }
            }
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
    let scrollbars = fit_window_to_content(window, content_size.0, content_size.1);
    configure_content_view(
        picture,
        canvas,
        scroller,
        scrollbars,
        content_size.0,
        content_size.1,
        native_size,
    );
}

enum Rendered {
    // Both variants describe what GTK needs after bbcat has decoded the input.
    Static(gdk::Texture),
    Animation(Rc<Vec<bbcat::AnimationFrame>>),
}

fn render_document(mut document: bbcat::Document) -> Result<Rendered, String> {
    // Retain decoded screens rather than pre-rendering every texture. Large
    // animations would otherwise consume width * height * 4 bytes per frame.
    if let Some(animation) = document.animation.take()
        && !animation.frames.is_empty()
    {
        return Ok(Rendered::Animation(Rc::new(animation.frames)));
    }

    // Static documents can use bbcat's high-level PNG convenience method.
    document
        .encode_png(1)
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
    frames: Rc<Vec<bbcat::AnimationFrame>>,
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
    let frame = &frames[index];
    let png = bbcat::encode_screen(&frame.screen, 0, frame.screen.height)?;
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
            if let Err(error) =
                show_animation_frame(&picture, frames, playback, generation, next, next_window)
                && let Some(window) = window.upgrade()
            {
                show_error(&window, &error);
            }
        }
    });
    Ok(size)
}

fn fit_window_to_content(
    window: &gtk4::ApplicationWindow,
    content_width: i32,
    content_height: i32,
) -> (bool, bool) {
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
    let (width, height) = fitted_window_size(
        content_width,
        content_height,
        titlebar_height,
        monitor_size.0,
        monitor_size.1,
    );
    window.set_default_size(width, height);
    required_scrollbars(
        content_width,
        content_height,
        titlebar_height,
        monitor_size.0,
        monitor_size.1,
    )
}

fn configure_content_view(
    picture: &gtk4::Picture,
    canvas: &gtk4::Fixed,
    scroller: &gtk4::ScrolledWindow,
    scrollbars: (bool, bool),
    content_width: i32,
    content_height: i32,
    native_size: &Rc<Cell<Option<(i32, i32)>>>,
) {
    let (horizontal_scrollbar, vertical_scrollbar) = scrollbars;
    if horizontal_scrollbar || vertical_scrollbar {
        picture.set_size_request(content_width, content_height);
        picture.set_can_shrink(false);
        native_size.set(Some((content_width, content_height)));
    } else {
        // Responsive mode disables scrolling; update_content_layout sizes the
        // picture to the viewport for aspect-preserving ContentFit letterboxing.
        scroller.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Never);
        native_size.set(None);
        picture.set_can_shrink(true);
    }
    update_content_layout(picture, canvas, scroller, native_size.get());
}

fn update_content_layout(
    picture: &gtk4::Picture,
    canvas: &gtk4::Fixed,
    scroller: &gtk4::ScrolledWindow,
    native_size: Option<(i32, i32)>,
) {
    // Choose scrollbar axes ourselves. GtkFixed's changing visible child area
    // must not make GTK invent a cross-axis scrollbar while scrolling.
    let vertical_scrollbar = scroller.vscrollbar();
    let horizontal_scrollbar = scroller.hscrollbar();

    if let Some((content_width, content_height)) = native_size {
        let scrollbar_width = vertical_scrollbar
            .measure(gtk4::Orientation::Horizontal, -1)
            .1;
        let scrollbar_height = horizontal_scrollbar
            .measure(gtk4::Orientation::Vertical, -1)
            .1;
        let scrollbars = viewport_scrollbars(
            content_width,
            content_height,
            scroller.width(),
            scroller.height(),
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
        let viewport_width = scroller
            .width()
            .saturating_sub(if scrollbars.1 { scrollbar_width } else { 0 })
            .max(1);
        let viewport_height = scroller
            .height()
            .saturating_sub(if scrollbars.0 { scrollbar_height } else { 0 })
            .max(1);
        // An overflowing axis grows beyond the viewport and scrolls. A fitting
        // axis matches the viewport and places the artwork in its center.
        canvas.set_size_request(
            content_width.max(viewport_width),
            content_height.max(viewport_height),
        );
        picture.set_size_request(content_width, content_height);
        canvas.move_(
            picture,
            f64::from(viewport_width.saturating_sub(content_width).max(0) / 2),
            f64::from(viewport_height.saturating_sub(content_height).max(0) / 2),
        );
    } else {
        // Picture performs aspect-preserving scaling within the whole viewport.
        let viewport_width = scroller.width().max(1);
        let viewport_height = scroller.height().max(1);
        canvas.set_size_request(viewport_width, viewport_height);
        picture.set_size_request(viewport_width, viewport_height);
        canvas.move_(picture, 0.0, 0.0);
    }
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
