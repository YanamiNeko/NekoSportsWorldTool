//! 地图瓦片层：高德道路瓦片（GCJ-02，免 key）+ 磁盘缓存 + egui 纹理 LRU。
//!
//! - 瓦片源：`webrd0{1-4}.is.autonavi.com/appmaptile?style=7`（高德公开道路瓦片，
//!   GCJ-02 基准，与境内地图显示一致，无需 key；仅作地图底图显示用途）。
//! - 下载线程：独立 std 线程 + 双通道（请求/完成），失败瓦片本次会话不再重试。
//! - 磁盘缓存：`<数据目录>/map_tiles/{z}/{x}/{y}.png`，重启后直接命中。
//! - 纹理：egui TextureHandle 按 LRU 保留约 256 张。

use eframe::egui;
use std::collections::{HashMap, HashSet};

pub const TILE_PX: f64 = 256.0;
pub const MAX_TILES: usize = 256;
pub const ZOOM_MIN: f32 = 3.0;
/// 高德 style=7 瓦片最高可用级别（z18 起返回 1×1 空白占位图）。
pub const ZOOM_MAX: f32 = 17.0;

/// 高德道路瓦片 URL（style=7 路网；子域 1-4 轮换）。
pub fn amap_tile_url(x: u32, y: u32, z: u32) -> String {
    let sub = (x + y) % 4 + 1;
    format!(
        "https://webrd0{sub}.is.autonavi.com/appmaptile?lang=zh_cn&size=1&scale=1&style=7&x={x}&y={y}&z={z}"
    )
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct TileKey {
    pub z: u32,
    pub x: u32,
    pub y: u32,
}

/// Web Mercator：经纬度（GCJ-02）→ 世界像素（zoom z，256 瓦片）。
pub fn lonlat_to_world(lon: f64, lat: f64, z: f64) -> (f64, f64) {
    let n = TILE_PX * 2f64.powf(z);
    let x = (lon + 180.0) / 360.0 * n;
    let lat_rad = lat.to_radians();
    let y = (1.0 - (lat_rad.tan() + 1.0 / lat_rad.cos()).ln() / std::f64::consts::PI) / 2.0 * n;
    (x, y)
}

/// Web Mercator 逆变换：世界像素 → (lat, lon)。
pub fn world_to_lonlat(x: f64, y: f64, z: f64) -> (f64, f64) {
    let n = TILE_PX * 2f64.powf(z);
    let lon = x / n * 360.0 - 180.0;
    let lat_rad = (std::f64::consts::PI * (1.0 - 2.0 * y / n)).sinh().atan();
    (lat_rad.to_degrees(), lon)
}

fn cache_path(key: TileKey) -> std::path::PathBuf {
    crate::platform::data_dir()
        .join("map_tiles")
        .join(key.z.to_string())
        .join(key.x.to_string())
        .join(format!("{}.png", key.y))
}

/// 瓦片状态：纹理 LRU + 请求队列 + 下载线程句柄（生命周期随 App）。
pub struct TileState {
    textures: HashMap<TileKey, egui::TextureHandle>,
    order: Vec<TileKey>,
    pending: HashSet<TileKey>,
    failed: HashSet<TileKey>,
    attempts: HashMap<TileKey, (u32, std::time::Instant)>,
    /// 诊断：已加载纹理数。
    pub loaded: u32,
    /// 诊断：累计失败次数。
    pub failed_total: u32,
    /// 诊断：最近一次失败原因。
    pub last_error: Option<String>,
    req_tx: std::sync::mpsc::Sender<TileKey>,
    done_rx: std::sync::mpsc::Receiver<(TileKey, Result<(), String>)>,
}

/// 失败重试间隔与上限。
const RETRY_AFTER: std::time::Duration = std::time::Duration::from_secs(30);
const MAX_ATTEMPTS: u32 = 4;

impl TileState {
    /// 创建状态并启动下载线程。
    pub fn new() -> Self {
        let (req_tx, req_rx) = std::sync::mpsc::channel::<TileKey>();
        let (done_tx, done_rx) = std::sync::mpsc::channel::<(TileKey, Result<(), String>)>();
        std::thread::Builder::new()
            .name("tile-fetch".into())
            .spawn(move || {
                while let Ok(key) = req_rx.recv() {
                    let res = fetch_tile(key);
                    if done_tx.send((key, res)).is_err() {
                        return;
                    }
                }
            })
            .ok();
        Self {
            textures: HashMap::new(),
            order: Vec::new(),
            pending: HashSet::new(),
            failed: HashSet::new(),
            attempts: HashMap::new(),
            loaded: 0,
            failed_total: 0,
            last_error: None,
            req_tx,
            done_rx,
        }
    }

    /// 帧内调用：收取下载完成的瓦片并加载为纹理。
    pub fn poll(&mut self, ctx: &egui::Context) {
        while let Ok((key, res)) = self.done_rx.try_recv() {
            self.pending.remove(&key);
            match res {
                Ok(()) => {
                    let path = cache_path(key);
                    let bytes = match std::fs::read(&path) {
                        Ok(b) => b,
                        Err(e) => {
                            self.mark_failed(key, format!("读缓存失败: {e}"));
                            continue;
                        }
                    };
                    let img = match image::load_from_memory(&bytes) {
                        Ok(img) => img.to_rgba8(),
                        Err(e) => {
                            self.mark_failed(key, format!("解码失败: {e}"));
                            continue;
                        }
                    };
                    let (w, h) = (img.width() as usize, img.height() as usize);
                    let color = egui::ColorImage::from_rgba_unmultiplied([w, h], img.as_raw());
                    let tex = ctx.load_texture(
                        format!("tile{}_{}_{}", key.z, key.x, key.y),
                        color,
                        egui::TextureOptions::LINEAR,
                    );
                    if let Some(old) = self.textures.insert(key, tex) {
                        let _ = old; // 旧纹理由 egui 延迟回收
                    }
                    self.order.push(key);
                    while self.order.len() > MAX_TILES {
                        let old_key = self.order.remove(0);
                        self.textures.remove(&old_key);
                    }
                    self.loaded += 1;
                }
                Err(e) => self.mark_failed(key, e),
            }
        }
    }

    fn mark_failed(&mut self, key: TileKey, reason: String) {
        self.failed.insert(key);
        self.failed_total += 1;
        self.last_error = Some(format!("{}/{}/{}: {}", key.z, key.x, key.y, reason));
    }

    /// 清空失败/等待状态（「重试瓦片」按钮用）。
    pub fn reset_errors(&mut self) {
        self.failed.clear();
        self.pending.clear();
        self.attempts.clear();
        self.last_error = None;
    }

    /// 请求当前视野内的缺失瓦片（防重复；失败瓦片 30s 后重试，最多 4 次）。
    pub fn request_visible(
        &mut self,
        center: (f64, f64),
        zoom: f32,
        offset: egui::Vec2,
        rect: egui::Rect,
    ) {
        let zt = zoom.round().clamp(ZOOM_MIN, ZOOM_MAX) as u32;
        let zr = zoom as f64;
        let (wx, wy) = lonlat_to_world(center.1, center.0, zr);
        let c = rect.center();
        // 屏幕左上角对应的世界像素（原始级别，与 draw/to_screen 同一约定）
        let left_w = wx - (c.x - rect.left()) as f64 - offset.x as f64;
        let top_w = wy - (c.y - rect.top()) as f64 - offset.y as f64;
        let sc = 2f64.powf(zr - zt as f64);
        let left = left_w / sc;
        let top = top_w / sc;
        let x0 = (left / TILE_PX).floor() as i64;
        let y0 = (top / TILE_PX).floor() as i64;
        let n = (rect.width() as f64 / TILE_PX).ceil() as i64 + 2;
        let m = (rect.height() as f64 / TILE_PX).ceil() as i64 + 2;
        let max_tile = (1i64 << zt) - 1;
        let now = std::time::Instant::now();
        for dy in 0..m {
            for dx in 0..n {
                let x = x0 + dx;
                let y = y0 + dy;
                if x < 0 || y < 0 || x > max_tile || y > max_tile {
                    continue;
                }
                let key = TileKey { z: zt, x: x as u32, y: y as u32 };
                if self.textures.contains_key(&key) || self.pending.contains(&key) {
                    continue;
                }
                if self.failed.contains(&key) {
                    let entry = self.attempts.entry(key).or_insert((0, now));
                    if entry.0 >= MAX_ATTEMPTS || now.duration_since(entry.1) < RETRY_AFTER {
                        continue;
                    }
                    entry.1 = now;
                    self.failed.remove(&key);
                }
                self.attempts
                    .entry(key)
                    .and_modify(|(c, _)| *c += 1)
                    .or_insert((1, now));
                self.pending.insert(key);
                if self.req_tx.send(key).is_err() {
                    self.pending.remove(&key);
                    return;
                }
            }
        }
    }

    /// 绘制可见瓦片（世界像素 → 屏幕矩形）。
    /// 与 map.rs to_screen 完全同一约定（原始缩放级别世界像素），
    /// 保证分数级缩放（捏合）时瓦片与路线覆盖层、缩放锚定补偿严格一致、不漂移。
    /// - offset 增大 → 内容向右（跟随手指）。
    /// - 当前层级瓦片缺失时，用相邻层级（±2）瓦片缩放兜底，避免缩放瞬间空白。
    /// - 所有瓦片裁剪到 rect 内，不越出地图框。
    pub fn draw(
        &self,
        painter: &egui::Painter,
        center: (f64, f64),
        zoom: f32,
        offset: egui::Vec2,
        rect: egui::Rect,
    ) {
        let zr = zoom as f64;
        let zt = zoom.round().clamp(ZOOM_MIN, ZOOM_MAX) as u32;
        let (wx, wy) = lonlat_to_world(center.1, center.0, zr);
        let c = rect.center();

        // 从远层级到近层级绘制（当前层级最后画，盖在兜底瓦片上方）
        let mut levels: Vec<u32> = self
            .textures
            .keys()
            .map(|k| k.z)
            .filter(|z| (*z as i64 - zt as i64).abs() <= 2)
            .collect();
        levels.sort_by_key(|z| (z.abs_diff(zt), *z));
        levels.dedup();
        for lv in levels.iter().rev() {
            for (key, tex) in &self.textures {
                if key.z != *lv {
                    continue;
                }
                let full = tile_screen_rect(*key, center, zoom, offset, rect);
                draw_image_clipped(painter, tex.id(), full, rect);
            }
        }
    }
}

/// 瓦片在屏幕上的矩形：与 map.rs to_screen 完全同一约定
/// （原始缩放级别世界像素：p = c + (W - w) + offset），分数级缩放时不漂移。
fn tile_screen_rect(
    key: TileKey,
    center: (f64, f64),
    zoom: f32,
    offset: egui::Vec2,
    rect: egui::Rect,
) -> egui::Rect {
    let zr = zoom as f64;
    let (wx, wy) = lonlat_to_world(center.1, center.0, zr);
    let c = rect.center();
    let lv_sc = 2f64.powf(zr - key.z as f64);
    let xw = key.x as f64 * TILE_PX * lv_sc;
    let yw = key.y as f64 * TILE_PX * lv_sc;
    let sx = c.x as f64 + (xw - wx) + offset.x as f64;
    let sy = c.y as f64 + (yw - wy) + offset.y as f64;
    egui::Rect::from_min_size(
        egui::Pos2::new(sx as f32, sy as f32),
        egui::Vec2::splat((TILE_PX * lv_sc) as f32),
    )
}

/// 绘制图片并裁剪到 clip 内（手动求交并换算 UV，避免本设备上 clip 区域异常）。
fn draw_image_clipped(
    painter: &egui::Painter,
    tex: egui::TextureId,
    full: egui::Rect,
    clip: egui::Rect,
) {
    let inter = full.intersect(clip);
    if !inter.is_positive() {
        return;
    }
    let uv0 = egui::Pos2::new(
        (inter.min.x - full.min.x) / full.width(),
        (inter.min.y - full.min.y) / full.height(),
    );
    let uv1 = egui::Pos2::new(
        (inter.max.x - full.min.x) / full.width(),
        (inter.max.y - full.min.y) / full.height(),
    );
    painter.image(
        tex,
        inter,
        egui::Rect::from_min_max(uv0, uv1),
        egui::Color32::WHITE,
    );
}

/// 下载瓦片并写磁盘缓存；已缓存直接命中。返回详细错误原因供诊断。
fn fetch_tile(key: TileKey) -> Result<(), String> {
    let path = cache_path(key);
    if path.is_file() {
        return Ok(());
    }
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(12))
        .build();
    let url = amap_tile_url(key.x, key.y, key.z);
    let resp = match agent
        .get(&url)
        .set("User-Agent", "Mozilla/5.0 NekoSportsWorldTool/0.3")
        .call()
    {
        Ok(r) => r,
        Err(e) => return Err(format!("请求失败: {}", crate::api::client::ureq_err(e))),
    };
    let mut r = resp.into_reader();
    let mut bytes = Vec::new();
    if std::io::Read::read_to_end(&mut r, &mut bytes).is_err() || bytes.len() < 60 {
        return Err(format!("响应过短（{} 字节）", bytes.len()));
    }
    // 校验是合法 PNG（高德越界瓦片返回小图或非图数据；1×1 空白占位图合法）
    if bytes.len() < 4 || &bytes[0..4] != b"\x89PNG" {
        return Err("响应不是 PNG".into());
    }
    let dir = path.parent().unwrap().to_path_buf();
    if std::fs::create_dir_all(&dir).is_err() {
        return Err("创建缓存目录失败".into());
    }
    std::fs::write(&path, &bytes).map_err(|e| format!("写缓存失败: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mercator_roundtrip() {
        let (x, y) = lonlat_to_world(121.54, 38.90, 15.0);
        let (lat, lon) = world_to_lonlat(x, y, 15.0);
        assert!((lat - 38.90).abs() < 1e-9, "lat={lat}");
        assert!((lon - 121.54).abs() < 1e-9, "lon={lon}");
    }

    #[test]
    fn tile_url_has_valid_domain() {
        let url = amap_tile_url(10, 20, 12);
        assert!(url.contains("webrd0"));
        assert!(url.contains("x=10"));
        assert!(url.contains("y=20"));
        assert!(url.contains("z=12"));
    }

    /// 分数级缩放时，视野中心（c + offset）必须落在中心所在瓦片的矩形内——
    /// 保证瓦片与 map.to_screen/缩放锚定补偿严格一致、不漂移。
    #[test]
    fn tile_rect_anchors_center_at_fractional_zoom() {
        let center = (38.90, 121.54);
        let zoom = 16.4f32;
        let offset = egui::Vec2::new(37.5, -22.25);
        let rect = egui::Rect::from_min_max(
            egui::Pos2::new(8.0, 380.0),
            egui::Pos2::new(352.0, 784.0),
        );
        let zt = zoom.round() as u32;
        let (wx, wy) = lonlat_to_world(center.1, center.0, zt as f64);
        let key = TileKey {
            z: zt,
            x: (wx / TILE_PX).floor() as u32,
            y: (wy / TILE_PX).floor() as u32,
        };
        let r = tile_screen_rect(key, center, zoom, offset, rect);
        let anchor = rect.center() + offset;
        assert!(
            r.contains(anchor),
            "anchor={anchor:?} not in tile rect {r:?}"
        );
    }
}
