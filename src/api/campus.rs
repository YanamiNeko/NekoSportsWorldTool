//! Only server-provided campus metadata and polygons are selectable.
use super::fence::GeoFenceResponse;
use sha2::{Digest, Sha256};
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Campus {
    pub key: String,
    pub name: String,
    pub polygons: Vec<Vec<(f64, f64)>>,
    pub fences: Vec<serde_json::Value>,
}
impl Campus {
    /// Only explicit server area IDs may scope the points request; fence IDs are unrelated.
    pub fn run_area_id(&self, fallback: i64) -> Result<Option<String>, String> {
        let mut ids = Vec::new();
        for fence in &self.fences {
            let Some(value) = fence.get("runAreaId").filter(|v| !v.is_null()) else {
                continue;
            };
            let id = value
                .as_i64()
                .or_else(|| value.as_str().and_then(|s| s.trim().parse::<i64>().ok()))
                .ok_or("校区 runAreaId 格式无效")?;
            if id >= 0 && !ids.contains(&id) {
                ids.push(id);
            }
        }
        if ids.len() > 1 {
            return Err("所选校区包含多个不同区域 ID，不能确定点位查询范围".into());
        }
        Ok(ids
            .first()
            .copied()
            .or((fallback >= 0).then_some(fallback))
            .map(|id| id.to_string()))
    }
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Selection {
    pub school: String,
    pub key: String,
    pub confirmed: bool,
}
fn field(v: &serde_json::Value, key: &str) -> Option<String> {
    v.get(key).and_then(|v| match v {
        serde_json::Value::String(s) if !s.trim().is_empty() => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        _ => None,
    })
}
pub fn catalog(response: &GeoFenceResponse) -> Vec<Campus> {
    let mut grouped: Vec<(String, Campus)> = Vec::new();
    for (i, (poly, fence)) in response
        .polygons
        .iter()
        .zip(&response.area.geo_fences)
        .enumerate()
    {
        let group = field(fence, "campusId")
            .map(|v| format!("campus:{v}"))
            .unwrap_or_else(|| format!("fence:{i}"));
        if let Some((_, campus)) = grouped.iter_mut().find(|(g, _)| *g == group) {
            campus.polygons.push(poly.clone());
            campus.fences.push(fence.clone());
            continue;
        }
        let name = field(fence, "campusName")
            .or_else(|| {
                field(fence, "fenceName")
                    .or_else(|| field(fence, "name"))
                    .map(|n| format!("围栏：{n}"))
            })
            .unwrap_or_else(|| format!("服务端围栏 {}（未提供校区名）", i + 1));
        grouped.push((
            group,
            Campus {
                key: String::new(),
                name,
                polygons: vec![poly.clone()],
                fences: vec![fence.clone()],
            },
        ));
    }
    grouped
        .into_iter()
        .map(|(group, mut c)| {
            c.key = format!(
                "{:x}",
                Sha256::digest(serde_json::to_vec(&(group, &c.polygons)).unwrap())
            );
            c
        })
        .collect()
}
pub fn chosen<'a>(
    list: &'a [Campus],
    selection: Option<&Selection>,
    school: &str,
) -> Option<&'a Campus> {
    selection
        .filter(|s| s.school == school)
        .and_then(|s| list.iter().find(|c| c.key == s.key))
}
pub fn selected_response(
    response: GeoFenceResponse,
    selection: Option<&Selection>,
    school: &str,
) -> Result<GeoFenceResponse, String> {
    let list = catalog(&response);
    let c = chosen(&list, selection, school)
        .filter(|_| selection.map(|s| s.confirmed).unwrap_or(false))
        .ok_or("请到路网页选择校区并确认围栏；围栏变化后需要重新确认")?;
    let mut area = response.area;
    area.geo_fences = c.fences.clone();
    area.freedom_show_fence = true;
    area.run_area_id = c
        .run_area_id(area.run_area_id)?
        .and_then(|id| id.parse().ok())
        .unwrap_or(-1);
    Ok(GeoFenceResponse {
        polygons: c.polygons.clone(),
        area,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names_and_confirmation_are_bound_to_real_fences_and_school() {
        let mut r = GeoFenceResponse::default();
        r.polygons = vec![
            vec![(1., 1.), (1., 2.), (2., 1.)],
            vec![(3., 3.), (3., 4.), (4., 3.)],
        ];
        r.area.geo_fences = vec![
            serde_json::json!({"campusId":1,"campusName":"东校区"}),
            serde_json::json!({"id":9}),
        ];
        let c = catalog(&r);
        assert_eq!(c[0].name, "东校区");
        assert!(c[1].name.contains("未提供"));
        let sel = Selection {
            school: "a".into(),
            key: c[0].key.clone(),
            confirmed: true,
        };
        assert!(chosen(&c, Some(&sel), "b").is_none());
        let picked = selected_response(r, Some(&sel), "a").unwrap();
        assert_eq!(picked.polygons.len(), 1);
        assert_eq!(picked.area.run_area_id, -1);
    }
}

#[cfg(test)]
mod area_tests {
    use super::*;
    #[test]
    fn points_area_uses_explicit_selected_id_without_inventing_one() {
        let mut c = Campus {
            key: String::new(),
            name: String::new(),
            polygons: vec![],
            fences: vec![serde_json::json!({"id":999})],
        };
        assert_eq!(c.run_area_id(-1).unwrap(), None);
        assert_eq!(c.run_area_id(42).unwrap().as_deref(), Some("42"));
        c.fences = vec![serde_json::json!({"runAreaId":"73"})];
        assert_eq!(c.run_area_id(42).unwrap().as_deref(), Some("73"));
        c.fences.push(serde_json::json!({"runAreaId":73}));
        assert_eq!(c.run_area_id(-1).unwrap().as_deref(), Some("73"));
        c.fences.push(serde_json::json!({"runAreaId":74}));
        assert!(c.run_area_id(42).is_err());
    }
}
