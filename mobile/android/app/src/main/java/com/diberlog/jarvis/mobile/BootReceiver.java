package com.diberlog.jarvis.mobile;
import android.content.*;
public final class BootReceiver extends BroadcastReceiver { @Override public void onReceive(Context c,Intent i){if(Intent.ACTION_BOOT_COMPLETED.equals(i.getAction()))ReminderReceiver.schedule(c);} }
