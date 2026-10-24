//! 地图画布绘制原语：等距投影 + 拖拽平移 + 滚轮缩放。
//!
//! 供「路网」页与「跑步」页复用，纯离线绘制折线/多边形/点。

use eframe::egui;
use egui::{Color32, Pos2, Rect, Stroke, Vec2};

use crate::track::geom::{MET_PER_DEG_LAT, MET_PER_DEG_LNG};

/// 地图视图状态（世界坐标为度）。
pub struct MapState {
    /// 参考中心（lat, lon）。
    pub center: (f64, f64),
    /// 屏幕每米像素（缩放）。
    pub scale: f32,
    /// 平移偏移（像素）。
    pub offset: Vec2,
    /// 最近是否被拖拽过（供外部判断）。
    pub dragged: bool,
}

impl Default for MapState {
    fn default() -> Self {
        MapState {
            center: (0.0, 0.0),
            scale: 0.3,
            offset: Vec2::ZERO,
            dragged: false,
        }
    }
}

impl MapState {
    pub fn world_bounds(&self, rect: Rect) -> (route_planner::Coord, route_planner::Coord) {
        let x = |screen: f32| {
            self.center.1
                + (screen - rect.center().x - self.offset.x) as f64
                    / (self.scale as f64 * MET_PER_DEG_LNG)
        };
        let y = |screen: f32| {
            self.center.0
                - (screen - rect.center().y - self.offset.y) as f64
                    / (self.scale as f64 * MET_PER_DEG_LAT)
        };
        (
            route_planner::Coord::new(x(rect.left()), y(rect.bottom())),
            route_planner::Coord::new(x(rect.right()), y(rect.top())),
        )
    }
    /// 依据一组点（lat, lon）自动适配视野。
    pub fn fit(&mut self, rect: Rect, pts: &[(f64, f64)]) {
        if pts.len() < 2 {
            return;
        }
        let (mut slat, mut slon) = (0.0f64, 0.0f64);
        for (la, lo) in pts {
            slat += *la;
            slon += *lo;
        }
        let n = pts.len() as f64;
        self.center = (slat / n, slon / n);
        let (mut minx, mut maxx, mut miny, mut maxy) = (
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
        );
        for (la, lo) in pts {
            let x = (lo - self.center.1) * MET_PER_DEG_LNG;
            let y = (la - self.center.0) * MET_PER_DEG_LAT;
            minx = minx.min(x);
            maxx = maxx.max(x);
            miny = miny.min(y);
            maxy = maxy.max(y);
        }
        let w = (maxx - minx).max(40.0);
        let h = (maxy - miny).max(40.0);
        let sx = rect.width() as f64 / w;
        let sy = rect.height() as f64 / h;
        self.scale = ((sx.min(sy)) * 0.92) as f32;
        self.offset = Vec2::ZERO;
    }

    /// 度坐标 → 屏幕坐标。
    pub fn to_screen(&self, rect: Rect, lat: f64, lon: f64) -> Pos2 {
        let x = (lon - self.center.1) * MET_PER_DEG_LNG * self.scale as f64;
        let y = -(lat - self.center.0) * MET_PER_DEG_LAT * self.scale as f64;
        let c = rect.center();
        Pos2::new(
            (c.x as f64 + x + self.offset.x as f64) as f32,
            (c.y as f64 + y + self.offset.y as f64) as f32,
        )
    }

