//! Identity anchors are BD-09; maps, system fixes and Overpass use WGS84.
use crate::{crypto::header::HeaderIdentity, track::geom};

pub(super) fn anchor_wgs84(identity: &HeaderIdentity) -> (f64, f64) {
    geom::bd09_to_wgs84(identity.anchor_lat, identity.anchor_lon)
}

pub(super) fn set_anchor_wgs84(identity: &mut HeaderIdentity, point: (f64, f64)) {
    (identity.anchor_lat, identity.anchor_lon) = geom::wgs84_to_bd09(point.0, point.1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_and_system_fixes_roundtrip_through_persisted_bd09_anchor() {
        for point in [(30.0, 110.0), (40.0, 120.0), (0.0, 0.0), (-20.0, -60.0)] {
            let mut identity = HeaderIdentity::default();
            set_anchor_wgs84(&mut identity, point);
            assert_eq!(
                (identity.anchor_lat, identity.anchor_lon),
                geom::wgs84_to_bd09(point.0, point.1)
            );
            let restored: HeaderIdentity =
                serde_json::from_str(&serde_json::to_string(&identity).unwrap()).unwrap();
            let actual = anchor_wgs84(&restored);
            assert!((actual.0 - point.0).abs() < 1e-8);
            assert!((actual.1 - point.1).abs() < 1e-8);
        }
    }
}
