package com.erik_tesar.car.remote;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.Service;
import android.content.ContentResolver;
import android.content.Context;
import android.content.Intent;
import android.database.Cursor;
import android.os.IBinder;
import android.provider.ContactsContract;
import android.util.Log;

import java.util.ArrayList;
import java.util.List;


public class RustService extends Service {
    private static final String CHANNEL_ID = "RustServiceChannel";
    private static final int NOTIFICATION_ID = 1;

    public static boolean isRunning = false;
    static {
        System.loadLibrary("car_remote");
    }

    private native void startService();
    private native void provideAuthorizedPhoneNumbers(List<String> numbers);

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

        provideAuthorizedPhoneNumbers(getPhoneNumbersByName("Admin"));

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

    public List<String> getPhoneNumbersByName(String targetName) {
        Context context = getApplicationContext();
        List<String> phoneNumbers = new ArrayList<>();

        ContentResolver contentResolver = context.getContentResolver();
        Cursor cursor = contentResolver.query(
                ContactsContract.Contacts.CONTENT_URI,
                null,
                null,
                null,
                null
        );

        if (cursor != null && cursor.getCount() > 0) {
            while (cursor.moveToNext()) {
                String contactId = cursor.getString(cursor.getColumnIndexOrThrow(ContactsContract.Contacts._ID));
                String name = cursor.getString(cursor.getColumnIndexOrThrow(ContactsContract.Contacts.DISPLAY_NAME));

                if (name != null && name.equalsIgnoreCase(targetName)) {
                    int hasPhoneNumber = cursor.getInt(cursor.getColumnIndexOrThrow(ContactsContract.Contacts.HAS_PHONE_NUMBER));
                    if (hasPhoneNumber > 0) {
                        Cursor phoneCursor = contentResolver.query(
                                ContactsContract.CommonDataKinds.Phone.CONTENT_URI,
                                null,
                                ContactsContract.CommonDataKinds.Phone.CONTACT_ID + " = ?",
                                new String[]{contactId},
                                null
                        );

                        if (phoneCursor != null) {
                            while (phoneCursor.moveToNext()) {
                                String phoneNumber = phoneCursor.getString(
                                        phoneCursor.getColumnIndexOrThrow(ContactsContract.CommonDataKinds.Phone.NUMBER));
                                phoneNumbers.add(phoneNumber);
                            }
                            phoneCursor.close();
                        }
                    }
                }
            }
            cursor.close();
        }

        return phoneNumbers;
    }


}
