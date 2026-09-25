//! 轨迹数据结构，序列化键名与协议逐字段一致。

#![allow(non_snake_case)]

use crate::location::Coordinate;
use serde::Serialize;

#[derive(Serialize, Clone, Debug)]
pub struct GenPoint {
    pub id: i64,
    pub flag: i64,
    pub lat: f64,
    pub lng: f64,
    pub gLat: f64,
    pub gLng: f64,
    pub speed: f64,
    pub avgSpeed: f64,
    pub radius: f64,
    pub accuracy: f64,
    #[serde(rename = "type")]
    pub ptype: i64,
    pub locType: i64,
    pub hasAltitude: bool,
    pub totalTime: i64,
    pub totalDis: f64,
    pub validDis: f64,
    pub validTime: i64,
    pub steps: i64,
    pub stepDistance: f64,
    pub gainTime: String,
    pub gainTimeMs: i64,
    pub queueNum: i64,
    pub coorType: String,
    pub bdA: f64,
    pub bdD: f64,
    pub bdS: f64,
    pub bdG: i64,
    pub count: i64,
    pub dtr: f64,
    pub state: i64,
    pub locationId: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct TenWindow {
    pub time: i64,
    pub value: f64,
}

#[derive(Serialize, Clone, Debug)]
pub struct Segment {
    pub totalTime: i64,
    pub distance: i64,
    pub startTime: i64,
    pub endTime: i64,
    pub avgSpeed: f64,
    pub avgStep: i64,
    pub state: i64,
}

#[derive(Serialize, Clone, Debug)]
pub struct Track {
    pub totalTime: i64,
    pub totalDistance: f64,
    pub validDistance: f64,
    pub validTime: i64,
    pub startTime: i64,
    pub startLatitude: f64,
    pub startLongitude: f64,
    pub locations: Vec<GenPoint>,
    pub totalSteps: i64,
    pub speedPerTenSec: Vec<TenWindow>,
    pub stepsPerTenSec: Vec<TenWindow>,
    pub segments: Vec<Segment>,
    /// 用户指定海拔范围时提交协议使用的目标累计爬升；自动海拔时为空。
    #[serde(skip)]
    pub altitude_gain_override: Option<f64>,
}

impl Track {
    /// Interpolate cumulative distance and steps at a whole-second boundary.
    /// Start sentinels are intentionally zeroed, so time-zero points do not
    /// replace the synthetic origin. The final anchor is tied to Track totals
    /// to keep the last submission window conserved despite point rounding.
    pub fn cumulative_at(&self, target: i64) -> (f64, i64) {
        let duration = self.totalTime.max(0);
        let target = target.clamp(0, duration) as f64;
        let mut anchors = vec![(0.0f64, 0.0f64, 0.0f64)];

        for point in &self.locations {
            let time = point.totalTime as f64;
            if time <= 0.0 || time > duration as f64 {
                continue;
            }
            let mut current = (time, point.totalDis.max(0.0), point.steps.max(0) as f64);
            let last = anchors.last().copied().unwrap_or((0.0, 0.0, 0.0));
            if current.0 < last.0 {
                continue;
            }
            current.1 = current.1.max(last.1);
            current.2 = current.2.max(last.2);
            if current.0 == last.0 {
                if let Some(anchor) = anchors.last_mut() {
                    *anchor = current;
                }
            } else {
                anchors.push(current);
            }
        }

        let endpoint = (
            duration as f64,
            self.totalDistance.max(0.0),
            self.totalSteps.max(0) as f64,
        );
        if let Some(last) = anchors.last_mut() {
            if (last.0 - endpoint.0).abs() < f64::EPSILON {
                *last = endpoint;
            } else if last.0 < endpoint.0 {
                anchors.push(endpoint);
            }
        }

        for pair in anchors.windows(2) {
            let (t0, d0, s0) = pair[0];
            let (t1, d1, s1) = pair[1];
            if target <= t1 {
                let ratio = ((target - t0) / (t1 - t0)).clamp(0.0, 1.0);
                let distance = d0 + (d1 - d0) * ratio;
                let steps = (s0 + (s1 - s0) * ratio).round().max(0.0) as i64;
                return (distance, steps);
            }
        }
        (endpoint.1, endpoint.2 as i64)
    }

    /// Build the single canonical set of 10-second windows used by both the
    /// record request and OBS. A shorter final window keeps its real duration.
    pub fn ten_second_windows(&self) -> (Vec<TenWindow>, Vec<TenWindow>) {
        let mut speed = Vec::new();
        let mut steps = Vec::new();
        let mut lo = 0i64;
        while lo < self.totalTime {
            let hi = (lo + 10).min(self.totalTime);
            let (d_lo, s_lo) = self.cumulative_at(lo);
            let (d_hi, s_hi) = self.cumulative_at(hi);
            speed.push(TenWindow {
                time: hi - lo,
                value: ((d_hi - d_lo).max(0.0) * 10_000.0).round() / 10_000.0,
            });
            steps.push(TenWindow {
                time: hi - lo,
                value: (s_hi - s_lo).max(0) as f64,
            });
            lo = hi;
        }
        (speed, steps)
    }

