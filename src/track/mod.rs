//! 轨迹层：生成器 / OBS 组装 / 官方卡路里。

pub mod altitude;
pub mod calorie;
pub mod generator;
pub mod geom;
pub mod model;
pub mod postfix;
pub mod wire;

#[cfg(test)]
mod tests {
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

    /// 轨迹生成抽样断言：距离精确、采样间隔分布、哨兵/断崖/位移语义。
    #[test]
    fn test_generator_distribution() {
        let pts = sample_points();
        let start = 1_788_958_186_123i64;
        let track = build(3300.0, 1220, 42, (38.9, 121.54), start, &pts);
        assert!(
            track.locations.iter().enumerate().all(|(i, point)| {
                let expected_type = if i == 1 {
                    5
                } else if i == track.locations.len() - 1 {
                    6
                } else {
                    0
                };
                point.ptype == expected_type && point.state == 1 && point.locType == 1
            }),
            "campus route points must use ordinary fields, except for endpoint sentinels"
        );
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
        // 哨兵：索引0 type=0/totalTime=0/state=1；索引1 type=5 全零；末点 type=6
        assert_eq!(track.locations[0].ptype, 0);
        assert_eq!(track.locations[0].totalTime, 0);
        assert_eq!(track.locations[0].state, 1);
        assert_eq!(track.locations[1].ptype, 5);
        assert_eq!(track.locations[1].totalDis, 0.0);
        assert_eq!(track.locations[1].totalTime, 0);
        assert_eq!(track.locations[1].steps, 0);
        assert_eq!(track.locations[0].totalDis, 0.0);
        assert_eq!(track.locations[0].steps, 0);
        assert!(track.locations[2].totalTime > 0);
        assert!(track.locations[2].totalDis > 0.0);
        assert!(track.locations[2].steps > 0);
        assert_eq!(track.locations.last().unwrap().ptype, 6);
        assert_eq!(track.locations.last().unwrap().totalTime, track.totalTime);
        assert_eq!(track.locations.last().unwrap().steps, track.totalSteps);
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
        assert!(track.validate_consistency().is_ok());
    }

    /// Reproduce the reported 1 km / 390 s route size and guard its point flags.
    #[test]
    fn test_one_kilometer_route_uses_stable_point_flags() {
        let track = build(
            1000.0,
            390,
            42,
            (38.9, 121.54),
            1_788_958_186_123,
            &sample_points(),
        );
        assert!((track.totalDistance - 1000.0).abs() < 0.5);
        assert!((70usize..=100usize).contains(&track.locations.len()));
        assert!(track.locations.iter().enumerate().all(|(i, point)| {
            let expected_type = if i == 1 {
                5
            } else if i == track.locations.len() - 1 {
                6
            } else {
                0
            };
            point.ptype == expected_type && point.state == 1 && point.locType == 1
        }));
        assert_eq!(track.locations.last().unwrap().totalTime, track.totalTime);
        assert_eq!(track.locations.last().unwrap().totalTime, 390);
        assert!(track.validate_consistency().is_ok());
    }

    #[test]
    fn test_374_second_windows_have_four_second_tail_and_conserve_data() {
        let track = build(
            1000.0,
            374,
            42,
            (38.9, 121.54),
            1_788_958_186_123,
            &sample_points(),
        );
        assert_eq!(track.speedPerTenSec.len(), 38);
        assert_eq!(track.stepsPerTenSec.len(), 38);
        assert_eq!(track.speedPerTenSec.last().unwrap().time, 4);
        assert_eq!(track.stepsPerTenSec.last().unwrap().time, 4);
        assert!(track.speedPerTenSec[..37]
            .iter()
            .all(|window| window.time == 10));
        assert!(track.stepsPerTenSec[..37]
            .iter()
            .all(|window| window.time == 10));
        assert!(
            (track
                .speedPerTenSec
                .iter()
                .map(|window| window.value)
                .sum::<f64>()
                - track.totalDistance)
                .abs()
                < 0.01
        );
        assert_eq!(
            track
                .stepsPerTenSec
                .iter()
                .map(|window| window.value as i64)
                .sum::<i64>(),
            track.totalSteps
        );
    }

