//! Opt-in online street-map picker. Tiles use WGS84 Web Mercator.
use eframe::egui::{self, Color32, Pos2, Rect, Vec2};
use std::{
    collections::{HashMap, HashSet},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc, Arc,
    },
    time::{Duration, Instant},
};

const TILE_SIZE: f64 = 256.0;
const MAX_LAT: f64 = 85.0511287798066;
const MAX_ZOOM: u8 = 19;
const CACHE_SIZE: usize = 96;
const MAX_PENDING: usize = 12;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum Provider {
    Street,
    Satellite,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct Tile {
    provider: Provider,
    zoom: u8,
    x: u32,
    y: u32,
}

#[derive(Debug)]
struct View {
    center: (f64, f64),
    zoom: u8,
}

impl View {
    fn world(&self, lat: f64, lon: f64) -> (f64, f64) {
        let size = TILE_SIZE * (1u32 << self.zoom) as f64;
        let lat = lat.clamp(-MAX_LAT, MAX_LAT).to_radians();
        (
            (lon + 180.0) / 360.0 * size,
            (1.0 - lat.tan().asinh() / std::f64::consts::PI) / 2.0 * size,
        )
    }

    fn coordinate(&self, x: f64, y: f64) -> (f64, f64) {
        let size = TILE_SIZE * (1u32 << self.zoom) as f64;
        let lat = (std::f64::consts::PI * (1.0 - 2.0 * y.clamp(0.0, size) / size))
            .sinh()
            .atan()
            .to_degrees();
        (lat, (x / size * 360.0).rem_euclid(360.0) - 180.0)
    }

    fn at(&self, rect: Rect, position: Pos2) -> (f64, f64) {
        let (x, y) = self.world(self.center.0, self.center.1);
        self.coordinate(
            x + (position.x - rect.center().x) as f64,
            y + (position.y - rect.center().y) as f64,
        )
    }

    fn screen(&self, rect: Rect, coordinate: (f64, f64)) -> Pos2 {
        let (cx, cy) = self.world(self.center.0, self.center.1);
        let (x, y) = self.world(coordinate.0, coordinate.1);
        let size = TILE_SIZE * (1u32 << self.zoom) as f64;
        // Render the nearest copy when the view crosses the antimeridian.
        let dx = (x - cx + size / 2.0).rem_euclid(size) - size / 2.0;
        rect.center() + Vec2::new(dx as f32, (y - cy) as f32)
    }

    fn pan(&mut self, delta: Vec2) {
        let (x, y) = self.world(self.center.0, self.center.1);
        self.center = self.coordinate(x - delta.x as f64, y - delta.y as f64);
    }

    fn zoom_at(&mut self, rect: Rect, position: Pos2, step: i8) {
        let target = (self.zoom as i8 + step).clamp(3, MAX_ZOOM as i8) as u8;
        if target == self.zoom {
            return;
        }
        let anchor = self.at(rect, position);
        self.zoom = target;
        let (x, y) = self.world(anchor.0, anchor.1);
        self.center = self.coordinate(
            x - (position.x - rect.center().x) as f64,
            y - (position.y - rect.center().y) as f64,
        );
    }
}

type TileReply = (Tile, Option<Result<egui::ColorImage, String>>);

pub struct LocationPicker {
    provider: Provider,
    sports: Vec<super::sports_places::SportsPlace>,
    sports_rx: Option<mpsc::Receiver<Result<Vec<super::sports_places::SportsPlace>, String>>>,
    sports_status: String,
    view: View,
    requests: mpsc::SyncSender<(Tile, u64)>,
    replies: mpsc::Receiver<TileReply>,
    pending: HashSet<Tile>,
    textures: HashMap<Tile, (egui::TextureHandle, u64)>,
    failures: HashMap<Tile, Instant>,
    frame: u64,
    error: Option<String>,
    blocked: bool,
    pinch: f32,
    generation: Arc<AtomicU64>,
}

impl LocationPicker {
    pub fn new(coordinate: (f64, f64)) -> Self {
        let (requests, queue) = mpsc::sync_channel(MAX_PENDING);
        let (sender, replies) = mpsc::channel();
        let generation = Arc::new(AtomicU64::new(0));
        let worker_generation = generation.clone();
        // One worker keeps requests bounded and off the UI thread.
        std::thread::spawn(move || {
            let agent = ureq::AgentBuilder::new()
                .timeout(Duration::from_secs(10))
                .build();
            while let Ok((tile, requested_generation)) = queue.recv() {
                if requested_generation != worker_generation.load(Ordering::Relaxed) {
                    if sender.send((tile, None)).is_err() {
                        break;
                    }
                    continue;
                }
                let result = fetch_tile(&agent, tile);
                let restricted = result
                    .as_ref()
                    .err()
                    .is_some_and(|error| error.contains("429") || error.contains("403"));
                if sender.send((tile, Some(result))).is_err() {
                    break;
                }
                // Do not keep querying a provider that has denied or rate-limited access.
                if restricted {
                    break;
                }
            }
        });
        Self {
            provider: Provider::Street,
            sports: Vec::new(),
            sports_rx: None,
            sports_status: String::new(),
            view: View {
                center: (coordinate.0.clamp(-MAX_LAT, MAX_LAT), coordinate.1),
                zoom: 16,
            },
            requests,
            replies,
            pending: HashSet::new(),
            textures: HashMap::new(),
            failures: HashMap::new(),
            frame: 0,
            error: None,
            blocked: false,
            pinch: 0.0,
            generation,
        }
    }

    /// Returns an explicitly clicked point; panning never changes the selected coordinate.
    pub fn draw(&mut self, ui: &mut egui::Ui, selected: (f64, f64)) -> Option<(f64, f64)> {
        self.frame += 1;
        if let Some(receiver) = &self.sports_rx {
            let result = match receiver.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Disconnected) => Some(Err("操场查询线程已退出".into())),
                Err(mpsc::TryRecvError::Empty) => None,
            };
            if let Some(result) = result {
                self.sports_rx = None;
                match result {
                    Ok(places) => {
                        self.sports_status = format!(
                            "找到 {} 个学校或运动设施；校园位置仅用于查看，请在卫星图上点击实际操场。",
                            places.len()
                        );
                        self.sports = places;
                    }
                    Err(error) => {
                        self.sports_status =
                            format!("操场查询失败：{error}；仍可用卫星图手动选点。")
                    }
                }
            }
        }
        while let Ok((tile, reply)) = self.replies.try_recv() {
            self.pending.remove(&tile);
            let Some(reply) = reply else { continue };
            match reply {
                Ok(image) => {
                    if self.textures.len() >= CACHE_SIZE {
                        if let Some(oldest) = self
                            .textures
                            .iter()
                            .min_by_key(|(_, (_, frame))| frame)
                            .map(|(tile, _)| *tile)
                        {
                            self.textures.remove(&oldest);
                        }
                    }
                    let texture = ui.ctx().load_texture(
                        format!("map-{}-{}-{}", tile.zoom, tile.x, tile.y),
                        image,
                        egui::TextureOptions::LINEAR,
                    );
                    self.textures.insert(tile, (texture, self.frame));
                }
                Err(error) => {
                    self.blocked |= error.contains("429") || error.contains("403");
                    self.error = Some(error);
                    self.failures.insert(tile, Instant::now());
                }
            }
        }
        self.failures
            .retain(|_, time| time.elapsed() < Duration::from_secs(60));
        let previous_view = (self.view.center, self.view.zoom);
        let mut provider = self.provider;
        ui.horizontal_wrapped(|ui| {
            ui.selectable_value(&mut provider, Provider::Street, "街道（OSM）");
            ui.selectable_value(&mut provider, Provider::Satellite, "卫星（Esri）");
            if ui
                .add_enabled(
                    self.sports_rx.is_none(),
                    egui::Button::new("查找附近学校/操场（OSM）"),
                )
                .clicked()
            {
                let center = self.view.center;
                let (sender, receiver) = mpsc::channel();
                self.sports_rx = Some(receiver);
                self.sports_status = "正在查询地图中心附近 2 公里内的学校、操场和跑道…".into();
                std::thread::spawn(move || {
                    let _ = sender.send(super::sports_places::find(center));
                });
            }
        });
        if provider != self.provider {
            let mut replacement = Self::new(self.view.center);
            replacement.provider = provider;
            replacement.view.zoom = self.view.zoom;
            replacement.sports = std::mem::take(&mut self.sports);
            replacement.sports_rx = self.sports_rx.take();
            replacement.sports_status = std::mem::take(&mut self.sports_status);
            *self = replacement;
        }
        if !self.sports_status.is_empty() {
            ui.label(&self.sports_status);
        }
        if !self.sports.is_empty() {
            egui::CollapsingHeader::new("附近学校、跑道和运动场（点击查看）")
                .default_open(true)
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("sports_results")
                        .max_height(160.0)
                        .show(ui, |ui| {
                            for place in self.sports.iter().take(40) {
                                ui.horizontal_wrapped(|ui| {
                                    if ui.button("查看").clicked() {
                                        self.view.center = place.center;
                                        self.view.zoom = 18;
                                    }
                                    ui.label(format!(
                                        "{} · {} · {}",
                                        place.name, place.kind, place.sport
                                    ));
                                });
                            }
                        });
                });
        }
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(self.view.zoom < MAX_ZOOM, egui::Button::new("＋ 放大"))
                .clicked()
            {
                self.view.zoom += 1;
            }
            if ui
                .add_enabled(self.view.zoom > 3, egui::Button::new("－ 缩小"))
                .clicked()
            {
                self.view.zoom -= 1;
            }
            if ui.button("回到所选点").clicked() {
                self.view.center = (selected.0.clamp(-MAX_LAT, MAX_LAT), selected.1);
            }
            ui.label(format!("地图缩放 {}", self.view.zoom));
        });
        let height = if ui.available_width() < 600.0 {
            300.0
        } else {
            380.0
        };
        let (response, painter) = ui.allocate_painter(
            Vec2::new(ui.available_width(), height),
            egui::Sense::click_and_drag(),
        );
        let rect = response.rect;
        let painter = painter.with_clip_rect(rect);
        if response.dragged_by(egui::PointerButton::Primary) {
            self.view.pan(ui.input(|input| input.pointer.delta()));
        }
        if response.hovered() {
            let (scroll, pinch, position) = ui.ctx().input_mut(|input| {
                let scroll = input.raw_scroll_delta.y;
                input.raw_scroll_delta = Vec2::ZERO;
                input.smooth_scroll_delta = Vec2::ZERO;
                (
                    scroll,
                    input.zoom_delta(),
                    input.pointer.hover_pos().unwrap_or(rect.center()),
                )
            });
            self.pinch += pinch.log2();
            let step = if scroll != 0.0 {
                scroll.signum() as i8
            } else if self.pinch.abs() >= 0.75 {
                let step = self.pinch.signum() as i8;
                self.pinch = 0.0;
                step
            } else {
                0
            };
            self.view.zoom_at(rect, position, step);
        }
        if previous_view != (self.view.center, self.view.zoom) {
            self.generation.fetch_add(1, Ordering::Relaxed);
        }
        painter.rect_filled(rect, 0.0, Color32::from_rgb(233, 238, 235));
        let (cx, cy) = self.view.world(self.view.center.0, self.view.center.1);
        let left = cx - rect.width() as f64 / 2.0;
        let top = cy - rect.height() as f64 / 2.0;
        let count = 1i64 << self.view.zoom;
        for y in (top / TILE_SIZE).floor() as i64
            ..=((top + rect.height() as f64) / TILE_SIZE).floor() as i64
        {
            if !(0..count).contains(&y) {
                continue;
            }
            for x in (left / TILE_SIZE).floor() as i64
                ..=((left + rect.width() as f64) / TILE_SIZE).floor() as i64
            {
                let tile = Tile {
                    provider: self.provider,
                    zoom: self.view.zoom,
                    x: x.rem_euclid(count) as u32,
                    y: y as u32,
                };
                let position = rect.min
                    + Vec2::new(
                        (x as f64 * TILE_SIZE - left) as f32,
                        (y as f64 * TILE_SIZE - top) as f32,
                    );
                let area = Rect::from_min_size(position, Vec2::splat(TILE_SIZE as f32));
                if let Some((texture, frame)) = self.textures.get_mut(&tile) {
                    *frame = self.frame;
                    painter.image(
                        texture.id(),
                        area,
                        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                        Color32::WHITE,
                    );
                } else {
                    painter.rect_stroke(
                        area,
                        0.0,
                        egui::Stroke::new(1.0_f32, Color32::from_gray(210)),
                    );
                    if !self.blocked
                        && self.pending.len() < MAX_PENDING
                        && !self.pending.contains(&tile)
                        && !self.failures.contains_key(&tile)
                    {
                        match self
                            .requests
                            .try_send((tile, self.generation.load(Ordering::Relaxed)))
                        {
                            Ok(()) => {
                                self.pending.insert(tile);
                            }
                            Err(mpsc::TrySendError::Disconnected(_)) => {
                                self.blocked = true;
                            }
                            Err(mpsc::TrySendError::Full(_)) => {}
                        }
                    }
                }
            }
        }
        for place in &self.sports {
            for outline in &place.outlines {
                for segment in outline.windows(2) {
                    painter.line_segment(
                        [
                            self.view.screen(rect, segment[0]),
                            self.view.screen(rect, segment[1]),
                        ],
                        egui::Stroke::new(2.0_f32, Color32::from_rgb(255, 160, 35)),
                    );
                }
            }
            let position = self.view.screen(rect, place.center);
            if rect.contains(position) {
                painter.circle_filled(position, 4.0, Color32::from_rgb(255, 160, 35));
                painter.text(
                    position + Vec2::new(6.0, 0.0),
                    egui::Align2::LEFT_CENTER,
                    &place.kind,
                    egui::FontId::proportional(13.0),
                    Color32::from_rgb(180, 65, 0),
                );
            }
        }
        let clicked = response
            .clicked()
            .then(|| response.interact_pointer_pos())
            .flatten()
            .map(|position| self.view.at(rect, position));
        let marker = self.view.screen(rect, clicked.unwrap_or(selected));
        if clicked.unwrap_or(selected).0.abs() <= MAX_LAT && rect.contains(marker) {
            painter.circle_filled(marker, 7.0, super::theme::accent());
            painter.circle_stroke(marker, 9.0, egui::Stroke::new(2.0_f32, Color32::WHITE));
            painter.line_segment(
                [marker - Vec2::new(14.0, 0.0), marker + Vec2::new(14.0, 0.0)],
                egui::Stroke::new(1.0_f32, super::theme::accent()),
            );
            painter.line_segment(
                [marker - Vec2::new(0.0, 14.0), marker + Vec2::new(0.0, 14.0)],
                egui::Stroke::new(1.0_f32, super::theme::accent()),
            );
        }
        // Ground scale varies with latitude; show an actual 50-pixel distance.
        let meters = 50.0 * 40075016.686 * self.view.center.0.to_radians().cos()
            / (TILE_SIZE * (1u32 << self.view.zoom) as f64);
        let start = rect.left_bottom() + Vec2::new(12.0, -20.0);
        painter.rect_filled(
            Rect::from_min_size(start - Vec2::new(4.0, 18.0), Vec2::new(95.0, 30.0)),
            2.0,
            Color32::from_white_alpha(225),
        );
        painter.line_segment(
            [start, start + Vec2::new(50.0, 0.0)],
            egui::Stroke::new(2.0_f32, Color32::BLACK),
        );
        painter.text(
            start - Vec2::new(0.0, 4.0),
            egui::Align2::LEFT_BOTTOM,
            format!("{meters:.1} 米"),
            egui::FontId::proportional(12.0),
            Color32::BLACK,
        );
        ui.hyperlink_to(
            "© OpenStreetMap contributors",
            "https://www.openstreetmap.org/copyright",
        );
        if self.provider == Provider::Satellite {
            ui.hyperlink_to(
                "卫星影像：Esri, Vantor, Earthstar Geographics, GIS User Community",
                "https://services.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer",
            );
        }
        ui.label("OSM 可能缺少校园操场标注；切换卫星图，放大到椭圆跑道后点击实际位置。运动场面状边界不会被当作可跑步道路。");
        ui.label("点击选点，拖动平移，滚轮或按钮缩放；坐标为 WGS84，城市需手动核对。");
        if selected.0.abs() > MAX_LAT {
            ui.colored_label(
                super::theme::warn(),
                "所选纬度超出街道地图覆盖范围（±85.0511°），请使用手动输入。",
            );
        }
        if let Some(error) = &self.error {
            ui.colored_label(
                super::theme::warn(),
                format!("地图加载失败：{error}；可关闭后重新打开，或手动填写坐标。"),
            );
        }
        if !self.pending.is_empty() || self.sports_rx.is_some() {
            ui.ctx().request_repaint_after(Duration::from_millis(150));
        }
        clicked
    }
}

