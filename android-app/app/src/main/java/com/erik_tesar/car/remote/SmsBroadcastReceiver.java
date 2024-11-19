package com.erik_tesar.car.remote;

import android.content.BroadcastReceiver;
import android.content.ComponentName;
import android.content.Context;
import android.content.Intent;
import android.content.ServiceConnection;
import android.os.Bundle;
import android.os.IBinder;
import android.telephony.SmsMessage;
import android.util.Log;

import java.util.Objects;

public class SmsBroadcastReceiver extends BroadcastReceiver {
    public static final String TAG = "SMS";
    public static final String SMS_BUNDLE = "pdus";
    public static BleService bleService;
    boolean mBond = false;
    private String pendingCommand = null;
    private final ServiceConnection connection = new ServiceConnection() {
        @Override
        public void onServiceConnected(ComponentName name, IBinder service) {
            bleService = ((BleService.BleBinder) service).getService();
            mBond = true;
            Log.i(TAG, "Bound to Ble service");
            if (pendingCommand != null) {
                executeCommand(pendingCommand);
                pendingCommand = null;
            }
        }

        @Override
        public void onServiceDisconnected(ComponentName name) {
            mBond = false;
            Log.w(TAG, "Unbound from Ble service");
        }
    };

    @Override
    public void onReceive(Context context, Intent intent) {

        Bundle intentExtras = intent.getExtras();
        if (intentExtras != null) {
            Object[] sms = (Object[]) intentExtras.get(SMS_BUNDLE);
            String smsMessageStr = "";
            for (int i = 0; i < Objects.requireNonNull(sms).length; ++i) {
                String format = intentExtras.getString("format");
                SmsMessage smsMessage = SmsMessage.createFromPdu((byte[]) sms[i], format);

                String smsBody = smsMessage.getMessageBody();
                String address = smsMessage.getOriginatingAddress();

                smsMessageStr += "SMS From: " + address + "\n";
                smsMessageStr += smsBody + "\n";
                if (address.equals("+4915203088784")) {
                    String command = smsBody.toLowerCase().strip();
                    Log.i(TAG, "Got command: " + command);
                    // moves the command to be executed after the services has started.
                    pendingCommand = command;
                    Intent bleIntent = new Intent(context, BleService.class);
                    context.getApplicationContext().bindService(bleIntent, connection, Context.BIND_AUTO_CREATE);
                } else {
                    Log.w(TAG, "Received sms from wrong number: " + address);
                }
            }

        }
    }

    private void executeCommand(String command) {
        switch (command) {
            case "off":
                bleService.setValue((byte) 0);
                break;
            case "radio":
                bleService.setValue((byte) 1);
                break;
            case "engine":
                bleService.setValue((byte) 2);
                break;
            case "ignition":
                bleService.setValue((byte) 3);
                break;
            default:
                Log.e(TAG, "Got invalid sms command: " + command);
        }
    }
}