    /// 处理拖拽/缩放交互（需在分配好 rect 后调用）。
    pub fn interact(&mut self, ui: &egui::Ui, rect: Rect) {
        self.dragged = false;
        let (drag, scroll) = ui.input(|i| {
            let hover = i
                .pointer
                .interact_pos()
                .map(|p| rect.contains(p))
                .unwrap_or(false);
            let dragging = hover && i.pointer.primary_down() && i.pointer.is_decidedly_dragging();
            (dragging, if hover { i.raw_scroll_delta.y } else { 0.0 })
        });
        if drag {
            let d = ui.input(|i| i.pointer.delta());
            self.offset += d;
            self.dragged = true;
        }
        if scroll != 0.0 {
            let factor = (scroll * 0.0015).exp() as f32;
            self.scale = (self.scale * factor).clamp(0.002, 200.0);
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
        let (a, b) = self.world_bounds(rect);
        let (mut minlat, mut maxlat, mut minlon, mut maxlon) = (
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
        );
        for &(lat, lon) in pts {
            minlat = minlat.min(lat);
            maxlat = maxlat.max(lat);
            minlon = minlon.min(lon);
            maxlon = maxlon.max(lon);
        }
        if maxlon < a.lon || minlon > b.lon || maxlat < a.lat || minlat > b.lat {
            return;
        }
        let ps: Vec<Pos2> = pts
            .iter()
            .map(|(la, lo)| self.to_screen(rect, *la, *lo))
            .collect();
        painter.add(egui::Shape::line(ps, Stroke::new(width, color)));
    }

    /// Display only exposed boundary edges, not triangulation diagonals.
    pub fn draw_boundary(&self, painter:&egui::Painter, rect:Rect, rings:&[Vec<(f64,f64)>], color:Color32, width:f32) {
        let painter=painter.with_clip_rect(rect);
        for (a,b) in boundary_edges(rings) {
            painter.line_segment([self.to_screen(rect,a.0,a.1),self.to_screen(rect,b.0,b.1)],Stroke::new(width,color));
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
        let mut ps: Vec<Pos2> = pts
            .iter()
            .map(|(la, lo)| self.to_screen(rect, *la, *lo))
            .collect();
        ps.push(ps[0]);
        painter.add(egui::Shape::closed_line(ps, Stroke::new(width, color)));
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
        painter.circle_filled(c, radius, color);
    }

    /// Draw the unmodified WGS84 OSM graph, without school API data or route planning.
    pub fn draw_network(
        &self,
        painter: &egui::Painter,
        rect: Rect,
        net: &route_planner::RoadGraph,
    ) {
        let road_color = Color32::from_rgb(190, 200, 196);
        let (min, max) = self.world_bounds(rect);
        let visible = net.visible_edges(min, max);
        let stride = if self.scale < 0.1 {
            (visible.len() / 4000).max(1)
        } else {
            1
        };
        for edge in visible.iter().step_by(stride) {
            self.draw_polyline(
                painter,
                rect,
                &[(edge[0].lat, edge[0].lon), (edge[1].lat, edge[1].lon)],
                road_color,
                1.0,
            );
        }
        if stride > 1 {
            painter.text(
                rect.left_top() + egui::vec2(6., 6.),
                egui::Align2::LEFT_TOP,
                "缩小视野：道路显示已简化",
                egui::FontId::proportional(12.),
                road_color,
            );
        }
        for building in &net.buildings {
            let points: Vec<_> = building.iter().map(|c| (c.lat, c.lon)).collect();
            self.draw_polygon(
                painter,
                rect,
                &points,
                Color32::from_rgb(214, 182, 158),
                1.0,
            );
        }
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

/// 地图图例：颜色方块 + 说明文字。
pub fn legend(ui: &mut egui::Ui, items: &[(&str, Color32)]) {
    ui.horizontal_wrapped(|ui| {
        ui.label("图例：");
        for (label, color) in items {
            let (rect, _) = ui.allocate_exact_size(Vec2::new(10.0, 10.0), egui::Sense::hover());
            ui.painter()
                .rect_filled(rect, egui::Rounding::same(2.0), *color);
            ui.label(*label);
        }
    });
}

fn boundary_edges(rings:&[Vec<(f64,f64)>])->Vec<((f64,f64),(f64,f64))> {
    use std::collections::BTreeMap;
    let mut edges=BTreeMap::new();
    let key=|p:(f64,f64)| (if p.0==0. {0} else {p.0.to_bits()},if p.1==0. {0} else {p.1.to_bits()});
    for ring in rings {
        if ring.len()<3 { continue; }
        for (&a,&b) in ring.iter().zip(ring.iter().cycle().skip(1)).take(ring.len()) {
            if a==b {continue;}
            let (ka,kb)=(key(a),key(b));
            let (k,sign)=if ka<kb {((ka,kb),1)} else {((kb,ka),-1)};
            let value=edges.entry(k).or_insert((a,b,0i32));value.2+=sign;
        }
    }
    edges.into_values().filter(|e|e.2!=0).map(|(a,b,_)|(a,b)).collect()
}

#[cfg(test)]
mod boundary_display_tests {
    use super::*;
    #[test]
    fn triangulated_real_campus_draws_only_its_perimeter() {
        let items=route_planner::campus::parse(include_bytes!("../../crates/route-planner/tests/fixtures/dinghu.osm")).unwrap();
        let c=&items[0];let triangles:Vec<Vec<_>>=c.triangles().unwrap().iter().map(|r|r.iter().map(|p|(p.lat,p.lon)).collect()).collect();
        let edges=boundary_edges(&triangles);
        assert_eq!(edges.len(),c.outer[0].len()-1);
        assert!(edges.iter().all(|&(a,b)|c.outer[0].windows(2).any(|w| {let x=(w[0].lat,w[0].lon);let y=(w[1].lat,w[1].lon);(a==x && b==y)||(a==y && b==x)})));
    }
    #[test]
    fn boundary_display_retains_holes_and_duplicate_outlines() {
        let c=route_planner::campus::Campus{id:"test".into(),name:"test".into(),outer:vec![vec![(0.,0.),(4.,0.),(4.,4.),(0.,4.),(0.,0.)].into_iter().map(|(x,y)|route_planner::Coord::new(x,y)).collect()],inner:vec![vec![(1.,1.),(2.,1.),(2.,2.),(1.,2.),(1.,1.)].into_iter().map(|(x,y)|route_planner::Coord::new(x,y)).collect()]};
        let triangles:Vec<Vec<_>>=c.triangles().unwrap().iter().map(|r|r.iter().map(|p|(p.lat,p.lon)).collect()).collect();assert_eq!(boundary_edges(&triangles).len(),8);
        let ring=vec![(0.,0.),(1.,0.),(1.,1.),(0.,1.)];assert_eq!(boundary_edges(&[ring.clone(),ring]).len(),4);
    }
}
