use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use adw::prelude::*;
use gtk::glib;

pub fn decode_from_path(path: &Path) -> Result<String, String> {
    let image = image::open(path).map_err(|error| format!("Could not read image: {error}"))?;
    let luma = image.to_luma8();
    let mut prepared = rqrr::PreparedImage::prepare(luma);

    for grid in prepared.detect_grids() {
        if let Ok((_meta, content)) = grid.decode() {
            let content = content.trim().to_string();
            if !content.is_empty() {
                return Ok(content);
            }
        }
    }

    Err("No QR code found".to_string())
}

pub fn camera_available() -> bool {
    command_available("zbarcam")
        && std::fs::read_dir("/dev")
            .map(|entries| {
                entries
                    .flatten()
                    .any(|entry| entry.file_name().to_string_lossy().starts_with("video"))
            })
            .unwrap_or(false)
}

pub fn open_scanner<F, E>(parent: Option<&gtk::Window>, on_result: F, on_error: E)
where
    F: Fn(String) + 'static,
    E: Fn(String) + 'static,
{
    if !camera_available() {
        pick_and_decode(parent, on_result, on_error);
        return;
    }

    let dialog = adw::Dialog::builder()
        .title("Scan QR")
        .content_width(360)
        .build();

    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.set_margin_top(20);
    content.set_margin_bottom(20);
    content.set_margin_start(20);
    content.set_margin_end(20);

    let scanner_icon = gtk::Image::from_icon_name("camera-photo-symbolic");
    scanner_icon.set_pixel_size(56);
    scanner_icon.add_css_class("dim-label");
    content.append(&scanner_icon);

    let status = gtk::Label::new(Some("Scanning"));
    status.add_css_class("dim-label");
    status.set_halign(gtk::Align::Center);
    content.append(&status);

    let pick_image = gtk::Button::with_label("Image");
    pick_image.add_css_class("flat");
    content.append(&pick_image);

    dialog.set_child(Some(&content));

    let stop_flag = Arc::new(AtomicBool::new(false));
    let child = Arc::new(Mutex::new(None));
    let (event_tx, event_rx) = mpsc::channel::<CameraEvent>();

    {
        let stop = stop_flag.clone();
        let child = child.clone();
        thread::spawn(move || capture_loop(stop, child, event_tx));
    }

    let on_result = Rc::new(on_result);
    let on_error = Rc::new(on_error);

    {
        let on_result = on_result.clone();
        let on_error = on_error.clone();
        let dialog = dialog.clone();
        let stop = stop_flag.clone();
        glib::timeout_add_local(Duration::from_millis(150), move || {
            while let Ok(event) = event_rx.try_recv() {
                match event {
                    CameraEvent::Found(text) => {
                        stop.store(true, Ordering::SeqCst);
                        (on_result)(text);
                        dialog.close();
                        return glib::ControlFlow::Break;
                    }
                    CameraEvent::Error(error) => {
                        (on_error)(error);
                        return glib::ControlFlow::Break;
                    }
                }
            }
            glib::ControlFlow::Continue
        });
    }

    {
        let stop = stop_flag.clone();
        let on_result = on_result.clone();
        let on_error = on_error.clone();
        let dialog = dialog.clone();
        let parent = parent.cloned();
        pick_image.connect_clicked(move |_| {
            let stop = stop.clone();
            let on_result = on_result.clone();
            let dialog = dialog.clone();
            pick_and_decode(
                parent.as_ref(),
                move |text| {
                    stop.store(true, Ordering::SeqCst);
                    (on_result)(text);
                    dialog.close();
                },
                {
                    let on_error = on_error.clone();
                    move |error| (on_error)(error)
                },
            );
        });
    }

    {
        let stop = stop_flag.clone();
        let child = child.clone();
        dialog.connect_closed(move |_| {
            stop.store(true, Ordering::SeqCst);
            kill_child(&child);
        });
    }

    dialog.present(parent);
}

fn pick_and_decode<F, E>(parent: Option<&gtk::Window>, on_result: F, on_error: E)
where
    F: Fn(String) + 'static,
    E: Fn(String) + 'static,
{
    let dialog = gtk::FileDialog::builder()
        .title("Import QR image")
        .accept_label("Import")
        .build();
    dialog.open(parent, gtk::gio::Cancellable::NONE, move |result| {
        let Ok(file) = result else {
            return;
        };
        let Some(path) = file.path() else {
            on_error("Could not open image".to_string());
            return;
        };
        match decode_from_path(&path) {
            Ok(text) => on_result(text),
            Err(error) => on_error(error),
        }
    });
}

enum CameraEvent {
    Found(String),
    Error(String),
}

fn capture_loop(
    stop: Arc<AtomicBool>,
    child_slot: Arc<Mutex<Option<Child>>>,
    event_tx: Sender<CameraEvent>,
) {
    let mut child = match Command::new("zbarcam")
        .args(["--raw", "--quiet"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            let _ = event_tx.send(CameraEvent::Error(format!(
                "Camera scanner unavailable: {error}"
            )));
            return;
        }
    };
    let stdout = child.stdout.take();
    {
        let mut slot = child_slot.lock().expect("camera child lock");
        *slot = Some(child);
    }

    let Some(stdout) = stdout else {
        kill_child(&child_slot);
        let _ = event_tx.send(CameraEvent::Error("Camera stream unavailable".to_string()));
        return;
    };

    let mut reader = std::io::BufReader::new(stdout);
    loop {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        let mut line = String::new();
        match std::io::BufRead::read_line(&mut reader, &mut line) {
            Ok(0) => break,
            Ok(_) => {
                let text = line.trim().to_string();
                if !text.is_empty() {
                    let _ = event_tx.send(CameraEvent::Found(text));
                    break;
                }
            }
            Err(error) => {
                let _ = event_tx.send(CameraEvent::Error(format!("Camera scan failed: {error}")));
                break;
            }
        }
    }
    kill_child(&child_slot);
}

fn kill_child(child_slot: &Arc<Mutex<Option<Child>>>) {
    if let Ok(mut slot) = child_slot.lock() {
        if let Some(mut child) = slot.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn command_available(name: &str) -> bool {
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .any(|path| path.join(name).is_file())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{GrayImage, Luma};
    use qrcode::QrCode;

    #[test]
    fn decodes_qr_png() {
        let text = "nvpn://invite/test";
        let code = QrCode::new(text.as_bytes()).expect("build qr");
        let modules = code.width();
        let quiet = 4usize;
        let scale = 8usize;
        let size = (modules + quiet * 2) * scale;
        let colors = code.to_colors();
        let image = GrayImage::from_fn(size as u32, size as u32, |x, y| {
            let module_x = x as usize / scale;
            let module_y = y as usize / scale;
            if module_x < quiet
                || module_y < quiet
                || module_x >= modules + quiet
                || module_y >= modules + quiet
            {
                return Luma([255]);
            }
            let index = (module_y - quiet) * modules + (module_x - quiet);
            if matches!(colors[index], qrcode::Color::Dark) {
                Luma([0])
            } else {
                Luma([255])
            }
        });
        let path = std::env::temp_dir().join(format!("nostr-vpn-qr-{}.png", std::process::id()));
        image.save(&path).expect("write qr png");

        let decoded = decode_from_path(&path).expect("decode qr png");
        let _ = std::fs::remove_file(&path);

        assert_eq!(decoded, text);
    }
}
