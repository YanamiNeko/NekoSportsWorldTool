//! 轨迹层：生成器 / OBS 组装 / 官方卡路里。

pub mod altitude;
pub mod calorie;
pub mod generate_road;
pub mod generator;
pub mod geom;
pub mod model;
pub mod postfix;
pub mod wire;

#[cfg(test)]
mod tests {
    use super::generate_road::build_road;
    use super::generator::build;
    use super::geom::{MET_PER_DEG_LAT, MET_PER_DEG_LNG};

    use super::wire::*;

    fn sample_points() -> Vec<(f64, f64)> {
        // 某校园 5 个打卡点（BD 系）
        vec![
            (38.901678, 121.540241),
            (38.902564, 121.541233),
            (38.900921, 121.542310),
            (38.899823, 121.541010),
            (38.900455, 121.539512),
        ]
    }

    /// 在打卡点质心周围构建 7x7 网格路网（双向，约 111m 间距）。
    fn road_grid() -> route_planner::RoadGraph {
        let pts = sample_points();
        let n = pts.len() as f64;
        let (clat, clng) = (
            pts.iter().map(|p| p.0).sum::<f64>() / n,
            pts.iter().map(|p| p.1).sum::<f64>() / n,
        );
        let mut g = route_planner::RoadGraph::new();
        let step = 0.0012;
        let mut nodes = vec![];
        for j in 0..7i64 {
            for i in 0..7i64 {
                let lat = clat + (j as f64 - 3.0) * step;
                let lon = clng + (i as f64 - 3.0) * step;
                nodes.push(g.add_node(route_planner::Coord::new(lon, lat)));
            }
        }
        let mut wid = 1u64;
        let add_edge = |g: &mut route_planner::RoadGraph, a, b, wid| {
            let d = route_planner::graph::EdgeData {
                way_id: wid,
                oneway: false,
                highway: "residential".into(),
                maxspeed_kmh: None,
                length_m: 111.0,
            };
            g.add_edge(a, b, d.clone());
            g.add_edge(b, a, d);
        };
        for j in 0..7usize {
            for i in 0..6usize {
                let a = nodes[j * 7 + i];
                let b = nodes[j * 7 + i + 1];
                add_edge(&mut g, a, b, wid);
                wid += 1;
            }
        }
        for i in 0..7usize {
            for j in 0..6usize {
                let a = nodes[j * 7 + i];
                let b = nodes[(j + 1) * 7 + i];
                add_edge(&mut g, a, b, wid);
                wid += 1;
            }
        }
        g.compute_anchor();
        g.rebuild_index();
        g
    }

    /// 模式 B（真实道路路由）：距离精确、哨兵/断崖语义与模式 A 一致。
    #[test]
    fn test_build_road_distribution() {
        let pts = sample_points();
        let net = road_grid();
        let start = 1_788_958_186_123i64;
        let track = build_road(3300.0, 1220, 42, start, &pts, &pts[1..], &net).expect("build_road");
        assert!(
            (track.totalDistance - 3300.0).abs() < 0.5,
            "dist={}",
            track.totalDistance
        );
        assert_eq!(track.totalTime, 1220);
        assert!(!track.locations.is_empty());
        assert!([0, 7].contains(&track.locations[0].ptype));
        assert_eq!(track.locations[1].ptype, 5);
        assert_eq!(track.locations.last().unwrap().ptype, 6);
        // 步数为正
        assert!(track.totalSteps > 500, "steps={}", track.totalSteps);
    }

