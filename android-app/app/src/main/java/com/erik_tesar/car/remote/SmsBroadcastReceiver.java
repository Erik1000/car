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

public class SmsBroadcastReceiver extends BroadcastReceiver {
    public static final String TAG = "SMS";
    public static final String SMS_BUNDLE = "pdus";
    private final ServiceConnection connection = new ServiceConnection() {
        @Override
        public void onServiceConnected(ComponentName name, IBinder service) {
        }

        @Override
        public void onServiceDisconnected(ComponentName name) {
        }
    };

    private native void recvSms(String number, String sms_text);

    @Override
    public void onReceive(Context context, Intent intent) {
        Log.i("SMS", "Received sms");
        Bundle bundle = intent.getExtras();
        StringBuilder fullMessage = new StringBuilder();
        String senderNumber = null;
        if (bundle != null) {
            Object[] pdus = (Object[]) bundle.get("pdus");
            if (pdus != null) {
                for (Object pdu : pdus) {
                    SmsMessage smsMessage = SmsMessage.createFromPdu((byte[]) pdu);
                    if (senderNumber == null) {
                        senderNumber = smsMessage.getDisplayOriginatingAddress();
                    }
                    fullMessage.append(smsMessage.getMessageBody());

                }
            }
            Log.i("SMS", "Got sms: " + fullMessage.toString());
            recvSms(senderNumber, fullMessage.toString());
        }
    }
}
