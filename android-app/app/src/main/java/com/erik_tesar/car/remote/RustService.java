package com.erik_tesar.car.remote;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.Service;
import android.content.Intent;
import android.os.Build;
import android.os.IBinder;

import com.erik_tesar.car.remote.R;

public class RustService extends Service {
    private static final String CHANNEL_ID = "RustServiceChannel";
    private static final int NOTIFICATION_ID = 1;

    static {
        System.loadLibrary("car_remote");
    }

    private static native void startService(String filesDir);

    @Override
    public void onCreate() {
        super.onCreate();
    }

    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        super.onStartCommand(intent, flags, startId);

        createNotificationChannel();
        startForeground(NOTIFICATION_ID, buildNotification());

        startService(this.getFilesDir().toString());

        return START_STICKY;
    }

    @Override
    public void onDestroy() {
        super.onDestroy();
    }

    @Override
    public IBinder onBind(Intent intent) {
        return null;
    }

    private void createNotificationChannel() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            NotificationChannel channel = new NotificationChannel(CHANNEL_ID,
                    "Rust Service Channel", NotificationManager.IMPORTANCE_DEFAULT);

            NotificationManager manager = getSystemService(NotificationManager.class);
            manager.createNotificationChannel(channel);
        }
    }

    private Notification buildNotification() {
        Notification.Builder builder;
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            builder = new Notification.Builder(this, CHANNEL_ID);
        } else {
            builder = new Notification.Builder(this);
        }

        builder.setContentTitle("Rust Service").setContentText("Running...")
                .setSmallIcon(R.mipmap.ic_launcher).setPriority(Notification.PRIORITY_DEFAULT);

        return builder.build();
    }
}
