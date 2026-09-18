package org.nekosportsworld.tool.tests;

import android.app.Activity;
import android.app.AlertDialog;
import android.app.Instrumentation;
import android.content.Intent;
import android.os.Bundle;
import android.text.InputType;
import android.view.WindowManager;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.InputConnection;
import android.view.inputmethod.InputMethodManager;
import android.widget.EditText;
import org.nekosportsworld.tool.MainActivity;

/** Offline Android integration checks. Never logs in or submits business data. */
public final class SmokeInstrumentation extends Instrumentation {
    private Throwable failure;
    private int passed;
    private boolean verifyStoredBrand;

    @Override public void onCreate(Bundle arguments) {
        super.onCreate(arguments);
        verifyStoredBrand = arguments != null && "true".equals(arguments.getString("verifyStoredBrand"));
        start();
    }

    private void check(boolean condition, String message) {
        if (!condition) throw new AssertionError(message);
        passed++;
    }

    private void onUi(Runnable action) {
        java.util.concurrent.CountDownLatch done = new java.util.concurrent.CountDownLatch(1);
        new android.os.Handler(android.os.Looper.getMainLooper()).post(() -> {
            try { action.run(); } catch (Throwable error) { failure = error; }
            finally { done.countDown(); }
        });
        try {
            if (!done.await(15, java.util.concurrent.TimeUnit.SECONDS)) throw new AssertionError("Android main thread did not respond within 15 seconds");
        } catch (InterruptedException error) { throw new AssertionError(error); }
        if (failure != null) throw new AssertionError("UI check failed", failure);
    }

    private AlertDialog editor(MainActivity activity) {
        return dialog(activity, "editorDialog");
    }

    private AlertDialog dialog(MainActivity activity, String name) {
        try {
            java.lang.reflect.Field field = MainActivity.class.getDeclaredField(name);
            field.setAccessible(true);
            return (AlertDialog) field.get(activity);
        } catch (ReflectiveOperationException error) { throw new AssertionError(error); }
    }

    private org.json.JSONObject readIdentity(android.content.Context activity) {
        try {
            return new org.json.JSONObject(new String(java.nio.file.Files.readAllBytes(
                new java.io.File(activity.getFilesDir(), "identity.json").toPath()),
                java.nio.charset.StandardCharsets.UTF_8));
        } catch (Exception error) { throw new AssertionError(error); }
    }

    private boolean identityHas(MainActivity activity, String field, String value) {
        try { return value.equals(readIdentity(activity).optString(field)); }
        catch (AssertionError incompleteWrite) { return false; }
    }

    private void awaitUi(java.util.function.BooleanSupplier condition, String description) throws InterruptedException {
        long deadline = android.os.SystemClock.uptimeMillis() + 15000;
        java.util.concurrent.atomic.AtomicBoolean ready = new java.util.concurrent.atomic.AtomicBoolean();
        while (android.os.SystemClock.uptimeMillis() < deadline) {
            onUi(() -> ready.set(condition.getAsBoolean()));
            if (ready.get()) return;
            Thread.sleep(100);
        }
        throw new AssertionError("Timed out waiting for " + description);
    }

