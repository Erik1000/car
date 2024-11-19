package com.erik_tesar.car.remote;

import android.Manifest;
import android.content.Context;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.os.Build;
import android.os.Bundle;
import android.provider.Telephony;
import android.util.Log;
import android.widget.Button;
import android.widget.Toast;

import androidx.annotation.NonNull;
import androidx.appcompat.app.AppCompatActivity;
import androidx.core.content.ContextCompat;

import com.google.android.material.materialswitch.MaterialSwitch;

import java.io.File;
import java.io.FileOutputStream;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;

public class MainActivity extends AppCompatActivity {
    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);

        setContentView(R.layout.activity_main);
        checkPermission(getApplicationContext());
        Intent bleServiceIntent = new Intent(getApplicationContext(), BleService.class);
        getApplicationContext().startForegroundService(bleServiceIntent);

        Intent rustIntent = new Intent(this, RustService.class);
        startForegroundService(rustIntent);
//        ((MaterialSwitch) findViewById(R.id.serviceToggle))
//                .setOnCheckedChangeListener((buttonView, isChecked) -> {
//                    Intent serviceIntent = new Intent(this, RustService.class);
//                    if (isChecked)
//                        startForegroundService(serviceIntent);
//                    else
//                        stopService(serviceIntent);
//                });
//
//        findViewById(R.id.compose).setOnClickListener(view -> {
//            Log.i("Main", "Trigger change default sms app");
//            Intent intent =
//                    new Intent(Telephony.Sms.Intents.ACTION_CHANGE_DEFAULT);
//            intent.putExtra(Telephony.Sms.Intents.EXTRA_PACKAGE_NAME,
//                    getPackageName());
//            startActivity(intent);
//        });
//        findViewById(R.id.clearLogs).setOnClickListener(view -> {
//            try {
//                new FileOutputStream(new File(this.getFilesDir(), "fsmon_log.yaml")).close();
//                Toast.makeText(this, "Logs Cleared!", Toast.LENGTH_SHORT).show();
//            } catch (Exception e) {
//                Toast.makeText(this, "Failed to clear logs!", Toast.LENGTH_SHORT).show();
//            }
//        });
    }

    private void checkPermission(Context context) {
        List<String> mustRequest = new ArrayList<>();
        String[] requiredPermission = new String[]{
                        // bluetooth
                        Manifest.permission.BLUETOOTH,
                        Manifest.permission.BLUETOOTH_ADMIN,
                        Manifest.permission.BLUETOOTH_SCAN,
                        Manifest.permission.BLUETOOTH_ADVERTISE,
                        Manifest.permission.BLUETOOTH_CONNECT,
                        // Manifest.permission.BLUETOOTH_PRIVILEGED, not granted
                        // other
                        Manifest.permission.POST_NOTIFICATIONS,
                        Manifest.permission.FOREGROUND_SERVICE,
                        // Manifest.permission.START_FOREGROUND_SERVICES_FROM_BACKGROUND,
                        Manifest.permission.INTERNET,
                        // live location
                        Manifest.permission.ACCESS_COARSE_LOCATION,
                        Manifest.permission.ACCESS_FINE_LOCATION,
                        Manifest.permission.ACCESS_BACKGROUND_LOCATION,
                        // sms
                        Manifest.permission.SEND_SMS,
                        Manifest.permission.RECEIVE_SMS,
                        Manifest.permission.READ_SMS,
                        Manifest.permission.RECEIVE_MMS,

                        Manifest.permission.RECEIVE_BOOT_COMPLETED,
        };
        for (String permission : requiredPermission) {
            if (ContextCompat.checkSelfPermission(context, permission) != PackageManager.PERMISSION_GRANTED) {
                mustRequest.add(permission);
            } else {
                Log.i("PermissionCheck", "Got permission: " + permission);
            }
        }
        if (!mustRequest.isEmpty()) {
            requestPermissions(mustRequest.toArray(new String[0]), 1);
        }
    }

    @Override
    public void onRequestPermissionsResult(int requestCode, @NonNull String[] permissions, @NonNull int[] grantResults) {
        Log.i("PermissionRequest", "with code " + requestCode + " permission:\n" + permissions[0] + "\ngranted: " + grantResults[0]);
        super.onRequestPermissionsResult(requestCode, permissions, grantResults);
    }
}
