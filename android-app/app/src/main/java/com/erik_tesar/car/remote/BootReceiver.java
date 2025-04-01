package com.erik_tesar.car.remote;

import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;
import android.util.Log;

import java.util.Objects;

public class BootReceiver extends BroadcastReceiver {
    @Override
    public void onReceive(Context context, Intent intent) {
        if (Objects.requireNonNull(intent.getAction()).equalsIgnoreCase(Intent.ACTION_BOOT_COMPLETED)) {
            // Start your service here..
            Log.i("AutoStart", "BOOTED!");
            Intent serviceIntent = new Intent(context.getApplicationContext(), BleService.class);
            context.getApplicationContext().startForegroundService(serviceIntent);
        }
    }
}
