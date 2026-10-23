//! Free-run geometry and the same motion model used by road routing.
//! Local control points are simulation geometry, never server checkpoints.
use super::geom::{make_point_ring, to_bd, Rng};
use super::model::Track;
use route_planner::{Coord, RoadGraph, Route, RoutePoint};

pub fn plan(center: (f64, f64), distance: f64, seed: u64) -> Result<Route, String> {
    crate::location::Coordinate::new(center.0, center.1, 0.0)?;
    if !distance.is_finite() || distance <= 0.0 {
        return Err("自由跑距离必须为正数".into());
    }
    let mut rng = Rng::new(seed.wrapping_add(0x85EB_CA6B));
    let rotation = rng.uniform(0.0, std::f64::consts::TAU);
    let ratio = rng.uniform(0.58, 0.82);
    let controls: Vec<_> = (0..12).map(|i| {
        let angle = std::f64::consts::TAU * i as f64 / 12.0;
        let radius = rng.uniform(0.88, 1.12);
        let x = radius * angle.cos();
        let y = ratio * radius * angle.sin();
        to_bd(x * rotation.cos() - y * rotation.sin(),
              x * rotation.sin() + y * rotation.cos(), center.0, center.1)
    }).collect();
    let (dense, arcs, origin) = make_point_ring(&controls);
    let circumference = (distance / 2.0).clamp(250.0, 600.0);
    let scale = circumference / arcs.last().copied().filter(|n| *n > 0.0).ok_or("自由跑路线长度无效")?;
    let plane: Vec<_> = dense.iter().map(|&(x,y)| (x * scale,y * scale)).collect();
    let mut points = Vec::with_capacity(plane.len() + 1);
    for i in 0..plane.len() {
        let a = plane[(i + plane.len() - 1) % plane.len()];
        let b = plane[i];
        let c = plane[(i + 1) % plane.len()];
        let ab = (b.0-a.0,b.1-a.1);
        let bc = (c.0-b.0,c.1-b.1);
        let ac = (c.0-a.0,c.1-a.1);
        let denominator = ab.0.hypot(ab.1) * bc.0.hypot(bc.1) * ac.0.hypot(ac.1);
        let curvature = if denominator > 1e-12 { 2.0 * (ab.0 * bc.1 - ab.1 * bc.0).abs() / denominator } else { 0.0 };
        let (lat, lon) = to_bd(b.0,b.1,origin.0,origin.1);
        crate::location::Coordinate::new(lat,lon,0.0)?;
        points.push(RoutePoint {lat,lon,s:arcs[i]*scale,curvature});
    }
    let first = points[0];
    points.push(RoutePoint {s:circumference,..first});
    Ok(Route {points,length_m:circumference})
}

pub fn build(distance: f64, duration: i64, seed: u64, center: (f64,f64), start: i64) -> Result<Track,String> {
    let route = plan(center,distance,seed)?;
    super::generate_road::build_on_route(distance,duration,seed,start,&route,center,&RoadGraph::new(),&[])
}

/// Use actual roads/buildings without representing the starting coordinate as a checkpoint.
pub fn build_on_roads(distance:f64,duration:i64,seed:u64,center:(f64,f64),start:i64,net:&RoadGraph) -> Result<Track,String> {
    let anchor = net.to_m(Coord::new(center.1,center.0));
    let (_,_,position,offset) = net.nearest_edge(anchor).ok_or("定位附近没有可用道路")?;
    if offset > 150.0 {
        return Err(format!("定位距离最近道路 {:.0} 米，请在道路附近设置定位锚点",offset));
    }
    let start_point = net.to_deg(position);
    let speed=distance/super::generate_road::effective_duration(distance,duration) as f64;
    let options=route_planner::RouteOptions { min_radius_m:(speed*speed/2.5).max(6.0), ..Default::default() };
    // Free runs explore the actual road graph, rather than repeating the nearest single edge.
    let route=route_planner::plan_route_split(net,&[start_point],&[],distance,seed,&options)?;
    super::generate_road::build_on_route(distance,duration,seed,start,&route,
        (start_point.lat,start_point.lon),net,&[])
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::geom::{MET_PER_DEG_LAT,MET_PER_DEG_LNG};
    #[test]
    fn local_route_varies_by_seed_is_closed_and_has_nonconstant_curvature() {
        let center=(39.9,116.4);
        let a=plan(center,1200.0,42).unwrap();
        let b=plan(center,1200.0,43).unwrap();
        assert_eq!(a.points.first().unwrap().lat,a.points.last().unwrap().lat);
        assert_eq!(a.points.first().unwrap().lon,a.points.last().unwrap().lon);
        assert_ne!(a.points[0].lat,b.points[0].lat);
        assert!((a.points.last().unwrap().s-a.length_m).abs()<1e-6);
        let radii:Vec<_>=a.points.iter().map(|p| ((p.lat-center.0)*MET_PER_DEG_LAT).hypot((p.lon-center.1)*MET_PER_DEG_LNG)).collect();
        assert!(radii.iter().copied().fold(f64::NEG_INFINITY,f64::max)-radii.iter().copied().fold(f64::INFINITY,f64::min)>20.0);
        assert!(a.points.iter().all(|p| p.curvature.is_finite() && p.curvature>=0.0));
        assert!(a.points.iter().any(|p| p.curvature>0.02));
    }
    #[test]
    fn generated_motion_is_reproducible_and_has_consistent_variable_statistics() {
        let center=(39.9,116.4);
        let free=build(1200.0,600,42,center,1_700_000_000_000).unwrap();
        let repeated=build(1200.0,600,42,center,1_700_000_000_000).unwrap();
        assert_eq!(serde_json::to_value(&free).unwrap(),serde_json::to_value(&repeated).unwrap());
        free.validate_consistency().unwrap();
        assert_eq!(free.totalDistance,1200.0);
        assert!((free.speedPerTenSec.iter().map(|p| p.value).sum::<f64>()-1200.0).abs()<0.01);
        assert!(free.totalSteps>0);
        let speeds:Vec<_>=free.locations.iter().filter(|p| ![-1,5,6].contains(&p.ptype)).map(|p| p.avgSpeed).collect();
        assert!(speeds.iter().copied().fold(f64::NEG_INFINITY,f64::max)-speeds.iter().copied().fold(f64::INFINITY,f64::min)>0.05);
    }
    #[test]
    fn invalid_inputs_fail_without_panicking() {
        for distance in [0.0,-1.0,f64::NAN,f64::INFINITY] {
            assert!(build(distance,600,42,(39.9,116.4),1).is_err());
        }
        assert!(build(1200.0,0,42,(39.9,116.4),1).is_err());
        assert!(plan((91.0,116.4),1200.0,42).is_err());
    }
}
