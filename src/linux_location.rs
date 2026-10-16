//! GeoClue2 one-shot location on a cancellable worker; no UI thread D-Bus waits.
use crate::location::Coordinate;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use zbus::{
    blocking::{connection::Builder, Connection, Proxy},
    zvariant::OwnedObjectPath,
};

const SERVICE: &str = "org.freedesktop.GeoClue2";
pub(crate) struct Request {
    receiver: mpsc::Receiver<Result<Coordinate, String>>,
    canceled: Arc<AtomicBool>,
    started: Instant,
}
impl Request {
    pub fn start() -> Result<Self, String> {
        let (sender, receiver) = mpsc::channel();
        let canceled = Arc::new(AtomicBool::new(false));
        let worker_cancel = canceled.clone();
        std::thread::Builder::new().name("geoclue-location".into()).spawn(move || {
            let result = locate(&worker_cancel).map_err(|error| format!("Linux 定位失败：{error}；请检查 GeoClue2 服务、桌面位置权限及应用启动器，或使用地图选点"));
            let _ = sender.send(result);
        }).map_err(|error| error.to_string())?;
        Ok(Self {
            receiver,
            canceled,
            started: Instant::now(),
        })
    }
    pub fn poll(&mut self) -> Option<Result<Coordinate, String>> {
        if self.started.elapsed() >= Duration::from_secs(60) {
            self.canceled.store(true, Ordering::Relaxed);
            return Some(Err("Linux 定位超时（60 秒）；可使用地图选点".into()));
        }
        match self.receiver.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => Some(Err("Linux 定位线程已退出".into())),
        }
    }
}
impl Drop for Request {
    fn drop(&mut self) {
        self.canceled.store(true, Ordering::Relaxed);
    }
}

struct Client<'a> {
    proxy: Proxy<'a>,
    manager: Proxy<'a>,
    path: OwnedObjectPath,
}
impl Drop for Client<'_> {
    fn drop(&mut self) {
        let _ = self.proxy.call::<_, _, ()>("Stop", &());
        let _ = self
            .manager
            .call::<_, _, ()>("DeleteClient", &(&self.path,));
    }
}
fn locate(canceled: &AtomicBool) -> Result<Coordinate, String> {
    let connection: Connection = Builder::system()
        .map_err(|e| e.to_string())?
        .method_timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| e.to_string())?;
    let manager = Proxy::new(
        &connection,
        SERVICE,
        "/org/freedesktop/GeoClue2/Manager",
        "org.freedesktop.GeoClue2.Manager",
    )
    .map_err(|e| e.to_string())?;
    let path: OwnedObjectPath = manager.call("GetClient", &()).map_err(|e| e.to_string())?;
    let proxy: Proxy<'_> = zbus::blocking::proxy::Builder::new(&connection)
        .destination(SERVICE)
        .map_err(|e| e.to_string())?
        .path(path.clone())
        .map_err(|e| e.to_string())?
        .interface("org.freedesktop.GeoClue2.Client")
        .map_err(|e| e.to_string())?
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .map_err(|e| e.to_string())?;
    let client = Client {
        proxy,
        manager,
        path,
    };
    client
        .proxy
        .set_property("DesktopId", "org.nekosportsworld.tool")
        .map_err(|e| e.to_string())?;
    client
        .proxy
        .set_property("RequestedAccuracyLevel", 8u32)
        .map_err(|e| e.to_string())?;
    if canceled.load(Ordering::Relaxed) {
        return Err("已取消定位".into());
    }
    client
        .proxy
        .call::<_, _, ()>("Start", &())
        .map_err(|e| e.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(55);
    while !canceled.load(Ordering::Relaxed) && Instant::now() < deadline {
        // Disable property caching: polling must see newly published locations.
        let path: OwnedObjectPath = client
            .proxy
            .get_property("Location")
            .map_err(|e| e.to_string())?;
        if path.as_str() != "/" {
            let location: Proxy<'_> = zbus::blocking::proxy::Builder::new(&connection)
                .destination(SERVICE)
                .map_err(|e| e.to_string())?
                .path(path)
                .map_err(|e| e.to_string())?
                .interface("org.freedesktop.GeoClue2.Location")
                .map_err(|e| e.to_string())?
                .cache_properties(zbus::proxy::CacheProperties::No)
                .build()
                .map_err(|e| e.to_string())?;
            let timestamp: (u64, u64) = location
                .get_property("Timestamp")
                .map_err(|e| e.to_string())?;
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|e| e.to_string())?
                .as_secs();
            if now.abs_diff(timestamp.0) <= 30 {
                return Coordinate::new(
                    location
                        .get_property("Latitude")
                        .map_err(|e| e.to_string())?,
                    location
                        .get_property("Longitude")
                        .map_err(|e| e.to_string())?,
                    location
                        .get_property("Accuracy")
                        .map_err(|e| e.to_string())?,
                );
            }
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    Err(if canceled.load(Ordering::Relaxed) {
        "已取消定位"
    } else {
        "未获取到新鲜位置"
    }
    .into())
}
