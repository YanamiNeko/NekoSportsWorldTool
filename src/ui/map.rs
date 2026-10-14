//! 地图画布：Web Mercator 投影 + 高德瓦片底图 + 拖拽平移 + 滚轮缩放（以指针为中心）。
//!
//! 供「路网」页 /「跑步」页 /「手绘」页复用。**所有坐标为 GCJ-02**（与高德瓦片对齐）；
//! 应用内部 BD-09 数据在绘制前需经 `bd09_to_gcj02_display` 转换。

use eframe::egui;
use egui::{Color32, Pos2, Rect, Stroke, Vec2};

use super::tiles::{lonlat_to_world, world_to_lonlat, ZOOM_MAX, ZOOM_MIN};

/// BD-09 → GCJ-02 显示转换（轨迹/打卡点/路网等内部 BD 数据上地图前调用）。
pub fn bd09_to_gcj02_display(pts: &[(f64, f64)]) -> Vec<(f64, f64)> {
    pts.iter()
        .map(|&(la, lo)| crate::track::wire::bd09_to_gcj02(la, lo))
        .collect()
}

/// 地图视图状态（世界坐标 GCJ-02）。
pub struct MapState {
    /// 参考中心（lat, lon）GCJ-02。
    pub center: (f64, f64),
    /// 瓦片缩放级别（可小数，瓦片按 round 后的级别绘制）。
    pub zoom: f32,
    /// 平移偏移（像素）。
    pub offset: Vec2,
    /// 最近是否被拖拽过（供外部判断「点击 vs 拖拽」）。
    pub dragged: bool,
}

impl Default for MapState {
    fn default() -> Self {
        MapState {
            center: (35.0, 105.0),
            zoom: 5.0,
            offset: Vec2::ZERO,
            dragged: false,
        }
    }
}

impl MapState {
    /// 视野中心的世界像素（按当前 zoom）。
    fn world(&self) -> (f64, f64) {
        lonlat_to_world(self.center.1, self.center.0, self.zoom as f64)
    }

    /// 度坐标 → 屏幕坐标。
    pub fn to_screen(&self, rect: Rect, lat: f64, lon: f64) -> Pos2 {
        let (wx, wy) = self.world();
        let (px, py) = lonlat_to_world(lon, lat, self.zoom as f64);
        let c = rect.center();
        Pos2::new(
            (c.x as f64 + (px - wx) + self.offset.x as f64) as f32,
            (c.y as f64 + (py - wy) + self.offset.y as f64) as f32,
        )
    }

    /// 屏幕坐标 → 度坐标 (lat, lon)。
    pub fn to_lonlat(&self, rect: Rect, pos: Pos2) -> (f64, f64) {
        let (wx, wy) = self.world();
        let c = rect.center();
        let px = wx + (pos.x as f64 - c.x as f64) - self.offset.x as f64;
        let py = wy + (pos.y as f64 - c.y as f64) - self.offset.y as f64;
        world_to_lonlat(px, py, self.zoom as f64)
    }

    /// 依据一组点（lat, lon，GCJ-02）自动适配视野。
    pub fn fit(&mut self, rect: Rect, pts: &[(f64, f64)]) {
        if pts.len() < 2 {
            if let Some(&(la, lo)) = pts.first() {
                self.center = (la, lo);
                self.zoom = 16.0;
            }
            return;
        }
        let (mut slat, mut slon) = (0.0f64, 0.0f64);
        for (la, lo) in pts {
            slat += *la;
            slon += *lo;
        }
        let n = pts.len() as f64;
        self.center = (slat / n, slon / n);
        // zoom 0 下的世界像素跨度
        let (mut minx, mut maxx, mut miny, mut maxy) = (
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
        );
        for (la, lo) in pts {
            let (x, y) = lonlat_to_world(*lo, *la, 0.0);
            minx = minx.min(x);
            maxx = maxx.max(x);
            miny = miny.min(y);
            maxy = maxy.max(y);
        }
        let w = (maxx - minx).max(1e-9);
        let h = (maxy - miny).max(1e-9);
        let zx = ((rect.width() as f64 * 0.92) / w).log2();
        let zy = ((rect.height() as f64 * 0.92) / h).log2();
        self.zoom = (zx.min(zy) as f32).clamp(ZOOM_MIN, ZOOM_MAX);
        self.offset = Vec2::ZERO;
    }