    #[test]
    fn post_processing_preserves_monotone_cumulative_fields() {
        let track = build(
            1000.0,
            374,
            42,
            (38.9, 121.54),
            1_788_958_186_123,
            &sample_points(),
        );
        let mut locations = track.locations.clone();
        super::postfix::apply_post_fixes(
            &mut locations,
            &mut super::geom::Rng::new(7),
            track.startTime,
        );
        for pair in locations.windows(2) {
            assert!(pair[1].totalTime >= pair[0].totalTime);
            assert!(pair[1].totalDis + 1e-6 >= pair[0].totalDis);
            assert!(pair[1].validTime >= pair[0].validTime);
            assert!(pair[1].validDis + 1e-6 >= pair[0].validDis);
            assert!(pair[1].steps >= pair[0].steps);
        }
        assert_eq!(locations.last().unwrap().totalTime, track.totalTime);
        assert_eq!(locations.last().unwrap().steps, track.totalSteps);
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

    #[test]
    fn test_unsorted_checkpoints_form_a_track_perimeter_and_route_hits_them() {
        // Same rectangle in deliberately crossed service response order.
        let points = [
            (38.9009, 121.5410), // northeast
            (38.8991, 121.5390), // southwest
            (38.9009, 121.5390), // northwest
            (38.8991, 121.5410), // southeast
        ];
        let (_, arcs, _) = super::geom::make_point_ring(&points);
        let perimeter = *arcs.last().unwrap();
        let expected = 2.0 * (0.0018 * MET_PER_DEG_LAT + 0.002 * MET_PER_DEG_LNG);
        assert!((perimeter - expected).abs() < 0.1, "perimeter={perimeter}");

        let track = build(2200.0, 900, 7, (38.9, 121.54), 1_788_958_186_123, &points);
        for point in &points {
            let min_m = track
                .locations
                .iter()
                .map(|location| {
                    (((location.gLat - point.0) * MET_PER_DEG_LAT).powi(2)
                        + ((location.gLng - point.1) * MET_PER_DEG_LNG).powi(2))
                    .sqrt()
                })
                .fold(f64::INFINITY, f64::min);
            assert!(min_m < 1.0, "route missed checkpoint {point:?}: {min_m}m");
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
                    let pace = 1000.0 / (w.value / w.time as f64); // 按实际窗口秒数计算秒/km
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

    /// OBS 对象：10 键、gzip+base64 可解、run_data 28 键点集。
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
        // run_data 解包 → 28 键点集
        let raw = crate::crypto::envelope::b64_decode(obj["run_data"].as_str().unwrap()).unwrap();
        let mut dec = flate2::read::GzDecoder::new(&raw[..]);
        let mut s = String::new();
        dec.read_to_string(&mut s).unwrap();
        let wrap: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(wrap["useZip"], false);
        let pts: Vec<serde_json::Value> =
            serde_json::from_str(wrap["allLocJson"].as_str().unwrap()).unwrap();
        assert!(pts.len() > 20, "run_data 必须包含完整路线点");
        assert_eq!(pts.len(), track.locations.len(), "OBS 不应丢弃生成的路线点");
        assert!(
            pts.iter()
                .all(|point| point["gLat"].as_f64().is_some() && point["gLng"].as_f64().is_some()),
            "路线点必须包含 GCJ 坐标"
        );
        for (wire_point, generated_point) in pts.iter().zip(&track.locations) {
            let rounded = |value: f64, digits: usize| super::geom::round_to(value, digits);
            assert_eq!(
                wire_point["avgSpeed"].as_f64(),
                Some(rounded(generated_point.avgSpeed, 4))
            );
            assert_eq!(
                wire_point["bdA"].as_f64(),
                Some(rounded(generated_point.bdA, 2))
            );
            assert_eq!(
                wire_point["bdD"].as_f64(),
                Some(rounded(generated_point.bdD, 2))
            );
            assert_eq!(wire_point["bdG"].as_i64(), Some(generated_point.bdG));
            assert_eq!(
                wire_point["bdS"].as_f64(),
                Some(rounded(generated_point.bdS, 4))
            );
            assert_eq!(wire_point["coorType"], "gcj02");
            assert_eq!(wire_point["count"].as_i64(), Some(generated_point.count));
            assert_eq!(wire_point["dtr"].as_f64(), Some(0.0));
            assert_eq!(wire_point["flag"].as_i64(), Some(track.startTime));
            assert_eq!(wire_point["type"].as_i64(), Some(generated_point.ptype));
            assert_eq!(wire_point["state"].as_i64(), Some(generated_point.state));
            assert_eq!(
                wire_point["locType"].as_i64(),
                Some(generated_point.locType)
            );
            let (expected_lat, expected_lng) =
                bd09_to_gcj02(generated_point.gLat, generated_point.gLng);
            assert!(
                (wire_point["gLat"].as_f64().unwrap() - super::geom::round_to(expected_lat, 7))
                    .abs()
                    < 1e-9
            );
            assert!(
                (wire_point["gLng"].as_f64().unwrap() - super::geom::round_to(expected_lng, 7))
                    .abs()
                    < 1e-9
            );
            assert_eq!(wire_point["gainTime"], generated_point.gainTime);
            assert_eq!(wire_point["id"].as_i64(), Some(generated_point.id));
            assert_eq!(wire_point["lat"].as_f64(), Some(-1.0));
            assert_eq!(wire_point["lng"].as_f64(), Some(-1.0));
            assert_eq!(wire_point["locationId"], "");
            assert_eq!(
                wire_point["queueNum"].as_i64(),
                Some(generated_point.queueNum)
            );
            assert_eq!(
                wire_point["radius"].as_f64(),
                Some(rounded(generated_point.radius, 2))
            );
            assert_eq!(
                wire_point["speed"].as_f64(),
                Some(rounded(generated_point.speed, 4))
            );
            assert_eq!(
                wire_point["stepDistance"].as_f64(),
                Some(rounded(generated_point.stepDistance, 4))
            );
            assert_eq!(
                wire_point["totalTime"].as_i64(),
                Some(generated_point.totalTime)
            );
            assert!(
                (wire_point["totalDis"].as_f64().unwrap()
                    - super::geom::round_to(generated_point.totalDis, 4))
                .abs()
                    < 1e-9
            );
            assert_eq!(wire_point["steps"].as_i64(), Some(generated_point.steps));
            assert_eq!(
                wire_point["validDis"].as_f64(),
                Some(rounded(generated_point.validDis, 4))
            );
            assert_eq!(
                wire_point["validTime"].as_i64(),
                Some(generated_point.validTime)
            );
        }
        assert_eq!(pts[0].as_object().unwrap().len(), 28, "点键数必须 28");
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
