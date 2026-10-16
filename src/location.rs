//! 运行位置的统一数据结构与校验。
//!
//! 设备锚点、点位请求和跑步记录使用同一套纬度/经度语义。

use serde::{Deserialize, Serialize};

const DEFAULT_ANCHOR_LAT: f64 = 38.901678;
const DEFAULT_ANCHOR_LON: f64 = 121.540241;
const DEFAULT_CITY: &str = "大连市";

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Coordinate {
    pub latitude: f64,
    pub longitude: f64,
    #[serde(default)]
    pub accuracy: f64,
}

impl Coordinate {
    #[cfg(any(target_os = "android", test))]
    pub(crate) fn from_system_json(json: &str) -> Result<Self, String> {
        let coordinate: Self = serde_json::from_str(json).map_err(|_| "系统定位返回无效数据")?;
        coordinate.validate()?;
        Ok(coordinate)
    }
    pub fn new(latitude: f64, longitude: f64, accuracy: f64) -> Result<Self, String> {
        let coordinate = Self {
            latitude,
            longitude,
            accuracy,
        };
        coordinate.validate()?;
        Ok(coordinate)
    }

    pub fn validate(self) -> Result<(), String> {
        if !self.latitude.is_finite() || !(-90.0..=90.0).contains(&self.latitude) {
            return Err(format!("纬度超出范围: {}", self.latitude));
        }
        if !self.longitude.is_finite() || !(-180.0..=180.0).contains(&self.longitude) {
            return Err(format!("经度超出范围: {}", self.longitude));
        }
        if !self.accuracy.is_finite() || self.accuracy < 0.0 {
            return Err(format!("定位精度无效: {}", self.accuracy));
        }
        Ok(())
    }

    pub fn is_near(self, other: Self, tolerance: f64) -> bool {
        (self.latitude - other.latitude).abs() <= tolerance
            && (self.longitude - other.longitude).abs() <= tolerance
    }

    pub fn is_default_dalian(self) -> bool {
        self.is_near(
            Self {
                latitude: DEFAULT_ANCHOR_LAT,
                longitude: DEFAULT_ANCHOR_LON,
                accuracy: 0.0,
            },
            1e-6,
        )
    }
}

pub fn default_anchor() -> Coordinate {
    Coordinate {
        latitude: DEFAULT_ANCHOR_LAT,
        longitude: DEFAULT_ANCHOR_LON,
        accuracy: 0.0,
    }
}

pub fn default_city() -> &'static str {
    DEFAULT_CITY
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_bounds_and_accuracy() {
        assert!(Coordinate::new(39.9, 116.4, 5.0).is_ok());
        assert!(Coordinate::new(91.0, 116.4, 5.0).is_err());
        assert!(Coordinate::new(39.9, 181.0, 5.0).is_err());
        assert!(Coordinate::new(39.9, 116.4, -1.0).is_err());
    }

    #[test]
    fn system_location_rejects_invalid_values_and_preserves_accuracy() {
        let location = Coordinate::from_system_json(r#"{"latitude":39.9,"longitude":116.4,"accuracy":250.5}"#).unwrap();
        assert_eq!(location.accuracy, 250.5);
        assert!(Coordinate::from_system_json(r#"{"latitude":91,"longitude":116.4}"#).is_err());
        assert!(Coordinate::from_system_json(r#"{"latitude":39.9,"longitude":116.4,"accuracy":-1}"#).is_err());
        assert!(Coordinate::from_system_json("{}").is_err());
    }

    #[test]
    fn detects_legacy_default_location() {
        assert!(default_anchor().is_default_dalian());
        assert!(!Coordinate::new(39.9, 116.4, 0.0)
            .unwrap()
            .is_default_dalian());
    }
}