    /// 轨迹生成抽样断言：距离精确、采样间隔分布、哨兵/断崖/位移语义。
    #[test]
    fn test_generator_distribution() {
        let pts = sample_points();
        let start = 1_788_958_186_123i64;
        let track = build(3300.0, 1220, 42, (38.9, 121.54), start, &pts);
        // 总距离精确等于目标（±0.5m 舍入容差）
        assert!(
            (track.totalDistance - 3300.0).abs() < 0.5,
            "dist={}",
            track.totalDistance
        );
        assert_eq!(track.totalTime, 1220);
        // 点数合理（主 5s 采样）
        let n = track.locations.len();
        assert!((200..320).contains(&n), "n={n}");
        // 哨兵：索引0 type∈{0,7}/totalTime=0/state=1；索引1 type=5 全零；末点 type=6
        assert!([0, 7].contains(&track.locations[0].ptype));
        assert_eq!(track.locations[0].totalTime, 0);
        assert_eq!(track.locations[0].state, 1);
        assert_eq!(track.locations[1].ptype, 5);
        assert_eq!(track.locations[1].totalDis, 0.0);
        assert_eq!(track.locations[1].steps, 0);
        assert_eq!(track.locations.last().unwrap().ptype, 6);
        // 累计距离单调不减、末点 ≈ 总距离
        let mut prev = 0.0;
        for p in &track.locations {
            assert!(p.totalDis >= prev - 1e-6, "totalDis 回退");
            prev = p.totalDis;
        }
        // 距离只由正常点承担：终点哨兵(type=6)携带全程累计距离
        assert!(
            (track.locations.last().unwrap().totalDis - 3300.0).abs() < 2.0,
            "末点={}",
            track.locations.last().unwrap().totalDis
        );
        // 采样间隔：5s 占比 ≥ 60%
        let mut fives = 0;
        let mut total = 0;
        for w in track.locations.windows(2) {
            let dt = w[1].totalTime - w[0].totalTime;
            if dt > 0 {
                total += 1;
                if dt == 5 {
                    fives += 1;
                }
            }
        }
        assert!(fives as f64 / total as f64 > 0.6, "5s 占比不足");
        // 10s 窗非空、结构合法
        assert!(!track.speedPerTenSec.is_empty());
        assert_eq!(track.speedPerTenSec.len(), track.stepsPerTenSec.len());
        // 首点 lat/lng 占位 -1.0，coorType gcj02
        assert_eq!(track.locations[0].lat, -1.0);
        assert_eq!(track.locations[0].coorType, "gcj02");
        // 步数为正、步频在合理范围
        assert!(track.totalSteps > 500, "steps={}", track.totalSteps);
    }

    /// 打卡点吸附：轨迹必过点位（<40m 落位）。
    #[test]
    fn test_point_snapping() {
        let pts = sample_points();
        let track = build(2200.0, 900, 7, (38.9, 121.54), 1_788_958_186_123, &pts);
        for pl in &pts {
            let min_m = track
                .locations
                .iter()
                .map(|p| {
                    (((p.gLat - pl.0) * MET_PER_DEG_LAT).powi(2)
                        + ((p.gLng - pl.1) * MET_PER_DEG_LNG).powi(2))
                    .sqrt()
                })
                .fold(f64::INFINITY, f64::min);
            assert!(min_m < 1.0, "点位吸附失败: {min_m}m");
        }
    }

    /// 10 秒窗均值配速全部落在有效窗口内（判定规则 2'21"-10'00"/km），且总距精确。
    /// 逐点 avgSpeed 允许越界（真人爬坡期同样低于窗口，见 OBS 样本）。
    #[test]
    fn test_speeds_within_valid_pace_window() {
        let pts = sample_points();
        let combos = [
            (1050.0, 480i64),
            (1440.0, 661),
            (1920.0, 719),
            (2100.0, 900),
            (3300.0, 1220),
        ];
        for seed in 0..16u64 {
            for &(dist, dur) in &combos {
                let t = build(dist, dur, seed, (38.9, 121.54), 1_788_958_186_123, &pts);
                for (i, w) in t.speedPerTenSec.iter().enumerate() {
                    let pace = 1000.0 / (w.value / 10.0); // 秒/km
                    assert!(
                        (141.0..=600.0).contains(&pace),
                        "seed={seed} dist={dist} 窗{i} 配速 {}/km 越界",
                        format_args!("{}:{:02}", pace as i64 / 60, (pace as i64) % 60)
                    );
                }
                assert!(
                    (t.totalDistance - dist).abs() < 2.0,
                    "seed={seed} dist={}: {}",
                    dist,
                    t.totalDistance
                );
            }
        }
    }

    /// BD→GCJ 实测向量。
    #[test]
    fn test_bd09_to_gcj02_vector() {
        let (lat, lng) = bd09_to_gcj02(38.901678, 121.540241);
        assert!((lat - 38.8956025774013).abs() < 1e-9, "lat={lat}");
        assert!((lng - 121.5337497718317).abs() < 1e-9, "lng={lng}");
    }

    /// 坐标基准变换：gcj02_to_bd09 是 bd09_to_gcj02 的逆；WGS84→BD 有境内偏移。
    #[test]
    fn test_coord_datum_roundtrip() {
        use super::geom::{gcj02_to_bd09, wgs84_to_bd09};
        let bd = (38.901678, 121.540241);
        let gcj = bd09_to_gcj02(bd.0, bd.1);
        let back = gcj02_to_bd09(gcj.0, gcj.1);
        assert!((back.0 - bd.0).abs() < 1e-6, "lat={} 期望 {}", back.0, bd.0);
        assert!((back.1 - bd.1).abs() < 1e-6, "lng={} 期望 {}", back.1, bd.1);
        // WGS84 校园坐标转 BD 后应带明显偏移（约 0.004~0.006 度）
        let wgs = (38.8956, 121.5337);
        let bd2 = wgs84_to_bd09(wgs.0, wgs.1);
        assert!((bd2.0 - wgs.0).abs() > 0.003, "lat 偏移过小 {}", bd2.0);
        assert!((bd2.1 - wgs.1).abs() > 0.003, "lng 偏移过小 {}", bd2.1);
    }

