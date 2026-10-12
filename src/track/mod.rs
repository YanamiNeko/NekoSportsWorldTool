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
    use super::generate_road::{
        build_road, ensure_enough_ordinary, reachable_ordinary, select_route_points,
        select_route_points_reachable, DEFAULT_ORDINARY_PICK, MIN_ORDINARY_PICK,
    };
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

    /// 选点：必经点全选 + 普通点恰好取 pick 个，seed 可复现，不足则全取。
    #[test]
    fn test_select_route_points() {
        let fixed = vec![(1.0, 1.0)];
        let ordinary = vec![(2.0, 2.0), (3.0, 3.0), (4.0, 4.0), (5.0, 5.0)];
        let sel = select_route_points(&fixed, &ordinary, 42, DEFAULT_ORDINARY_PICK);
        assert_eq!(sel.len(), 1 + DEFAULT_ORDINARY_PICK, "sel={sel:?}");
        assert!(sel.contains(&(1.0, 1.0)), "必经点必须入选");
        // 同 seed 可复现
        assert_eq!(
            sel,
            select_route_points(&fixed, &ordinary, 42, DEFAULT_ORDINARY_PICK)
        );
        // pick 可变：取 3 个普通点
        assert_eq!(
            select_route_points(&fixed, &ordinary, 42, 3).len(),
            4,
            "pick=3 应含必经 + 3 普通"
        );
        // 普通点不足：有几个取几个
        let few = select_route_points(&fixed, &[(9.0, 9.0)], 1, DEFAULT_ORDINARY_PICK);
        assert_eq!(few.len(), 2);
        // fixed 与 ordinary 重叠时去重，不重复
        let overlap =
            select_route_points(&[(1.0, 1.0)], &[(1.0, 1.0), (2.0, 2.0)], 7, DEFAULT_ORDINARY_PICK);
        assert_eq!(overlap.len(), 2);
    }

    /// 必经点保序：去掉起点后，`fixed` 子序列必须与原序一致（起点仍是辐射序首点）。
    #[test]
    fn test_select_route_points_keeps_fixed_order() {
        let fixed = vec![(1.0, 1.0), (3.0, 3.0), (5.0, 5.0)];
        let ordinary = vec![(2.0, 2.0), (4.0, 4.0), (6.0, 6.0), (7.0, 7.0)];
        let sel = select_route_points(&fixed, &ordinary, 3, 2);
        assert_eq!(sel.len(), fixed.len() + 2, "{sel:?}");
        let tail_fixed: Vec<(f64, f64)> =
            sel.iter().skip(1).filter(|q| fixed.contains(q)).copied().collect();
        let expect: Vec<(f64, f64)> =
            fixed.iter().filter(|q| **q != sel[0]).copied().collect();
        assert_eq!(tail_fixed, expect, "必经点顺序被破坏");
    }

    /// 起点旋转（辐射序首点非 fixed[0]）时：预览（`plan_road_view`）与真跑（flow）
    /// 共用 `select_route_points_reachable`，同 seed 必须选出同一旋转起点，
    /// 且去掉起点后 fixed 子序列仍保持原序。
    #[test]
    fn test_start_rotation_preview_matches_actual() {
        let net = road_grid();
        let fixed = sample_points(); // 该分布下辐射序首点为 fixed[4]，起点发生旋转
        // 两个主分量内的普通点，满足 MIN_ORDINARY_PICK
        let ordinary = vec![(38.900921, 121.541610), (38.900600, 121.541900)];
        let seed = 20_261_012u64;
        let pick = MIN_ORDINARY_PICK;
        let sel = select_route_points_reachable(&net, &fixed, &ordinary, seed, pick)
            .expect("select_route_points_reachable");
        // 用例前提：起点确实旋转（否则测试无意义）
        assert_ne!(sel[0], fixed[0], "用例未触发起点旋转: sel={sel:?}");
        // 预览与真跑同 seed 可复现
        assert_eq!(
            sel,
            select_route_points_reachable(&net, &fixed, &ordinary, seed, pick).unwrap(),
            "同 seed 预览与真跑选点不一致"
        );
        // 与底层选点一致（plan_road_view 与 flow 均调用二者）
        let reachable = reachable_ordinary(&net, &fixed, &ordinary);
        assert_eq!(
            sel,
            select_route_points(&fixed, &reachable, seed, pick),
            "预览/真跑共用路径与底层选点不一致"
        );
        // 去掉起点后 fixed 子序列保持原序（旋转不破坏相对顺序）
        let tail: Vec<(f64, f64)> =
            sel.iter().skip(1).filter(|q| fixed.contains(q)).copied().collect();
        let expect: Vec<(f64, f64)> =
            fixed.iter().filter(|q| **q != sel[0]).copied().collect();
        assert_eq!(tail, expect, "起点旋转破坏了 fixed 相对顺序");
    }

    /// pick=0/1 会被夹到 MIN_ORDINARY_PICK（普通点足够时）。
    #[test]
    fn test_select_route_points_clamps_min() {
        let fixed = vec![(1.0, 1.0)];
        let ordinary = vec![(2.0, 2.0), (3.0, 3.0), (4.0, 4.0), (5.0, 5.0)];
        for pick in [0usize, 1] {
            let sel = select_route_points(&fixed, &ordinary, 9, pick);
            assert_eq!(sel.len(), 1 + MIN_ORDINARY_PICK, "pick={pick} sel={sel:?}");
        }
    }

    /// 可用普通点不足 MIN_ORDINARY_PICK 时直接报错。
    #[test]
    fn test_ensure_enough_ordinary() {
        assert!(ensure_enough_ordinary(MIN_ORDINARY_PICK).is_ok());
        assert!(ensure_enough_ordinary(MIN_ORDINARY_PICK - 1).is_err());
        assert!(ensure_enough_ordinary(0).is_err());
    }

    /// 连通性过滤：与必经点主分量断开的普通点被剔除，连通的保留。
    #[test]
    fn test_reachable_ordinary_filters_disconnected() {
        let mut g = road_grid();
        // 约 2km 外再加一个孤立小分量。
        let a = g.add_node(route_planner::Coord::new(121.560, 38.920));
        let b = g.add_node(route_planner::Coord::new(121.5602, 38.920));
        let d = route_planner::graph::EdgeData {
            way_id: 9999,
            oneway: false,
            highway: "residential".into(),
            maxspeed_kmh: None,
            length_m: 20.0,
        };
        g.add_edge(a, b, d.clone());
        g.add_edge(b, a, d);
        g.rebuild_index();

        let fixed = vec![(38.901678, 121.540241)]; // 主网格内
        let near = (38.901678, 121.540941); // 主网格内
        let far = (38.920, 121.560); // 孤立分量上
        let keep = reachable_ordinary(&g, &fixed, &[near, far]);
        assert_eq!(keep, vec![near], "孤立分量上的普通点必须被剔除");
    }

    /// 点位数分类：isFixed 字段决定必经/普通。
    #[test]
    fn test_points_fixed_free_split() {
        let pts = vec![
            serde_json::json!({"lat": 1.0, "lon": 1.0, "isFixed": 1}),
            serde_json::json!({"lat": 2.0, "lon": 2.0, "isFixed": 0}),
            serde_json::json!({"lat": 3.0, "lon": 3.0}),
        ];
        assert_eq!(crate::api::points::points_fixed(&pts), vec![(1.0, 1.0)]);
        assert_eq!(
            crate::api::points::points_free(&pts),
            vec![(2.0, 2.0), (3.0, 3.0)]
        );
    }

    /// 硬点解析：isFixed 与策略 must_points 合并去重，普通点不受影响。
    #[test]
    fn test_resolve_hard_points_merges_must() {
        let pts = vec![
            serde_json::json!({"lat": 1.0, "lon": 1.0, "isFixed": 1}),
            serde_json::json!({"lat": 2.0, "lon": 2.0, "isFixed": 0}),
        ];
        // must_points 含与 isFixed 重合的点（应去重）及一个新点。
        let (fixed, ordinary) =
            crate::api::points::resolve_hard_points(&pts, &[(1.0, 1.0), (9.0, 9.0)]);
        assert_eq!(fixed, vec![(1.0, 1.0), (9.0, 9.0)]);
        assert_eq!(ordinary, vec![(2.0, 2.0)]);
    }

    /// 提交点集构造：命中实时点位的沿用原始 Value；未命中的硬点合成（isFixed=1、
    /// BD→GCJ 得到 glat/glon），顺序与 selected 一致。
    #[test]
    fn test_submitted_points_for_synthesizes_missing_must() {
        let pts = vec![
            serde_json::json!({"lat": 1.0, "lon": 1.0, "isFixed": 1, "pointName": "A",
                "glat": 1.006, "glon": 1.006}),
            serde_json::json!({"lat": 2.0, "lon": 2.0, "isFixed": 0, "pointName": "B"}),
        ];
        // selected：命中 A、B，外加一个不在 pts 中的硬点 (9,9)
        let selected = vec![(1.0, 1.0), (9.0, 9.0), (2.0, 2.0)];
        let out = crate::api::points::submitted_points_for(&selected, &pts);
        assert_eq!(out.len(), 3, "{out:?}");
        assert_eq!(out[0]["pointName"], "A");
        assert_eq!(out[2]["pointName"], "B");
        // 合成点：BD 原坐标、isFixed=1、glat/glon 非零（BD→GCJ）
        assert_eq!(out[1]["lat"].as_f64(), Some(9.0));
        assert_eq!(out[1]["lon"].as_f64(), Some(9.0));
        assert_eq!(out[1]["isFixed"].as_i64(), Some(1));
        assert!(out[1]["glat"].as_f64().unwrap() != 0.0);
        assert!(out[1]["glon"].as_f64().unwrap() != 0.0);
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
