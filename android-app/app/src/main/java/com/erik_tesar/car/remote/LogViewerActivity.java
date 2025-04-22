package com.erik_tesar.car.remote;

import android.graphics.Typeface;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.util.Log;
import android.view.View;
import android.widget.ScrollView;
import android.widget.TextView;

import androidx.appcompat.app.AppCompatActivity;

import java.io.BufferedReader;
import java.io.IOException;
import java.io.InputStreamReader;

public class LogViewerActivity extends AppCompatActivity {

    private final Handler mainHandler = new Handler(Looper.getMainLooper());
    private final StringBuilder logBuffer = new StringBuilder();
    ScrollView scrollView;
    TextView logTextView;
    private final Runnable updateRunnable = new Runnable() {
        private int lastLength = 0;

        @Override
        public void run() {
            if (logBuffer.length() > lastLength) {
                String newText = logBuffer.substring(lastLength);
                lastLength = logBuffer.length();

                logTextView.append(newText);
                scrollView.post(() -> scrollView.fullScroll(View.FOCUS_DOWN));
            }
            mainHandler.postDelayed(this, 1000);
        }

    };

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        ScrollView scrollView = new ScrollView(this);
        TextView logTextView = new TextView(this);
        logTextView.setTextSize(12);
        logTextView.setTypeface(Typeface.MONOSPACE);
        logTextView.setPadding(16, 16, 16, 16);
        logTextView.setTextIsSelectable(true);

        scrollView.addView(logTextView);
        setContentView(scrollView);

        // Store reference for later update
        this.logTextView = logTextView;
        this.scrollView = scrollView;

        showLogs();
    }

    @Override
    protected void onDestroy() {
        mainHandler.removeCallbacks(updateRunnable);
        super.onDestroy();
    }

    private void showLogs() {
        Log.i("LOGS", "Starting thread...");
        new Thread(() -> {
            try {
                Log.i("LOGS", "Running logcat");
                Process process = Runtime.getRuntime().exec("logcat *:D | grep car");
                BufferedReader bufferedReader = new BufferedReader(
                        new InputStreamReader(process.getInputStream()));

                mainHandler.post(updateRunnable);
                String line;
                while ((line = bufferedReader.readLine()) != null) {
                    logBuffer.append(line).append("\n");

                }

            } catch (IOException e) {
                Log.e("LOGS", e.toString());
            }
        }).start();
    }
}

