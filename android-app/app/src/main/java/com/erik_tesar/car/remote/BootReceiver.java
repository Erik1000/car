package com.erik_tesar.car.remote;

import android.content.BroadcastReceiver;
import android.content.ComponentName;
import android.content.Context;
import android.content.Intent;
import android.content.ServiceConnection;
import android.os.IBinder;
import android.util.Log;

import java.util.Objects;

public class BootReceiver extends BroadcastReceiver {
    public boolean mBound;
    public BleService mService;
    @Override
    public void onReceive(Context context, Intent intent) {
        if (Objects.requireNonNull(intent.getAction()).equalsIgnoreCase(Intent.ACTION_BOOT_COMPLETED)) {
            // Start your service here..
            Log.i("AutoStart", "BOOTED!");
            Intent serviceIntent = new Intent(context.getApplicationContext(), BleService.class);
            context.getApplicationContext().startForegroundService(serviceIntent);
        }
    }

    private ServiceConnection connection  = new ServiceConnection() {
        @Override
        public void onServiceConnected(ComponentName name, IBinder service) {
            BleService.BleBinder bleBinder = (BleService.BleBinder) service;
            mService = bleBinder.getService();
            mBound = true;
        }

        @Override
        public void onServiceDisconnected(ComponentName name) {
            mBound = false;
        }
    };
}
