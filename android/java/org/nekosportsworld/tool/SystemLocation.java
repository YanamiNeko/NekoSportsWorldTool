package org.nekosportsworld.tool;

import android.Manifest;
import android.app.Activity;
import android.content.pm.PackageManager;
import android.location.Location;
import android.location.LocationListener;
import android.location.LocationManager;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.os.SystemClock;

/** One foreground location fix, with bounded lifetime and no background tracking. */
final class SystemLocation {
    static final int PERMISSION = 204;
    interface Callback { void finish(String json, String error); }
    private final Activity activity;
    private final Callback callback;
    private final Handler handler = new Handler(Looper.getMainLooper());
    private LocationManager manager;
    private LocationListener listener;
    private boolean pending;
    private boolean permissionPending;
    private Location best;
    private final Runnable timeout = () -> {
        if (best != null) complete(best);
        else finish("", "定位超时，请检查位置服务后重试；设备信息仍可保存");
    };

    SystemLocation(Activity activity, Callback callback) {
        this.activity = activity;
        this.callback = callback;
    }

    void request() {
        if (pending) return;
        if (permissionPending) {
            callback.finish("", "请先关闭系统位置权限弹窗，再重新读取");
            return;
        }
        pending = true;
        best = null;
        handler.postDelayed(timeout, 60000);
        if (activity.isFinishing() || activity.isDestroyed()) {
            finish("", "应用已退出，请重新读取");
        } else if (!hasPermission(Manifest.permission.ACCESS_FINE_LOCATION)
                && !hasPermission(Manifest.permission.ACCESS_COARSE_LOCATION)) {
            permissionPending = true;
            try {
                activity.requestPermissions(new String[]{Manifest.permission.ACCESS_COARSE_LOCATION,
                    Manifest.permission.ACCESS_FINE_LOCATION}, PERMISSION);
            } catch (Exception error) {
                permissionPending = false;
                finish("", "无法请求位置权限，请手动填写坐标");
            }
        } else start();
    }

    private boolean hasPermission(String permission) {
        return activity.checkSelfPermission(permission) == PackageManager.PERMISSION_GRANTED;
    }

    void permissionsResult() {
        permissionPending = false;
        if (!pending) return;
        if (hasPermission(Manifest.permission.ACCESS_FINE_LOCATION)
                || hasPermission(Manifest.permission.ACCESS_COARSE_LOCATION)) start();
        else finish("", "位置权限被拒绝；设备信息仍可保存，坐标可手动填写");
    }

    private void start() {
        try {
            manager = (LocationManager) activity.getSystemService(Activity.LOCATION_SERVICE);
            listener = new LocationListener() {
                @Override public void onLocationChanged(Location location) {
                    if (!pending) return;
                    long age = SystemClock.elapsedRealtimeNanos() - location.getElapsedRealtimeNanos();
                    if (age < 0 || age > 60000000000L || !location.hasAccuracy()
                            || !Float.isFinite(location.getAccuracy()) || location.getAccuracy() < 0
                            || !Double.isFinite(location.getLatitude()) || Math.abs(location.getLatitude()) > 90
                            || !Double.isFinite(location.getLongitude()) || Math.abs(location.getLongitude()) > 180) return;
                    if (best == null || location.getAccuracy() < best.getAccuracy()) best = new Location(location);
                    // Give GPS a chance to improve a coarse network fix before the timeout.
                    if (location.getAccuracy() <= 50 || !hasPermission(Manifest.permission.ACCESS_FINE_LOCATION)) complete(best);
                }
                @Override public void onProviderDisabled(String provider) { }
                @Override public void onProviderEnabled(String provider) { }
                @Override public void onStatusChanged(String provider, int status, Bundle extras) { }
            };
            boolean requested = false;
            for (String provider : new String[]{LocationManager.NETWORK_PROVIDER, LocationManager.GPS_PROVIDER}) {
                if (provider.equals(LocationManager.GPS_PROVIDER) && !hasPermission(Manifest.permission.ACCESS_FINE_LOCATION)) continue;
                if (manager != null && manager.isProviderEnabled(provider)) {
                    manager.requestLocationUpdates(provider, 0, 0, listener, Looper.getMainLooper());
                    requested = true;
                }
            }
            if (!requested) finish("", "位置服务未开启或没有可用定位提供者；请手动填写坐标");
        } catch (Exception error) { finish("", "无法获取系统定位，请检查位置服务和权限"); }
    }

    private void complete(Location location) {
        try {
            org.json.JSONObject json = new org.json.JSONObject();
            json.put("latitude", location.getLatitude());
            json.put("longitude", location.getLongitude());
            json.put("accuracy", location.getAccuracy());
            finish(json.toString(), "");
        } catch (Exception error) { finish("", "系统定位返回无效数据，请手动填写"); }
    }

    void pause() { if (!permissionPending) cancel(); }
    void cancel() { finish("", "定位已取消，可再次读取本机信息或手动填写坐标"); }
    private void finish(String json, String error) {
        if (!pending) return;
        pending = false;
        handler.removeCallbacks(timeout);
        if (manager != null && listener != null) {
            try { manager.removeUpdates(listener); } catch (Exception ignored) { }
        }
        listener = null;
        best = null;
        callback.finish(json, error);
    }
}
