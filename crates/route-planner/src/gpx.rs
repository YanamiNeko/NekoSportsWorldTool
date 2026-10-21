//! Bounded GPX track-point parsing; segments remain separate.
use crate::Coord;
use quick_xml::{events::Event, Reader};
#[derive(Clone, Debug)]
pub struct Sample {
    pub coordinate: Coord,
    pub time: Option<String>,
    pub segment: usize,
}
pub fn parse(text: &str) -> Result<Vec<Sample>, String> {
    if text.len() > 24 * 1024 * 1024 {
        return Err("GPX 超过 24 MiB".into());
    }
    let mut reader = Reader::from_str(text);
    reader.config_mut().trim_text(true);
    reader.config_mut().expand_empty_elements = true;
    let (mut points, mut current, mut segment, mut in_time, mut depth, mut root) =
        (Vec::new(), None::<Sample>, 0, false, 0usize, false);
    loop {
        match reader
            .read_event()
            .map_err(|e| format!("GPX XML 无效：{e}"))?
        {
            Event::Start(e) => {
                let name = e.local_name();
                let name = name.as_ref();
                if name == b"gpx" {
                    root = true;
                }
                if name == b"trkseg" {
                    segment += 1;
                }
                if name == b"trkpt" {
                    if current.is_some() {
                        return Err("GPX 点嵌套无效".into());
                    }
                    let (mut lat, mut lon) = (None, None);
                    for a in e.attributes() {
                        let a = a.map_err(|e| e.to_string())?;
                        let v = std::str::from_utf8(&a.value).map_err(|e| e.to_string())?;
                        if a.key.as_ref() == b"lat" {
                            lat = v.parse::<f64>().ok();
                        }
                        if a.key.as_ref() == b"lon" {
                            lon = v.parse::<f64>().ok();
                        }
                    }
                    let (lat, lon) = (lat.ok_or("GPX 点缺少纬度")?, lon.ok_or("GPX 点缺少经度")?);
                    if !lat.is_finite() || !lon.is_finite() || lat.abs() > 90. || lon.abs() > 180. {
                        return Err("GPX 坐标无效".into());
                    }
                    current = Some(Sample {
                        coordinate: Coord::new(lon, lat),
                        time: None,
                        segment,
                    });
                }
                if name == b"time" && current.is_some() {
                    in_time = true;
                }
                depth += 1;
            }
            Event::End(e) => {
                depth = depth.saturating_sub(1);
                match e.local_name().as_ref() {
                    b"time" => in_time = false,
                    b"trkpt" => {
                        if let Some(p) = current.take() {
                            points.push(p);
                        }
                    }
                    _ => {}
                }
            }
            Event::Text(e) if in_time => {
                if let Some(p) = &mut current {
                    p.time = Some(e.decode().map_err(|e| e.to_string())?.to_string());
                }
            }
            Event::DocType(_) => return Err("不支持带 DTD 的 GPX".into()),
            Event::Eof => break,
            _ => {}
        }
    }
    if !root || depth != 0 || current.is_some() || points.len() < 2 {
        return Err("GPX 不完整或少于两个轨迹点".into());
    }
    Ok(points)
}