    /// 处理拖拽/缩放交互（需在分配好 rect 后调用）。
    /// 缩放以指针位置为中心：指针下的地理点保持不动。
    /// 支持滚轮（raw_scroll_delta）与触控双指捏合（zoom_delta）两种缩放输入。
    pub fn interact(&mut self, ui: &egui::Ui, rect: Rect) {
        self.dragged = false;
        let (drag, scroll, zoom_gesture, pointer) = ui.input(|i| {
            let hover = i
                .pointer
                .interact_pos()
                .map(|p| rect.contains(p))
                .unwrap_or(false);
            let dragging = hover && i.pointer.primary_down() && i.pointer.is_decidedly_dragging();
            (
                dragging,
                i.raw_scroll_delta.y,
                i.zoom_delta(),
                i.pointer.interact_pos(),
            )
        });
        if drag {
            let d = ui.input(|i| i.pointer.delta());
            self.offset += d;
            self.dragged = true;
        }
        // 缩放增量（级别）：滚轮每档 0.5 级；捏合按比例换算成级别 log2(ratio)
        let delta_zoom = if scroll != 0.0 {
            (scroll / 50.0) * 0.5
        } else if (zoom_gesture - 1.0).abs() > 1e-4 {
            zoom_gesture.log2()
        } else {
            0.0
        };
        if delta_zoom.abs() > 1e-4 {
            if let Some(pos) = pointer.filter(|p| rect.contains(*p)) {
                self.zoom_at(rect, pos, delta_zoom);
            } else {
                self.zoom = (self.zoom + delta_zoom).clamp(ZOOM_MIN, ZOOM_MAX);
            }
        }
    }

    /// 按级别增量缩放，并保持 rect 内 pos 处的地理点屏幕位置不动。
    pub fn zoom_at(&mut self, rect: Rect, pos: Pos2, delta: f32) {
        let zoom_new = (self.zoom + delta).clamp(ZOOM_MIN, ZOOM_MAX);
        let c = rect.center();
        let (wx, wy) = self.world();
        let z_old = self.zoom as f64;
        let z_new = zoom_new as f64;
        let sc = 2f64.powf(z_new - z_old);
        // 指针下地理点（旧级别世界像素）
        let gx = wx + (pos.x as f64 - c.x as f64) - self.offset.x as f64;
        let gy = wy + (pos.y as f64 - c.y as f64) - self.offset.y as f64;
        let (wx2, wy2) = lonlat_to_world(self.center.1, self.center.0, z_new);
        self.offset.x += ((gx - wx) - (gx * sc - wx2)) as f32;
        self.offset.y += ((gy - wy) - (gy * sc - wy2)) as f32;
        self.zoom = zoom_new;
    }

    /// 以画布中心缩放（放大/缩小按钮用，每级 +delta）。
    /// 视野中心点始终渲染在 rect.center + offset，因此只需改 zoom。
    pub fn zoom_around_center(&mut self, delta: f32) {
        self.zoom = (self.zoom + delta).clamp(ZOOM_MIN, ZOOM_MAX);
    }