    @Override public void onStart() {
        Bundle report = new Bundle();
        try {
            if (verifyStoredBrand) {
                // A second instrumentation invocation starts a fresh app process.
                org.json.JSONObject before = readIdentity(getTargetContext());
                check(android.os.Build.MANUFACTURER.equals(before.getString("manufacturer")), "previous process persisted the real brand");
                Intent restart = new Intent().setClassName("org.nekosportsworld.tool", "org.nekosportsworld.tool.MainActivity");
                restart.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
                MainActivity restarted = (MainActivity) startActivitySync(restart);
                awaitUi(restarted::hasWindowFocus, "fresh process window focus");
                check(dialog(restarted, "deviceInfoDialog") == null, "fresh process must retain the consent choice");
                org.json.JSONObject after = readIdentity(restarted);
                check(before.toString().equals(after.toString()), "process restart must retain the complete saved identity");
                check(android.os.Build.MANUFACTURER.equals(after.getString("manufacturer")), "brand retained across process restart");
                report.putString("stream", "PASS: " + passed + " Android persistence restart checks\n");
                finish(Activity.RESULT_OK, report);
                return;
            }
            // The runner only targets an explicitly named emulator. Reset this fixture
            // so denial, first-run consent and UUID reuse are exercised on every run.
            getTargetContext().getSharedPreferences("device_info", 0).edit().clear().commit();
            java.nio.file.Files.deleteIfExists(new java.io.File(getTargetContext().getFilesDir(), "identity.json").toPath());
            Intent launch = new Intent().setClassName("org.nekosportsworld.tool", "org.nekosportsworld.tool.MainActivity");
            launch.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
            MainActivity activity = (MainActivity) startActivitySync(launch);
            waitForIdleSync();
            java.io.File initialIdentityFile = new java.io.File(activity.getFilesDir(), "identity.json");
            awaitUi(() -> identityHas(activity, "platform", "android"), "initial Android identity persistence");
            org.json.JSONObject initialIdentity = new org.json.JSONObject(new String(
                java.nio.file.Files.readAllBytes(initialIdentityFile.toPath()), java.nio.charset.StandardCharsets.UTF_8));
            check("android".equals(initialIdentity.getString("platform")), "fresh Android install must use Android identity");
            String originalUuid = initialIdentity.getString("device_id");
            check(java.util.UUID.fromString(originalUuid) != null, "DeviceId remains an application UUID");
            awaitUi(() -> dialog(activity, "deviceInfoDialog") != null, "first-run device information consent");
            check("Android".equals(initialIdentity.getString("device_name")), "real model must not be read into identity before consent");
            check(initialIdentity.optString("manufacturer").isEmpty(), "brand must stay empty before consent");
            onUi(() -> dialog(activity, "deviceInfoDialog").getButton(AlertDialog.BUTTON_NEGATIVE).performClick());
            awaitUi(() -> dialog(activity, "deviceInfoDialog") == null, "device information denial");
            check(readIdentity(activity).toString().equals(initialIdentity.toString()), "denial must leave identity unchanged");
            onUi(() -> {
                activity.requestDeviceInfo(true);
                check(dialog(activity, "deviceInfoDialog") == null, "first-run refusal must be remembered");
                activity.requestDeviceInfo(false);
                check(dialog(activity, "deviceInfoDialog") != null, "device page can request consent after refusal");
                dialog(activity, "deviceInfoDialog").cancel();
            });
            awaitUi(() -> dialog(activity, "deviceInfoDialog") == null, "manual request cancellation");
            check(readIdentity(activity).toString().equals(initialIdentity.toString()), "cancelling manual import must preserve identity");
            onUi(() -> {
                activity.getSharedPreferences("device_info", 0).edit().clear().commit();
                activity.requestDeviceInfo(true);
                dialog(activity, "deviceInfoDialog").getButton(AlertDialog.BUTTON_POSITIVE).performClick();
            });
            awaitUi(() -> identityHas(activity, "device_name", android.os.Build.MODEL), "accepted device information saved through Rust JNI bridge");
            org.json.JSONObject accepted = readIdentity(activity);
            check(android.os.Build.MANUFACTURER.equals(accepted.getString("manufacturer")), "real brand must be persisted after consent");
            check(android.os.Build.VERSION.RELEASE.equals(accepted.getString("os_version")), "real Android version imported");
            check(originalUuid.equals(accepted.getString("device_id")), "consenting must preserve the original UUID");
            for (String field : new String[]{"idfa", "mac_address", "app_install_time", "city", "anchor_lat", "anchor_lon"}) {
                check(accepted.get(field).equals(initialIdentity.get(field)), "import must preserve " + field);
            }
            awaitUi(() -> dialog(activity, "deviceInfoDialog") == null, "consent dialog dismissal");
            awaitUi(() -> activity.hasWindowFocus(), "native activity window focus");
            onUi(() -> {
                check(activity.getFilesDir().isDirectory(), "private storage directory");
                check((activity.getWindow().getAttributes().flags & WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON) == 0, "idle must permit screen timeout");
                activity.setTaskActive(true);
                check((activity.getWindow().getAttributes().flags & WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON) != 0, "active task must keep screen on");
                activity.setTaskActive(false);
                check((activity.getWindow().getAttributes().flags & WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON) == 0, "completed task must release screen");
                activity.openEditor(901, "before", 0);
            });
            awaitUi(() -> {
                AlertDialog dialog = editor(activity);
                if (dialog == null) return false;
                EditText input = dialog.findViewById(android.R.id.edit);
                InputMethodManager ime = (InputMethodManager) activity.getSystemService(Activity.INPUT_METHOD_SERVICE);
                return input.hasWindowFocus() && ime.isActive(input);
            }, "dialog focus and default IME connection");
            onUi(() -> {
                AlertDialog dialog = editor(activity);
                check(dialog != null && dialog.isShowing(), "native editor dialog shown");
                EditText input = dialog.findViewById(android.R.id.edit);
                check(input.hasFocus(), "native editor focused");
                InputMethodManager ime = (InputMethodManager) activity.getSystemService(Activity.INPUT_METHOD_SERVICE);
                check(ime.isActive(input), "system default IME connected");
                input.setText("");
                InputConnection connection = input.onCreateInputConnection(new EditorInfo());
                check(connection != null, "real Android input connection");
                connection.setComposingText("zhong", 1);
                connection.commitText("中文🙂abc", 1);
                check(input.getText().toString().equals("中文🙂abc"), "Chinese and emoji composition committed");
                dialog.getButton(AlertDialog.BUTTON_POSITIVE).performClick();
            });
            awaitUi(() -> editor(activity) == null, "confirmation and JNI submission");
            awaitUi(activity::hasWindowFocus, "activity focus after IME dismissal");
            onUi(() -> {
                check(editor(activity) == null, "confirmation closes editor without JNI failure");
                check((activity.getWindow().getInsetsController().getSystemBarsAppearance()
                    & android.view.WindowInsetsController.APPEARANCE_LIGHT_STATUS_BARS) != 0,
                    "status bar icons remain readable after editor dismissal");
                activity.openEditor(902, "test-password", 1);
                EditText password = editor(activity).findViewById(android.R.id.edit);
                check((password.getInputType() & InputType.TYPE_MASK_VARIATION) == InputType.TYPE_TEXT_VARIATION_PASSWORD, "password editor mode");
                check(password.getTransformationMethod() != null, "password masked");
                editor(activity).cancel();
            });
            awaitUi(() -> editor(activity) == null, "password editor cancellation");
            onUi(() -> {
                activity.openEditor(903, "1.25", 3);
                EditText decimal = editor(activity).findViewById(android.R.id.edit);
                check((decimal.getInputType() & InputType.TYPE_NUMBER_FLAG_DECIMAL) != 0, "decimal keypad mode");
                editor(activity).cancel();
            });
            awaitUi(() -> editor(activity) == null, "decimal editor cancellation");
            java.io.File identityFile = new java.io.File(activity.getFilesDir(), "identity.json");
            check(identityFile.isFile() && identityFile.length() > 0, "Rust writes identity to Android private storage");
            onUi(activity::finish);
            awaitUi(activity::isDestroyed, "Activity destruction");
            check(true, "Activity finish returns without blocking Android main thread");
            MainActivity reopened = (MainActivity) startActivitySync(launch);
            awaitUi(reopened::hasWindowFocus, "reopened activity window focus");
            check(originalUuid.equals(readIdentity(reopened).getString("device_id")), "reopening must reuse UUID");
            check(android.os.Build.MANUFACTURER.equals(readIdentity(reopened).getString("manufacturer")), "reopening must retain the saved brand");
            onUi(() -> {
                check(dialog(reopened, "deviceInfoDialog") == null, "consent must not repeat after reopening");
                check(reopened != activity, "a new Activity can start in the same process");
                reopened.openEditor(904, "reopened", 0);
                check(editor(reopened) != null, "editor usable after reopening");
                editor(reopened).cancel();
            });
            awaitUi(() -> editor(reopened) == null, "reopened editor cancellation");
            ActivityMonitor monitor = addMonitor(MainActivity.class.getName(), null, false);
            onUi(reopened::recreate);
            MainActivity recreated = (MainActivity) monitor.waitForActivityWithTimeout(15000);
            removeMonitor(monitor);
            check(recreated != null && recreated != reopened, "Activity recreation completes");
            awaitUi(recreated::hasWindowFocus, "recreated activity window focus");
            check(android.os.Build.MANUFACTURER.equals(readIdentity(recreated).getString("manufacturer")), "Activity recreation must retain the saved brand");
            onUi(() -> {
                recreated.openEditor(905, "recreated", 0);
                check(editor(recreated) != null, "editor usable after Activity recreation");
                editor(recreated).cancel();
            });
            awaitUi(() -> editor(recreated) == null, "last editor cancellation");
            org.json.JSONObject pendingIdentity = readIdentity(recreated);
            pendingIdentity.put("device_name", "Pending test import");
            java.nio.file.Files.write(new java.io.File(recreated.getFilesDir(), "identity.json").toPath(),
                pendingIdentity.toString().getBytes(java.nio.charset.StandardCharsets.UTF_8));
            onUi(() -> {
                recreated.getSharedPreferences("device_info", 0).edit().clear().commit();
                recreated.requestDeviceInfo(true);
                dialog(recreated, "deviceInfoDialog").getButton(AlertDialog.BUTTON_POSITIVE).performClick();
                recreated.finish();
            });
            awaitUi(recreated::isDestroyed, "immediate exit after consent");
            MainActivity recovered = (MainActivity) startActivitySync(launch);
            awaitUi(() -> dialog(recovered, "deviceInfoDialog") != null
                || identityHas(recovered, "device_name", android.os.Build.MODEL), "saved import or renewed consent after immediate exit");
            onUi(() -> {
                AlertDialog consent = dialog(recovered, "deviceInfoDialog");
                if (consent != null) consent.getButton(AlertDialog.BUTTON_POSITIVE).performClick();
            });
            awaitUi(() -> identityHas(recovered, "device_name", android.os.Build.MODEL), "recovered import persistence");
            check(android.os.Build.MODEL.equals(readIdentity(recovered).getString("device_name")), "immediate exit must not permanently lose accepted information");
            check(originalUuid.equals(readIdentity(recovered).getString("device_id")), "consent recovery must preserve UUID");
            check(android.os.Build.MANUFACTURER.equals(readIdentity(recovered).getString("manufacturer")), "consent recovery must preserve brand");
            awaitUi(() -> recovered.getSharedPreferences("device_info", 0).getBoolean("asked", false), "persisted consent acknowledgement");
            check(true, "consent completion is recorded after identity persistence");
            report.putString("stream", "PASS: " + passed + " Android integration checks\n");
            finish(Activity.RESULT_OK, report);
        } catch (Throwable error) {
            report.putString("stream", "FAIL after " + passed + " checks: " + android.util.Log.getStackTraceString(error));
            finish(Activity.RESULT_CANCELED, report);
        }
    }
}
