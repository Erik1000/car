package com.erik_tesar.car.remote;

import android.Manifest;
import android.content.Context;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.os.Bundle;
import android.util.Log;
import android.view.View;
import android.widget.ImageView;

import androidx.annotation.NonNull;
import androidx.appcompat.app.AppCompatActivity;

import java.util.ArrayList;
import java.util.List;

public class MainActivity extends AppCompatActivity {
    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);


        setContentView(R.layout.activity_main);
        if (checkPermission(getApplicationContext())) {
            if (!RustService.isRunning) {
                Intent rustIntent = new Intent(this, RustService.class);
                startForegroundService(rustIntent);
            }
        }

        ImageView img = findViewById(R.id.autoButton);
        img.setOnClickListener(v -> {
            Intent intent = new Intent(v.getContext(), LogViewerActivity.class);
            startActivity(intent);
        });
   }

    private boolean checkPermission(Context context) {
        List<String> mustRequest = new ArrayList<>();
        String[] requiredPermission = new String[]{
                Manifest.permission.READ_CONTACTS,
                // live location
                Manifest.permission.ACCESS_FINE_LOCATION,
                Manifest.permission.ACCESS_COARSE_LOCATION,
                Manifest.permission.ACCESS_BACKGROUND_LOCATION,
                // bluetooth
                Manifest.permission.BLUETOOTH,
                Manifest.permission.BLUETOOTH_ADMIN,
                Manifest.permission.BLUETOOTH_SCAN,
                Manifest.permission.BLUETOOTH_ADVERTISE,
                Manifest.permission.BLUETOOTH_CONNECT,
                // Manifest.permission.BLUETOOTH_PRIVILEGED, // not granted
                // other
                Manifest.permission.POST_NOTIFICATIONS,
                Manifest.permission.FOREGROUND_SERVICE,
                // Manifest.permission.START_FOREGROUND_SERVICES_FROM_BACKGROUND, not granted
                Manifest.permission.INTERNET,
                // sms
                Manifest.permission.SEND_SMS,
                Manifest.permission.RECEIVE_SMS,
                Manifest.permission.READ_SMS,
                Manifest.permission.RECEIVE_MMS,

                Manifest.permission.RECEIVE_BOOT_COMPLETED,
        };
        for (String permission : requiredPermission) {
            if (checkSelfPermission(permission) == PackageManager.PERMISSION_GRANTED) {
                Log.i("PermissionCheck", "Got permission: " + permission);
            } else {
                Log.i("PermissionCheck", "Must request: " + permission);
                mustRequest.add(permission);
            }
        }
        if (!mustRequest.isEmpty()) {
            Log.i("PermissionCheck", "Requesting permission: " + mustRequest);
            requestPermissions(mustRequest.toArray(new String[0]), 1);
            return false;
        } else {
            return true;
        }
    }

    @Override
    public void onRequestPermissionsResult(int requestCode, @NonNull String[] permissions, @NonNull int[] grantResults) {
        Log.i("PermissionRequest", "with code " + requestCode + " permission:\n" + permissions[0] + "\ngranted: " + grantResults[0]);
        super.onRequestPermissionsResult(requestCode, permissions, grantResults);
    }
}
