package com.erik_tesar.car.remote;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.Service;
import android.content.Intent;
import android.os.IBinder;
import android.util.Log;


public class RustService extends Service {
    private static final String CHANNEL_ID = "RustServiceChannel";
    private static final int NOTIFICATION_ID = 1;

    public static boolean isRunning = false;
    static {
        System.loadLibrary("car_remote");
    }

    private native void startService();

   @Override
    public void onCreate() {
        super.onCreate();
        isRunning = true;
    }

    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        super.onStartCommand(intent, flags, startId);

        createNotificationChannel();
        startForeground(NOTIFICATION_ID, buildNotification());

        Thread tokio = new Thread(this::startService);
        tokio.start();

        Log.i("Rust", "Service started!");
        return START_STICKY;
    }

    @Override
    public void onDestroy() {
        super.onDestroy();
        isRunning = false;
    }

    @Override
    public IBinder onBind(Intent intent) {
        return null;
    }

    private void createNotificationChannel() {
        NotificationChannel channel = new NotificationChannel(CHANNEL_ID,
                "Rust Service Channel", NotificationManager.IMPORTANCE_HIGH);

        NotificationManager manager = getSystemService(NotificationManager.class);
        manager.createNotificationChannel(channel);
    }

    private Notification buildNotification() {
        Notification.Builder builder;
        builder = new Notification.Builder(this, CHANNEL_ID);

        builder.setContentTitle("Rust Service").setContentText("Running...")
                .setSmallIcon(R.mipmap.ic_launcher).setOngoing(true);

        return builder.build();
    }
}
