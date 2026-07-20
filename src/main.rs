use std::{cell::Cell, path::Path, rc::Rc, time::Duration};

use gtk4::{gdk, gio, glib, prelude::*};

fn main() -> glib::ExitCode {
    let app = gtk4::Application::builder()
        .application_id("dev.bbcat.GtkViewer")
        .flags(gio::ApplicationFlags::HANDLES_OPEN)
        .build();

    app.connect_startup(|_| {
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
    let picture = gtk4::Picture::builder()
        .can_shrink(true)
        .content_fit(gtk4::ContentFit::Contain)
        .hexpand(true)
        .vexpand(true)
        .build();
    let playback = Rc::new(Cell::new(0_u64));
    let scroller = gtk4::ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vscrollbar_policy(gtk4::PolicyType::Never)
        .overlay_scrolling(false)
        .hexpand(true)
        .vexpand(true)
        .child(&picture)
        .build();
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
        let window = window.downgrade();
        let picture = picture.downgrade();
        let scroller = scroller.downgrade();
        let playback = playback.clone();
        move |_| {
            if let (Some(window), Some(picture), Some(scroller)) =
                (window.upgrade(), picture.upgrade(), scroller.upgrade())
            {
                choose_file(&window, &picture, &scroller, &playback);
            }
        }
    });

    window.present();

    if let Some(path) = path {
        load_file(path, &window, &picture, &scroller, &playback);
    }
}

fn choose_file(
    window: &gtk4::ApplicationWindow,
    picture: &gtk4::Picture,
    scroller: &gtk4::ScrolledWindow,
    playback: &Rc<Cell<u64>>,
) {
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
        let scroller = scroller.clone();
        let playback = playback.clone();
        move |chooser, response| {
            if response == gtk4::ResponseType::Accept
                && let Some(path) = chooser.file().and_then(|file| file.path())
            {
                load_file(&path, &window, &picture, &scroller, &playback);
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
    scroller: &gtk4::ScrolledWindow,
    playback: &Rc<Cell<u64>>,
) {
    let generation = playback.get().wrapping_add(1);
    playback.set(generation);

    let result = std::fs::read(path)
        .map_err(|error| error.to_string())
        .and_then(|data| {
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
    let scrolls_native_content = fit_window_to_content(window, content_size.0, content_size.1);
    configure_content_view(
        picture,
        scroller,
        scrolls_native_content,
        content_size.0,
        content_size.1,
    );
}

enum Rendered {
    Static(gdk::Texture),
    Animation(Rc<Vec<bbcat::AnimationFrame>>),
}

fn render_document(mut document: bbcat::Document) -> Result<Rendered, String> {
    if let Some(animation) = document.animation.take()
        && !animation.frames.is_empty()
    {
        return Ok(Rendered::Animation(Rc::new(animation.frames)));
    }

    document
        .encode_png(1)
        .map_err(|error| error.to_string())
        .and_then(texture_from_png)
        .map(Rendered::Static)
}

fn texture_from_png(png: Vec<u8>) -> Result<gdk::Texture, String> {
    gdk::Texture::from_bytes(&glib::Bytes::from_owned(png)).map_err(|error| error.to_string())
}

fn frame_duration(frame: &bbcat::AnimationFrame) -> Duration {
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
    if playback.get() != generation {
        return Ok((0, 0));
    }

    let frame = &frames[index];
    let png = bbcat::encode_screen(&frame.screen, 0, frame.screen.height)?;
    let texture = texture_from_png(png)?;
    let size = (texture.width(), texture.height());
    picture.set_paintable(Some(&texture));
    let delay = frame_duration(frame).max(Duration::from_millis(1));
    let next = (index + 1) % frames.len();
    let picture = picture.downgrade();
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
) -> bool {
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
    let (horizontal, vertical) = required_scrollbars(
        content_width,
        content_height,
        titlebar_height,
        monitor_size.0,
        monitor_size.1,
    );
    horizontal || vertical
}

fn configure_content_view(
    picture: &gtk4::Picture,
    scroller: &gtk4::ScrolledWindow,
    scrolls_native_content: bool,
    content_width: i32,
    content_height: i32,
) {
    if scrolls_native_content {
        scroller.set_policy(gtk4::PolicyType::Automatic, gtk4::PolicyType::Automatic);
        picture.set_size_request(content_width, content_height);
        picture.set_can_shrink(false);
        picture.set_halign(gtk4::Align::Start);
        picture.set_valign(gtk4::Align::Start);
        picture.set_hexpand(false);
        picture.set_vexpand(false);
    } else {
        scroller.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Never);
        picture.set_size_request(-1, -1);
        picture.set_can_shrink(true);
        picture.set_halign(gtk4::Align::Fill);
        picture.set_valign(gtk4::Align::Fill);
        picture.set_hexpand(true);
        picture.set_vexpand(true);
    }
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
    use super::{fitted_window_size, format_sauce_date, required_scrollbars};

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
    fn formats_sauce_dates_for_display() {
        assert_eq!(format_sauce_date("20260720"), "2026-07-20");
        assert_eq!(format_sauce_date("unknown"), "unknown");
    }
}
