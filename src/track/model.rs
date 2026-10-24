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
}

impl Track {
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
        if self.totalTime <= 0
            || !self.totalDistance.is_finite()
            || self.totalDistance <= 0.0
            || !self.validDistance.is_finite()
        {
            return Err("轨迹时长和距离必须为正数".into());
        }
        for point in &self.locations {
            Coordinate::new(point.gLat, point.gLng, point.accuracy)?;
            if !point.totalDis.is_finite()
                || !point.validDis.is_finite()
                || point.totalDis < 0.0
                || point.validDis < 0.0
                || point.validDis > point.totalDis
                || point.totalTime < 0
                || point.validTime < 0
                || point.validTime > point.totalTime
                || point.steps < 0
            {
                return Err("轨迹累计统计无效".into());
            }
        }
        for pair in self.locations.windows(2) {
            let (a, b) = (&pair[0], &pair[1]);
            if b.totalTime < a.totalTime
                || b.totalDis < a.totalDis
                || b.steps < a.steps
                || b.validDis < a.validDis
                || b.validTime < a.validTime
            {
                return Err("轨迹累计统计发生倒退".into());
            }
        }
        let last = self.locations.last().ok_or("轨迹不能为空")?;
        if self.totalTime != last.totalTime
            || (self.totalDistance - last.totalDis).abs() > 1e-6
            || self.totalSteps != last.steps
            || self.validTime != last.validTime
            || (self.validDistance - last.validDis).abs() > 1e-6
        {
            return Err("轨迹总量与终点统计不一致".into());
        }
        if self.speedPerTenSec.len() != self.stepsPerTenSec.len() {
            return Err("距离与步数窗口数量不一致".into());
        }
        if self.speedPerTenSec.is_empty() { return Err("缺少距离和步数窗口".into()); }
        for (distance,steps) in self.speedPerTenSec.iter().zip(&self.stepsPerTenSec) {
            if distance.time <= 0 || distance.time != steps.time
                || !distance.value.is_finite() || distance.value < 0.
                || !steps.value.is_finite() || steps.value < 0. || steps.value.fract() != 0.
                || (distance.value > 0. && steps.value < 1.) {
                return Err("窗口统计无效：移动窗口不能为零步；不补造总步数".into());
            }
        }
        if self.speedPerTenSec.iter().map(|w| w.time).sum::<i64>() != self.totalTime
            || (self.speedPerTenSec.iter().map(|w| w.value).sum::<f64>()-self.totalDistance).abs() > 1e-5
            || (self.stepsPerTenSec.iter().map(|w| w.value).sum::<f64>()-self.totalSteps as f64).abs() > 1e-5 {
            return Err("窗口总量与轨迹总量不一致".into());
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
            speedPerTenSec: vec![TenWindow { time:1, value:1. }],
            stepsPerTenSec: vec![TenWindow { time:1, value:1. }],
            segments: vec![],
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
}
