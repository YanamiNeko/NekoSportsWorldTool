package org.nekosportsworld.tool.tests;

import android.app.AlertDialog;
import android.app.Instrumentation;
import android.content.Intent;
import android.location.Location;
import android.location.LocationListener;
import android.os.Bundle;
import android.os.SystemClock;
import org.nekosportsworld.tool.MainActivity;

/** Emulator checks: no login, business calls, or location persistence. */
public final class LocationInstrumentation extends Instrumentation {
    private int passed;
    private Throwable failure;
    private boolean denyPermission;
    @Override public void onCreate(Bundle arguments) {
        super.onCreate(arguments);
        denyPermission = arguments != null && "true".equals(arguments.getString("denyPermission"));
        start();
    }
    private void check(boolean condition, String message) {
        if (!condition) throw new AssertionError(message);
        passed++;
    }
    private Object field(Object object, String name) throws Exception {
        java.lang.reflect.Field field = object.getClass().getDeclaredField(name);
        field.setAccessible(true);
        return field.get(object);
    }
    private void call(Object object, String name) throws Exception {
        java.lang.reflect.Method method = object.getClass().getDeclaredMethod(name);
        method.setAccessible(true);
        method.invoke(object);
    }
    private void shell(String command) throws Exception {
        try (android.os.ParcelFileDescriptor descriptor = getUiAutomation().executeShellCommand(command);
             java.io.InputStream stream = new java.io.FileInputStream(descriptor.getFileDescriptor())) {
            while (stream.read() != -1) { }
        }
    }
    private void onUi(Runnable action) {
        runOnMainSync(() -> { try { action.run(); } catch (Throwable error) { failure = error; } });
        if (failure != null) throw new AssertionError("UI check failed", failure);
    }
    private Location fix(float accuracy, long age) {
        Location location = new Location("gps");
        location.setLatitude(39.900321);
        location.setLongitude(116.400654);
        location.setAccuracy(accuracy);
        location.setElapsedRealtimeNanos(SystemClock.elapsedRealtimeNanos() - age);
        return location;
    }

    private android.view.accessibility.AccessibilityNodeInfo denyButton(android.view.accessibility.AccessibilityNodeInfo node) {
        if (node == null) return null;
        String id = node.getViewIdResourceName();
        if (id != null && id.contains("permission_deny_button")) return node;
        for (int index = 0; index < node.getChildCount(); index++) {
            android.view.accessibility.AccessibilityNodeInfo found = denyButton(node.getChild(index));
            if (found != null) return found;
        }
        return null;
    }

