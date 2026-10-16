//! One-shot WinRT location requests polled on the foreground UI thread.
use crate::location::Coordinate;
use std::{
    marker::PhantomData,
    rc::Rc,
    time::{Duration, Instant},
};
use windows::{
    Devices::Geolocation::{GeolocationAccessStatus, Geolocator, Geoposition, PositionAccuracy},
    Foundation::{AsyncStatus, IAsyncOperation, TimeSpan},
    Win32::System::WinRT::{RoInitialize, RoUninitialize, RO_INIT_SINGLETHREADED},
};

// COM initialization and cleanup must occur on the same UI thread. Winit may
// already have initialized an apartment; RPC_E_CHANGED_MODE means reuse it.
struct Runtime {
    owned: bool,
    _ui_thread: PhantomData<Rc<()>>,
}
impl Runtime {
    fn new() -> Result<Self, String> {
        let owned = match unsafe { RoInitialize(RO_INIT_SINGLETHREADED) } {
            Ok(()) => true,
            Err(error) if error.code().0 == 0x80010106u32 as i32 => false,
            Err(error) => return Err(format!("初始化 Windows 定位失败：{error}")),
        };
        Ok(Self {
            owned,
            _ui_thread: PhantomData,
        })
    }
}
impl Drop for Runtime {
    fn drop(&mut self) {
        if self.owned {
            unsafe { RoUninitialize() };
        }
    }
}
enum Stage {
    Access(IAsyncOperation<GeolocationAccessStatus>),
    Position(IAsyncOperation<Geoposition>),
}
pub(crate) struct Request {
    // Operations must be released before uninitializing the apartment.
    stage: Stage,
    started: Instant,
    _runtime: Runtime,
}
fn failure(error: windows::core::Error) -> String {
    match error.code().0 as u32 {
        0x80070005 => {
            "Windows 拒绝位置访问；请在「设置 → 隐私和安全性 → 位置」开启位置服务和桌面应用访问权限"
                .into()
        }
        0x800705B4 => "Windows 定位超时；请检查位置服务或使用地图选点".into(),
        _ => format!("Windows 定位失败：{error}；可使用地图选点"),
    }
}
impl Request {
    pub fn start() -> Result<Self, String> {
        let runtime = Runtime::new()?;
        // Microsoft requires RequestAccessAsync on a foreground UI thread.
        let access = Geolocator::RequestAccessAsync().map_err(failure)?;
        Ok(Self {
            stage: Stage::Access(access),
            started: Instant::now(),
            _runtime: runtime,
        })
    }
    pub fn poll(&mut self) -> Option<Result<Coordinate, String>> {
        if self.started.elapsed() >= Duration::from_secs(60) {
            self.cancel();
            return Some(Err(
                "Windows 定位超时（60 秒）；请检查位置服务或使用地图选点".into(),
            ));
        }
        let status = match &self.stage {
            Stage::Access(op) => op.Status(),
            Stage::Position(op) => op.Status(),
        };
        match status {
            Ok(AsyncStatus::Started) => return None,
            Err(error) => return Some(Err(failure(error))),
            _ => {}
        }
        match &self.stage {
            Stage::Access(op) => {
                let access = match op.GetResults() {
                    Ok(access) => access,
                    Err(error) => return Some(Err(failure(error))),
                };
                if access != GeolocationAccessStatus::Allowed {
                    return Some(Err("Windows 未授权定位；请开启系统位置服务和桌面应用位置访问权限，或使用地图选点".into()));
                }
                let result = (|| {
                    let locator = Geolocator::new()?;
                    locator.SetDesiredAccuracy(PositionAccuracy::High)?;
                    locator.GetGeopositionAsyncWithAgeAndTimeout(
                        TimeSpan {
                            Duration: 30 * 10_000_000,
                        },
                        TimeSpan {
                            Duration: 60 * 10_000_000,
                        },
                    )
                })();
                match result {
                    Ok(op) => {
                        self.stage = Stage::Position(op);
                        None
                    }
                    Err(error) => Some(Err(failure(error))),
                }
            }
            Stage::Position(op) => Some(
                op.GetResults()
                    .and_then(|position| {
                        let coordinate = position.Coordinate()?;
                        let position = coordinate.Point()?.Position()?;
                        Ok((
                            position.Latitude,
                            position.Longitude,
                            coordinate.Accuracy()?,
                        ))
                    })
                    .map_err(failure)
                    .and_then(|(lat, lon, accuracy)| Coordinate::new(lat, lon, accuracy)),
            ),
        }
    }
    pub fn cancel(&self) {
        let _ = match &self.stage {
            Stage::Access(op) => op.Cancel(),
            Stage::Position(op) => op.Cancel(),
        };
    }
}
impl Drop for Request {
    fn drop(&mut self) {
        self.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn access_denial_has_actionable_guidance() {
        let message = failure(windows::core::Error::from_hresult(windows::core::HRESULT(
            0x80070005u32 as i32,
        )));
        assert!(message.contains("桌面应用") && message.contains("拒绝"));
    }
    #[test]
    #[ignore = "live Windows location-service activation; does not request coordinates"]
    fn live_windows_location_service() {
        let _runtime = Runtime::new().unwrap();
        let locator = Geolocator::new().unwrap();
        assert!(locator.LocationStatus().is_ok());
    }
}
