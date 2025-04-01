package com.erik_tesar.car.remote;

import android.annotation.SuppressLint;
import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.Service;
import android.bluetooth.BluetoothAdapter;
import android.bluetooth.BluetoothDevice;
import android.bluetooth.BluetoothGatt;
import android.bluetooth.BluetoothGattCallback;
import android.bluetooth.BluetoothGattCharacteristic;
import android.bluetooth.le.BluetoothLeScanner;
import android.bluetooth.le.ScanCallback;
import android.bluetooth.le.ScanRecord;
import android.bluetooth.le.ScanResult;
import android.content.Intent;
import android.os.Binder;
import android.os.Handler;
import android.os.IBinder;
import android.os.ParcelUuid;
import android.util.Log;

import androidx.annotation.NonNull;
import androidx.annotation.Nullable;

import java.util.UUID;

// permission ensured at start up
@SuppressLint("MissingPermission")
public class BleService extends Service {
    public static final String TAG = "BLE";
    public static final ParcelUuid CAR_STARTER_SERVICE = new ParcelUuid(UUID.fromString("0e353531-5159-42a0-92ff-38e9e49ab7d1"));
    public static final UUID CAR_STARTER_STATE_CHARACTERISTIC = UUID.fromString("13d24b59-3d13-4ef7-98db-e174869078e0");
    private static final String CHANNEL_ID = "BleServiceChannel";
    private static final int NOTIFICATION_ID = 2;

    private BluetoothGatt bluetoothGatt;

    public BluetoothDevice car_starter = null;

    private final BluetoothLeScanner bluetoothLeScanner = BluetoothAdapter.getDefaultAdapter().getBluetoothLeScanner();

    private final IBinder binder = new BleBinder();


    private final ScanCallback leScanCallback = new ScanCallback() {
        @Override
        public void onScanResult(int callbackType, ScanResult result) {
            BluetoothDevice device = result.getDevice();
            ScanRecord record = result.getScanRecord();
            if (record != null) {
                if (record.getServiceUuids() != null && record.getServiceUuids().contains(CAR_STARTER_SERVICE)) {
                    if (car_starter != null && !car_starter.getAddress().equals( device.getAddress())) {
                        Log.w(TAG, "Found another device advertising car service: " + device.getName() + " " + device.getAddress());
                    }
                    Log.i(TAG, "Set Bluetooth Device to " + device.getName() + " " + device.getAddress());
                    car_starter = device;
                    bluetoothLeScanner.stopScan(this);
                    bluetoothGatt = car_starter.connectGatt(BleService.this, false, gattCallback);
                }
            }
        }
    };



    private final BluetoothGattCallback gattCallback = new BluetoothGattCallback() {
        @Override
        public void onConnectionStateChange(BluetoothGatt gatt, int status, int newState) {
            if (newState == BluetoothGatt.STATE_CONNECTED) {
                Log.i(TAG, "Connected to GATT server. Attempting to start service discovery: " + gatt.discoverServices());
            } else if (newState == BluetoothGatt.STATE_DISCONNECTED) {
                Log.w(TAG, "Disconnected from GATT server.");
                reconnectToDeviceWithDelay();
            }
        }

        private final Handler reconnectHandler = new Handler();

        private void reconnectToDeviceWithDelay() {
            // Retry every 5 seconds
            int RECONNECT_DELAY_MS = 5000;
            reconnectHandler.postDelayed(() -> {
                if (car_starter != null) {
                    Log.i(TAG, "Retrying connection...");
                    bluetoothGatt = car_starter.connectGatt(BleService.this, false, gattCallback);
                }
            }, RECONNECT_DELAY_MS);
        }

        @Override
        public void onServicesDiscovered(BluetoothGatt gatt, int status) {
            if (status == BluetoothGatt.GATT_SUCCESS) {
                Log.i(TAG, "Services discovered: " + gatt.getServices());
                // Perform further actions like writing to a characteristic if needed
            } else {
                Log.w(TAG, "Service discovery failed with status: " + status);
            }
        }

        @Override
        public void onCharacteristicWrite(BluetoothGatt gatt, BluetoothGattCharacteristic characteristic, int status) {
            if (status == BluetoothGatt.GATT_SUCCESS) {
                Log.i(TAG, "Characteristic write successful: " + characteristic.getUuid());
            } else {
                Log.e(TAG, "Characteristic write failed with status: " + status);
            }
        }
    };
    @Nullable
    @Override
    public IBinder onBind(Intent intent) {
        return binder;
    }

    public void setValue(Byte value) {
        if (car_starter != null) {
            Log.i(TAG, "Sending value to car starter: " + value);
            byte[] a = {value};
            BluetoothGattCharacteristic c = bluetoothGatt.getService(CAR_STARTER_SERVICE.getUuid()).getCharacteristic(CAR_STARTER_STATE_CHARACTERISTIC);
            c.setValue(a);
            bluetoothGatt.writeCharacteristic(c);

        } else {
            Log.e(TAG, "Received start command but car starter is not found");
        }
    }

    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        Log.i(TAG, "Starting BLE service");
        createNotificationChannel();
        startForeground(NOTIFICATION_ID, buildNotification());
        scanLeDevice();

        return START_STICKY;
    }

    @Override
    public void onDestroy() {
        Log.w(TAG, "BLE Service getting destroyed!");
        super.onDestroy();
    }

    private void scanLeDevice() {
        Log.i(TAG, "Enable scanner");
        bluetoothLeScanner.startScan(leScanCallback);
    }
    private void createNotificationChannel() {
        NotificationChannel channel = new NotificationChannel(CHANNEL_ID,
                "Ble Service Channel", NotificationManager.IMPORTANCE_HIGH);

        NotificationManager manager = getSystemService(NotificationManager.class);
        manager.createNotificationChannel(channel);
    }

    private Notification buildNotification() {
        Notification.Builder builder;
        builder = new Notification.Builder(this, CHANNEL_ID);

        builder.setContentTitle("Ble Service").setContentText("Running...")
                .setSmallIcon(R.mipmap.ic_launcher).setOngoing(true);

        return builder.build();
    }

    public class BleBinder extends Binder {
        @NonNull
        BleService getService() {
            return BleService.this;
        }
    }
}