    /// 围栏裁剪：保留边整体都在围栏内，不允许「端点在内、中段越出」的穿越边。
    #[test]
    fn test_apply_fences_clips_crossing_edges() {
        use super::generate_road::apply_fences;
        use route_planner::{point_in_polygon, Coord};
        let net = road_grid();
        let pts = sample_points();
        let n = pts.len() as f64;
        let (clat, clng) = (
            pts.iter().map(|p| p.0).sum::<f64>() / n,
            pts.iter().map(|p| p.1).sum::<f64>() / n,
        );
        let s = 0.0020;
        let fence: Vec<(f64, f64)> = vec![
            (clat - s, clng - s),
            (clat - s, clng + s),
            (clat + s, clng + s),
            (clat + s, clng - s),
        ];
        let polys: Vec<Coord> = fence.iter().map(|p| Coord::new(p.1, p.0)).collect();
        let g = apply_fences(&net, &[fence]);
        assert!(g.graph.edge_count() > 0, "裁剪后仍应有道路");
        for e in g.edges_deg() {
            let a = e[0];
            let b = e[1];
            for k in 0..=8 {
                let t = k as f64 / 8.0;
                let c = Coord::new(a.lon + (b.lon - a.lon) * t, a.lat + (b.lat - a.lat) * t);
                assert!(point_in_polygon(&polys, c), "边越出围栏");
            }
        }
    }

    /// OBS 对象：10 键、gzip+base64 可解、run_data 27 键点集。
    #[test]
    fn test_obs_object_structure() {
        let pts: Vec<serde_json::Value> = sample_points()
            .iter()
            .enumerate()
            .map(|(i, (la, lo))| {
                serde_json::json!({
                    "lon": lo, "lat": la, "isFixed": 0,
                    "pointName": format!("P{i}"), "glon": lo - 0.006,
                    "glat": la - 0.006,
                })
            })
            .collect();
        let track = build(
            3300.0,
            1220,
            42,
            (38.9, 121.54),
            1_788_958_186_123,
            &sample_points(),
        );
        let obj = build_obs_object(&track, 1320403809, "UUID-TEST", 13056447, &pts);
        let keys: Vec<&str> = obj
            .as_object()
            .unwrap()
            .keys()
            .map(|s| s.as_str())
            .collect();
        assert_eq!(
            keys,
            vec![
                "rrid",
                "uuid",
                "uid",
                "run_data",
                "fixed_point_json",
                "segment_json",
                "speed_json",
                "step_freq_json",
                "laps_json",
                "runFaceCheck"
            ]
        );
        // rrid gzip 可解
        let raw = crate::crypto::envelope::b64_decode(obj["rrid"].as_str().unwrap()).unwrap();
        let mut dec = flate2::read::GzDecoder::new(&raw[..]);
        use std::io::Read;
        let mut s = String::new();
        dec.read_to_string(&mut s).unwrap();
        assert_eq!(s, "1320403809");
        // run_data 解包 → 27 键点集
        let raw = crate::crypto::envelope::b64_decode(obj["run_data"].as_str().unwrap()).unwrap();
        let mut dec = flate2::read::GzDecoder::new(&raw[..]);
        let mut s = String::new();
        dec.read_to_string(&mut s).unwrap();
        let wrap: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(wrap["useZip"], false);
        let pts: Vec<serde_json::Value> =
            serde_json::from_str(wrap["allLocJson"].as_str().unwrap()).unwrap();
        assert_eq!(pts[0].as_object().unwrap().len(), 27, "点键数必须 27");
        // segment_json 是空串 gzip
        let raw =
            crate::crypto::envelope::b64_decode(obj["segment_json"].as_str().unwrap()).unwrap();
        let mut dec = flate2::read::GzDecoder::new(&raw[..]);
        let mut s = String::new();
        dec.read_to_string(&mut s).unwrap();
        assert_eq!(s, "");
        // obs keys 两个
        let ks = obs_keys(&track, 1320403809, "UUID-TEST");
        assert_eq!(ks.len(), 2);
        assert!(ks[0].contains("run_data/"));
        assert!(ks[1].starts_with("run_data/1320/1320403809.json"));
    }
}
