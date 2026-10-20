use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use slint::{Image, Model, ModelRc, Rgba8Pixel, SharedPixelBuffer, VecModel};
use tex_platform::http_get;
use tex_reader::{Decoded, MAX_FILE_BYTES, Reason, Source, decode, resolve};

use super::{Banner, show_banner, window_of, with_reader};
use crate::ReaderBlock;
use crate::image_cache::{BUDGET_BYTES, Cache, State};

pub(super) struct Entry {
    frames: Vec<Image>,
    delays: Vec<i32>,
}

pub(super) struct ImageState {
    cache: Cache<Entry>,
    cancel: Arc<AtomicBool>,
    remote_once: bool,
}

impl ImageState {
    pub(super) fn new() -> ImageState {
        ImageState {
            cache: Cache::new(BUDGET_BYTES),
            cancel: Arc::new(AtomicBool::new(false)),
            remote_once: false,
        }
    }

    pub(super) fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub(super) fn allow_remote(&mut self) {
        self.remote_once = true;
    }
}

/// Forgets every image and stops the running download; used when the file is loaded again.
pub(super) fn reset_images(state: &mut ImageState) {
    state.cancel();
    state.cancel = Arc::new(AtomicBool::new(false));
    state.cache.reset();
}

fn fetch(source: &Source) -> Result<Vec<u8>, Reason> {
    match source {
        Source::Local(path) => {
            let size = fs::metadata(path).map_err(|_| Reason::Unreadable)?.len();
            if size > MAX_FILE_BYTES {
                return Err(Reason::TooLarge);
            }
            fs::read(path).map_err(|_| Reason::Unreadable)
        }
        Source::Remote(url) => http_get(url, MAX_FILE_BYTES).map_err(|_| Reason::Network),
        Source::Unsupported(reason) => Err(*reason),
    }
}

fn key_of(source: &Source) -> Option<String> {
    match source {
        Source::Local(path) => Some(path.to_string_lossy().into_owned()),
        Source::Remote(url) => Some(url.clone()),
        Source::Unsupported(_) => None,
    }
}

fn entry_of(decoded: Decoded) -> (Entry, u64) {
    let bytes = decoded
        .frames
        .iter()
        .map(|frame| frame.rgba.len() as u64)
        .sum();
    let mut delays = Vec::new();
    let mut frames = Vec::new();
    let animated = decoded.frames.len() > 1;
    for frame in decoded.frames {
        let buffer = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(
            &frame.rgba,
            decoded.width,
            decoded.height,
        );
        frames.push(Image::from_rgba8(buffer));
        if animated {
            delays.push(frame.delay_ms as i32);
        }
    }
    (Entry { frames, delays }, bytes)
}

fn set_row(
    model: &VecModel<ReaderBlock>,
    row: usize,
    state: i32,
    reason: i32,
    entry: Option<&Entry>,
) {
    let Some(mut block) = model.row_data(row) else {
        return;
    };
    block.image_state = state;
    block.image_reason = reason;
    if let Some(entry) = entry {
        block.image = entry.frames[0].clone();
        if entry.frames.len() > 1 {
            block.frames = ModelRc::new(VecModel::from(entry.frames.clone()));
            block.delays = ModelRc::new(VecModel::from(entry.delays.clone()));
        }
    }
    model.set_row_data(row, block);
}

/// Gives every image row its state: shows what is cached, marks what cannot be shown and starts
/// the download of the rest. Safe to call again; rows already settled are left alone.
pub(super) fn refresh(id: u64) {
    let Some(window) = window_of(id) else {
        return;
    };
    let models = window.get_blocks();
    let Some(model) = models.as_any().downcast_ref::<VecModel<ReaderBlock>>() else {
        return;
    };
    let remote_setting = crate::app::with_app(|app| app.settings.general.remote_images);
    let mut jobs: Vec<(String, Source)> = Vec::new();
    let mut remote_skipped = false;
    let Some((generation, cancel)) = with_reader(id, |reader| {
        let dir = reader
            .path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default();
        let allowed = remote_setting || reader.images.remote_once;
        for row in 0..model.row_count() {
            let Some(block) = model.row_data(row) else {
                continue;
            };
            let unsettled = block.image_state == 0
                || (block.image_state == 2 && block.image_reason == Reason::RemoteOff.code());
            if block.kind != 7 || !unsettled {
                continue;
            }
            let source = resolve(&block.src, &dir);
            if let Source::Unsupported(reason) = &source {
                set_row(model, row, 2, reason.code(), None);
                continue;
            }
            if matches!(source, Source::Remote(_)) && !allowed {
                remote_skipped = true;
                set_row(model, row, 2, Reason::RemoteOff.code(), None);
                continue;
            }
            let Some(key) = key_of(&source) else {
                continue;
            };
            match reader.images.cache.get(&key) {
                Some(State::Ready(entry)) => set_row(model, row, 1, 0, Some(entry)),
                Some(State::Failed(reason)) => set_row(model, row, 2, reason.code(), None),
                Some(State::Pending) => {}
                None => {
                    reader.images.cache.begin(&key);
                    jobs.push((key, source));
                }
            }
        }
        (
            reader.images.cache.generation(),
            reader.images.cancel.clone(),
        )
    }) else {
        return;
    };
    if remote_skipped && window.get_banner() == 0 {
        show_banner(&window, Banner::ImagesOff);
    }
    if !jobs.is_empty() {
        spawn(id, generation, cancel, jobs);
    }
}

fn spawn(id: u64, generation: u64, cancel: Arc<AtomicBool>, jobs: Vec<(String, Source)>) {
    std::thread::spawn(move || {
        for (key, source) in jobs {
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            let result = fetch(&source).and_then(|bytes| decode(&bytes));
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            let _ = slint::invoke_from_event_loop(move || image_done(id, generation, key, result));
        }
    });
}

fn image_done(id: u64, generation: u64, key: String, result: Result<Decoded, Reason>) {
    with_reader(id, |reader| match result {
        Ok(decoded) => {
            let (entry, bytes) = entry_of(decoded);
            reader
                .images
                .cache
                .finish_ok(generation, &key, entry, bytes);
        }
        Err(reason) => reader.images.cache.finish_err(generation, &key, reason),
    });
    refresh(id);
}