    /// 记录顶层坐标唯一从轨迹首点派生，避免调用方重复传入另一套坐标。
    pub fn start_coordinate(&self) -> Result<Coordinate, String> {
        let first = self.locations.first().ok_or("轨迹不能为空".to_string())?;
        let coordinate = Coordinate::new(self.startLatitude, self.startLongitude, first.accuracy)?;
        let first_coordinate = Coordinate::new(first.gLat, first.gLng, first.accuracy)?;
        if !coordinate.is_near(first_coordinate, 0.0000001) {
            return Err("轨迹首点与记录顶层坐标不一致".into());
        }
        Ok(coordinate)
    }

    pub fn validate_consistency(&self) -> Result<Coordinate, String> {
        if self.totalTime <= 0 || self.totalDistance <= 0.0 {
            return Err("轨迹时长和距离必须为正数".into());
        }
        let (mut prev_time, mut prev_dis, mut prev_valid_time, mut prev_valid_dis, mut prev_steps) =
            (0, 0.0, 0, 0.0, 0);
        for point in &self.locations {
            Coordinate::new(point.gLat, point.gLng, point.accuracy)?;
            if point.totalTime < prev_time
                || point.totalDis + 1e-6 < prev_dis
                || point.steps < prev_steps
            {
                return Err("轨迹点累计时间、距离或步数回退".into());
            }
            if point.validTime < prev_valid_time || point.validDis + 1e-6 < prev_valid_dis {
                return Err("轨迹点有效时间或有效距离回退".into());
            }
            (
                prev_time,
                prev_dis,
                prev_valid_time,
                prev_valid_dis,
                prev_steps,
            ) = (
                point.totalTime,
                point.totalDis,
                point.validTime,
                point.validDis,
                point.steps,
            );
        }
        let last = self.locations.last().ok_or("轨迹不能为空".to_string())?;
        if last.totalTime != self.totalTime
            || last.validTime != self.validTime
            || (last.totalDis - self.totalDistance).abs() > 0.01
            || (last.validDis - self.validDistance).abs() > 0.01
            || last.steps != self.totalSteps
        {
            return Err("终点累计值与轨迹总计不一致".into());
        }
        self.start_coordinate()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sample_track() -> Track {
        let point = GenPoint {
            id: 1,
            flag: 1,
            lat: -1.0,
            lng: -1.0,
            gLat: 39.9,
            gLng: 116.4,
            speed: 1.0,
            avgSpeed: 1.0,
            radius: 3.0,
            accuracy: 3.0,
            ptype: 0,
            locType: 1,
            hasAltitude: true,
            totalTime: 1,
            totalDis: 1.0,
            validDis: 1.0,
            validTime: 1,
            steps: 1,
            stepDistance: 0.0,
            gainTime: String::new(),
            gainTimeMs: 1,
            queueNum: 0,
            coorType: "gcj02".into(),
            bdA: 1.0,
            bdD: 0.0,
            bdS: 1.0,
            bdG: 1,
            count: 1,
            dtr: 0.0,
            state: 0,
            locationId: String::new(),
        };
        Track {
            totalTime: 1,
            totalDistance: 1.0,
            validDistance: 1.0,
            validTime: 1,
            startTime: 1,
            startLatitude: 39.9,
            startLongitude: 116.4,
            locations: vec![point],
            totalSteps: 1,
            speedPerTenSec: vec![],
            stepsPerTenSec: vec![],
            segments: vec![],
            altitude_gain_override: None,
        }
    }
    #[test]
    fn top_level_coordinate_is_derived_from_first_point() {
        assert!(sample_track().validate_consistency().is_ok());
        let mut invalid = sample_track();
        invalid.startLatitude = 38.9;
        assert!(invalid.validate_consistency().is_err());
    }
    #[test]
    fn empty_track_is_rejected() {
        let mut track = sample_track();
        track.locations.clear();
        assert!(track.validate_consistency().is_err());
    }

    #[test]
    fn cumulative_windows_keep_short_tail_and_conserve_totals() {
        let mut track = sample_track();
        track.totalTime = 374;
        track.totalDistance = 1000.0;
        track.validDistance = 1000.0;
        track.validTime = 374;
        track.totalSteps = 600;
        track.locations = (1..=374)
            .map(|time| GenPoint {
                totalTime: time,
                totalDis: time as f64 * 1000.0 / 374.0,
                validDis: time as f64 * 1000.0 / 374.0,
                validTime: time,
                steps: (time * 600) / 374,
                ..sample_track().locations[0].clone()
            })
            .collect();
        let (distance_windows, step_windows) = track.ten_second_windows();
        assert_eq!(distance_windows.len(), 38);
        assert_eq!(step_windows.len(), 38);
        assert_eq!(distance_windows.last().unwrap().time, 4);
        assert_eq!(step_windows.last().unwrap().time, 4);
        assert!(
            (distance_windows
                .iter()
                .map(|window| window.value)
                .sum::<f64>()
                - 1000.0)
                .abs()
                < 0.01
        );
        assert_eq!(
            step_windows
                .iter()
                .map(|window| window.value as i64)
                .sum::<i64>(),
            600
        );
    }
}
