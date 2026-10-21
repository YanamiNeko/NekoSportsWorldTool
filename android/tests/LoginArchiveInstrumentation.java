package org.nekosportsworld.tool.tests;

import android.app.Activity;
import android.app.Instrumentation;
import android.content.Intent;
import android.os.Bundle;
import org.nekosportsworld.tool.MainActivity;

/** Offline checks of SAF intents and cancellation through the actual JNI host. */
public final class LoginArchiveInstrumentation extends Instrumentation {
    private int passed;
    @Override public void onCreate(Bundle arguments) { super.onCreate(arguments); start(); }
    private void check(boolean ok, String message) {
        if (!ok) throw new AssertionError(message);
        passed++;
    }
    @Override public void onStart() {
        Bundle report = new Bundle();
        try {
            getTargetContext().getSharedPreferences("device_info", 0).edit().putBoolean("asked", true).commit();
            Intent launch = new Intent().setClassName("org.nekosportsworld.tool", "org.nekosportsworld.tool.MainActivity");
            launch.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
            MainActivity activity = (MainActivity) startActivitySync(launch);
            check(activity != null, "native activity launched");
            for (boolean exporting : new boolean[] { true, false }) {
                java.util.concurrent.CountDownLatch requested = new java.util.concurrent.CountDownLatch(1);
                java.util.concurrent.atomic.AtomicReference<Intent> captured = new java.util.concurrent.atomic.AtomicReference<>();
                ActivityMonitor monitor = new ActivityMonitor() {
                    @Override public ActivityResult onStartActivity(Intent intent) {
                        captured.set(intent);
                        requested.countDown();
                        return new ActivityResult(Activity.RESULT_CANCELED, null);
                    }
                };
                addMonitor(monitor);
                activity.loginArchivePicker(exporting, new byte[] { 80, 75, 3, 4 });
                check(requested.await(10, java.util.concurrent.TimeUnit.SECONDS), "picker dispatched");
                waitForIdleSync();
                removeMonitor(monitor);
                Intent intent = captured.get();
                check((exporting ? Intent.ACTION_CREATE_DOCUMENT : Intent.ACTION_OPEN_DOCUMENT).equals(intent.getAction()), "correct SAF action");
                check(intent.hasCategory(Intent.CATEGORY_OPENABLE), "openable documents only");
                check((exporting ? "application/zip" : "*/*").equals(intent.getType()), "correct MIME type");
                if (exporting) check("neko-login.zip".equals(intent.getStringExtra(Intent.EXTRA_TITLE)), "ZIP filename");
                java.lang.reflect.Field field = MainActivity.class.getDeclaredField("loginExport");
                field.setAccessible(true);
                long deadline = android.os.SystemClock.uptimeMillis() + 5000;
                while (field.get(activity) != null && android.os.SystemClock.uptimeMillis() < deadline) Thread.sleep(50);
                check(field.get(activity) == null, "cancel releases sensitive export bytes");
            }
            // Also open the actual system picker; leave it visible for screenshot QA.
            activity.loginArchivePicker(false, new byte[0]);
            Thread.sleep(1500);
            check(!activity.hasWindowFocus(), "system picker takes focus");
            report.putString("stream", "PASS: " + passed + " Android login archive checks\n");
            finish(Activity.RESULT_OK, report);
        } catch (Throwable error) {
            report.putString("stream", "FAIL: " + android.util.Log.getStackTraceString(error));
            finish(Activity.RESULT_CANCELED, report);
        }
    }
}
