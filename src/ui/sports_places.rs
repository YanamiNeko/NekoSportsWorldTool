//! Sports facilities are map features, not automatically walkable road edges.
use crate::location::Coordinate;
use std::{io::Read, time::Duration};

#[derive(Clone)]
pub(super) struct SportsPlace {
    pub name: String,
    pub kind: String,
    pub sport: String,
    pub center: (f64, f64),
    pub outlines: Vec<Vec<(f64, f64)>>,
}
fn point(value: &serde_json::Value) -> Option<(f64, f64)> {
    let coordinate = (value["lat"].as_f64()?, value["lon"].as_f64()?);
    Coordinate::new(coordinate.0, coordinate.1, 0.0).ok()?;
    Some(coordinate)
}
fn bounds_center(value: &serde_json::Value) -> Option<(f64, f64)> {
    let min = Coordinate::new(value["minlat"].as_f64()?, value["minlon"].as_f64()?, 0.0).ok()?;
    let max = Coordinate::new(value["maxlat"].as_f64()?, value["maxlon"].as_f64()?, 0.0).ok()?;
    if min.latitude > max.latitude || min.longitude > max.longitude {
        return None;
    }
    Some((
        (min.latitude + max.latitude) / 2.0,
        (min.longitude + max.longitude) / 2.0,
    ))
}
fn parse(value: &serde_json::Value) -> Result<Vec<SportsPlace>, String> {
    if value.get("remark").is_some() {
        return Err("操场查询未完成，请稍后重试".into());
    }
    let mut places = Vec::new();
    for element in value["elements"].as_array().ok_or("操场响应格式无效")? {
        let tags = &element["tags"];
        let leisure = tags["leisure"].as_str().unwrap_or("");
        let kind = match leisure {
            "track" => "跑道",
            "pitch" => "运动场",
            "stadium" => "体育场",
            "sports_centre" => "体育中心",
            _ => match tags["amenity"].as_str() {
                Some("school") => "学校校园",
                Some("college" | "university") => "高校校园",
                _ => continue,
            },
        };
        // Overpass `out geom` returns bounds instead of center for ways and relations.
        let center = point(&element["center"])
            .or_else(|| point(element))
            .or_else(|| bounds_center(&element["bounds"]));
        let Some(center) = center else { continue };
        let mut outlines = Vec::new();
        if let Some(geometry) = element["geometry"].as_array() {
            outlines.push(geometry.iter().filter_map(point).collect());
        }
        if let Some(members) = element["members"].as_array() {
            for member in members.iter().filter(|member| member["role"] == "outer") {
                if let Some(geometry) = member["geometry"].as_array() {
                    outlines.push(geometry.iter().filter_map(point).collect());
                }
            }
        }
        places.push(SportsPlace {
            name: tags["name:zh"]
                .as_str()
                .or_else(|| tags["name"].as_str())
                .unwrap_or("未命名")
                .to_owned(),
            kind: kind.into(),
            sport: tags["sport"].as_str().unwrap_or("未标注项目").into(),
            center,
            outlines,
        });
        if places.len() == 250 {
            break;
        }
    }
    Ok(places)
}
pub(super) fn find(center: (f64, f64)) -> Result<Vec<SportsPlace>, String> {
    Coordinate::new(center.0, center.1, 0.0)?;
    let area = format!("around:2000,{:.7},{:.7}", center.0, center.1);
    let query = format!("[out:json][timeout:30][maxsize:8388608];(nwr({area})[\"amenity\"~\"^(school|college|university)$\"];nwr({area})[\"leisure\"~\"^(track|stadium|sports_centre)$\"];nwr({area})[\"leisure\"=\"pitch\"][\"sport\"~\"(^|;)(soccer|athletics|running)(;|$)\"];);out tags center geom;");
    let response = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(45))
        .build()
        .post("https://overpass-api.de/api/interpreter")
        .set(
            "User-Agent",
            concat!(
                "NekoSportsWorldTool/",
                env!("CARGO_PKG_VERSION"),
                " (+https://github.com/YanamiNeko/NekoSportsWorldTool)"
            ),
        )
        .send_form(&[("data", &query)])
        .map_err(super::online_osm::request_error)?;
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(8388609)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > 8388608 {
        return Err("操场响应过大，请把地图移到目标学校附近".into());
    }
    let mut places = parse(&serde_json::from_slice(&bytes).map_err(|_| "操场响应无效")?)?;
    let distance = |place: &SportsPlace| {
        (place.center.0 - center.0).powi(2)
            + ((place.center.1 - center.1) * center.0.to_radians().cos()).powi(2)
    };
    places.sort_by(|a, b| distance(a).total_cmp(&distance(b)));
    Ok(places)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_pitch_geometry_and_names_without_inventing_tracks() {
        let value = serde_json::json!({"elements":[{"type":"way","center":{"lat":23.14,"lon":113.32},"tags":{"name":"天河体育场","leisure":"stadium","sport":"soccer"},"geometry":[{"lat":23.14,"lon":113.32},{"lat":23.141,"lon":113.321}]}]});
        let places = parse(&value).unwrap();
        assert_eq!(places[0].name, "天河体育场");
        assert_eq!(places[0].kind, "体育场");
        assert_eq!(places[0].outlines[0].len(), 2);
        assert!(parse(&serde_json::json!({"remark":"timeout","elements":[]})).is_err());
    }
    #[test]
    fn overpass_geometry_bounds_locate_ways_and_relations() {
        let value = serde_json::json!({"elements":[
            {"type":"way","bounds":{"minlat":23.1394141,"minlon":113.318316,"maxlat":23.1416378,"maxlon":113.3203931},"tags":{"name":"天河体育场","leisure":"stadium"},"geometry":[{"lat":23.1394141,"lon":113.318316}]},
            {"type":"relation","bounds":{"minlat":23.14,"minlon":113.31,"maxlat":23.15,"maxlon":113.32},"tags":{"leisure":"track"},"members":[{"role":"outer","geometry":[{"lat":23.14,"lon":113.31}]}]}
        ]});
        let places = parse(&value).unwrap();
        assert_eq!(places.len(), 2);
        assert!((places[0].center.0 - 23.14052595).abs() < 1e-8);
        assert_eq!(places[1].outlines[0].len(), 1);
        assert!(bounds_center(
            &serde_json::json!({"minlat":24,"minlon":113,"maxlat":23,"maxlon":114})
        )
        .is_none());
    }
    #[test]
    fn school_without_track_tag_is_a_campus_not_a_running_track() {
        let places = parse(&serde_json::json!({"elements":[{"type":"node","lat":23.14,"lon":113.32,"tags":{"amenity":"school","name":"示例学校"}}]})).unwrap();
        assert_eq!(places[0].name, "示例学校");
        assert_eq!(places[0].kind, "学校校园");
        assert!(places[0].outlines.is_empty());
    }
    #[test]
    #[ignore = "explicit public Tianhe sports facilities smoke test"]
    fn public_tianhe_facilities() {
        let places = find((23.1398, 113.3235)).unwrap();
        assert!(places.iter().any(|place| place.name == "天河体育场"));
        assert!(places.iter().any(|place| place.kind == "学校校园"));
    }
}
