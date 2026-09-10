//! 轨迹数据结构，序列化键名与协议逐字段一致。

#![allow(non_snake_case)]

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

