//! 轨迹数据结构，序列化键名与协议逐字段一致。

#![allow(non_snake_case)]

use serde::Serialize;
use crate::location::Coordinate;

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
        if !coordinate.is_near(first_coordinate, 0.0000001) { return Err("轨迹首点与记录顶层坐标不一致".into()); }
        Ok(coordinate)
    }

    pub fn validate_consistency(&self) -> Result<Coordinate, String> {
        if self.totalTime <= 0 || self.totalDistance <= 0.0 { return Err("轨迹时长和距离必须为正数".into()); }
        let (mut prev_time, mut prev_dis, mut prev_valid_time, mut prev_valid_dis, mut prev_steps) =
            (0, 0.0, 0, 0.0, 0);
        for point in &self.locations {
            Coordinate::new(point.gLat, point.gLng, point.accuracy)?;
            if point.totalTime < prev_time
                || point.totalDis + 1e-6 < prev_dis
                || point.steps < prev_steps
                || point.validTime < prev_valid_time
                || point.validDis + 1e-6 < prev_valid_dis
            {
                return Err("轨迹点累计时间、距离或步数回退".into());
            }
            (prev_time, prev_dis, prev_valid_time, prev_valid_dis, prev_steps) = (
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
        let point = GenPoint { id: 1, flag: 1, lat: -1.0, lng: -1.0, gLat: 39.9, gLng: 116.4, speed: 1.0, avgSpeed: 1.0, radius: 3.0, accuracy: 3.0, ptype: 0, locType: 1, hasAltitude: true, totalTime: 1, totalDis: 1.0, validDis: 1.0, validTime: 1, steps: 1, stepDistance: 0.0, gainTime: String::new(), gainTimeMs: 1, queueNum: 0, coorType: "gcj02".into(), bdA: 1.0, bdD: 0.0, bdS: 1.0, bdG: 1, count: 1, dtr: 0.0, state: 0, locationId: String::new() };
        Track { totalTime: 1, totalDistance: 1.0, validDistance: 1.0, validTime: 1, startTime: 1, startLatitude: 39.9, startLongitude: 116.4, locations: vec![point], totalSteps: 1, speedPerTenSec: vec![], stepsPerTenSec: vec![], segments: vec![] }
    }
    #[test] fn top_level_coordinate_is_derived_from_first_point() { assert!(sample_track().validate_consistency().is_ok()); let mut invalid = sample_track(); invalid.startLatitude = 38.9; assert!(invalid.validate_consistency().is_err()); }
    #[test] fn empty_track_is_rejected() { let mut track = sample_track(); track.locations.clear(); assert!(track.validate_consistency().is_err()); }
}