fn fetch_tile(agent: &ureq::Agent, tile: Tile) -> Result<egui::ColorImage, String> {
    use std::io::Read;
    let cache = crate::platform::data_dir().join("map_tiles");
    let cache = if tile.provider == Provider::Satellite {
        cache.join("esri")
    } else {
        cache
    };
    let path = cache.join(format!("{}-{}-{}.png", tile.zoom, tile.x, tile.y));
    let lifetime = path.with_extension("ttl");
    let ttl = std::fs::read_to_string(&lifetime)
        .ok()
        .and_then(|text| text.parse::<u64>().ok())
        .unwrap_or(604800)
        .max(604800);
    if std::fs::metadata(&path).ok().is_some_and(|metadata| {
        metadata.len() <= 1_048_576
            && metadata
                .modified()
                .ok()
                .and_then(|time| time.elapsed().ok())
                .is_some_and(|age| age < Duration::from_secs(ttl))
    }) {
        if let Ok(bytes) = std::fs::read(&path) {
            if let Ok(image) = decode_tile(&bytes) {
                return Ok(image);
            }
        }
    }
    let url = match tile.provider {
        Provider::Street => format!("https://tile.openstreetmap.org/{}/{}/{}.png", tile.zoom, tile.x, tile.y),
        Provider::Satellite => format!("https://services.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer/tile/{}/{}/{}", tile.zoom, tile.y, tile.x),
    };
    let response = agent
        .get(&url)
        .set(
            "User-Agent",
            concat!(
                "NekoSportsWorldTool/",
                env!("CARGO_PKG_VERSION"),
                " (+https://github.com/LightWhite520/NekoSportsWorldTool)"
            ),
        )
        .call()
        .map_err(|error| error.to_string())?;
    let ttl = response
        .header("Cache-Control")
        .unwrap_or("")
        .split(',')
        .find_map(|field| {
            field
                .trim()
                .strip_prefix("max-age=")
                .and_then(|age| age.parse::<u64>().ok())
        })
        .unwrap_or(604800)
        .max(604800);
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(1_048_577)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > 1_048_576 {
        return Err("地图图片过大".into());
    }
    let image = decode_tile(&bytes)?;
    // Best-effort persistent caching avoids re-downloading tiles on every reopening.
    if std::fs::create_dir_all(&cache).is_ok() {
        let temporary = cache.join(format!("{}.tmp", uuid::Uuid::new_v4()));
        if std::fs::write(&temporary, &bytes).is_ok() {
            // Windows rename cannot overwrite an existing expired cache entry.
            let _ = std::fs::remove_file(&path);
            if std::fs::rename(&temporary, &path).is_ok() {
                let _ = std::fs::write(lifetime, ttl.to_string());
            }
            let _ = std::fs::remove_file(&temporary);
        }
    }
    Ok(image)
}