    /// 填充线段：沿线段放置重叠小方块（本机 Android/Mali 上除 rect/image/text
    /// 外的形状渲染异常，rect_filled 稳定可用）。
    fn fill_segment(&self, painter: &egui::Painter, rect: Rect, a: (f64, f64), b: (f64, f64), color: Color32, width: f32) {
        let pa = self.to_screen(rect, a.0, a.1);
        let pb = self.to_screen(rect, b.0, b.1);
        let d = pb - pa;
        let len = d.length();
        if len < 0.5 {
            return;
        }
        let w = width.max(2.0);
        let step = (w * 0.5).max(1.0);
        let size = egui::Vec2::splat(w);
        let mut t = 0.0f32;
        while t <= len {
            let c = pa + d * (t / len);
            painter.rect_filled(
                egui::Rect::from_center_size(c, size),
                egui::Rounding::same(w * 0.3),
                color,
            );
            t += step;
        }
    }

    pub fn draw_polyline(
        &self,
        painter: &egui::Painter,
        rect: Rect,
        pts: &[(f64, f64)],
        color: Color32,
        width: f32,
    ) {
        if pts.len() < 2 {
            return;
        }
        for w in pts.windows(2) {
            self.fill_segment(painter, rect, w[0], w[1], color, width);
        }
    }

    pub fn draw_polygon(
        &self,
        painter: &egui::Painter,
        rect: Rect,
        pts: &[(f64, f64)],
        color: Color32,
        width: f32,
    ) {
        if pts.len() < 3 {
            return;
        }
        // 逐边填充四边形（闭合轮廓）
        let n = pts.len();
        for i in 0..n {
            let a = pts[i];
            let b = pts[(i + 1) % n];
            self.fill_segment(painter, rect, a, b, color, width);
        }
    }

    pub fn draw_point(
        &self,
        painter: &egui::Painter,
        rect: Rect,
        lat: f64,
        lon: f64,
        color: Color32,
        radius: f32,
    ) {
        let c = self.to_screen(rect, lat, lon);
        // 填充方块代替圆点（本机 Android 上描边/圆形渲染异常，填充正常）
        painter.rect_filled(
            egui::Rect::from_center_size(c, egui::Vec2::splat((radius * 2.0).max(4.0))),
            egui::Rounding::same(2.0),
            color,
        );
    }
}

/// 收集一组折线/环的所有点，用于 fit。
pub fn collect_bounds(edges: &[Vec<(f64, f64)>]) -> Vec<(f64, f64)> {
    let mut pts: Vec<(f64, f64)> = Vec::new();
    for e in edges.iter().take(400) {
        for p in e {
            pts.push(*p);
        }
    }
    pts
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 缩放补偿：rect 内 pos 处的地理点在缩放前后屏幕位置不变。
    #[test]
    fn zoom_at_keeps_pointer_geo_point_fixed() {
        let mut m = MapState::default();
        m.center = (38.90, 121.54);
        m.zoom = 15.0;
        m.offset = Vec2::new(33.0, -17.0);
        let rect = Rect::from_min_max(Pos2::new(8.0, 380.0), Pos2::new(352.0, 784.0));
        let pos = Pos2::new(220.0, 600.0);
        let geo_before = m.to_lonlat(rect, pos);
        for delta in [0.5, -0.25, 1.0, -1.0] {
            m.zoom_at(rect, pos, delta);
            let p_after = m.to_screen(rect, geo_before.0, geo_before.1);
            assert!(
                (p_after - pos).length() < 0.01,
                "delta={delta} p_after={p_after:?} pos={pos:?}"
            );
        }
        // 缩放级别确实变化且被钳制
        assert!(m.zoom > 15.0);
        assert!(m.zoom <= ZOOM_MAX);
    }
}

/// 地图图例：颜色方块 + 说明文字。
pub fn legend(ui: &mut egui::Ui, items: &[(&str, Color32)]) {
    ui.horizontal(|ui| {
        ui.label("图例：");
        for (label, color) in items {
            let (rect, _) = ui.allocate_exact_size(Vec2::new(10.0, 10.0), egui::Sense::hover());
            ui.painter()
                .rect_filled(rect, egui::Rounding::same(2.0), *color);
            ui.label(*label);
        }
    });
}