    private void checkPermissionDenial(MainActivity activity) {
        onUi(() -> {
            try {
                AlertDialog initial = (AlertDialog) field(activity, "deviceInfoDialog");
                if (initial != null) initial.getButton(AlertDialog.BUTTON_NEGATIVE).performClick();
            } catch (Exception error) { throw new RuntimeException(error); }
        });
        onUi(() -> {
            try {
                activity.requestDeviceInfo(false);
                ((AlertDialog) field(activity, "deviceInfoDialog")).getButton(AlertDialog.BUTTON_POSITIVE).performClick();
            } catch (Exception error) { throw new RuntimeException(error); }
        });
        android.view.accessibility.AccessibilityNodeInfo deny = null;
        for (int attempt = 0; attempt < 30 && deny == null; attempt++) {
            SystemClock.sleep(100);
            deny = denyButton(getUiAutomation().getRootInActiveWindow());
        }
        check(deny != null, "combined read must show the system location permission dialog");
        check(deny.performAction(android.view.accessibility.AccessibilityNodeInfo.ACTION_CLICK), "permission denial button must be actionable");
        SystemClock.sleep(500);
        onUi(() -> {
            try {
                Object helper = field(activity, "systemLocation");
                check(!(Boolean) field(helper, "pending"), "denied request must complete");
                check(field(helper, "listener") == null, "denied request must not start tracking");
            } catch (Exception error) { throw new RuntimeException(error); }
        });
    }
    @Override public void onStart() {
        Bundle result = new Bundle();
        try {
            if (!denyPermission) {
                shell("pm grant org.nekosportsworld.tool android.permission.ACCESS_COARSE_LOCATION");
                shell("pm grant org.nekosportsworld.tool android.permission.ACCESS_FINE_LOCATION");
            }
            Intent intent = new Intent(getTargetContext(), MainActivity.class).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
            MainActivity activity = (MainActivity) startActivitySync(intent);
            SystemClock.sleep(800);
            if (denyPermission) {
                checkPermissionDenial(activity);
                result.putString("stream", "PASS: " + passed + " Android location denial checks");
                finish(android.app.Activity.RESULT_OK, result);
                return;
            }
            onUi(() -> {
                try {
                    AlertDialog initial = (AlertDialog) field(activity, "deviceInfoDialog");
                    if (initial != null) initial.getButton(AlertDialog.BUTTON_NEGATIVE).performClick();
                    Class<?> type = Class.forName("org.nekosportsworld.tool.SystemLocation", true, activity.getClassLoader());
                    Class<?> callbackType = Class.forName("org.nekosportsworld.tool.SystemLocation$Callback", true, activity.getClassLoader());
                    String[] received = new String[2];
                    Object callback = java.lang.reflect.Proxy.newProxyInstance(activity.getClassLoader(), new Class<?>[]{callbackType}, (proxy, method, values) -> {
                        received[0] = (String) values[0]; received[1] = (String) values[1]; return null;
                    });
                    java.lang.reflect.Constructor<?> constructor = type.getDeclaredConstructor(android.app.Activity.class, callbackType);
                    constructor.setAccessible(true);
                    Object helper = constructor.newInstance(activity, callback);
                    call(helper, "request");
                    LocationListener listener = (LocationListener) field(helper, "listener");
                    check(listener != null, "enabled emulator provider must register a listener");
                    listener.onLocationChanged(fix(5, 120000000000L));
                    check(received[0] == null, "stale fixes must be rejected");
                    listener.onLocationChanged(fix(5, 0));
                    org.json.JSONObject json = new org.json.JSONObject(received[0]);
                    check(json.getDouble("latitude") == 39.900321 && json.getDouble("longitude") == 116.400654, "fresh coordinate must be returned exactly");
                    check(json.getDouble("accuracy") == 5, "accuracy must be retained");
                    check(!(Boolean) field(helper, "pending") && field(helper, "listener") == null, "successful fix must release updates");
                    received[0] = null;
                    call(helper, "request");
                    ((LocationListener) field(helper, "listener")).onLocationChanged(fix(500, 0));
                    check(received[0] == null, "coarse fix must allow time for GPS improvement");
                    ((Runnable) field(helper, "timeout")).run();
                    check(new org.json.JSONObject(received[0]).getDouble("accuracy") == 500, "timeout may return the available coarse fix");
                    call(helper, "request");
                    call(helper, "pause");
                    check(received[1].contains("取消") && field(helper, "listener") == null, "backgrounding must cancel updates");
                    // Exercise the real confirmation -> native device -> native location bridge.
                    activity.requestDeviceInfo(false);
                    AlertDialog dialog = (AlertDialog) field(activity, "deviceInfoDialog");
                    check(dialog != null, "manual device read must ask for confirmation");
                    dialog.getButton(AlertDialog.BUTTON_POSITIVE).performClick();
                } catch (Exception error) { throw new RuntimeException(error); }
            });
            // AlertDialog dispatches its positive listener through the UI message queue.
            onUi(() -> {
                try {
                    Object nativeHelper = field(activity, "systemLocation");
                    LocationListener listener = (LocationListener) field(nativeHelper, "listener");
                    check(listener != null, "confirmed read must also start location acquisition");
                    listener.onLocationChanged(fix(5, 0));
                    check(!(Boolean) field(nativeHelper, "pending"), "native callback must complete");
                } catch (Exception error) { throw new RuntimeException(error); }
            });
            SystemClock.sleep(500);
            // Coordinate acquisition must not overwrite the saved identity.
            java.io.File identity = new java.io.File(activity.getFilesDir(), "identity.json");
            if (identity.exists()) {
                org.json.JSONObject saved = new org.json.JSONObject(new String(java.nio.file.Files.readAllBytes(identity.toPath()), java.nio.charset.StandardCharsets.UTF_8));
                check(saved.getDouble("anchor_lat") != 39.900321, "fresh location must wait for manual Save");
            }
            result.putString("stream", "PASS: " + passed + " Android location checks");
            finish(android.app.Activity.RESULT_OK, result);
        } catch (Throwable error) {
            result.putString("stream", "FAIL: " + android.util.Log.getStackTraceString(error));
            finish(android.app.Activity.RESULT_CANCELED, result);
        }
    }
}
