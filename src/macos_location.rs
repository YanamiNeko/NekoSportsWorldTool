//! Core Location must be created and polled on the main GUI thread.
use crate::location::Coordinate;
use objc2::{
    define_class, msg_send, rc::Retained, runtime::ProtocolObject, DefinedClass, MainThreadOnly,
};
use objc2_core_location::{
    CLAuthorizationStatus, CLLocation, CLLocationManager, CLLocationManagerDelegate,
};
use objc2_foundation::{
    ns_string, MainThreadMarker, NSArray, NSBundle, NSError, NSObject, NSObjectProtocol,
};
use std::{
    cell::RefCell,
    time::{Duration, Instant},
};

#[derive(Debug, Default)]
struct Ivars {
    result: RefCell<Option<Result<Coordinate, String>>>,
}
define_class!(
    // NSObject has no subclassing requirements; callbacks use the main run loop.
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = Ivars]
    struct Delegate;
    unsafe impl NSObjectProtocol for Delegate {}
    unsafe impl CLLocationManagerDelegate for Delegate {
        #[unsafe(method(locationManager:didUpdateLocations:))]
        fn updated(&self, _manager: &CLLocationManager, locations: &NSArray<CLLocation>) {
            if let Some(location) = locations.lastObject() {
                // Valid retained CLLocation received from the framework callback.
                let (point, accuracy, age) = unsafe {
                    (
                        location.coordinate(),
                        location.horizontalAccuracy(),
                        location.timestamp().timeIntervalSinceNow(),
                    )
                };
                if accuracy >= 0.0 && age.abs() <= 30.0 {
                    *self.ivars().result.borrow_mut() =
                        Some(Coordinate::new(point.latitude, point.longitude, accuracy));
                }
            }
        }
        #[unsafe(method(locationManager:didFailWithError:))]
        fn failed(&self, _manager: &CLLocationManager, error: &NSError) {
            // kCLErrorLocationUnknown is transient; wait for the next update.
            if error.code() != 0 {
                *self.ivars().result.borrow_mut() = Some(Err(format!(
                    "macOS 定位失败：{}；请检查位置服务和应用授权，或使用地图选点",
                    error.localizedDescription()
                )));
            }
        }
    }
);
pub(crate) struct Request {
    manager: Retained<CLLocationManager>,
    delegate: Retained<Delegate>,
    started: Instant,
    requesting: bool,
}
impl Request {
    pub fn start() -> Result<Self, String> {
        let marker = MainThreadMarker::new().ok_or("macOS 定位必须从 GUI 主线程发起")?;
        if unsafe {
            NSBundle::mainBundle()
                .objectForInfoDictionaryKey(ns_string!("NSLocationUsageDescription"))
        }
        .is_none()
        {
            return Err("请使用下载包内的 NekoSportsWorldTool.app 启动；裸二进制缺少 macOS 定位权限用途说明".into());
        }
        if !unsafe { CLLocationManager::locationServicesEnabled_class() } {
            return Err("macOS 位置服务未开启；请在「系统设置 → 隐私与安全性 → 定位服务」开启，或使用地图选点".into());
        }
        let allocated = Delegate::alloc(marker).set_ivars(Ivars::default());
        let delegate: Retained<Delegate> = unsafe { msg_send![super(allocated), init] };
        let manager = unsafe { CLLocationManager::new() };
        unsafe {
            manager.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
            manager.setDesiredAccuracy(10.0);
            manager.requestWhenInUseAuthorization();
        }
        Ok(Self {
            manager,
            delegate,
            started: Instant::now(),
            requesting: false,
        })
    }
    pub fn poll(&mut self) -> Option<Result<Coordinate, String>> {
        if let Some(result) = self.delegate.ivars().result.borrow_mut().take() {
            return Some(result);
        }
        if self.started.elapsed() >= Duration::from_secs(60) {
            return Some(Err(
                "macOS 定位超时（60 秒）；请检查授权或使用地图选点".into()
            ));
        }
        match unsafe { self.manager.authorizationStatus() } {
            CLAuthorizationStatus::Denied | CLAuthorizationStatus::Restricted => Some(Err(
                "macOS 未授权定位；请在定位服务中允许此应用，或使用地图选点".into(),
            )),
            CLAuthorizationStatus::AuthorizedAlways
            | CLAuthorizationStatus::AuthorizedWhenInUse => {
                if !self.requesting {
                    self.requesting = true;
                    unsafe {
                        self.manager.startUpdatingLocation();
                    }
                }
                None
            }
            _ => None,
        }
    }
}
impl Drop for Request {
    fn drop(&mut self) {
        unsafe {
            self.manager.stopUpdatingLocation();
            self.manager.setDelegate(None);
        }
    }
}