fn decode_tile(bytes: &[u8]) -> Result<egui::ColorImage, String> {
    use image::ImageDecoder;
    let format = image::guess_format(bytes).map_err(|error| error.to_string())?;
    if !matches!(format, image::ImageFormat::Png | image::ImageFormat::Jpeg) {
        return Err("地图图片格式无效".into());
    }
    let decoder = image::ImageReader::with_format(std::io::Cursor::new(bytes), format)
        .into_decoder()
        .map_err(|error| error.to_string())?;
    if decoder.dimensions() != (256, 256) {
        return Err("地图图片尺寸无效".into());
    }
    let image = image::DynamicImage::from_decoder(decoder)
        .map_err(|error| error.to_string())?
        .into_rgba8();
    if image.width() != 256 || image.height() != 256 {
        return Err("地图图片尺寸无效".into());
    }
    Ok(egui::ColorImage::from_rgba_unmultiplied(
        [256, 256],
        image.as_raw(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn click_selects_but_drag_only_pans_on_phone_and_desktop() {
        for width in [360.0, 1000.0] {
            let (requests, _) = mpsc::sync_channel(MAX_PENDING);
            let (_, replies) = mpsc::channel();
            let selected = (39.9, 116.4);
            let mut picker = LocationPicker {
                provider: Provider::Street,
                sports: Vec::new(),
                sports_rx: None,
                sports_status: String::new(),
                view: View {
                    center: selected,
                    zoom: 19,
                },
                requests,
                replies,
                pending: HashSet::new(),
                textures: HashMap::new(),
                failures: HashMap::new(),
                frame: 0,
                error: None,
                blocked: true,
                pinch: 0.0,
                generation: Arc::new(AtomicU64::new(0)),
            };
            let context = egui::Context::default();
            let mut frame = |events| {
                let mut point = None;
                let output = context.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(width, 800.0))),
                        events,
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            point = picker.draw(ui, selected);
                        });
                    },
                );
                let canvas = output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Rect(shape)
                            if shape.fill == Color32::from_rgb(233, 238, 235) =>
                        {
                            Some(shape.rect)
                        }
                        _ => None,
                    })
                    .unwrap();
                assert!(canvas.min.x >= 0.0 && canvas.max.x <= width);
                (point, canvas)
            };
            let (_, canvas) = frame(vec![]);
            let position = canvas.center() + Vec2::new(30.0, 20.0);
            let button = |pos, pressed| egui::Event::PointerButton {
                pos,
                pressed,
                button: egui::PointerButton::Primary,
                modifiers: Default::default(),
            };
            frame(vec![
                egui::Event::PointerMoved(position),
                button(position, true),
            ]);
            let (point, _) = frame(vec![button(position, false)]);
            let point = point.expect("click must select");
            assert!(point.0 < selected.0 && point.1 > selected.1);
            frame(vec![button(position, true)]);
            let moved = position + Vec2::new(60.0, 40.0);
            assert!(frame(vec![egui::Event::PointerMoved(moved)]).0.is_none());
            assert!(frame(vec![button(moved, false)]).0.is_none());
            drop(frame);
            assert!(picker.view.center.0 > selected.0 && picker.view.center.1 < selected.1);
        }
    }

    #[test]
    #[ignore = "explicit public OSM tile network/cache smoke test"]
    fn public_world_tile_download_and_cache() {
        let directory =
            std::env::temp_dir().join(format!("neko-map-test-{}", uuid::Uuid::new_v4()));
        crate::platform::TEST_DATA_DIR.with(|path| *path.borrow_mut() = Some(directory.clone()));
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                crate::platform::TEST_DATA_DIR.with(|path| *path.borrow_mut() = None);
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(directory.clone());
        let tile = Tile {
            provider: Provider::Street,
            zoom: 0,
            x: 0,
            y: 0,
        };
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(15))
            .build();
        assert_eq!(fetch_tile(&agent, tile).unwrap().size, [256, 256]);
        assert!(directory.join("map_tiles/0-0-0.png").exists());
        let offline = ureq::AgentBuilder::new()
            .proxy(ureq::Proxy::new("http://127.0.0.1:1").unwrap())
            .timeout(Duration::from_secs(1))
            .build();
        assert_eq!(fetch_tile(&offline, tile).unwrap().size, [256, 256]);
    }

    #[test]
    #[ignore = "explicit public Tianhe satellite tile smoke test"]
    fn public_satellite_tile() {
        let view = View {
            center: (23.1408, 113.3196),
            zoom: 18,
        };
        let (x, y) = view.world(view.center.0, view.center.1);
        let tile = Tile {
            provider: Provider::Satellite,
            zoom: 18,
            x: (x / TILE_SIZE).floor() as u32,
            y: (y / TILE_SIZE).floor() as u32,
        };
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(15))
            .build();
        assert_eq!(fetch_tile(&agent, tile).unwrap().size, [256, 256]);
    }

    #[test]
    fn coordinates_roundtrip_at_street_zoom_and_across_antimeridian() {
        let view = View {
            center: (39.9, 179.9999),
            zoom: 19,
        };
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 400.0));
        for expected in [(39.900001, 179.999901), (39.9, -179.9999)] {
            let actual = view.at(rect, view.screen(rect, expected));
            assert!((actual.0 - expected.0).abs() < 1e-8);
            assert!((actual.1 - expected.1).abs() < 1e-8);
        }
    }
    #[test]
    fn zoom_keeps_cursor_location_and_pan_moves_in_expected_direction() {
        let mut view = View {
            center: (39.9, 116.4),
            zoom: 16,
        };
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 400.0));
        let cursor = Pos2::new(150.0, 300.0);
        let before = view.at(rect, cursor);
        view.zoom_at(rect, cursor, 1);
        let after = view.at(rect, cursor);
        assert!((before.0 - after.0).abs() < 1e-9 && (before.1 - after.1).abs() < 1e-9);
        let before = view.center;
        view.pan(Vec2::new(50.0, 50.0));
        assert!(view.center.0 > before.0 && view.center.1 < before.1);
    }
    #[test]
    fn poles_and_world_edges_stay_in_bounds() {
        let view = View {
            center: (0.0, 0.0),
            zoom: 3,
        };
        for lat in [-90.0, 90.0] {
            let (x, y) = view.world(lat, 180.0);
            let (lat, lon) = view.coordinate(x, y);
            assert!(lat.abs() <= MAX_LAT + 1e-10);
            assert!((-180.0..180.0).contains(&lon));
        }
    }
}
